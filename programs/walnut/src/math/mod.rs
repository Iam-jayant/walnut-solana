//! Committed arithmetic range table (Task 6.3).
//!
//! Every later numeric step (oracle normalization in [`crate::oracle`], the
//! health and liquidation algebra in Tasks 9 and 12, and the Pool accounting in
//! Task 7) works on values drawn from a small set of domains. This module
//! *commits* the maximum of each domain as a constant and proves, in host
//! tests, that the worst-case intermediate product of the formulas the design
//! fixes stays inside `u128`. Downstream code that keeps its inputs inside
//! these bounds can then use checked arithmetic and never hit an overflow it did
//! not anticipate.
//!
//! The table is deliberately conservative: the hard ceilings are the on-chain
//! representable maxima (`u64::MAX` for any token balance, because every balance
//! is a `u64`), and the derived prices/indexes are bounded well below the point
//! where a formula would exceed `u128`. The design's health model is:
//!
//! ```text
//! sol_value_down(lamports) = floor(lamports × normalized_sol_price_low / 10^9)
//! usdc_value_up(units)     = ceil (units   × normalized_usdc_price_high / 10^6)
//! debt_units(shares, idx)  = ceil (shares  × idx / WAD)
//! ```
//!
//! so the two products that can grow largest are `token_amount × normalized_price`
//! and `shares × index`. The proof tests below pin both.
//!
//! No value here is a tunable parameter; these are invariants of the number
//! system. Governed risk parameters live in [`crate::market`].

pub mod checkpoint;
pub mod fixed;
pub mod rate;
pub mod supply;

// Task 7.6: Pool accounting property tests over the real math.
#[cfg(test)]
mod properties;

/// Deterministic linear-congruential sampler for the `prop_*` tests (the
/// workspace has no property-testing crate).
#[cfg(test)]
pub(crate) struct Lcg(pub u64);

#[cfg(test)]
impl Lcg {
    pub fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 11
    }

    /// Uniform-ish value in `0..n` (`n > 0`), up to 128 bits.
    pub fn u128_below(&mut self, n: u128) -> u128 {
        let wide =
            ((self.next() as u128) << 64) ^ ((self.next() as u128) << 11) ^ self.next() as u128;
        wide % n
    }
}

/// WAD fixed-point scale of the Borrow Index (1.0 = 10^18). Re-exported from
/// [`crate::market::WAD`] so the range proofs and the index domain agree.
pub const WAD: u128 = crate::market::WAD;

/// The single committed USD fixed-point scale. A normalized price of
/// `USD_SCALE` represents exactly 1.0 USD per whole token, so every normalized
/// collateral and debt value the risk math produces is expressed in these
/// units. 10^8 matches Pyth's usual exponent (-8), which keeps normalization of
/// a feed with exponent -8 an identity on the mantissa.
pub const USD_SCALE: u128 = 100_000_000; // 10^8

/// Hard ceiling of any token balance. Both wSOL lamports and USDC base units
/// are stored in a `u64` token account, so no balance can exceed this.
pub const MAX_TOKEN_AMOUNT: u128 = u64::MAX as u128;

/// SOL has 9 decimals; lamports convert to whole SOL by dividing by this.
pub const SOL_DECIMALS_POW: u128 = 1_000_000_000; // 10^9
/// USDC has 6 decimals; base units convert to whole USDC by dividing by this.
pub const USDC_DECIMALS_POW: u128 = 1_000_000; // 10^6

/// Smallest accepted normalized price. A conservative (floored) collateral
/// price that rounds to zero is rejected rather than used, so collateral can
/// never be valued from a sub-unit price.
pub const MIN_NORMALIZED_PRICE: u128 = 1;

/// Largest accepted normalized price: 10^18, i.e. 10^10 USD per whole token at
/// `USD_SCALE`. Any feed that normalizes above this is treated as malformed.
/// Chosen so `MAX_TOKEN_AMOUNT × MAX_NORMALIZED_PRICE` stays inside `u128`
/// (proved by [`tests::value_product_fits_u128`]).
pub const MAX_NORMALIZED_PRICE: u128 = 1_000_000_000_000_000_000; // 10^18

/// Lowest Pyth exponent the normalizer accepts. More negative than this would
/// demand dividing by more than 10^12 and only ever rounds a real price to
/// zero.
pub const MIN_PYTH_EXPONENT: i32 = -12;
/// Highest Pyth exponent the normalizer accepts.
pub const MAX_PYTH_EXPONENT: i32 = 12;

/// Largest accepted Borrow Index: 10^3 × WAD, i.e. a 1,000× cumulative interest
/// multiplier. The index starts at `WAD` (1.0) and only grows; Task 7's bounded
/// kinked rate model keeps it below this ceiling over any realistic lifetime
/// (1,000× is already far beyond reachable accrual). The ceiling is chosen with
/// [`MAX_SHARES`] so the naive `shares × index` product stays inside `u128`,
/// which lets the Task 7.1 WAD mul/div use checked `u128` arithmetic without a
/// 256-bit intermediate.
pub const MAX_BORROW_INDEX: u128 = 1_000 * WAD; // 10^3 × 10^18 = 10^21

/// Largest accepted share count (supply or debt shares). Shares scale above the
/// underlying token amount for early-depositor precision, so this ceiling sits
/// above a realistic capped supply rather than at `u64::MAX`. Task 7's supply
/// and debt caps keep live share balances within it; a computation that would
/// exceed it returns zero rather than wrapping. Chosen with [`MAX_BORROW_INDEX`]
/// so `shares × index` fits `u128` (proved by [`tests::shares_times_index_fits_u128`]).
pub const MAX_SHARES: u128 = 10_000_000_000_000_000; // 10^16

/// Largest elapsed time (seconds) an interest projection will accept in one
/// step. ~317 years; a longer gap is treated as a misconfiguration upstream
/// rather than compounded.
pub const MAX_ELAPSED_SECONDS: u128 = 10_000_000_000;

/// Basis-point denominator (100% = 10,000 bps).
pub const BPS_DENOMINATOR: u128 = 10_000;

/// `10^exp` as a `u128`, or `None` on overflow (`exp > 38`).
pub const fn pow10(exp: u32) -> Option<u128> {
    let mut value: u128 = 1;
    let mut i = 0;
    while i < exp {
        match value.checked_mul(10) {
            Some(next) => value = next,
            None => return None,
        }
        i += 1;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pow10_matches_direct_powers_and_saturates() {
        assert_eq!(pow10(0), Some(1));
        assert_eq!(pow10(8), Some(USD_SCALE));
        assert_eq!(pow10(18), Some(WAD));
        assert_eq!(pow10(38), Some(10u128.pow(38)));
        // 10^39 overflows u128.
        assert_eq!(pow10(39), None);
    }

    #[test]
    fn scales_match_the_design() {
        assert_eq!(USD_SCALE, 10u128.pow(8));
        assert_eq!(WAD, 10u128.pow(18));
        assert_eq!(SOL_DECIMALS_POW, 10u128.pow(9));
        assert_eq!(USDC_DECIMALS_POW, 10u128.pow(6));
        assert_eq!(MAX_TOKEN_AMOUNT, u64::MAX as u128);
    }

    /// The largest value product in the health model,
    /// `token_amount × normalized_price`, must fit in `u128` before the
    /// decimal division, with margin.
    #[test]
    fn value_product_fits_u128() {
        let product = MAX_TOKEN_AMOUNT
            .checked_mul(MAX_NORMALIZED_PRICE)
            .expect("token × price stays in u128");
        // A comfortable margin below the u128 ceiling (at least 10×).
        assert!(product <= u128::MAX / 10);
        // The final USD-scaled value after dividing by the SOL decimals still
        // fits (it is strictly smaller than the product).
        let value = product / SOL_DECIMALS_POW;
        assert!(value < u128::MAX);
    }

    /// `shares × index` (debt-units and NAV math) must fit in `u128` within the
    /// committed domain, so the Task 7.1 WAD mul/div stays in checked `u128`.
    #[test]
    fn shares_times_index_fits_u128() {
        let product = MAX_SHARES
            .checked_mul(MAX_BORROW_INDEX)
            .expect("shares × index stays in u128");
        // 10^16 × 10^21 = 10^37, comfortably below the u128 ceiling (~3.4e38).
        assert!(product <= u128::MAX / 3);
        // Dividing back by WAD yields a debt-unit count within the token domain.
        let units = product / WAD;
        assert!(units < u128::MAX);
    }

    /// A bps-weighted value (`value × bps / 10_000`) never overflows for any
    /// value up to the largest SOL collateral value.
    #[test]
    fn bps_weighting_fits_u128() {
        let max_value = MAX_TOKEN_AMOUNT * MAX_NORMALIZED_PRICE / SOL_DECIMALS_POW;
        let weighted = max_value
            .checked_mul(BPS_DENOMINATOR)
            .expect("value × bps stays in u128");
        assert!(weighted <= u128::MAX / 10);
    }

    /// The index domain is monotone and anchored at 1.0.
    #[test]
    fn index_domain_is_anchored_and_bounded() {
        const { assert!(MIN_NORMALIZED_PRICE <= MAX_NORMALIZED_PRICE) };
        const { assert!(WAD <= MAX_BORROW_INDEX) };
        assert_eq!(MAX_BORROW_INDEX, 10u128.pow(21));
        assert_eq!(MAX_SHARES, 10u128.pow(16));
        // The index ceiling exceeds the largest share count, so the index is
        // the dominant term of the binding `shares × index` product.
        const { assert!(MAX_SHARES < MAX_BORROW_INDEX) };
    }

    /// The exponent window is symmetric and within the pow10 domain.
    #[test]
    fn pyth_exponent_window_is_representable() {
        const { assert!(MIN_PYTH_EXPONENT < 0) };
        const { assert!(MAX_PYTH_EXPONENT > 0) };
        assert!(pow10(MAX_PYTH_EXPONENT.unsigned_abs()).is_some());
        assert!(pow10(MIN_PYTH_EXPONENT.unsigned_abs()).is_some());
    }

    /// Elapsed-time times a per-second rate fraction stays well inside u128.
    #[test]
    fn elapsed_time_times_wad_fits_u128() {
        // A per-second rate is at most WAD (100%/s is absurd but bounded);
        // elapsed × WAD must still fit.
        let product = MAX_ELAPSED_SECONDS
            .checked_mul(WAD)
            .expect("elapsed × WAD stays in u128");
        assert!(product <= u128::MAX / 10);
    }
}
