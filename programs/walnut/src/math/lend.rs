//! Walnut-Lend public share maths and the plain-Rust mirror of the `lend_op`
//! circuit (`encrypted-ixs/src/lib.rs`, task 1.1).
//!
//! The lending indexes use the 9-decimal scale [`INDEX_SCALE`] (`1e9`), not the
//! 18-decimal WAD of [`super::rate`]. Division only happens here, on public
//! values; the circuit only multiplies and compares.
//!
//! Rounding always favours the pool: borrow mint and withdraw-supply burn round
//! up, repay burn and supply shares round down.

use super::fixed::{mul_div_ceil, mul_div_floor, to_u64, MathError};

/// Scale of the lending indexes: `1.0 == 1e9`.
pub const INDEX_SCALE: u128 = 1_000_000_000;
/// Largest index the Program accepts (`10.0`).
pub const MAX_INDEX_1E9: u128 = 10_000_000_000;
/// Largest price (USDC base units per SOL) passed to the circuit.
pub const MAX_PRICE: u128 = 10_000_000_000;
/// Largest collateral per position in lamports (bounds the circuit products).
pub const MAX_COLLATERAL_LAMPORTS: u128 = 1_000_000_000_000_000;
const BPS: u128 = 10_000;
const U64_MAX: u128 = u64::MAX as u128;

/// Circuit op tags.
pub const OP_ADD_COLLATERAL: u64 = 0;
/// Withdraw collateral.
pub const OP_WITHDRAW_COLLATERAL: u64 = 1;
/// Borrow.
pub const OP_BORROW: u64 = 2;
/// Repay.
pub const OP_REPAY: u64 = 3;
/// Supply.
pub const OP_SUPPLY: u64 = 4;
/// Withdraw supply.
pub const OP_WITHDRAW_SUPPLY: u64 = 5;
/// Health check.
pub const OP_HEALTH_CHECK: u64 = 6;
/// Liquidate.
pub const OP_LIQUIDATE: u64 = 7;

/// Rejects an index outside `[1e9, 1e10]`.
pub fn check_index_1e9(index: u128) -> Result<u128, MathError> {
    if (INDEX_SCALE..=MAX_INDEX_1E9).contains(&index) {
        Ok(index)
    } else {
        Err(MathError::OutOfRange)
    }
}

/// Clamps a normalized price to [`MAX_PRICE`]; rejects zero.
pub fn clamp_price(price: u128) -> Result<u64, MathError> {
    if price == 0 {
        return Err(MathError::OutOfRange);
    }
    to_u64(price.min(MAX_PRICE))
}

/// Borrow: `mint = ceil(amount * 1e9 / borrow_index)`.
pub fn borrow_mint(amount: u64, borrow_index: u128) -> Result<u64, MathError> {
    check_index_1e9(borrow_index)?;
    to_u64(mul_div_ceil(amount as u128, INDEX_SCALE, borrow_index)?)
}

/// Repay: `burn = floor(amount * 1e9 / borrow_index)`.
pub fn repay_burn(amount: u64, borrow_index: u128) -> Result<u64, MathError> {
    check_index_1e9(borrow_index)?;
    to_u64(mul_div_floor(amount as u128, INDEX_SCALE, borrow_index)?)
}

/// Amount that clears `debt_shares` exactly: `ceil(shares * index / 1e9)`.
pub fn full_repay_amount(debt_shares: u64, borrow_index: u128) -> Result<u64, MathError> {
    check_index_1e9(borrow_index)?;
    to_u64(mul_div_ceil(
        debt_shares as u128,
        borrow_index,
        INDEX_SCALE,
    )?)
}

/// Supply: `shares = floor(amount * 1e9 / supply_index)`.
pub fn supply_shares_for(amount: u64, supply_index: u128) -> Result<u64, MathError> {
    check_index_1e9(supply_index)?;
    to_u64(mul_div_floor(amount as u128, INDEX_SCALE, supply_index)?)
}

/// Withdraw supply: `burn = ceil(amount * 1e9 / supply_index)`.
pub fn withdraw_supply_burn(amount: u64, supply_index: u128) -> Result<u64, MathError> {
    check_index_1e9(supply_index)?;
    to_u64(mul_div_ceil(amount as u128, INDEX_SCALE, supply_index)?)
}

/// USDC value of supply shares: `floor(shares * supply_index / 1e9)`.
pub fn supply_value(shares: u128, supply_index: u128) -> Result<u128, MathError> {
    mul_div_floor(shares, supply_index, INDEX_SCALE)
}

/// USDC value of debt shares: `floor(shares * borrow_index / 1e9)`.
pub fn debt_value(shares: u128, borrow_index: u128) -> Result<u128, MathError> {
    mul_div_floor(shares, borrow_index, INDEX_SCALE)
}

/// Liquidation debt burn: `floor(repay * 1e9 / borrow_index)`.
pub fn liquidation_burn(repay: u64, borrow_index: u128) -> Result<u64, MathError> {
    repay_burn(repay, borrow_index)
}

/// Liquidation seize in lamports:
/// `floor(repay * 1e9 * (10_000 + bonus_bps) / (price * 10_000))`.
pub fn liquidation_seize(repay: u64, bonus_bps: u16, price: u64) -> Result<u64, MathError> {
    if price == 0 {
        return Err(MathError::DivisionByZero);
    }
    let numerator = (repay as u128)
        .checked_mul(INDEX_SCALE)
        .and_then(|v| v.checked_mul(BPS + bonus_bps as u128))
        .ok_or(MathError::Overflow)?;
    let denominator = (price as u128) * BPS;
    to_u64(numerator / denominator)
}

/// Decrypted Position balances of the lending circuit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LendBalances {
    /// Free SOL (lamports).
    pub free_sol: u64,
    /// Free USDC (base units).
    pub free_usdc: u64,
    /// Collateral SOL (lamports).
    pub collateral_sol: u64,
    /// Supply shares.
    pub supply_shares: u128,
    /// Debt shares.
    pub debt_shares: u128,
}

/// Public circuit arguments besides `op`, `amount`, `shares`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LendCtx {
    /// Borrow index (1e9 scale).
    pub borrow_index_1e9: u64,
    /// USDC base units per 1 SOL.
    pub price: u64,
    /// Loan-to-value (bps).
    pub ltv_bps: u64,
    /// Liquidation threshold (bps).
    pub liq_threshold_bps: u64,
    /// Per-position debt cap (USDC base units).
    pub position_debt_cap: u64,
}

/// Plain-Rust mirror of the `lend_op` circuit. Returns the new balances and
/// `applied` (for op 6, `applied == unhealthy`); a refusal returns the input
/// state unchanged. Mirrors the circuit's `u128` arithmetic exactly; callers
/// must respect the bounds in the design (shares <= 1e12, index <= 1e10,
/// price <= 1e10, collateral <= 1e15).
pub fn lend_op_rules(
    state: LendBalances,
    op: u64,
    amount: u64,
    shares: u64,
    ctx: LendCtx,
) -> (LendBalances, bool) {
    let a = amount as u128;
    let sh = shares as u128;
    let idx = ctx.borrow_index_1e9 as u128;
    let px = ctx.price as u128;
    let free_sol = state.free_sol as u128;
    let free_usdc = state.free_usdc as u128;
    let coll = state.collateral_sol as u128;
    let sup = state.supply_shares;
    let debt = state.debt_shares;

    let debt_value = debt * idx * BPS;
    let unhealthy = debt_value > coll * px * (ctx.liq_threshold_bps as u128);

    let ok0 = a <= free_sol;
    let fits1 = a <= coll;
    let a1 = if fits1 { a } else { 0 };
    let ok1 = fits1 && debt_value <= (coll - a1) * px * (ctx.ltv_bps as u128);
    let new_debt2 = debt + sh;
    let ok2 = new_debt2 * idx * BPS <= coll * px * (ctx.ltv_bps as u128)
        && new_debt2 * idx <= (ctx.position_debt_cap as u128) * INDEX_SCALE
        && free_usdc + a <= U64_MAX;
    let ok3 = sh <= debt && a <= free_usdc;
    let ok4 = a <= free_usdc && sup + sh <= U64_MAX;
    let ok5 = sh <= sup && free_usdc + a <= U64_MAX;
    let ok7 = unhealthy && sh <= debt && (sh * 2 <= debt || sh == debt) && a <= coll;

    let applied = (op == OP_ADD_COLLATERAL && ok0)
        || (op == OP_WITHDRAW_COLLATERAL && ok1)
        || (op == OP_BORROW && ok2)
        || (op == OP_REPAY && ok3)
        || (op == OP_SUPPLY && ok4)
        || (op == OP_WITHDRAW_SUPPLY && ok5)
        || (op == OP_HEALTH_CHECK && unhealthy)
        || (op == OP_LIQUIDATE && ok7);
    let mutates = applied && op != OP_HEALTH_CHECK;
    let da = if mutates { a } else { 0 };
    let ds = if mutates { sh } else { 0 };

    let mut s = state;
    s.free_sol = match op {
        OP_ADD_COLLATERAL => free_sol - da,
        OP_WITHDRAW_COLLATERAL => free_sol + da,
        _ => free_sol,
    } as u64;
    s.collateral_sol = match op {
        OP_ADD_COLLATERAL => coll + da,
        OP_WITHDRAW_COLLATERAL | OP_LIQUIDATE => coll - da,
        _ => coll,
    } as u64;
    s.free_usdc = match op {
        OP_BORROW | OP_WITHDRAW_SUPPLY => free_usdc + da,
        OP_REPAY | OP_SUPPLY => free_usdc - da,
        _ => free_usdc,
    } as u64;
    s.debt_shares = match op {
        OP_BORROW => debt + ds,
        OP_REPAY | OP_LIQUIDATE => debt - ds,
        _ => debt,
    };
    s.supply_shares = match op {
        OP_SUPPLY => sup + ds,
        OP_WITHDRAW_SUPPLY => sup - ds,
        _ => sup,
    };
    (s, applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Lcg;

    const CASES: u64 = 2_000;

    fn ctx(index: u64, price: u64) -> LendCtx {
        LendCtx {
            borrow_index_1e9: index,
            price,
            ltv_bps: 7_500,
            liq_threshold_bps: 8_000,
            position_debt_cap: 1_000_000_000_000,
        }
    }

    fn random_state(rng: &mut Lcg) -> LendBalances {
        LendBalances {
            free_sol: rng.u128_below(1_000_000_000_000_000) as u64,
            free_usdc: rng.u128_below(1_000_000_000_000) as u64,
            collateral_sol: rng.u128_below(1_000_000_000_000_000) as u64,
            supply_shares: rng.u128_below(1_000_000_000_000),
            debt_shares: rng.u128_below(1_000_000_000_000),
        }
    }

    fn random_ctx(rng: &mut Lcg) -> LendCtx {
        ctx(
            (INDEX_SCALE + rng.u128_below(MAX_INDEX_1E9 - INDEX_SCALE)) as u64,
            (1 + rng.u128_below(MAX_PRICE)) as u64,
        )
    }

    #[test]
    fn share_rounding_directions() {
        // index 1.5: 10 / 1.5 = 6.67
        let idx = 1_500_000_000;
        assert_eq!(borrow_mint(10, idx).unwrap(), 7);
        assert_eq!(repay_burn(10, idx).unwrap(), 6);
        assert_eq!(supply_shares_for(10, idx).unwrap(), 6);
        assert_eq!(withdraw_supply_burn(10, idx).unwrap(), 7);
        assert_eq!(borrow_mint(10, 500).unwrap_err(), MathError::OutOfRange);
    }

    #[test]
    fn seize_matches_formula() {
        // repay 100 USDC, 5% bonus, price 150 USDC/SOL => 0.7 SOL
        let seize = liquidation_seize(100_000_000, 500, 150_000_000).unwrap();
        assert_eq!(seize, 700_000_000);
        assert!(liquidation_seize(1, 0, 0).is_err());
    }

    #[test]
    fn ltv_boundary_for_borrow() {
        let s = LendBalances {
            collateral_sol: 1_000_000_000,
            ..Default::default()
        };
        let c = ctx(1_000_000_000, 100_000_000); // 100 USDC/SOL, ltv 75%
        let (n, ok) = lend_op_rules(s, OP_BORROW, 75_000_000, 75_000_000, c);
        assert!(ok);
        assert_eq!(n.debt_shares, 75_000_000);
        assert_eq!(n.free_usdc, 75_000_000);
        let (n, ok) = lend_op_rules(s, OP_BORROW, 75_000_001, 75_000_001, c);
        assert!(!ok);
        assert_eq!(n, s);
    }

    #[test]
    fn health_check_never_mutates() {
        let s = LendBalances {
            collateral_sol: 1_000_000_000,
            debt_shares: 90_000_000,
            ..Default::default()
        };
        let c = ctx(1_000_000_000, 100_000_000);
        let (n, unhealthy) = lend_op_rules(s, OP_HEALTH_CHECK, 5, 5, c);
        assert!(unhealthy);
        assert_eq!(n, s);
    }

    /// Feature: walnut-lend, Property 1: Refusal leaves the Position unchanged.
    /// **Validates: Requirements 3.3, 5.5, 5.6, 2.2**
    #[test]
    fn prop_refusal_leaves_state_unchanged() {
        let mut rng = Lcg(11);
        for _ in 0..CASES {
            let s = random_state(&mut rng);
            let c = random_ctx(&mut rng);
            let op = rng.u128_below(8) as u64;
            let amount = rng.u128_below(1_000_000_000_000_000) as u64;
            let shares = rng.u128_below(1_000_000_000_000) as u64;
            let (n, applied) = lend_op_rules(s, op, amount, shares, c);
            if !applied || op == OP_HEALTH_CHECK {
                assert_eq!(n, s);
            }
        }
    }

    /// Feature: walnut-lend, Property 2: Borrow and withdraw respect the LTV.
    /// **Validates: Requirements 1.1, 4.6, 9.4**
    #[test]
    fn prop_ltv_holds_after_borrow_or_withdraw() {
        let mut rng = Lcg(22);
        for _ in 0..CASES {
            let mut s = random_state(&mut rng);
            s.collateral_sol = rng.u128_below(1_000_000_000_000_000) as u64;
            s.debt_shares = rng.u128_below(1_000_000);
            let c = random_ctx(&mut rng);
            let op = if rng.next().is_multiple_of(2) {
                OP_BORROW
            } else {
                OP_WITHDRAW_COLLATERAL
            };
            let amount = rng.u128_below(s.collateral_sol as u128 + 1) as u64;
            let shares = rng.u128_below(1_000_000_000) as u64;
            let (n, applied) = lend_op_rules(s, op, amount, shares, c);
            if applied {
                let lhs = n.debt_shares * c.borrow_index_1e9 as u128 * BPS;
                let rhs = n.collateral_sol as u128 * c.price as u128 * c.ltv_bps as u128;
                assert!(lhs <= rhs);
                if op == OP_BORROW {
                    assert!(
                        n.debt_shares * c.borrow_index_1e9 as u128
                            <= c.position_debt_cap as u128 * INDEX_SCALE
                    );
                }
            }
        }
    }

    /// Feature: walnut-lend, Property 3: Full repay clears debt exactly.
    /// **Validates: Requirements 4.6, 10.2**
    #[test]
    fn prop_full_repay_clears_debt() {
        let mut rng = Lcg(33);
        for _ in 0..CASES {
            let shares = rng.u128_below(1_000_000_000_000) as u64;
            let idx = INDEX_SCALE + rng.u128_below(MAX_INDEX_1E9 - INDEX_SCALE);
            let amount = full_repay_amount(shares, idx).unwrap();
            assert_eq!(repay_burn(amount, idx).unwrap(), shares);
        }
    }

    /// Feature: walnut-lend, Property 4: Supply then withdraw never creates value.
    /// **Validates: Requirements 3.1, 3.2, 4.3, 10.4**
    #[test]
    fn prop_supply_withdraw_never_creates_value() {
        let mut rng = Lcg(44);
        for _ in 0..CASES {
            let amount = rng.u128_below(1_000_000_000_000) as u64;
            let idx = INDEX_SCALE + rng.u128_below(MAX_INDEX_1E9 - INDEX_SCALE);
            let minted = supply_shares_for(amount, idx).unwrap();
            // Value of the minted shares at the same index never exceeds the deposit.
            assert!(supply_value(minted as u128, idx).unwrap() <= amount as u128);
            // Index growth never lowers the redeemable value.
            let grown = (idx + idx / 1_000).min(MAX_INDEX_1E9);
            let back = supply_value(minted as u128, grown).unwrap();
            assert!(back >= supply_value(minted as u128, idx).unwrap());
            // Burning shares for a withdrawal of the same amount never exceeds the minted shares.
            let burn = withdraw_supply_burn(amount, idx).unwrap();
            assert!(burn >= minted);
        }
    }

    /// Feature: walnut-lend, Property 6: Liquidation only when unhealthy and within the close factor.
    /// **Validates: Requirements 5.1, 5.2, 5.3, 5.4, 5.5, 5.6**
    #[test]
    fn prop_liquidation_rules() {
        let mut rng = Lcg(66);
        for _ in 0..CASES {
            let mut s = random_state(&mut rng);
            s.debt_shares = rng.u128_below(1_000_000_000);
            let c = random_ctx(&mut rng);
            let burn = rng.u128_below(s.debt_shares + 2) as u64;
            let seize = rng.u128_below(s.collateral_sol as u128 + 2) as u64;
            let (n, applied) = lend_op_rules(s, OP_LIQUIDATE, seize, burn, c);
            if applied {
                let unhealthy = s.debt_shares * c.borrow_index_1e9 as u128 * BPS
                    > s.collateral_sol as u128 * c.price as u128 * c.liq_threshold_bps as u128;
                assert!(unhealthy);
                assert!(burn as u128 * 2 <= s.debt_shares || burn as u128 == s.debt_shares);
                assert_eq!(n.collateral_sol, s.collateral_sol - seize);
                assert_eq!(n.debt_shares, s.debt_shares - burn as u128);
            } else {
                assert_eq!(n, s);
            }
        }
    }

    /// Feature: walnut-lend, Property 11: Pool totals match applied operations.
    /// **Validates: Requirements 3.5, 4.6**
    #[test]
    fn prop_totals_match_applied_ops() {
        let mut rng = Lcg(111);
        for _ in 0..200 {
            let mut s = LendBalances {
                collateral_sol: 1_000_000_000_000,
                ..Default::default()
            };
            let c = ctx(
                1_000_000_000 + rng.u128_below(1_000_000_000) as u64,
                100_000_000,
            );
            let (mut debt_total, mut sup_total) = (0u128, 0u128);
            for _ in 0..20 {
                let amount = 1 + rng.u128_below(1_000_000) as u64;
                let idx = c.borrow_index_1e9 as u128;
                let (op, shares) = match rng.next() % 4 {
                    0 => (OP_BORROW, borrow_mint(amount, idx).unwrap()),
                    1 => (OP_REPAY, repay_burn(amount, idx).unwrap()),
                    2 => (OP_SUPPLY, supply_shares_for(amount, idx).unwrap()),
                    _ => (
                        OP_WITHDRAW_SUPPLY,
                        withdraw_supply_burn(amount, idx).unwrap(),
                    ),
                };
                let (n, applied) = lend_op_rules(s, op, amount, shares, c);
                if applied {
                    match op {
                        OP_BORROW => debt_total += shares as u128,
                        OP_REPAY => debt_total -= shares as u128,
                        OP_SUPPLY => sup_total += shares as u128,
                        _ => sup_total -= shares as u128,
                    }
                }
                s = n;
                assert_eq!(s.debt_shares, debt_total);
                assert_eq!(s.supply_shares, sup_total);
            }
        }
    }
}
