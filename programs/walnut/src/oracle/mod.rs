//! Pyth price validation, conservative pricing, and oracle anomaly controls
//! (Tasks 6.1 and 6.2).
//!
//! Every check is a plain function over plain values so it runs in host tests,
//! mirroring [`crate::market::validation`]. The Anchor layer of a
//! price-sensitive instruction builds a [`PythPriceView`] from the loaded
//! `PriceUpdateV2` account and calls [`validate_price`] before it captures the
//! observation into an Operation Intent; the circuit and callback then consume
//! the conservative normalized value.
//!
//! # What the design fixes (design "Oracle design", Requirement 6)
//!
//! * The price account must be owned by the Pyth receiver program and carry the
//!   configured feed id ([`check_receiver_and_feed`]).
//! * Prices must be fresh, positive, within a confidence band, and have a
//!   representable exponent ([`validate_price`]).
//! * Collateral uses `price - confidence`; debt uses `price + confidence`
//!   ([`PriceUse`]), each normalized to the single [`crate::math::USD_SCALE`]
//!   with conservative rounding.
//! * A spot-versus-EMA breaker and a spot-versus-last-good breaker guard against
//!   a fresh-but-wrong print ([`deviation_within`]).
//! * Entering anomaly mode increments the oracle generation
//!   ([`crate::market::AssetState::oracle_generation`]); every pending
//!   price-sensitive callback captured the prior generation and so finalizes
//!   `Stale` through [`crate::operation::callback::verify_dependencies`]. New
//!   price-sensitive operations are blocked until both feeds pass every check in
//!   a fresh observation ([`OracleStatus`], [`ensure_price_sensitive_allowed`]).
//!
//! The on-chain feed ids, parameters, status, and last-good observation are
//! persisted and the generation bumped by the first price-sensitive instruction
//! (Task 9); this module is the validated, tested core those instructions call.

use anchor_lang::prelude::{AnchorDeserialize, AnchorSerialize, InitSpace, Pubkey};

use crate::math::{
    pow10, BPS_DENOMINATOR, MAX_NORMALIZED_PRICE, MAX_PYTH_EXPONENT, MIN_NORMALIZED_PRICE,
    MIN_PYTH_EXPONENT, USD_SCALE,
};
use crate::operation::OperationKind;

/// The Pyth Solana receiver program id (identical on devnet and mainnet). A
/// `PriceUpdateV2` account used by Walnut must be owned by this program.
pub const PYTH_RECEIVER_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ");

/// Minimum accepted price age bound (Requirement 6.2: 5 to 300 seconds).
pub const MIN_ORACLE_MAX_AGE_SECONDS: u32 = 5;
/// Maximum accepted price age bound.
pub const MAX_ORACLE_MAX_AGE_SECONDS: u32 = 300;
/// Minimum accepted confidence bound (Requirement 6.3: 0.1% to 10%).
pub const MIN_ORACLE_MAX_CONFIDENCE_BPS: u16 = 10;
/// Maximum accepted confidence bound.
pub const MAX_ORACLE_MAX_CONFIDENCE_BPS: u16 = 1_000;
/// Minimum accepted spot-versus-EMA / last-good deviation bound.
pub const MIN_DEVIATION_BPS: u16 = 50;
/// Maximum accepted deviation bound.
pub const MAX_DEVIATION_BPS: u16 = 5_000;
/// Launch default deviation bound (5%).
pub const DEFAULT_DEVIATION_BPS: u16 = 500;

/// Errors of the oracle checks. Mapped onto appended program `ErrorCode`s in
/// [`crate::errors`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OracleError {
    /// The price account is not owned by the Pyth receiver program.
    WrongReceiverProgram,
    /// The price account carries a different feed id than configured.
    FeedMismatch,
    /// The price is older than the configured maximum age.
    StalePrice,
    /// The price (or a conservative price) is not strictly positive.
    NonPositivePrice,
    /// The confidence interval is wider than the configured share of price.
    ConfidenceTooWide,
    /// The price exponent is outside the representable window.
    ExponentOutOfRange,
    /// The normalized price is outside the committed price domain.
    PriceOutOfRange,
    /// The spot price deviates from the EMA or last-good price by too much.
    DeviationTooHigh,
    /// An oracle parameter is outside its bounds.
    InvalidOracleParams,
    /// A publish time or the reference time is negative.
    NegativeTimestamp,
    /// The oracle is in anomaly mode, so the operation is blocked.
    OracleInAnomaly,
    /// A normalization product would overflow `u128`.
    ArithmeticOverflow,
}

/// Oracle tolerances. `max_age_seconds` and `max_confidence_bps` come from the
/// governed [`crate::market::RiskParams`]; `max_deviation_bps` bounds the
/// spot-versus-EMA and spot-versus-last-good circuit breakers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OracleParams {
    /// Maximum price age (seconds).
    pub max_age_seconds: u32,
    /// Maximum confidence as a share of price (bps).
    pub max_confidence_bps: u16,
    /// Maximum spot-versus-reference deviation (bps).
    pub max_deviation_bps: u16,
}

impl OracleParams {
    /// Launch defaults aligned with [`crate::market::RiskParams::LAUNCH_DEFAULT`]
    /// (60 s, 2%) plus the 5% deviation breaker.
    pub const LAUNCH_DEFAULT: Self = Self {
        max_age_seconds: 60,
        max_confidence_bps: 200,
        max_deviation_bps: DEFAULT_DEVIATION_BPS,
    };

    /// Builds the parameters from the governed risk parameters and the
    /// configured deviation bound.
    pub fn from_risk(risk: &crate::market::RiskParams, max_deviation_bps: u16) -> Self {
        Self {
            max_age_seconds: risk.oracle_max_age_seconds,
            max_confidence_bps: risk.oracle_max_confidence_bps,
            max_deviation_bps,
        }
    }

    /// Rejects parameters outside the Requirement 6.2/6.3 bounds.
    pub fn validate(&self) -> Result<(), OracleError> {
        if !(MIN_ORACLE_MAX_AGE_SECONDS..=MAX_ORACLE_MAX_AGE_SECONDS)
            .contains(&self.max_age_seconds)
        {
            return Err(OracleError::InvalidOracleParams);
        }
        if !(MIN_ORACLE_MAX_CONFIDENCE_BPS..=MAX_ORACLE_MAX_CONFIDENCE_BPS)
            .contains(&self.max_confidence_bps)
        {
            return Err(OracleError::InvalidOracleParams);
        }
        if !(MIN_DEVIATION_BPS..=MAX_DEVIATION_BPS).contains(&self.max_deviation_bps) {
            return Err(OracleError::InvalidOracleParams);
        }
        Ok(())
    }
}

/// The fields of a Pyth `PriceUpdateV2` account the checks read, lifted into a
/// plain value so validation is host-testable. The Anchor layer fills it from
/// the deserialized receiver account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PythPriceView {
    /// The price account address.
    pub account: Pubkey,
    /// The program that owns the price account (must be the Pyth receiver).
    pub owner: Pubkey,
    /// The 32-byte feed id the account reports.
    pub feed_id: [u8; 32],
    /// The signed price mantissa.
    pub price: i64,
    /// The confidence interval (same exponent as `price`).
    pub conf: u64,
    /// The price exponent (Pyth prices are `price Ãƒâ€” 10^exponent`).
    pub exponent: i32,
    /// The feed publish time (unix seconds).
    pub publish_time: i64,
    /// The EMA price mantissa.
    pub ema_price: i64,
    /// The EMA confidence interval.
    pub ema_conf: u64,
}

/// Which side of risk a price feeds, which fixes the conservative direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PriceUse {
    /// Collateral uses `price - confidence`, normalized with floor rounding.
    Collateral,
    /// Debt uses `price + confidence`, normalized with ceil rounding.
    Debt,
}

/// A validated, conservatively normalized price at [`crate::math::USD_SCALE`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValidatedPrice {
    /// The conservative normalized price (floor for collateral, ceil for debt).
    pub normalized: u128,
    /// The feed publish time, captured into the Operation Intent.
    pub publish_time: i64,
}

/// Requires the price account to be owned by the Pyth receiver program and to
/// report the configured feed id.
pub fn check_receiver_and_feed(
    view: &PythPriceView,
    expected_feed_id: &[u8; 32],
) -> Result<(), OracleError> {
    if view.owner != PYTH_RECEIVER_PROGRAM_ID {
        return Err(OracleError::WrongReceiverProgram);
    }
    if view.feed_id != *expected_feed_id {
        return Err(OracleError::FeedMismatch);
    }
    Ok(())
}

/// Requires the price to be no older than `max_age_seconds` relative to `now`.
/// A future-dated or negative time is rejected rather than treated as fresh.
pub fn check_age(publish_time: i64, now: i64, max_age_seconds: u32) -> Result<(), OracleError> {
    if publish_time < 0 || now < 0 {
        return Err(OracleError::NegativeTimestamp);
    }
    if publish_time > now {
        // A print from the future is not trustworthy.
        return Err(OracleError::StalePrice);
    }
    let age = (now - publish_time) as u64;
    if age > u64::from(max_age_seconds) {
        return Err(OracleError::StalePrice);
    }
    Ok(())
}

/// Requires `conf / price <= max_confidence_bps / 10_000`, with `price > 0`.
pub fn check_confidence(price: i64, conf: u64, max_confidence_bps: u16) -> Result<(), OracleError> {
    if price <= 0 {
        return Err(OracleError::NonPositivePrice);
    }
    // conf * 10_000 <= price * max_confidence_bps, all in u128.
    let lhs = (conf as u128)
        .checked_mul(BPS_DENOMINATOR)
        .ok_or(OracleError::ArithmeticOverflow)?;
    let rhs = (price as u128)
        .checked_mul(u128::from(max_confidence_bps))
        .ok_or(OracleError::ArithmeticOverflow)?;
    if lhs > rhs {
        return Err(OracleError::ConfidenceTooWide);
    }
    Ok(())
}

/// The conservative mantissa for a use: `price - conf` for collateral (must stay
/// positive) and `price + conf` for debt.
fn conservative_mantissa(price: i64, conf: u64, price_use: PriceUse) -> Result<u64, OracleError> {
    if price <= 0 {
        return Err(OracleError::NonPositivePrice);
    }
    let price = price as u64;
    match price_use {
        PriceUse::Collateral => price
            .checked_sub(conf)
            .filter(|low| *low > 0)
            .ok_or(OracleError::NonPositivePrice),
        PriceUse::Debt => price
            .checked_add(conf)
            .ok_or(OracleError::ArithmeticOverflow),
    }
}

/// Normalizes a positive mantissa with the given exponent to
/// [`crate::math::USD_SCALE`], rounding up or down, and rejects a result
/// outside the committed price domain.
pub fn normalize_price(mantissa: u64, exponent: i32, round_up: bool) -> Result<u128, OracleError> {
    if !(MIN_PYTH_EXPONENT..=MAX_PYTH_EXPONENT).contains(&exponent) {
        return Err(OracleError::ExponentOutOfRange);
    }
    let scaled = (mantissa as u128)
        .checked_mul(USD_SCALE)
        .ok_or(OracleError::ArithmeticOverflow)?;
    let normalized = if exponent >= 0 {
        let factor = pow10(exponent as u32).ok_or(OracleError::ArithmeticOverflow)?;
        scaled
            .checked_mul(factor)
            .ok_or(OracleError::ArithmeticOverflow)?
    } else {
        let den = pow10(exponent.unsigned_abs()).ok_or(OracleError::ArithmeticOverflow)?;
        if round_up {
            // ceil(scaled / den)
            scaled
                .checked_add(den - 1)
                .ok_or(OracleError::ArithmeticOverflow)?
                / den
        } else {
            scaled / den
        }
    };
    if !(MIN_NORMALIZED_PRICE..=MAX_NORMALIZED_PRICE).contains(&normalized) {
        return Err(OracleError::PriceOutOfRange);
    }
    Ok(normalized)
}

/// Requires `|spot - reference| / reference <= max_deviation_bps / 10_000`.
/// Both inputs are already-normalized positive prices.
pub fn deviation_within(
    spot: u128,
    reference: u128,
    max_deviation_bps: u16,
) -> Result<(), OracleError> {
    if reference == 0 {
        return Err(OracleError::NonPositivePrice);
    }
    let diff = spot.abs_diff(reference);
    // diff * 10_000 <= reference * max_deviation_bps
    let lhs = diff
        .checked_mul(BPS_DENOMINATOR)
        .ok_or(OracleError::ArithmeticOverflow)?;
    let rhs = reference
        .checked_mul(u128::from(max_deviation_bps))
        .ok_or(OracleError::ArithmeticOverflow)?;
    if lhs > rhs {
        return Err(OracleError::DeviationTooHigh);
    }
    Ok(())
}

/// The full price pipeline for one feed: receiver and feed id, parameter
/// bounds, age, confidence, positivity, exponent, conservative normalization,
/// and the spot-versus-EMA breaker. Returns the conservative normalized price
/// and the publish time to capture into the Operation Intent.
pub fn validate_price(
    view: &PythPriceView,
    expected_feed_id: &[u8; 32],
    params: &OracleParams,
    now: i64,
    price_use: PriceUse,
) -> Result<ValidatedPrice, OracleError> {
    params.validate()?;
    validate_price_inner(view, expected_feed_id, params, now, price_use)
}

/// [`validate_price`] with an explicit maximum age (walnut-lend: the
/// `LendingPool` carries a devnet-tolerant age above the governed 300 s
/// bound). Confidence and deviation bounds are still validated; only the age
/// bound of [`OracleParams::validate`] is replaced by `max_age_seconds > 0`.
pub fn validate_price_with_max_age(
    view: &PythPriceView,
    expected_feed_id: &[u8; 32],
    max_age_seconds: u32,
    max_confidence_bps: u16,
    max_deviation_bps: u16,
    now: i64,
    price_use: PriceUse,
) -> Result<ValidatedPrice, OracleError> {
    if max_age_seconds == 0 {
        return Err(OracleError::InvalidOracleParams);
    }
    // Validate confidence and deviation with an in-range age placeholder.
    OracleParams {
        max_age_seconds: MIN_ORACLE_MAX_AGE_SECONDS,
        max_confidence_bps,
        max_deviation_bps,
    }
    .validate()?;
    let params = OracleParams {
        max_age_seconds,
        max_confidence_bps,
        max_deviation_bps,
    };
    validate_price_inner(view, expected_feed_id, &params, now, price_use)
}

/// Offset of the verification-level tag inside a `PriceUpdateV2` account
/// (8-byte discriminator plus the 32-byte write authority).
const PRICE_UPDATE_V2_LEVEL_OFFSET: usize = 40;

/// Anchor discriminator of Pyth's `PriceUpdateV2` account
/// (`sha256("account:PriceUpdateV2")[..8]`).
pub const PRICE_UPDATE_V2_DISCRIMINATOR: [u8; 8] = [34, 241, 35, 99, 157, 126, 244, 205];

/// Reads a fully verified `PriceUpdateV2` account (discriminator, `Full`
/// verification level, `PriceFeedMessage`) without the Pyth SDK. Returns
/// `None` for any other shape. Layout after the discriminator: write
/// authority (32), verification level (tag byte, `1` = Full), then feed id
/// (32), price (i64), conf (u64), exponent (i32), publish time (i64), previous
/// publish time (i64), EMA price (i64), EMA conf (u64), all little-endian.
pub fn price_view_from_account(
    account: Pubkey,
    owner: Pubkey,
    data: &[u8],
) -> Option<PythPriceView> {
    if data.len() < PRICE_UPDATE_V2_LEVEL_OFFSET + 1 + 84
        || data[..8] != PRICE_UPDATE_V2_DISCRIMINATOR
        || data[PRICE_UPDATE_V2_LEVEL_OFFSET] != 1
    {
        return None;
    }
    let m = PRICE_UPDATE_V2_LEVEL_OFFSET + 1;
    let i64_at = |at: usize| i64::from_le_bytes(data[at..at + 8].try_into().unwrap());
    let u64_at = |at: usize| u64::from_le_bytes(data[at..at + 8].try_into().unwrap());
    let mut feed_id = [0u8; 32];
    feed_id.copy_from_slice(&data[m..m + 32]);
    Some(PythPriceView {
        account,
        owner,
        feed_id,
        price: i64_at(m + 32),
        conf: u64_at(m + 40),
        exponent: i32::from_le_bytes(data[m + 48..m + 52].try_into().unwrap()),
        publish_time: i64_at(m + 52),
        ema_price: i64_at(m + 68),
        ema_conf: u64_at(m + 76),
    })
}

fn validate_price_inner(
    view: &PythPriceView,
    expected_feed_id: &[u8; 32],
    params: &OracleParams,
    now: i64,
    price_use: PriceUse,
) -> Result<ValidatedPrice, OracleError> {
    check_receiver_and_feed(view, expected_feed_id)?;
    check_age(view.publish_time, now, params.max_age_seconds)?;
    check_confidence(view.price, view.conf, params.max_confidence_bps)?;

    let round_up = matches!(price_use, PriceUse::Debt);
    let mantissa = conservative_mantissa(view.price, view.conf, price_use)?;
    let normalized = normalize_price(mantissa, view.exponent, round_up)?;

    // Spot-versus-EMA breaker. The EMA must be positive and share the exponent;
    // both are normalized with floor rounding to compare magnitudes.
    if view.ema_price <= 0 {
        return Err(OracleError::NonPositivePrice);
    }
    let spot_mid = normalize_price(view.price as u64, view.exponent, false)?;
    let ema_norm = normalize_price(view.ema_price as u64, view.exponent, false)?;
    deviation_within(spot_mid, ema_norm, params.max_deviation_bps)?;

    Ok(ValidatedPrice {
        normalized,
        publish_time: view.publish_time,
    })
}

/// Oracle health. `Normal` allows new price-sensitive operations; `Anomaly`
/// blocks them until a fresh observation passes every check on both feeds.
#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub enum OracleStatus {
    /// Both feeds last passed every check.
    #[default]
    Normal,
    /// A feed failed a check; price-sensitive operations are blocked.
    Anomaly,
}

/// The last-good observation recorded while the oracle was `Normal`. A later
/// spot that deviates from it beyond the bound trips the second breaker.
#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub struct LastGoodObservation {
    /// Conservative (low) normalized SOL price.
    pub normalized_sol_low: u128,
    /// Conservative (high) normalized USDC price.
    pub normalized_usdc_high: u128,
    /// SOL feed publish time.
    pub sol_publish_time: i64,
    /// USDC feed publish time.
    pub usdc_publish_time: i64,
    /// Slot the observation was recorded at.
    pub slot: u64,
    /// Whether any observation has been recorded.
    pub recorded: bool,
}

/// Blocks a price-sensitive operation while the oracle is in anomaly mode.
/// Non-price-sensitive operations (repay, collateral addition, supply, Ã¢â‚¬Â¦) never
/// call this, so they stay available.
pub fn ensure_price_sensitive_allowed(status: OracleStatus) -> Result<(), OracleError> {
    match status {
        OracleStatus::Normal => Ok(()),
        OracleStatus::Anomaly => Err(OracleError::OracleInAnomaly),
    }
}

/// Whether an operation reads a price and so is blocked in anomaly mode and
/// must capture the oracle generation and publish times. The design blocks
/// health freezing, borrowing, liquidation opening/execution, collateral
/// withdrawal, and Absorb; it keeps repay and collateral addition available.
///
/// The match is exhaustive so a later task that adds a price-sensitive
/// [`OperationKind`] (collateral withdrawal, Absorb) must classify it here.
pub fn operation_is_price_sensitive(kind: OperationKind) -> bool {
    match kind {
        OperationKind::Borrow | OperationKind::Health | OperationKind::Liquidation => true,
        OperationKind::Probe
        | OperationKind::PositionInit
        | OperationKind::Shield
        | OperationKind::Unshield
        | OperationKind::Supply
        | OperationKind::WithdrawSupply
        | OperationKind::Repay
        | OperationKind::ViewingKeyRotation
        | OperationKind::UtilizationBand
        | OperationKind::ReserveWithdrawal
        | OperationKind::RecoverEmptyPool
        | OperationKind::PositionOp
        | OperationKind::LendOp
        | OperationKind::Disclose => false,
    }
}

/// The oracle status implied by a fresh observation: `Normal` only if both
/// feeds validated this round, otherwise `Anomaly`. Entering `Anomaly` (a
/// `Normal Ã¢â€ â€™ Anomaly` transition) is what bumps the oracle generation and
/// stales pending price-sensitive callbacks; resuming requires both feeds valid.
pub fn status_after_observation(both_feeds_valid: bool) -> OracleStatus {
    if both_feeds_valid {
        OracleStatus::Normal
    } else {
        OracleStatus::Anomaly
    }
}

/// Whether applying `next` after `current` is a transition that must increment
/// the oracle generation (entering anomaly) so pending callbacks go `Stale`.
pub fn enters_anomaly(current: OracleStatus, next: OracleStatus) -> bool {
    current == OracleStatus::Normal && next == OracleStatus::Anomaly
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOL_FEED: [u8; 32] = [0x11; 32];
    const USDC_FEED: [u8; 32] = [0x22; 32];

    fn params() -> OracleParams {
        OracleParams::LAUNCH_DEFAULT
    }

    /// A SOL/USD feed near $150 with exponent -8 and a tight confidence.
    fn sol_view() -> PythPriceView {
        PythPriceView {
            account: Pubkey::new_from_array([1; 32]),
            owner: PYTH_RECEIVER_PROGRAM_ID,
            feed_id: SOL_FEED,
            price: 150_00000000, // 150.00000000 at exp -8
            conf: 5_00000,       // 0.00500000 at exp -8 (~0.0033% of price)
            exponent: -8,
            publish_time: 1_000,
            ema_price: 150_00000000,
            ema_conf: 5_00000,
        }
    }

    /// A USDC/USD feed near $1.00 with exponent -8.
    fn usdc_view() -> PythPriceView {
        PythPriceView {
            account: Pubkey::new_from_array([2; 32]),
            owner: PYTH_RECEIVER_PROGRAM_ID,
            feed_id: USDC_FEED,
            price: 1_00000000, // 1.00000000
            conf: 10000,       // 0.0001
            exponent: -8,
            publish_time: 1_000,
            ema_price: 1_00000000,
            ema_conf: 10000,
        }
    }

    // ---- Receiver and feed -------------------------------------------------

    #[test]
    fn accepts_the_configured_receiver_and_feed() {
        assert_eq!(check_receiver_and_feed(&sol_view(), &SOL_FEED), Ok(()));
    }

    #[test]
    fn rejects_a_foreign_owner() {
        let view = PythPriceView {
            owner: Pubkey::new_from_array([9; 32]),
            ..sol_view()
        };
        assert_eq!(
            check_receiver_and_feed(&view, &SOL_FEED),
            Err(OracleError::WrongReceiverProgram)
        );
    }

    #[test]
    fn rejects_a_wrong_feed_id() {
        assert_eq!(
            check_receiver_and_feed(&sol_view(), &USDC_FEED),
            Err(OracleError::FeedMismatch)
        );
    }

    // ---- Age ---------------------------------------------------------------

    #[test]
    fn accepts_a_fresh_price_and_rejects_a_stale_one() {
        assert_eq!(check_age(1_000, 1_030, 60), Ok(()));
        assert_eq!(check_age(1_000, 1_060, 60), Ok(()));
        assert_eq!(check_age(1_000, 1_061, 60), Err(OracleError::StalePrice));
    }

    #[test]
    fn rejects_future_and_negative_times() {
        assert_eq!(check_age(1_050, 1_000, 60), Err(OracleError::StalePrice));
        assert_eq!(
            check_age(-1, 1_000, 60),
            Err(OracleError::NegativeTimestamp)
        );
        assert_eq!(
            check_age(1_000, -1, 60),
            Err(OracleError::NegativeTimestamp)
        );
    }

    // ---- Confidence --------------------------------------------------------

    #[test]
    fn rejects_a_nonpositive_price_in_confidence() {
        assert_eq!(
            check_confidence(0, 1, 200),
            Err(OracleError::NonPositivePrice)
        );
        assert_eq!(
            check_confidence(-5, 1, 200),
            Err(OracleError::NonPositivePrice)
        );
    }

    #[test]
    fn confidence_band_boundary() {
        // price 10_000, 2% band => conf <= 200.
        assert_eq!(check_confidence(10_000, 200, 200), Ok(()));
        assert_eq!(
            check_confidence(10_000, 201, 200),
            Err(OracleError::ConfidenceTooWide)
        );
    }

    // ---- Normalization -----------------------------------------------------

    #[test]
    fn normalizes_exponent_minus_eight_as_identity_on_mantissa() {
        // mantissa 150e8 at exp -8, USD_SCALE 1e8 => 150e8.
        assert_eq!(normalize_price(150_00000000, -8, false), Ok(150_00000000));
        assert_eq!(normalize_price(1_00000000, -8, true), Ok(1_00000000));
    }

    #[test]
    fn normalization_rounds_in_the_conservative_direction() {
        // mantissa 1 at exp -9: scaled = 1 * 1e8 = 1e8; den = 1e9; floor=0, ceil=1.
        assert_eq!(
            normalize_price(1, -9, false),
            Err(OracleError::PriceOutOfRange)
        ); // floors to 0
        assert_eq!(normalize_price(1, -9, true), Ok(1)); // ceils to 1
    }

    #[test]
    fn rejects_out_of_range_exponents() {
        assert_eq!(
            normalize_price(1, MIN_PYTH_EXPONENT - 1, false),
            Err(OracleError::ExponentOutOfRange)
        );
        assert_eq!(
            normalize_price(1, MAX_PYTH_EXPONENT + 1, false),
            Err(OracleError::ExponentOutOfRange)
        );
    }

    #[test]
    fn rejects_a_price_above_the_domain() {
        // A mantissa that normalizes above MAX_NORMALIZED_PRICE.
        assert_eq!(
            normalize_price(u64::MAX, 12, false),
            Err(OracleError::ArithmeticOverflow)
        );
        assert_eq!(
            normalize_price(2_000_000_000_000, 0, false),
            Err(OracleError::PriceOutOfRange)
        );
    }

    // ---- Conservative direction -------------------------------------------

    #[test]
    fn collateral_uses_price_minus_conf_and_debt_uses_price_plus_conf() {
        // price 100, conf 10 at exp 0, USD_SCALE 1e8.
        let low = conservative_mantissa(100, 10, PriceUse::Collateral).unwrap();
        let high = conservative_mantissa(100, 10, PriceUse::Debt).unwrap();
        assert_eq!(low, 90);
        assert_eq!(high, 110);
    }

    #[test]
    fn collateral_rejects_confidence_wider_than_price() {
        assert_eq!(
            conservative_mantissa(100, 100, PriceUse::Collateral),
            Err(OracleError::NonPositivePrice)
        );
        assert_eq!(
            conservative_mantissa(100, 150, PriceUse::Collateral),
            Err(OracleError::NonPositivePrice)
        );
    }

    // ---- Deviation breaker -------------------------------------------------

    #[test]
    fn deviation_within_bound_passes_and_beyond_fails() {
        // reference 10_000, 5% bound => |diff| <= 500.
        assert_eq!(deviation_within(10_500, 10_000, 500), Ok(()));
        assert_eq!(deviation_within(9_500, 10_000, 500), Ok(()));
        assert_eq!(
            deviation_within(10_501, 10_000, 500),
            Err(OracleError::DeviationTooHigh)
        );
    }

    // ---- Full pipeline -----------------------------------------------------

    #[test]
    fn validates_sol_as_collateral_and_usdc_as_debt() {
        let sol = validate_price(
            &sol_view(),
            &SOL_FEED,
            &params(),
            1_030,
            PriceUse::Collateral,
        )
        .expect("sol validates");
        // 150 - 0.005 = 149.995 at USD_SCALE 1e8, floored.
        assert_eq!(sol.normalized, 149_99500000);
        assert_eq!(sol.publish_time, 1_000);

        let usdc = validate_price(&usdc_view(), &USDC_FEED, &params(), 1_030, PriceUse::Debt)
            .expect("usdc validates");
        // 1.00 + 0.0001 = 1.0001, ceiled.
        assert_eq!(usdc.normalized, 1_00010000);
    }

    #[test]
    fn pipeline_rejects_a_wrong_feed_before_pricing() {
        assert_eq!(
            validate_price(
                &sol_view(),
                &USDC_FEED,
                &params(),
                1_030,
                PriceUse::Collateral
            ),
            Err(OracleError::FeedMismatch)
        );
    }

    #[test]
    fn pipeline_rejects_a_spot_far_from_ema() {
        let view = PythPriceView {
            ema_price: 100_00000000, // EMA 100 vs spot 150 => 50% deviation
            ..sol_view()
        };
        assert_eq!(
            validate_price(&view, &SOL_FEED, &params(), 1_030, PriceUse::Collateral),
            Err(OracleError::DeviationTooHigh)
        );
    }

    #[test]
    fn pipeline_rejects_bad_params() {
        let bad = OracleParams {
            max_age_seconds: 1, // below MIN
            ..params()
        };
        assert_eq!(
            validate_price(&sol_view(), &SOL_FEED, &bad, 1_030, PriceUse::Collateral),
            Err(OracleError::InvalidOracleParams)
        );
    }

    // ---- Params ------------------------------------------------------------

    #[test]
    fn launch_params_are_valid_and_match_risk_defaults() {
        assert_eq!(OracleParams::LAUNCH_DEFAULT.validate(), Ok(()));
        let from_risk = OracleParams::from_risk(
            &crate::market::RiskParams::LAUNCH_DEFAULT,
            DEFAULT_DEVIATION_BPS,
        );
        assert_eq!(from_risk, OracleParams::LAUNCH_DEFAULT);
    }

    #[test]
    fn param_bounds_are_enforced_on_every_field() {
        for age in [
            MIN_ORACLE_MAX_AGE_SECONDS - 1,
            MAX_ORACLE_MAX_AGE_SECONDS + 1,
        ] {
            let p = OracleParams {
                max_age_seconds: age,
                ..params()
            };
            assert_eq!(p.validate(), Err(OracleError::InvalidOracleParams));
        }
        for conf in [
            MIN_ORACLE_MAX_CONFIDENCE_BPS - 1,
            MAX_ORACLE_MAX_CONFIDENCE_BPS + 1,
        ] {
            let p = OracleParams {
                max_confidence_bps: conf,
                ..params()
            };
            assert_eq!(p.validate(), Err(OracleError::InvalidOracleParams));
        }
        for dev in [MIN_DEVIATION_BPS - 1, MAX_DEVIATION_BPS + 1] {
            let p = OracleParams {
                max_deviation_bps: dev,
                ..params()
            };
            assert_eq!(p.validate(), Err(OracleError::InvalidOracleParams));
        }
    }

    // ---- Anomaly -----------------------------------------------------------

    #[test]
    fn anomaly_blocks_price_sensitive_operations_only() {
        assert_eq!(ensure_price_sensitive_allowed(OracleStatus::Normal), Ok(()));
        assert_eq!(
            ensure_price_sensitive_allowed(OracleStatus::Anomaly),
            Err(OracleError::OracleInAnomaly)
        );
    }

    #[test]
    fn price_sensitive_classification_matches_the_design() {
        // Blocked in anomaly.
        for kind in [
            OperationKind::Borrow,
            OperationKind::Health,
            OperationKind::Liquidation,
        ] {
            assert!(operation_is_price_sensitive(kind), "{kind:?}");
        }
        // Always available.
        for kind in [
            OperationKind::Probe,
            OperationKind::PositionInit,
            OperationKind::Shield,
            OperationKind::Unshield,
            OperationKind::Supply,
            OperationKind::WithdrawSupply,
            OperationKind::Repay,
            OperationKind::ViewingKeyRotation,
        ] {
            assert!(!operation_is_price_sensitive(kind), "{kind:?}");
        }
    }

    #[test]
    fn entering_anomaly_is_a_normal_to_anomaly_transition() {
        assert_eq!(status_after_observation(true), OracleStatus::Normal);
        assert_eq!(status_after_observation(false), OracleStatus::Anomaly);
        assert!(enters_anomaly(OracleStatus::Normal, OracleStatus::Anomaly));
        assert!(!enters_anomaly(
            OracleStatus::Anomaly,
            OracleStatus::Anomaly
        ));
        assert!(!enters_anomaly(OracleStatus::Normal, OracleStatus::Normal));
        // Resuming is Anomaly -> Normal; it is not an anomaly entry.
        assert!(!enters_anomaly(OracleStatus::Anomaly, OracleStatus::Normal));
    }

    // ---- Property tests ----------------------------------------------------

    /// Deterministic linear-congruential sampler (no external crate), matching
    /// the generator style used in `market::validation` tests.
    struct Lcg(u64);
    impl Lcg {
        fn sample(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 11
        }
    }

    /// **Validates: Requirement 6.5** - over random price/confidence/age inputs,
    /// the pipeline accepts a feed exactly when every individual check accepts
    /// it, and the collateral value never exceeds the debt value of the same
    /// mid price (conservative directioning).
    #[test]
    fn prop_pipeline_is_the_conjunction_of_its_checks() {
        let mut rng = Lcg(0x5EED_0601);
        let mut accepted = 0u32;
        for _ in 0..20_000 {
            let price = (rng.sample() % 300_00000000) as i64 + 1; // 1 .. ~300 at e-8
            let conf = rng.sample() % 20_00000000; // up to ~20 at e-8
            let age = (rng.sample() % 400) as i64; // 0..399 s
            let now = 1_000_000;
            let publish = now - age;
            let view = PythPriceView {
                account: Pubkey::new_from_array([3; 32]),
                owner: PYTH_RECEIVER_PROGRAM_ID,
                feed_id: SOL_FEED,
                price,
                conf,
                exponent: -8,
                publish_time: publish,
                ema_price: price, // EMA equals spot so the breaker never trips here
                ema_conf: conf,
            };
            let p = params();
            let result = validate_price(&view, &SOL_FEED, &p, now, PriceUse::Collateral);

            let fresh = age <= i64::from(p.max_age_seconds);
            let conf_ok = (conf as u128) * BPS_DENOMINATOR
                <= (price as u128) * u128::from(p.max_confidence_bps);
            let low_positive = (price as u64).checked_sub(conf).is_some_and(|l| l > 0);
            let should_pass = fresh && conf_ok && low_positive;

            assert_eq!(
                result.is_ok(),
                should_pass,
                "price {price} conf {conf} age {age}"
            );

            if let Ok(low) = result {
                let high =
                    validate_price(&view, &SOL_FEED, &p, now, PriceUse::Debt).expect("debt side");
                assert!(low.normalized <= high.normalized, "collateral <= debt");
                accepted += 1;
            }
        }
        assert!(accepted > 0, "the generator must reach the accepted case");
    }

    // ---- Task 6.5: boundary-exact named tests -------------------------------

    const NOW: i64 = 1_000_030;

    /// A $10 SOL view at exponent -8, fresh at `NOW`, EMA equal to spot.
    fn ten_dollar_view() -> PythPriceView {
        PythPriceView {
            price: 10_00000000,
            conf: 1,
            ema_price: 10_00000000,
            publish_time: NOW - 1,
            ..sol_view()
        }
    }

    fn run(view: &PythPriceView, p: &OracleParams, use_: PriceUse) -> Result<u128, OracleError> {
        validate_price(view, &SOL_FEED, p, NOW, use_).map(|v| v.normalized)
    }

    #[test]
    fn wrong_owner_is_rejected_by_the_pipeline() {
        for owner in [Pubkey::default(), Pubkey::new_from_array([0xAB; 32])] {
            let view = PythPriceView {
                owner,
                ..ten_dollar_view()
            };
            assert_eq!(
                run(&view, &params(), PriceUse::Collateral),
                Err(OracleError::WrongReceiverProgram)
            );
        }
        // A one-byte difference from the receiver id is still wrong.
        let mut bytes = PYTH_RECEIVER_PROGRAM_ID.to_bytes();
        bytes[31] ^= 1;
        let view = PythPriceView {
            owner: Pubkey::new_from_array(bytes),
            ..ten_dollar_view()
        };
        assert_eq!(
            run(&view, &params(), PriceUse::Debt),
            Err(OracleError::WrongReceiverProgram)
        );
    }

    #[test]
    fn wrong_owner_is_reported_before_a_wrong_feed() {
        let view = PythPriceView {
            owner: Pubkey::default(),
            ..ten_dollar_view()
        };
        assert_eq!(
            check_receiver_and_feed(&view, &USDC_FEED),
            Err(OracleError::WrongReceiverProgram)
        );
    }

    #[test]
    fn wrong_feed_id_single_bit_is_rejected_by_the_pipeline() {
        for byte in [0usize, 15, 31] {
            let mut feed = SOL_FEED;
            feed[byte] ^= 0x80;
            let view = PythPriceView {
                feed_id: feed,
                ..ten_dollar_view()
            };
            assert_eq!(
                run(&view, &params(), PriceUse::Collateral),
                Err(OracleError::FeedMismatch)
            );
        }
    }

    #[test]
    fn age_exactly_max_passes_and_one_more_second_is_stale() {
        for max in [MIN_ORACLE_MAX_AGE_SECONDS, 60, MAX_ORACLE_MAX_AGE_SECONDS] {
            let p = OracleParams {
                max_age_seconds: max,
                ..params()
            };
            let at_max = PythPriceView {
                publish_time: NOW - i64::from(max),
                ..ten_dollar_view()
            };
            assert!(run(&at_max, &p, PriceUse::Collateral).is_ok(), "max {max}");
            let past = PythPriceView {
                publish_time: NOW - i64::from(max) - 1,
                ..ten_dollar_view()
            };
            assert_eq!(
                run(&past, &p, PriceUse::Collateral),
                Err(OracleError::StalePrice),
                "max {max}"
            );
        }
    }

    #[test]
    fn publish_time_equal_to_now_passes_and_future_time_fails() {
        let at_now = PythPriceView {
            publish_time: NOW,
            ..ten_dollar_view()
        };
        assert!(run(&at_now, &params(), PriceUse::Collateral).is_ok());
        let future = PythPriceView {
            publish_time: NOW + 1,
            ..ten_dollar_view()
        };
        assert_eq!(
            run(&future, &params(), PriceUse::Collateral),
            Err(OracleError::StalePrice)
        );
        assert_eq!(
            check_age(i64::MAX, i64::MAX - 1, 60),
            Err(OracleError::StalePrice)
        );
        // The widest legal span never overflows the subtraction.
        assert_eq!(check_age(0, i64::MAX, 300), Err(OracleError::StalePrice));
    }

    #[test]
    fn zero_price_is_rejected() {
        let view = PythPriceView {
            price: 0,
            conf: 0,
            ..ten_dollar_view()
        };
        assert_eq!(
            run(&view, &params(), PriceUse::Collateral),
            Err(OracleError::NonPositivePrice)
        );
        assert_eq!(
            run(&view, &params(), PriceUse::Debt),
            Err(OracleError::NonPositivePrice)
        );
    }

    #[test]
    fn negative_price_is_rejected_including_i64_min() {
        for price in [-1i64, -10_00000000, i64::MIN] {
            let view = PythPriceView {
                price,
                ..ten_dollar_view()
            };
            assert_eq!(
                run(&view, &params(), PriceUse::Collateral),
                Err(OracleError::NonPositivePrice),
                "price {price}"
            );
        }
    }

    #[test]
    fn zero_or_negative_ema_is_rejected() {
        for ema_price in [0i64, -1, i64::MIN] {
            let view = PythPriceView {
                ema_price,
                ..ten_dollar_view()
            };
            assert_eq!(
                run(&view, &params(), PriceUse::Collateral),
                Err(OracleError::NonPositivePrice),
                "ema {ema_price}"
            );
        }
    }

    #[test]
    fn confidence_exactly_at_the_bps_bound_passes_and_one_unit_more_fails() {
        // price = $10 (1e9 at -8): 10 bps => 1_000_000, 200 bps => 20_000_000,
        // 1_000 bps => 100_000_000.
        for (bps, max_conf) in [
            (10u16, 1_000_000u64),
            (200, 20_000_000),
            (1_000, 100_000_000),
        ] {
            let p = OracleParams {
                max_confidence_bps: bps,
                ..params()
            };
            let ok = PythPriceView {
                conf: max_conf,
                ..ten_dollar_view()
            };
            assert!(run(&ok, &p, PriceUse::Collateral).is_ok(), "bps {bps}");
            let wide = PythPriceView {
                conf: max_conf + 1,
                ..ten_dollar_view()
            };
            assert_eq!(
                run(&wide, &p, PriceUse::Collateral),
                Err(OracleError::ConfidenceTooWide),
                "bps {bps}"
            );
        }
    }

    #[test]
    fn confidence_check_does_not_overflow_at_u64_extremes() {
        assert_eq!(
            check_confidence(i64::MAX, u64::MAX, MAX_ORACLE_MAX_CONFIDENCE_BPS),
            Err(OracleError::ConfidenceTooWide)
        );
        assert_eq!(
            check_confidence(i64::MAX, 0, u16::MAX),
            Ok(()),
            "i64::MAX * u16::MAX fits u128"
        );
    }

    #[test]
    fn exponent_below_min_and_above_max_is_rejected_by_the_pipeline() {
        for exponent in [
            MIN_PYTH_EXPONENT - 1,
            MAX_PYTH_EXPONENT + 1,
            i32::MIN,
            i32::MAX,
        ] {
            let view = PythPriceView {
                exponent,
                ..ten_dollar_view()
            };
            assert_eq!(
                run(&view, &params(), PriceUse::Collateral),
                Err(OracleError::ExponentOutOfRange),
                "exponent {exponent}"
            );
        }
    }

    #[test]
    fn exponent_window_edges_are_accepted_by_normalization() {
        // -12: mantissa 1e12 is exactly $1 (1e8 at USD_SCALE).
        assert_eq!(
            normalize_price(1_000_000_000_000, MIN_PYTH_EXPONENT, false),
            Ok(USD_SCALE)
        );
        // +12 passes the exponent gate; only the price domain can reject it.
        assert_eq!(
            normalize_price(1, MAX_PYTH_EXPONENT, false),
            Err(OracleError::PriceOutOfRange)
        );
        assert_ne!(
            normalize_price(1, MAX_PYTH_EXPONENT, false),
            Err(OracleError::ExponentOutOfRange)
        );
    }

    #[test]
    fn pow10_covers_the_whole_exponent_window() {
        // The largest power the normalizer asks for is |exponent| <= 12, far
        // below the u128 limit of 10^38, so `pow10` never returns None there.
        for exp in 0..=MAX_PYTH_EXPONENT
            .unsigned_abs()
            .max(MIN_PYTH_EXPONENT.unsigned_abs())
        {
            assert!(pow10(exp).is_some(), "10^{exp}");
        }
        assert!(pow10(38).is_some());
        assert_eq!(pow10(39), None);
    }

    #[test]
    fn normalization_overflow_is_an_error_not_a_wrap() {
        // u64::MAX * 1e8 * 1e12 > u128::MAX.
        assert_eq!(
            normalize_price(u64::MAX, MAX_PYTH_EXPONENT, false),
            Err(OracleError::ArithmeticOverflow)
        );
        // Fits u128 but exceeds the committed price domain.
        assert_eq!(
            normalize_price(u64::MAX, 0, true),
            Err(OracleError::PriceOutOfRange)
        );
        // Most negative exponent keeps the value in domain.
        assert!(normalize_price(u64::MAX, MIN_PYTH_EXPONENT, true).is_ok());
        assert!(normalize_price(u64::MAX, MIN_PYTH_EXPONENT, false).is_ok());
    }

    #[test]
    fn price_plus_confidence_overflow_is_an_error() {
        assert_eq!(
            conservative_mantissa(i64::MAX, u64::MAX, PriceUse::Debt),
            Err(OracleError::ArithmeticOverflow)
        );
        // Exact edge: i64::MAX + (2^63 + 1) = u64::MAX passes, one more overflows.
        let price = i64::MAX;
        let conf_ok = u64::MAX - price as u64;
        assert_eq!(
            conservative_mantissa(price, conf_ok, PriceUse::Debt),
            Ok(u64::MAX)
        );
        assert_eq!(
            conservative_mantissa(price, conf_ok + 1, PriceUse::Debt),
            Err(OracleError::ArithmeticOverflow)
        );
    }

    #[test]
    fn normalized_price_domain_boundaries_are_exact() {
        // At exponent -8 the normalized value equals the mantissa.
        let max = MAX_NORMALIZED_PRICE as u64;
        assert_eq!(normalize_price(max, -8, false), Ok(MAX_NORMALIZED_PRICE));
        assert_eq!(
            normalize_price(max + 1, -8, false),
            Err(OracleError::PriceOutOfRange)
        );
        assert_eq!(normalize_price(1, -8, false), Ok(MIN_NORMALIZED_PRICE));
        assert_eq!(
            normalize_price(0, -8, true),
            Err(OracleError::PriceOutOfRange)
        );
    }

    #[test]
    fn spot_versus_ema_deviation_boundary_is_exact_in_both_directions() {
        // EMA $10, 5% bound: spot in [9.50, 10.50] passes.
        let ema = 10_00000000i64;
        let p = params();
        let at = |spot: i64| {
            let view = PythPriceView {
                price: spot,
                ema_price: ema,
                ..ten_dollar_view()
            };
            run(&view, &p, PriceUse::Collateral)
        };
        assert!(at(10_50000000).is_ok());
        assert_eq!(at(10_50000001), Err(OracleError::DeviationTooHigh));
        assert!(at(9_50000000).is_ok());
        assert_eq!(at(9_49999999), Err(OracleError::DeviationTooHigh));
    }

    #[test]
    fn spot_versus_ema_extreme_deviation_is_rejected() {
        // Spot at the top of the domain against a 1-unit EMA.
        let view = PythPriceView {
            price: MAX_NORMALIZED_PRICE as i64,
            conf: 1,
            ema_price: 1,
            ..ten_dollar_view()
        };
        assert_eq!(
            run(&view, &params(), PriceUse::Collateral),
            Err(OracleError::DeviationTooHigh)
        );
        // And the reverse: a 1-unit spot against a top-of-domain EMA.
        let view = PythPriceView {
            price: 1_000_000,
            conf: 0,
            ema_price: MAX_NORMALIZED_PRICE as i64,
            ..ten_dollar_view()
        };
        assert_eq!(
            run(&view, &params(), PriceUse::Collateral),
            Err(OracleError::DeviationTooHigh)
        );
    }

    #[test]
    fn spot_versus_last_good_deviation_boundary_is_exact() {
        let last_good = LastGoodObservation {
            normalized_sol_low: 149_99500000,
            normalized_usdc_high: 1_00010000,
            sol_publish_time: 1_000,
            usdc_publish_time: 1_000,
            slot: 5,
            recorded: true,
        };
        let reference = last_good.normalized_sol_low;
        let bound = DEFAULT_DEVIATION_BPS;
        // 5% of 149.995 = 7.49975 => 7_49975000 exactly.
        let delta = reference * u128::from(bound) / BPS_DENOMINATOR;
        assert_eq!(reference * u128::from(bound) % BPS_DENOMINATOR, 0);
        assert_eq!(
            deviation_within(reference + delta, reference, bound),
            Ok(())
        );
        assert_eq!(
            deviation_within(reference + delta + 1, reference, bound),
            Err(OracleError::DeviationTooHigh)
        );
        assert_eq!(
            deviation_within(reference - delta, reference, bound),
            Ok(())
        );
        assert_eq!(
            deviation_within(reference - delta - 1, reference, bound),
            Err(OracleError::DeviationTooHigh)
        );
    }

    #[test]
    fn deviation_check_handles_zero_reference_and_overflow() {
        assert_eq!(
            deviation_within(1, 0, 500),
            Err(OracleError::NonPositivePrice)
        );
        assert_eq!(
            deviation_within(u128::MAX, 1, 500),
            Err(OracleError::ArithmeticOverflow)
        );
        assert_eq!(
            deviation_within(1, u128::MAX, u16::MAX),
            Err(OracleError::ArithmeticOverflow)
        );
        assert_eq!(deviation_within(7, 7, MIN_DEVIATION_BPS), Ok(()));
    }

    #[test]
    fn collateral_rounds_down_and_debt_rounds_up_through_the_pipeline() {
        // price 1000, conf 1 at exponent -9: low 999 -> 99.9 floors to 99,
        // high 1001 -> 100.1 ceils to 101 (USD_SCALE 1e8).
        let view = PythPriceView {
            price: 1_000,
            conf: 1,
            exponent: -9,
            ema_price: 1_000,
            ..ten_dollar_view()
        };
        assert_eq!(run(&view, &params(), PriceUse::Collateral), Ok(99));
        assert_eq!(run(&view, &params(), PriceUse::Debt), Ok(101));
    }

    #[test]
    fn collateral_with_confidence_equal_to_price_is_rejected() {
        // 100% confidence is wider than any allowed bound, so the band check
        // fires first; the conservative-low rule is the second line of defence.
        let view = PythPriceView {
            conf: 10_00000000,
            ..ten_dollar_view()
        };
        assert_eq!(
            run(&view, &params(), PriceUse::Collateral),
            Err(OracleError::ConfidenceTooWide)
        );
    }

    // ---- Decision Expiry and oracle-generation staling ---------------------

    use crate::operation::callback::{
        classify_callback, CallbackContext, CallbackDecision, CallbackReport, CurrentDependencies,
        StaleReason,
    };
    use crate::operation::commitment::{account_commitment, input_commitment};
    use crate::operation::OperationIntent;

    const ORACLE_GENERATION: u64 = 5;
    const DECISION_EXPIRY_SLOT: u64 = 100;

    /// A queued price-sensitive Borrow Intent that captured oracle generation
    /// `ORACLE_GENERATION`, both publish times, and a Decision Expiry.
    fn price_sensitive_intent() -> OperationIntent {
        OperationIntent {
            key: Pubkey::new_from_array([42; 32]),
            kind: OperationKind::Borrow,
            computation_offset: 9,
            oracle_generation: ORACLE_GENERATION,
            decision_expiry_slot: DECISION_EXPIRY_SLOT,
            oracle_publish_time_sol: Some(1_000),
            oracle_publish_time_usdc: Some(1_000),
            accounts_commitment: account_commitment(&[]),
            inputs_commitment: input_commitment(OperationKind::Borrow, &[]),
            ..OperationIntent::default()
        }
    }

    fn classify(intent: &OperationIntent, oracle_generation: u64, slot: u64) -> CallbackDecision {
        let mut current = CurrentDependencies::captured_by(intent);
        current.oracle_generation = oracle_generation;
        let ctx = CallbackContext {
            output_authenticated: true,
            computation_offset: intent.computation_offset,
            intent_key: intent.key,
            kind: intent.kind,
            accounts: &[],
            input_fields: &[],
            current,
            current_slot: slot,
            report: CallbackReport::Success { nonzero: true },
        };
        classify_callback(intent, &ctx).expect("a well-formed callback classifies")
    }

    #[test]
    fn decision_expiry_slot_is_inclusive_for_a_price_sensitive_callback() {
        let intent = price_sensitive_intent();
        assert_eq!(
            classify(&intent, ORACLE_GENERATION, DECISION_EXPIRY_SLOT),
            CallbackDecision::Applied
        );
        assert_eq!(
            classify(&intent, ORACLE_GENERATION, DECISION_EXPIRY_SLOT + 1),
            CallbackDecision::Expired
        );
    }

    #[test]
    fn a_price_sensitive_intent_captures_publish_times_and_oracle_generation() {
        let intent = price_sensitive_intent();
        assert!(intent.oracle_publish_time_sol.is_some());
        assert!(intent.oracle_publish_time_usdc.is_some());
        assert_eq!(intent.oracle_generation, ORACLE_GENERATION);
        assert!(operation_is_price_sensitive(intent.kind));
    }

    #[test]
    fn an_oracle_generation_change_stales_the_callback_before_expiry() {
        let intent = price_sensitive_intent();
        assert_eq!(
            classify(&intent, ORACLE_GENERATION + 1, DECISION_EXPIRY_SLOT - 1),
            CallbackDecision::Stale(StaleReason::OracleGeneration)
        );
    }

    #[test]
    fn stale_oracle_generation_takes_precedence_over_expiry() {
        let intent = price_sensitive_intent();
        assert_eq!(
            classify(&intent, ORACLE_GENERATION + 1, DECISION_EXPIRY_SLOT + 50),
            CallbackDecision::Stale(StaleReason::OracleGeneration)
        );
    }

    // ---- Property tests (Task 6.5) -----------------------------------------

    impl Lcg {
        fn below(&mut self, n: u64) -> u64 {
            self.sample() % n
        }
        fn pick<T: Copy>(&mut self, items: &[T]) -> T {
            items[self.below(items.len() as u64) as usize]
        }
    }

    /// A value from a mix of small, boundary, and extreme magnitudes.
    fn wild_u64(rng: &mut Lcg) -> u64 {
        match rng.below(6) {
            0 => rng.below(1_000),
            1 => rng.below(300_00000000),
            2 => (rng.sample() << 11) | rng.below(2_048),
            3 => u64::MAX - rng.below(4),
            4 => i64::MAX as u64 - rng.below(4),
            _ => rng.pick(&[0, 1, 10_000_000_000_000_000_000, 1_000_000_000_000_000_000]),
        }
    }

    fn wild_i64(rng: &mut Lcg) -> i64 {
        match rng.below(8) {
            0 => -(rng.below(1_000) as i64),
            1 => i64::MIN + rng.below(3) as i64,
            2 => 0,
            _ => (wild_u64(rng) & (i64::MAX as u64)) as i64,
        }
    }

    fn wild_exponent(rng: &mut Lcg) -> i32 {
        match rng.below(8) {
            0 => i32::MIN + rng.below(3) as i32,
            1 => i32::MAX - rng.below(3) as i32,
            2 => -8,
            _ => rng.below(33) as i32 - 16,
        }
    }

    fn wild_view(rng: &mut Lcg, now: i64) -> PythPriceView {
        let anchor_price = (rng.below(300_00000000) + 1) as i64;
        let near = rng.below(2) == 0;
        let price = if near { anchor_price } else { wild_i64(rng) };
        let conf = if near && rng.below(4) != 0 {
            rng.below(anchor_price as u64 / 20 + 1)
        } else {
            wild_u64(rng)
        };
        let ema_price = match rng.below(4) {
            0 => wild_i64(rng),
            1 => price.saturating_add((rng.below(600) as i64 - 300) * (price / 5_000)),
            _ => price,
        };
        PythPriceView {
            account: Pubkey::new_from_array([3; 32]),
            owner: if rng.below(12) == 0 {
                Pubkey::new_from_array([7; 32])
            } else {
                PYTH_RECEIVER_PROGRAM_ID
            },
            feed_id: if rng.below(12) == 0 {
                USDC_FEED
            } else {
                SOL_FEED
            },
            price,
            conf,
            exponent: if near && rng.below(3) != 0 {
                -8
            } else {
                wild_exponent(rng)
            },
            publish_time: match rng.below(6) {
                0 => now + rng.below(5) as i64,
                1 => -(rng.below(5) as i64),
                2 => now - 400 + rng.below(450) as i64,
                _ => now - rng.below(70) as i64,
            },
            ema_price,
            ema_conf: wild_u64(rng),
        }
    }

    fn wild_params(rng: &mut Lcg) -> OracleParams {
        if rng.below(10) == 0 {
            // Occasionally out-of-bounds parameters.
            OracleParams {
                max_age_seconds: rng.pick(&[0, 4, 301, u32::MAX]),
                max_confidence_bps: rng.pick(&[0, 9, 1_001, u16::MAX]),
                max_deviation_bps: rng.pick(&[0, 49, 5_001, u16::MAX]),
            }
        } else {
            OracleParams {
                max_age_seconds: 5 + rng.below(296) as u32,
                max_confidence_bps: 10 + rng.below(991) as u16,
                max_deviation_bps: 50 + rng.below(4_951) as u16,
            }
        }
    }

    /// Reference normalization written independently of `normalize_price`:
    /// divides with `/` and `%` instead of the add-then-divide ceiling.
    fn reference_normalize(mantissa: u64, exponent: i32, up: bool) -> Option<u128> {
        if !(-12..=12).contains(&exponent) {
            return None;
        }
        let shift = exponent + 8; // value = mantissa * 10^shift
        let m = u128::from(mantissa);
        let value = if shift >= 0 {
            m.checked_mul(10u128.checked_pow(shift as u32)?)?
        } else {
            let den = 10u128.pow(shift.unsigned_abs());
            let q = m / den;
            if up && m % den != 0 {
                q + 1
            } else {
                q
            }
        };
        (1..=1_000_000_000_000_000_000u128)
            .contains(&value)
            .then_some(value)
    }

    /// The accept/reject rule list, written as a flat conjunction.
    fn reference_validate(
        view: &PythPriceView,
        p: &OracleParams,
        now: i64,
        price_use: PriceUse,
    ) -> Option<u128> {
        let params_ok = (5..=300).contains(&p.max_age_seconds)
            && (10..=1_000).contains(&p.max_confidence_bps)
            && (50..=5_000).contains(&p.max_deviation_bps);
        if !params_ok
            || view.owner != PYTH_RECEIVER_PROGRAM_ID
            || view.feed_id != SOL_FEED
            || view.publish_time < 0
            || now < 0
            || view.publish_time > now
            || (now - view.publish_time) > i64::from(p.max_age_seconds)
            || view.price <= 0
            || view.ema_price <= 0
        {
            return None;
        }
        let price = view.price as u64;
        if u128::from(view.conf) * 10_000 > u128::from(price) * u128::from(p.max_confidence_bps) {
            return None;
        }
        let (mantissa, up) = match price_use {
            PriceUse::Collateral => (price.checked_sub(view.conf).filter(|m| *m > 0)?, false),
            PriceUse::Debt => (price.checked_add(view.conf)?, true),
        };
        let result = reference_normalize(mantissa, view.exponent, up)?;
        let spot = reference_normalize(price, view.exponent, false)?;
        let ema = reference_normalize(view.ema_price as u64, view.exponent, false)?;
        let diff = spot.abs_diff(ema);
        if diff * 10_000 > ema * u128::from(p.max_deviation_bps) {
            return None;
        }
        Some(result)
    }

    /// **Validates: Requirements 6.1, 6.2, 6.3, 6.4, 6.5, 6.6** - over random
    /// valid and invalid views and parameters (including extreme mantissas and
    /// exponents), `validate_price` accepts exactly when an independently
    /// written rule list accepts, and returns the same conservative value.
    #[test]
    fn prop_validate_price_accepts_iff_every_rule_holds() {
        let mut rng = Lcg(0x5EED_0651);
        let (mut accepted, mut rejected) = (0u32, 0u32);
        for _ in 0..60_000 {
            let now = 1_000_000 + rng.below(1_000) as i64;
            let view = wild_view(&mut rng, now);
            let p = wild_params(&mut rng);
            let price_use = if rng.below(2) == 0 {
                PriceUse::Collateral
            } else {
                PriceUse::Debt
            };
            let actual = validate_price(&view, &SOL_FEED, &p, now, price_use);
            let expected = reference_validate(&view, &p, now, price_use);
            assert_eq!(
                actual.ok().map(|v| v.normalized),
                expected,
                "{view:?} {p:?} {price_use:?}"
            );
            if expected.is_some() {
                accepted += 1;
            } else {
                rejected += 1;
            }
        }
        assert!(accepted > 500 && rejected > 500, "{accepted}/{rejected}");
    }

    /// **Validates: Requirement 6.5** - for every accepted view,
    /// `low <= floor(spot) <= high` (collateral price never above spot, debt
    /// price never below it).
    #[test]
    fn prop_low_never_exceeds_spot_and_spot_never_exceeds_high() {
        let mut rng = Lcg(0x5EED_0652);
        let mut checked = 0u32;
        for _ in 0..40_000 {
            let now = 1_000_000;
            let view = wild_view(&mut rng, now);
            let p = OracleParams {
                max_confidence_bps: MAX_ORACLE_MAX_CONFIDENCE_BPS,
                max_deviation_bps: MAX_DEVIATION_BPS,
                ..params()
            };
            let low = validate_price(&view, &SOL_FEED, &p, now, PriceUse::Collateral);
            let high = validate_price(&view, &SOL_FEED, &p, now, PriceUse::Debt);
            if let (Ok(low), Ok(high)) = (low, high) {
                let spot = normalize_price(view.price as u64, view.exponent, false)
                    .expect("accepted views have an in-domain spot");
                assert!(low.normalized <= spot, "{view:?}");
                assert!(spot <= high.normalized, "{view:?}");
                checked += 1;
            }
        }
        assert!(checked > 500, "{checked}");
    }

    /// **Validates: Requirement 6.4** - normalization is monotone in the
    /// mantissa for both rounding modes, and ceil exceeds floor by at most one.
    #[test]
    fn prop_normalization_is_monotone_in_the_mantissa() {
        let mut rng = Lcg(0x5EED_0653);
        let mut compared = 0u32;
        for _ in 0..40_000 {
            let exponent = rng.below(25) as i32 - 12;
            let (a, b) = (wild_u64(&mut rng), wild_u64(&mut rng));
            let (lo, hi) = (a.min(b), a.max(b));
            for up in [false, true] {
                if let (Ok(n_lo), Ok(n_hi)) = (
                    normalize_price(lo, exponent, up),
                    normalize_price(hi, exponent, up),
                ) {
                    assert!(n_lo <= n_hi, "{lo} {hi} e{exponent} up={up}");
                    compared += 1;
                }
            }
            if let (Ok(floor), Ok(ceil)) = (
                normalize_price(hi, exponent, false),
                normalize_price(hi, exponent, true),
            ) {
                assert!(floor <= ceil && ceil - floor <= 1, "{hi} e{exponent}");
            }
        }
        assert!(compared > 500, "{compared}");
    }

    /// **Validates: Requirement 6.4** - no function panics or wraps anywhere in
    /// the input domain, including i64/u64/i32/u16 extremes.
    #[test]
    fn prop_no_panics_on_the_full_input_domain() {
        let mut rng = Lcg(0x5EED_0654);
        for _ in 0..60_000 {
            let now = wild_i64(&mut rng);
            let view = PythPriceView {
                price: wild_i64(&mut rng),
                conf: wild_u64(&mut rng),
                exponent: wild_exponent(&mut rng),
                publish_time: wild_i64(&mut rng),
                ema_price: wild_i64(&mut rng),
                ..sol_view()
            };
            let p = OracleParams {
                max_age_seconds: rng.sample() as u32,
                max_confidence_bps: rng.sample() as u16,
                max_deviation_bps: rng.sample() as u16,
            };
            for price_use in [PriceUse::Collateral, PriceUse::Debt] {
                let _ = validate_price(&view, &SOL_FEED, &p, now, price_use);
                let _ = conservative_mantissa(view.price, view.conf, price_use);
            }
            let _ = check_age(view.publish_time, now, p.max_age_seconds);
            let _ = check_confidence(view.price, view.conf, p.max_confidence_bps);
            let _ = normalize_price(wild_u64(&mut rng), view.exponent, rng.below(2) == 0);
            let (x, y) = (
                u128::from(wild_u64(&mut rng)) * u128::from(wild_u64(&mut rng)),
                u128::from(wild_u64(&mut rng)),
            );
            let _ = deviation_within(x, y, p.max_deviation_bps);
            let _ = deviation_within(u128::MAX, y, p.max_deviation_bps);
        }
    }

    /// A random observation of both feeds; `true` entries are valid views.
    fn observe(rng: &mut Lcg, now: i64) -> (bool, bool) {
        let p = params();
        let sol_view = wild_view(rng, now);
        let usdc_view = PythPriceView {
            feed_id: USDC_FEED,
            ..wild_view(rng, now)
        };
        let sol = validate_price(&sol_view, &SOL_FEED, &p, now, PriceUse::Collateral).is_ok();
        let usdc = validate_price(&usdc_view, &USDC_FEED, &p, now, PriceUse::Debt).is_ok();
        (sol, usdc)
    }

    /// **Validates: Requirements 6.7, 6.8, 6.9** - across random observation
    /// sequences: entering anomaly mode always bumps the oracle generation and
    /// stales a pending price-sensitive callback; a callback captured at the
    /// current generation is never staled by oracle movement; price-sensitive
    /// operations are blocked in anomaly and resume only after a fresh
    /// observation where both feeds pass.
    #[test]
    fn prop_anomaly_bumps_generation_stales_pending_and_resume_needs_both_feeds() {
        let mut rng = Lcg(0x5EED_0655);
        let mut status = OracleStatus::Normal;
        let mut generation = ORACLE_GENERATION;
        let (mut entered, mut resumed) = (0u32, 0u32);
        for _ in 0..30_000 {
            let now = 1_000_000;
            let pending = OperationIntent {
                oracle_generation: generation,
                ..price_sensitive_intent()
            };
            let (sol_ok, usdc_ok) = observe(&mut rng, now);
            let next = status_after_observation(sol_ok && usdc_ok);
            // Resume only when both feeds pass.
            assert_eq!(next == OracleStatus::Normal, sol_ok && usdc_ok);
            if enters_anomaly(status, next) {
                generation += 1;
                entered += 1;
                assert_eq!(
                    classify(&pending, generation, DECISION_EXPIRY_SLOT),
                    CallbackDecision::Stale(StaleReason::OracleGeneration)
                );
            } else {
                assert_eq!(
                    classify(&pending, generation, DECISION_EXPIRY_SLOT),
                    CallbackDecision::Applied
                );
            }
            if status == OracleStatus::Anomaly && next == OracleStatus::Normal {
                assert!(sol_ok && usdc_ok);
                resumed += 1;
            }
            status = next;
            assert_eq!(
                ensure_price_sensitive_allowed(status).is_ok(),
                sol_ok && usdc_ok
            );
        }
        assert!(entered > 20 && resumed > 20, "{entered}/{resumed}");
    }
}
