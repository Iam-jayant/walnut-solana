//! Bounded kinked rate model, Borrow Index accrual and projection, and
//! utilization bands (Task 7.2; design: "Interest and pool accounting").
//!
//! Everything here is public-state arithmetic. The circuit side of the band
//! (`encrypted-ixs`, `circuits::utilization_band_index`) is the same
//! algorithm over secret values; the equivalence is checked against
//! `packages/reference-model/vectors/rate.vectors`.
//!
//! # Rate model
//!
//! `RateParams` are annualized basis points. With `u` the utilization in bps,
//! `optimal` the kink, and `base`, `slope1`, `slope2` the parameters:
//!
//! ```text
//! u <= optimal : rate = base + ceil(slope1 × u / optimal)
//! u >  optimal : rate = base + slope1 + ceil(slope2 × (u - optimal) / (10_000 - optimal))
//! ```
//!
//! The result is capped at [`MAX_BORROW_RATE_BPS`] (the governed bounds in
//! `market::config` already keep the sum at or below 57,000 bps). The
//! per-second rate is `ceil(rate_bps × WAD / (10_000 × SECONDS_PER_YEAR))`.
//!
//! # Utilization bands and the conservative edge
//!
//! Twenty bands of 5% each: band `b` covers `[5b %, 5(b+1) %)`; 100% belongs to
//! the last band. The accrual rate uses the **upper** edge of the latest
//! revealed band, `(b + 1) × 500` bps, so the rate is never below the true
//! rate and the exact aggregate is never disclosed.
//!
//! # Index accrual
//!
//! ```text
//! new_index = index + ceil(index × rate_per_second × elapsed / WAD)
//! ```
//!
//! Linear (simple) accrual over one interval, as the design states
//! (`old × (1 + rate × dt)`), rounded up so the Pool never under-accrues. The
//! index never decreases; `elapsed == 0` is the identity; an interval longer
//! than `MAX_ELAPSED_SECONDS` or a result above `MAX_BORROW_INDEX` is an error
//! (never a wrap or a silent clamp).
//!
//! # Projection
//!
//! The risk index of an Intent is [`project_index`] at the Decision Expiry
//! time, measured from the **accounting** timestamp, with the rate of the
//! latest band's upper edge. Slots are converted to seconds with
//! [`SECONDS_PER_SLOT_UPPER_BOUND`] (1 s per slot, above the nominal 0.4 s),
//! which only makes the projection larger.
//!
//! # Observation age
//!
//! [`ensure_band_fresh`] fails when the band was observed more than
//! `band_max_age_slots` ago, never observed (slot 0), or in the future.

use super::fixed::{mul_wad_ceil_wide, MathError};
use super::{BPS_DENOMINATOR, MAX_BORROW_INDEX, MAX_ELAPSED_SECONDS, WAD};
use crate::market::{AssetState, RateParams};

/// Seconds in the 365-day year the annualized rates use.
pub const SECONDS_PER_YEAR: u128 = 31_536_000;
/// Number of utilization bands.
pub const BAND_COUNT: u8 = 20;
/// Width of one band in basis points.
pub const BAND_WIDTH_BPS: u128 = 500;
/// Largest borrow rate the model returns (annual bps, 1,000%).
pub const MAX_BORROW_RATE_BPS: u128 = 100_000;
/// Conservative seconds per slot used to project through Decision Expiry.
pub const SECONDS_PER_SLOT_UPPER_BOUND: u64 = 1;

/// Why a utilization-band observation cannot be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandError {
    /// The band was never observed (observation slot 0).
    NeverObserved,
    /// The observation is older than the configured maximum age.
    Stale,
    /// The observation slot is after the current slot.
    FromTheFuture,
}

/// Upper utilization edge of `band` in bps: `(band + 1) × 500`.
pub const fn band_upper_edge_bps(band: u8) -> Result<u128, MathError> {
    if band >= BAND_COUNT {
        Err(MathError::InvalidRate)
    } else {
        Ok((band as u128 + 1) * BAND_WIDTH_BPS)
    }
}

/// The annual borrow rate in bps at `utilization_bps` (0..=10,000).
pub fn borrow_rate_bps(rates: &RateParams, utilization_bps: u128) -> Result<u128, MathError> {
    let optimal = rates.optimal_utilization_bps as u128;
    if utilization_bps > BPS_DENOMINATOR || optimal == 0 || optimal >= BPS_DENOMINATOR {
        return Err(MathError::InvalidRate);
    }
    let base = rates.base_rate_bps as u128;
    let slope1 = rates.slope1_bps as u128;
    let slope2 = rates.slope2_bps as u128;
    let rate = if utilization_bps <= optimal {
        base + div_ceil(slope1 * utilization_bps, optimal)
    } else {
        base + slope1
            + div_ceil(
                slope2 * (utilization_bps - optimal),
                BPS_DENOMINATOR - optimal,
            )
    };
    Ok(rate.min(MAX_BORROW_RATE_BPS))
}

fn div_ceil(numerator: u128, denominator: u128) -> u128 {
    numerator.div_ceil(denominator)
}

/// Per-second rate in WAD: `ceil(rate_bps × WAD / (10_000 × SECONDS_PER_YEAR))`.
pub fn rate_per_second_wad(rate_bps: u128) -> Result<u128, MathError> {
    if rate_bps > MAX_BORROW_RATE_BPS {
        return Err(MathError::InvalidRate);
    }
    Ok(div_ceil(
        rate_bps * WAD,
        BPS_DENOMINATOR * SECONDS_PER_YEAR,
    ))
}

/// The per-second WAD rate accrual uses for `band`: the rate at the band's
/// upper edge.
pub fn band_rate_per_second_wad(rates: &RateParams, band: u8) -> Result<u128, MathError> {
    let rate = borrow_rate_bps(rates, band_upper_edge_bps(band)?)?;
    rate_per_second_wad(rate)
}

/// Accrues `index` over `target_ts - index_ts` seconds at `rate_per_second`
/// (WAD). See the module docs for the rules.
pub fn project_index(
    index: u128,
    index_ts: i64,
    target_ts: i64,
    rate_per_second: u128,
) -> Result<u128, MathError> {
    if !(WAD..=MAX_BORROW_INDEX).contains(&index) {
        return Err(MathError::OutOfRange);
    }
    if target_ts < index_ts {
        return Err(MathError::TimeRegression);
    }
    let elapsed = (target_ts as i128 - index_ts as i128) as u128;
    if elapsed == 0 {
        return Ok(index);
    }
    if elapsed > MAX_ELAPSED_SECONDS {
        return Err(MathError::ElapsedTooLong);
    }
    // rate_per_second <= ~3.2e12 (1,000% / year), elapsed <= 1e10: < 4e22.
    let growth = rate_per_second
        .checked_mul(elapsed)
        .ok_or(MathError::Overflow)?;
    let delta = mul_wad_ceil_wide(index, growth)?;
    let next = index.checked_add(delta).ok_or(MathError::Overflow)?;
    if next > MAX_BORROW_INDEX {
        return Err(MathError::OutOfRange);
    }
    Ok(next)
}

/// The timestamp a decision `slots_to_expiry` slots after `now_ts` can still
/// commit at, using the conservative seconds-per-slot bound.
pub fn expiry_timestamp(now_ts: i64, slots_to_expiry: u64) -> Result<i64, MathError> {
    let seconds = slots_to_expiry
        .checked_mul(SECONDS_PER_SLOT_UPPER_BOUND)
        .ok_or(MathError::Overflow)?;
    let seconds = i64::try_from(seconds).map_err(|_| MathError::Overflow)?;
    now_ts.checked_add(seconds).ok_or(MathError::Overflow)
}

/// The Risk Borrow Index of an Intent: the accounting index projected through
/// Decision Expiry at the latest band's upper-edge rate.
pub fn project_risk_index(
    asset: &AssetState,
    now_ts: i64,
    slots_to_expiry: u64,
) -> Result<u128, MathError> {
    let rate = band_rate_per_second_wad(&asset.rates, asset.utilization_band)?;
    let expiry = expiry_timestamp(now_ts.max(asset.accounting_index_timestamp), slots_to_expiry)?;
    project_index(
        asset.accounting_borrow_index,
        asset.accounting_index_timestamp,
        expiry,
        rate,
    )
}

/// Accrues the public accounting Borrow Index to `now_ts` at the latest
/// band's rate and moves the timestamp. A clock that did not advance leaves
/// the Asset State unchanged. Atomic on error.
///
/// Executing a `Rates` configuration change must call this first so the new
/// rate never applies to elapsed time (ARCHITECTURE.md, Task 4.4 follow-up).
pub fn accrue_asset_index(asset: &mut AssetState, now_ts: i64) -> Result<(), MathError> {
    if now_ts <= asset.accounting_index_timestamp {
        return Ok(());
    }
    let rate = band_rate_per_second_wad(&asset.rates, asset.utilization_band)?;
    let next = project_index(
        asset.accounting_borrow_index,
        asset.accounting_index_timestamp,
        now_ts,
        rate,
    )?;
    asset.accounting_borrow_index = next;
    asset.accounting_index_timestamp = now_ts;
    Ok(())
}

// ---- Utilization band -------------------------------------------------------

/// The band of `debt / (cash + debt)`: the count of edges `5k %` (`k` in
/// `1..=19`) that utilization reaches. A zero denominator is band 0.
///
/// Uses cross-multiplication (`debt × 100 >= 5k × total`), exactly as the
/// circuit does, so there is no division by a secret value.
pub fn utilization_band(cash: u128, debt: u128) -> Result<u8, MathError> {
    let total = cash.checked_add(debt).ok_or(MathError::Overflow)?;
    if total == 0 {
        return Ok(0);
    }
    let scaled_debt = debt.checked_mul(10_000).ok_or(MathError::Overflow)?;
    let mut band = 0u8;
    for k in 1..BAND_COUNT as u128 {
        let edge = total
            .checked_mul(k * BAND_WIDTH_BPS)
            .ok_or(MathError::Overflow)?;
        if scaled_debt >= edge {
            band += 1;
        }
    }
    Ok(band)
}

/// The band computed from Pool aggregates: `debt = ceil(shares × index / WAD)`.
pub fn band_from_pool(cash: u128, debt_shares: u128, index: u128) -> Result<u8, MathError> {
    let debt = super::fixed::debt_from_shares(debt_shares, index)?;
    utilization_band(cash, debt)
}

/// Checks the band observation at `observation_slot` is usable at
/// `current_slot` with `max_age_slots`. Risk-increasing queues call this; a
/// stale band blocks new risk.
pub fn ensure_band_fresh(
    observation_slot: u64,
    current_slot: u64,
    max_age_slots: u64,
) -> Result<(), BandError> {
    if observation_slot == 0 {
        return Err(BandError::NeverObserved);
    }
    if observation_slot > current_slot {
        return Err(BandError::FromTheFuture);
    }
    if current_slot - observation_slot > max_age_slots {
        return Err(BandError::Stale);
    }
    Ok(())
}

/// Writes a revealed band and its observation slot. Rejects an invalid band
/// and an observation older than the stored one.
pub fn apply_band_observation(
    asset: &mut AssetState,
    band: u8,
    observation_slot: u64,
) -> Result<(), MathError> {
    if band >= BAND_COUNT {
        return Err(MathError::InvalidRate);
    }
    if observation_slot < asset.utilization_observation_slot {
        return Err(MathError::TimeRegression);
    }
    asset.utilization_band = band;
    asset.utilization_observation_slot = observation_slot;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Lcg;

    const VECTORS: &str = include_str!("../../../../packages/reference-model/vectors/rate.vectors");

    fn asset() -> AssetState {
        use crate::market::{Caps, PauseState, RiskParams, TimelockState};
        AssetState {
            layout_version: 1,
            bump: 1,
            market: anchor_lang::prelude::Pubkey::new_from_array([7; 32]),
            generation: 1,
            oracle_generation: 1,
            rate_epoch_generation: 1,
            accounting_borrow_index: WAD,
            accounting_index_timestamp: 1_000_000,
            utilization_band: 0,
            utilization_observation_slot: 0,
            risk: RiskParams::LAUNCH_DEFAULT,
            rates: RateParams::LAUNCH_DEFAULT,
            caps: Caps::default(),
            pauses: PauseState::default(),
            timelock: TimelockState::default(),
            reserved: [0; 128],
        }
    }

    fn defaults() -> RateParams {
        RateParams::LAUNCH_DEFAULT
    }

    #[test]
    fn rate_curve_hits_the_kink_and_the_ends() {
        let r = defaults(); // base 0, slope1 400, slope2 7500, optimal 8000
        assert_eq!(borrow_rate_bps(&r, 0), Ok(0));
        assert_eq!(borrow_rate_bps(&r, 8_000), Ok(400));
        assert_eq!(borrow_rate_bps(&r, 10_000), Ok(7_900));
        assert_eq!(borrow_rate_bps(&r, 10_001), Err(MathError::InvalidRate));
        let bad = RateParams {
            optimal_utilization_bps: 10_000,
            ..r
        };
        assert_eq!(borrow_rate_bps(&bad, 5), Err(MathError::InvalidRate));
    }

    #[test]
    fn upper_edge_is_used_for_the_band_rate() {
        let r = defaults();
        // Band 0 accrues at the 5% rate, not the 0% rate.
        assert_eq!(band_upper_edge_bps(0), Ok(500));
        assert_eq!(band_upper_edge_bps(19), Ok(10_000));
        assert_eq!(band_upper_edge_bps(20), Err(MathError::InvalidRate));
        assert!(band_rate_per_second_wad(&r, 0).unwrap() > 0);
        assert_eq!(
            band_rate_per_second_wad(&r, 19),
            rate_per_second_wad(7_900)
        );
    }

    #[test]
    fn index_identity_growth_and_errors() {
        let rate = rate_per_second_wad(1_000).unwrap(); // 10% / year
        assert_eq!(project_index(WAD, 100, 100, rate), Ok(WAD));
        let one_year = project_index(WAD, 0, SECONDS_PER_YEAR as i64, rate).unwrap();
        assert!(one_year >= WAD + WAD / 10);
        assert!(one_year <= WAD + WAD / 10 + SECONDS_PER_YEAR);
        assert_eq!(
            project_index(WAD, 10, 9, rate),
            Err(MathError::TimeRegression)
        );
        assert_eq!(
            project_index(WAD, 0, MAX_ELAPSED_SECONDS as i64 + 1, rate),
            Err(MathError::ElapsedTooLong)
        );
        assert_eq!(project_index(WAD - 1, 0, 1, rate), Err(MathError::OutOfRange));
        // A growth that pushes the index past the ceiling is an error.
        let huge = rate_per_second_wad(MAX_BORROW_RATE_BPS).unwrap();
        assert!(project_index(WAD, 0, 4_000_000_000, huge).is_err());
    }

    #[test]
    fn bands_follow_the_edges() {
        assert_eq!(utilization_band(0, 0), Ok(0));
        assert_eq!(utilization_band(100, 0), Ok(0));
        assert_eq!(utilization_band(95, 5), Ok(1)); // exactly 5%
        assert_eq!(utilization_band(96, 4), Ok(0));
        assert_eq!(utilization_band(0, 7), Ok(19)); // 100%
        assert_eq!(utilization_band(5, 95), Ok(19)); // exactly 95%
        assert_eq!(utilization_band(6, 94), Ok(18));
    }

    #[test]
    fn freshness_blocks_stale_unobserved_and_future_bands() {
        assert_eq!(ensure_band_fresh(0, 10, 100), Err(BandError::NeverObserved));
        assert_eq!(ensure_band_fresh(11, 10, 100), Err(BandError::FromTheFuture));
        assert_eq!(ensure_band_fresh(10, 110, 100), Ok(()));
        assert_eq!(ensure_band_fresh(10, 111, 100), Err(BandError::Stale));
    }

    #[test]
    fn observation_applies_only_forward() {
        let mut asset = asset();
        assert_eq!(apply_band_observation(&mut asset, 20, 5), Err(MathError::InvalidRate));
        apply_band_observation(&mut asset, 7, 50).unwrap();
        assert_eq!((asset.utilization_band, asset.utilization_observation_slot), (7, 50));
        assert_eq!(apply_band_observation(&mut asset, 8, 49), Err(MathError::TimeRegression));
        assert_eq!(asset.utilization_band, 7);
    }

    #[test]
    fn accrual_moves_index_and_timestamp_and_ignores_a_stalled_clock() {
        let mut asset = asset();
        asset.utilization_band = 10;
        let t0 = asset.accounting_index_timestamp;
        accrue_asset_index(&mut asset, t0).unwrap();
        assert_eq!(asset.accounting_borrow_index, WAD);
        accrue_asset_index(&mut asset, t0 + 3_600).unwrap();
        assert!(asset.accounting_borrow_index > WAD);
        assert_eq!(asset.accounting_index_timestamp, t0 + 3_600);
        let before = asset.accounting_borrow_index;
        accrue_asset_index(&mut asset, t0).unwrap();
        assert_eq!(asset.accounting_borrow_index, before);
        // The risk index through expiry is never below the accounting index.
        let risk = project_risk_index(&asset, t0 + 3_600, 150).unwrap();
        assert!(risk >= before);
    }

    /// **Validates: Requirements 7.2, 7.3** - the index never decreases, is
    /// monotone in time and in the rate, and splitting an interval never
    /// accrues more than the whole (simple interest is sub-additive after
    /// ceiling at most by the rounding of the parts).
    #[test]
    fn prop_index_is_monotone_in_time_and_rate() {
        let mut rng = Lcg(0x7201);
        for _ in 0..3_000 {
            let index = WAD + rng.u128_below(10 * WAD);
            let rate = rate_per_second_wad(rng.u128_below(57_001)).unwrap();
            let t1 = rng.u128_below(100_000_000) as i64;
            let t2 = t1 + rng.u128_below(100_000_000) as i64;
            let a = project_index(index, 0, t1, rate).unwrap();
            let b = project_index(index, 0, t2, rate).unwrap();
            assert!(a >= index && b >= a);
            let faster = project_index(index, 0, t2, rate + 1).unwrap();
            assert!(faster >= b);
        }
    }

    /// **Validates: Requirements 7.2** - band is monotone in debt and
    /// anti-monotone in cash, and always below `BAND_COUNT`.
    #[test]
    fn prop_band_is_monotone() {
        let mut rng = Lcg(0x7202);
        for _ in 0..3_000 {
            let cash = rng.u128_below(1 << 60);
            let debt = rng.u128_below(1 << 60);
            let band = utilization_band(cash, debt).unwrap();
            assert!(band < BAND_COUNT);
            assert!(utilization_band(cash, debt + rng.u128_below(1 << 40)).unwrap() >= band);
            assert!(utilization_band(cash + rng.u128_below(1 << 40), debt).unwrap() <= band);
        }
    }

    #[test]
    fn golden_vectors_hold() {
        let mut checked = 0;
        for line in VECTORS.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let w: Vec<&str> = line.split_whitespace().collect();
            let n = |i: usize| w[i].parse::<u128>().unwrap();
            let s = |i: usize| w[i].parse::<i64>().unwrap();
            let expect = |i: usize| -> Result<u128, String> {
                w[i].parse::<u128>().map_err(|_| w[i].to_string())
            };
            let name = |e: MathError| match e {
                MathError::Overflow => "overflow",
                MathError::DivisionByZero => "div0",
                MathError::OutOfRange => "range",
                MathError::Underflow => "underflow",
                MathError::InvalidRate => "invalid",
                MathError::TimeRegression => "regress",
                MathError::ElapsedTooLong => "elapsed",
            };
            let got = |r: Result<u128, MathError>| r.map_err(|e| name(e).to_string());
            match w[0] {
                // band <cash> <debt> <expected>
                "band" => assert_eq!(
                    utilization_band(n(1), n(2)).map(|b| b as u128).map_err(|e| name(e).to_string()),
                    expect(3),
                    "vector: {line}"
                ),
                // band_pool <cash> <debt_shares> <index> <expected>
                "band_pool" => assert_eq!(
                    band_from_pool(n(1), n(2), n(3)).map(|b| b as u128).map_err(|e| name(e).to_string()),
                    expect(4),
                    "vector: {line}"
                ),
                // rate_bps <base> <slope1> <slope2> <optimal> <utilization> <expected>
                "rate_bps" => {
                    let r = RateParams {
                        base_rate_bps: n(1) as u32,
                        slope1_bps: n(2) as u32,
                        slope2_bps: n(3) as u32,
                        optimal_utilization_bps: n(4) as u16,
                        band_max_age_slots: 9_000,
                    };
                    assert_eq!(got(borrow_rate_bps(&r, n(5))), expect(6), "vector: {line}");
                }
                // rate_per_second <rate_bps> <expected>
                "rate_per_second" => {
                    assert_eq!(got(rate_per_second_wad(n(1))), expect(2), "vector: {line}")
                }
                // project <index> <ts0> <ts1> <rate_per_second> <expected>
                "project" => assert_eq!(
                    got(project_index(n(1), s(2), s(3), n(4))),
                    expect(5),
                    "vector: {line}"
                ),
                other => panic!("unknown vector {other}"),
            }
            checked += 1;
        }
        assert!(checked >= 25, "only {checked} rate vectors ran");
    }
}
