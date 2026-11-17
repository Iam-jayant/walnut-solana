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
    /// The price exponent (Pyth prices are `price × 10^exponent`).
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
/// Non-price-sensitive operations (repay, collateral addition, supply, …) never
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
        | OperationKind::Repay => false,
    }
}

/// The oracle status implied by a fresh observation: `Normal` only if both
/// feeds validated this round, otherwise `Anomaly`. Entering `Anomaly` (a
/// `Normal → Anomaly` transition) is what bumps the oracle generation and
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
            let conf = (rng.sample() % 20_00000000) as u64; // up to ~20 at e-8
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
}
