//! `LendingPool` (walnut-lend): public totals, 1e9-scale interest indexes and
//! the oracle configuration of the lending market.
//!
//! PDA `["lending_pool", market]`. Layout (offsets include the 8-byte
//! discriminator): `market` 8..40, `bump` 40, `oracle` 41..73, `feed_id`
//! 73..105, `max_price_age_seconds` 105..109, `total_supply_shares` 109..125,
//! `total_debt_shares` 125..141, `supply_index_1e9` 141..157,
//! `borrow_index_1e9` 157..173, `last_accrual_ts` 173..181, `reserves_usdc`
//! 181..189, `rate_source` 189, `reserved` 190..253. [`LendingPool::ACCOUNT_LEN`]
//! is 253 bytes. `rate_source` took the first reserved byte (zero before), so
//! existing accounts read as [`RATE_SOURCE_TOTALS`].

use anchor_lang::prelude::*;
use core::result::Result;

use crate::market::{AssetState, RateParams};
use crate::math::fixed::{mul_div_floor, MathError};
use crate::math::lend::{check_index_1e9, debt_value, supply_value, INDEX_SCALE, MAX_INDEX_1E9};
use crate::math::rate::{borrow_rate_bps, project_index, rate_per_second_wad};
use crate::math::{BPS_DENOMINATOR, WAD};

/// PDA seed prefix.
pub const LENDING_POOL_SEED: &[u8] = b"lending_pool";

/// Rates follow the public totals (walnut-lend path).
pub const RATE_SOURCE_TOTALS: u8 = 0;
/// Rates follow the revealed utilization band of the encrypted Pool
/// (ADR-0004): borrowers at the band's upper edge, suppliers at its lower edge.
pub const RATE_SOURCE_BAND: u8 = 1;

/// Ratio between the WAD (1e18) used by `math::rate` and the lending index
/// scale (1e9).
const WAD_PER_INDEX: u128 = WAD / INDEX_SCALE;

/// Public lending pool state.
#[account]
#[derive(InitSpace)]
pub struct LendingPool {
    /// Owning Market.
    pub market: Pubkey,
    /// PDA bump.
    pub bump: u8,
    /// Pyth SOL/USD price account.
    pub oracle: Pubkey,
    /// Pyth feed id.
    pub feed_id: [u8; 32],
    /// Maximum accepted price age (devnet: 86_400).
    pub max_price_age_seconds: u32,
    /// Total supply shares.
    pub total_supply_shares: u128,
    /// Total debt shares.
    pub total_debt_shares: u128,
    /// Supply index, `1e9` = 1.0.
    pub supply_index_1e9: u128,
    /// Borrow index, `1e9` = 1.0.
    pub borrow_index_1e9: u128,
    /// Timestamp of the last accrual.
    pub last_accrual_ts: i64,
    /// Protocol reserves in USDC base units.
    pub reserves_usdc: u64,
    /// [`RATE_SOURCE_TOTALS`] or [`RATE_SOURCE_BAND`].
    pub rate_source: u8,
    /// Reserved for later layout additions.
    pub reserved: [u8; 63],
}

impl LendingPool {
    /// Account data length including Anchor's 8-byte discriminator.
    pub const ACCOUNT_LEN: usize = 8 + Self::INIT_SPACE;

    /// A fresh pool: both indexes 1.0, no shares.
    pub fn new(
        market: Pubkey,
        bump: u8,
        oracle: Pubkey,
        feed_id: [u8; 32],
        max_price_age_seconds: u32,
        now: i64,
    ) -> Self {
        Self {
            market,
            bump,
            oracle,
            feed_id,
            max_price_age_seconds,
            total_supply_shares: 0,
            total_debt_shares: 0,
            supply_index_1e9: INDEX_SCALE,
            borrow_index_1e9: INDEX_SCALE,
            last_accrual_ts: now,
            reserves_usdc: 0,
            rate_source: RATE_SOURCE_TOTALS,
            reserved: [0; 63],
        }
    }

    /// Current supply value in USDC base units.
    pub fn total_supply_value(&self) -> Result<u128, MathError> {
        supply_value(self.total_supply_shares, self.supply_index_1e9)
    }

    /// Current debt value in USDC base units.
    pub fn total_debt_value(&self) -> Result<u128, MathError> {
        debt_value(self.total_debt_shares, self.borrow_index_1e9)
    }

    /// `min(10_000, debt_value * 10_000 / supply_value)`; 0 when nothing is supplied.
    pub fn utilization_bps(&self) -> Result<u128, MathError> {
        let supply = self.total_supply_value()?;
        if supply == 0 {
            return Ok(0);
        }
        let debt = self.total_debt_value()?;
        Ok(mul_div_floor(debt, BPS_DENOMINATOR, supply)?.min(BPS_DENOMINATOR))
    }

    /// Accrues with the configured [`LendingPool::rate_source`]; the band is
    /// the Asset State's last revealed utilization band.
    pub fn accrue_auto(&mut self, now: i64, asset: &AssetState) -> Result<(), MathError> {
        if self.rate_source == RATE_SOURCE_BAND {
            self.accrue_banded(
                now,
                &asset.rates,
                asset.risk.reserve_factor_bps,
                asset.utilization_band,
            )
        } else {
            self.accrue(now, &asset.rates, asset.risk.reserve_factor_bps)
        }
    }

    /// Band-mode accrual (ADR-0004). The borrow index grows at the kinked rate
    /// of the band's upper edge (rounded up). The supply index grows at
    /// `rate(lower edge) * lower edge * (1 - reserve factor)` (rounded down), so
    /// suppliers are never credited more than borrowers pay; the difference
    /// stays in the encrypted Pool as reserves. Public totals are not read.
    /// On error the pool is left unchanged.
    pub fn accrue_banded(
        &mut self,
        now: i64,
        rates: &RateParams,
        reserve_factor_bps: u16,
        band: u8,
    ) -> Result<(), MathError> {
        if now < self.last_accrual_ts {
            return Err(MathError::TimeRegression);
        }
        let rf = reserve_factor_bps as u128;
        if rf > BPS_DENOMINATOR {
            return Err(MathError::InvalidRate);
        }
        check_index_1e9(self.borrow_index_1e9)?;
        check_index_1e9(self.supply_index_1e9)?;
        if now == self.last_accrual_ts {
            return Ok(());
        }
        let band = u128::from(band.min(19));
        let upper = ((band + 1) * 500).min(BPS_DENOMINATOR);
        let lower = band * 500;
        let lift = |index: u128| index.checked_mul(WAD_PER_INDEX).ok_or(MathError::Overflow);
        let borrow_rate = borrow_rate_bps(rates, upper)?;
        let projected = project_index(
            lift(self.borrow_index_1e9)?,
            self.last_accrual_ts,
            now,
            rate_per_second_wad(borrow_rate)?,
        )?;
        let new_borrow = projected.div_ceil(WAD_PER_INDEX).max(self.borrow_index_1e9);
        let lower_rate = borrow_rate_bps(rates, lower)?;
        let supply_rate = mul_div_floor(
            mul_div_floor(lower_rate, lower, BPS_DENOMINATOR)?,
            BPS_DENOMINATOR - rf,
            BPS_DENOMINATOR,
        )?;
        let new_supply = if supply_rate == 0 {
            self.supply_index_1e9
        } else {
            let projected = project_index(
                lift(self.supply_index_1e9)?,
                self.last_accrual_ts,
                now,
                rate_per_second_wad(supply_rate)?,
            )?;
            (projected / WAD_PER_INDEX).max(self.supply_index_1e9)
        };
        if new_borrow > MAX_INDEX_1E9 || new_supply > MAX_INDEX_1E9 {
            return Err(MathError::OutOfRange);
        }
        self.borrow_index_1e9 = new_borrow;
        self.supply_index_1e9 = new_supply;
        self.last_accrual_ts = now;
        Ok(())
    }

    /// Accrues interest up to `now`. Borrow index grows at the kinked rate for
    /// the current utilization (rounded up); interest is split between
    /// reserves (`reserve_factor_bps`) and suppliers (the rest, via the supply
    /// index). Both indexes never decrease and stay within `[1e9, 1e10]`.
    /// On error the pool is left unchanged.
    pub fn accrue(
        &mut self,
        now: i64,
        rates: &RateParams,
        reserve_factor_bps: u16,
    ) -> Result<(), MathError> {
        if now < self.last_accrual_ts {
            return Err(MathError::TimeRegression);
        }
        if reserve_factor_bps as u128 > BPS_DENOMINATOR {
            return Err(MathError::InvalidRate);
        }
        check_index_1e9(self.borrow_index_1e9)?;
        check_index_1e9(self.supply_index_1e9)?;
        if now == self.last_accrual_ts {
            return Ok(());
        }
        let supply_val = self.total_supply_value()?;
        let debt_before = self.total_debt_value()?;
        let rate_bps = borrow_rate_bps(rates, self.utilization_bps()?)?;
        let per_second = rate_per_second_wad(rate_bps)?;

        // `project_index` works at 1e18 scale; lift, project, and round up.
        let lifted = self
            .borrow_index_1e9
            .checked_mul(WAD_PER_INDEX)
            .ok_or(MathError::Overflow)?;
        let projected = project_index(lifted, self.last_accrual_ts, now, per_second)?;
        let new_borrow = projected.div_ceil(WAD_PER_INDEX).max(self.borrow_index_1e9);
        if new_borrow > MAX_INDEX_1E9 {
            return Err(MathError::OutOfRange);
        }

        let debt_after = debt_value(self.total_debt_shares, new_borrow)?;
        let interest = debt_after.saturating_sub(debt_before);
        let reserve_credit = mul_div_floor(interest, reserve_factor_bps as u128, BPS_DENOMINATOR)?;
        let supplier_interest = interest - reserve_credit;
        let new_supply = if supply_val == 0 {
            self.supply_index_1e9
        } else {
            let growth = mul_div_floor(self.supply_index_1e9, supplier_interest, supply_val)?;
            self.supply_index_1e9
                .checked_add(growth)
                .ok_or(MathError::Overflow)?
        };
        if new_supply > MAX_INDEX_1E9 {
            return Err(MathError::OutOfRange);
        }
        let new_reserves = (self.reserves_usdc as u128)
            .checked_add(reserve_credit)
            .ok_or(MathError::Overflow)?;
        let new_reserves = crate::math::fixed::to_u64(new_reserves)?;

        self.borrow_index_1e9 = new_borrow;
        self.supply_index_1e9 = new_supply;
        self.reserves_usdc = new_reserves;
        self.last_accrual_ts = now;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Lcg;

    fn pool(supply: u128, debt: u128) -> LendingPool {
        let mut p = LendingPool::new(
            Pubkey::default(),
            255,
            Pubkey::default(),
            [0; 32],
            86_400,
            0,
        );
        p.total_supply_shares = supply;
        p.total_debt_shares = debt;
        p
    }

    #[test]
    fn account_len_is_253() {
        assert_eq!(LendingPool::ACCOUNT_LEN, 253);
    }

    #[test]
    fn empty_pool_does_not_move() {
        let mut p = pool(0, 0);
        p.accrue(1_000_000, &RateParams::LAUNCH_DEFAULT, 1_000)
            .unwrap();
        assert_eq!(p.borrow_index_1e9, INDEX_SCALE);
        assert_eq!(p.supply_index_1e9, INDEX_SCALE);
        assert_eq!(p.utilization_bps().unwrap(), 0);
    }

    #[test]
    fn interest_accrues_and_splits() {
        let mut p = pool(1_000_000_000_000, 800_000_000_000); // 80% util
        let rates = RateParams::LAUNCH_DEFAULT;
        p.accrue(31_536_000, &rates, 1_000).unwrap(); // one year
        assert!(p.borrow_index_1e9 > INDEX_SCALE);
        assert!(p.supply_index_1e9 > INDEX_SCALE);
        assert!(p.reserves_usdc > 0);
        assert_eq!(p.last_accrual_ts, 31_536_000);
        // time regression is rejected and leaves the pool unchanged
        let before = (p.borrow_index_1e9, p.supply_index_1e9, p.reserves_usdc);
        assert!(p.accrue(1, &rates, 1_000).is_err());
        assert_eq!(
            before,
            (p.borrow_index_1e9, p.supply_index_1e9, p.reserves_usdc)
        );
    }

    /// Feature: walnut-lend, Property 5: Indexes are monotonic and interest is conserved.
    /// **Validates: Requirements 4.1, 4.2, 4.3**
    #[test]
    fn prop_indexes_monotonic_and_interest_conserved() {
        let mut rng = Lcg(55);
        let rates = RateParams::LAUNCH_DEFAULT;
        for _ in 0..2_000 {
            let supply = 1 + rng.u128_below(1_000_000_000_000_000);
            let debt = rng.u128_below(supply + 1);
            let rf = rng.u128_below(10_001) as u16;
            let mut p = pool(supply, debt);
            let dt = rng.u128_below(100_000_000) as i64;
            let (b0, s0, r0) = (p.borrow_index_1e9, p.supply_index_1e9, p.reserves_usdc);
            let debt0 = p.total_debt_value().unwrap();
            let supply0 = p.total_supply_value().unwrap();
            p.accrue(dt, &rates, rf).unwrap();
            assert!(p.borrow_index_1e9 >= b0 && p.borrow_index_1e9 >= INDEX_SCALE);
            assert!(p.supply_index_1e9 >= s0 && p.supply_index_1e9 >= INDEX_SCALE);
            let interest = p.total_debt_value().unwrap() - debt0;
            let reserve_credit = (p.reserves_usdc - r0) as u128;
            let supplier_gain = p.total_supply_value().unwrap() - supply0;
            // suppliers + reserves never receive more than borrowers pay (+1 rounding unit)
            assert!(supplier_gain + reserve_credit <= interest + 1);
            // and reserves equal the floor share of interest
            assert_eq!(reserve_credit, interest * rf as u128 / BPS_DENOMINATOR);
        }
    }

    #[test]
    fn existing_accounts_read_as_the_totals_rate_source() {
        // `rate_source` replaced the first (zero) reserved byte.
        assert_eq!(pool(0, 0).rate_source, RATE_SOURCE_TOTALS);
        assert_eq!(LendingPool::ACCOUNT_LEN, 253);
    }

    #[test]
    fn band_zero_grows_only_the_borrow_index() {
        let mut p = pool(0, 0);
        p.accrue_banded(31_536_000, &RateParams::LAUNCH_DEFAULT, 1_000, 0)
            .unwrap();
        assert!(p.borrow_index_1e9 > INDEX_SCALE);
        assert_eq!(p.supply_index_1e9, INDEX_SCALE);
        assert_eq!(p.reserves_usdc, 0);
    }

    #[test]
    fn banded_accrual_rejects_time_regression_unchanged() {
        let mut p = pool(0, 0);
        p.accrue_banded(100, &RateParams::LAUNCH_DEFAULT, 1_000, 10)
            .unwrap();
        let before = (p.borrow_index_1e9, p.supply_index_1e9, p.last_accrual_ts);
        assert!(p
            .accrue_banded(50, &RateParams::LAUNCH_DEFAULT, 1_000, 10)
            .is_err());
        assert_eq!(
            before,
            (p.borrow_index_1e9, p.supply_index_1e9, p.last_accrual_ts)
        );
    }

    /// ADR-0004: for any real utilization inside the revealed band, suppliers
    /// are credited at most what borrowers pay (+1 rounding unit), and both
    /// indexes are monotonic.
    #[test]
    fn prop_banded_suppliers_never_over_credited() {
        let mut rng = Lcg(77);
        let rates = RateParams::LAUNCH_DEFAULT;
        for _ in 0..2_000 {
            let band = rng.u128_below(20) as u8;
            let supply = 1_000_000 + rng.u128_below(1_000_000_000_000_000);
            // a debt whose utilization lies inside the band
            let lo = supply * u128::from(band) * 500 / 10_000;
            let hi = (supply * (u128::from(band) + 1) * 500 / 10_000).min(supply);
            let debt = lo + rng.u128_below(hi - lo + 1);
            let rf = rng.u128_below(10_001) as u16;
            let dt = 1 + rng.u128_below(100_000_000) as i64;
            let mut p = pool(0, 0);
            let (b0, s0) = (p.borrow_index_1e9, p.supply_index_1e9);
            p.accrue_banded(dt, &rates, rf, band).unwrap();
            assert!(p.borrow_index_1e9 >= b0 && p.supply_index_1e9 >= s0);
            let paid = debt_value(debt, p.borrow_index_1e9).unwrap() - debt;
            let credited = supply_value(supply, p.supply_index_1e9).unwrap() - supply;
            assert!(
                credited <= paid + 1,
                "band {band} credited {credited} > paid {paid}"
            );
        }
    }

    /// Golden vectors shared with packages/sdk/test/private-op.test.ts.
    #[test]
    fn banded_accrual_golden_vectors() {
        let rates = RateParams::LAUNCH_DEFAULT;
        for (band, rf, dt, borrow, supply) in [
            (
                15u8,
                1_000u16,
                31_536_000i64,
                1_040_000_001u128,
                1_025_200_000u128,
            ),
            (3, 500, 86_400, 1_000_027_398, 1_000_002_739),
            (19, 0, 3_600, 1_000_090_183, 1_000_065_331),
        ] {
            let mut p = pool(0, 0);
            p.accrue_banded(dt, &rates, rf, band).unwrap();
            assert_eq!((p.borrow_index_1e9, p.supply_index_1e9), (borrow, supply));
        }
    }

    #[test]
    fn accrue_auto_follows_the_rate_source() {
        let mut asset = crate::state::pool_mutation::tests::asset();
        asset.utilization_band = 15;
        let mut totals = pool(1_000_000, 0);
        totals.accrue_auto(31_536_000, &asset).unwrap();
        // no debt: the totals path leaves the supply index flat
        assert_eq!(totals.supply_index_1e9, INDEX_SCALE);
        let mut banded = pool(1_000_000, 0);
        banded.rate_source = RATE_SOURCE_BAND;
        banded.accrue_auto(31_536_000, &asset).unwrap();
        // band 15: the supply index grows from the band's lower edge
        assert!(banded.supply_index_1e9 > INDEX_SCALE);
    }
}
