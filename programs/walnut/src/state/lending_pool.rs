//! `LendingPool` (walnut-lend): public totals, 1e9-scale interest indexes and
//! the oracle configuration of the lending market.
//!
//! PDA `["lending_pool", market]`. Layout (offsets include the 8-byte
//! discriminator): `market` 8..40, `bump` 40, `oracle` 41..73, `feed_id`
//! 73..105, `max_price_age_seconds` 105..109, `total_supply_shares` 109..125,
//! `total_debt_shares` 125..141, `supply_index_1e9` 141..157,
//! `borrow_index_1e9` 157..173, `last_accrual_ts` 173..181, `reserves_usdc`
//! 181..189, `reserved` 189..253. [`LendingPool::ACCOUNT_LEN`] is 253 bytes.

use anchor_lang::prelude::*;
use core::result::Result;

use crate::market::RateParams;
use crate::math::fixed::{mul_div_floor, MathError};
use crate::math::lend::{check_index_1e9, debt_value, supply_value, INDEX_SCALE, MAX_INDEX_1E9};
use crate::math::rate::{borrow_rate_bps, project_index, rate_per_second_wad};
use crate::math::{BPS_DENOMINATOR, WAD};

/// PDA seed prefix.
pub const LENDING_POOL_SEED: &[u8] = b"lending_pool";

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
    /// Reserved for later layout additions.
    pub reserved: [u8; 64],
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
            reserved: [0; 64],
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
}
