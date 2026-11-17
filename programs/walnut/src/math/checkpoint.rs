//! Encrypted Pool interest checkpoint, plaintext reference (Task 7.3).
//!
//! The checkpoint is the FIRST step of every Pool-mutating circuit (supply,
//! withdrawal, borrow, repay, reserve withdrawal, partial liquidation, Absorb)
//! and runs before any share mutation. `encrypted-ixs/src/lib.rs`
//! (`circuits::checkpoint_pool_values` and `checkpoint_pool`) is the circuit
//! form; this module is its plaintext definition. Both are exercised by
//! `packages/reference-model/vectors/checkpoint.vectors`, and
//! `packages/reference-model/src/checkpoint.ts` mirrors it.
//!
//! ```text
//! old_debt        = ceil(total_debt_shares × accounted_index  / WAD)
//! new_debt        = ceil(total_debt_shares × accounting_index / WAD)
//! interest        = new_debt - old_debt                   (>= 0: ceil is monotone)
//! reserve_add     = floor(interest × reserve_factor_bps / 10_000)
//! supplier_part   = interest - reserve_add                (reserve + suppliers = interest, exactly)
//! reserves'       = reserves + reserve_add
//! accounted_index'= accounting_index
//! supplier_equity = cash + new_debt - reserves'           (the NAV identity)
//! ```
//!
//! Cash does not change at a checkpoint: interest is a claim on borrowers
//! until repaid, and the NAV counts it through `new_debt`. Rounding: debt up
//! (the Pool never under-counts what is owed), reserve allocation down (the
//! remainder stays with suppliers).
//!
//! # Failure
//!
//! Anything outside the committed domain is an error here and a `ok == false`
//! outcome in the circuit (state returned unchanged, so the caller resolves
//! `Zero` and moves nothing): an index outside `[WAD, MAX_BORROW_INDEX]`, an
//! accounting index below the accounted one (the index never decreases), debt
//! shares above `MAX_SHARES`, cash or reserves above [`MAX_POOL_VALUE`], a
//! reserve factor above 10,000 bps, or reserves above `cash + new_debt`.

use super::fixed::{check_index, debt_from_shares, mul_div_floor, MathError};
use super::BPS_DENOMINATOR;

/// Largest Pool cash or reserves value the checkpoint accepts (`2^96`). Far
/// above any `u64`-backed vault, and small enough that sums and the
/// band-comparison products stay inside `u128`.
pub const MAX_POOL_VALUE: u128 = 1 << 96;

/// The decrypted Pool payload (plaintext twin of `PoolSecret`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolPlain {
    /// USDC Cash held for the Pool.
    pub cash: u128,
    /// Total supply shares outstanding.
    pub total_supply_shares: u128,
    /// Total debt shares outstanding.
    pub total_debt_shares: u128,
    /// Protocol reserves.
    pub reserves: u128,
}

/// The result of one checkpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    /// The Pool payload after the checkpoint (only `reserves` can change).
    pub pool: PoolPlain,
    /// Debt units at the previously accounted index.
    pub old_debt: u128,
    /// Debt units at the new accounting index.
    pub new_debt: u128,
    /// Borrower interest accrued since the last checkpoint.
    pub interest: u128,
    /// The part of `interest` added to reserves.
    pub reserve_add: u128,
    /// The part of `interest` that stays with suppliers.
    pub supplier_interest: u128,
    /// Encrypted supplier equity (NAV): `cash + new_debt - reserves'`.
    pub supplier_equity: u128,
}

/// Supplier equity `cash + debt - reserves`. Fails when reserves exceed
/// `cash + debt`.
pub fn supplier_equity(cash: u128, debt: u128, reserves: u128) -> Result<u128, MathError> {
    cash.checked_add(debt)
        .ok_or(MathError::Overflow)?
        .checked_sub(reserves)
        .ok_or(MathError::Underflow)
}

/// Runs the checkpoint (see the module docs). Pure and atomic.
pub fn checkpoint(
    pool: &PoolPlain,
    accounted_index: u128,
    accounting_index: u128,
    reserve_factor_bps: u16,
) -> Result<Checkpoint, MathError> {
    check_index(accounted_index)?;
    check_index(accounting_index)?;
    if accounting_index < accounted_index {
        return Err(MathError::TimeRegression);
    }
    if reserve_factor_bps as u128 > BPS_DENOMINATOR {
        return Err(MathError::InvalidRate);
    }
    if pool.cash > MAX_POOL_VALUE || pool.reserves > MAX_POOL_VALUE {
        return Err(MathError::OutOfRange);
    }
    let old_debt = debt_from_shares(pool.total_debt_shares, accounted_index)?;
    let new_debt = debt_from_shares(pool.total_debt_shares, accounting_index)?;
    let interest = new_debt - old_debt;
    let reserve_add = mul_div_floor(interest, reserve_factor_bps as u128, BPS_DENOMINATOR)?;
    let reserves = pool
        .reserves
        .checked_add(reserve_add)
        .ok_or(MathError::Overflow)?;
    let equity = supplier_equity(pool.cash, new_debt, reserves)?;
    Ok(Checkpoint {
        pool: PoolPlain { reserves, ..*pool },
        old_debt,
        new_debt,
        interest,
        reserve_add,
        supplier_interest: interest - reserve_add,
        supplier_equity: equity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{Lcg, MAX_BORROW_INDEX, MAX_SHARES, WAD};

    const VECTORS: &str =
        include_str!("../../../../packages/reference-model/vectors/checkpoint.vectors");

    fn pool(cash: u128, debt_shares: u128, reserves: u128) -> PoolPlain {
        PoolPlain {
            cash,
            total_supply_shares: 1_000,
            total_debt_shares: debt_shares,
            reserves,
        }
    }

    #[test]
    fn identity_for_zero_debt_and_zero_elapsed() {
        let p = pool(500, 0, 7);
        let c = checkpoint(&p, WAD, 2 * WAD, 1_000).unwrap();
        assert_eq!((c.interest, c.reserve_add), (0, 0));
        assert_eq!(c.pool, p);
        assert_eq!(c.supplier_equity, 500 - 7);
        let q = pool(500, 300, 7);
        let c = checkpoint(&q, 3 * WAD / 2, 3 * WAD / 2, 1_000).unwrap();
        assert_eq!(c.pool, q);
        assert_eq!(c.interest, 0);
    }

    #[test]
    fn interest_is_split_and_rounded_as_designed() {
        // 1000 shares, index 1.0 -> 1.1: debt 1000 -> 1100, interest 100,
        // reserve 10% = 10.
        let c = checkpoint(&pool(900, 1_000, 0), WAD, WAD + WAD / 10, 1_000).unwrap();
        assert_eq!((c.old_debt, c.new_debt, c.interest), (1_000, 1_100, 100));
        assert_eq!((c.reserve_add, c.supplier_interest), (10, 90));
        assert_eq!(c.pool.reserves, 10);
        assert_eq!(c.supplier_equity, 900 + 1_100 - 10);
        // Reserve allocation rounds down: 9 × 10% = 0.9 -> 0.
        let c = checkpoint(&pool(0, 10, 0), WAD, WAD * 19 / 10, 1_000).unwrap();
        assert_eq!(c.interest, 9);
        assert_eq!(c.reserve_add, 0);
    }

    #[test]
    fn invalid_inputs_are_rejected() {
        let p = pool(10, 10, 0);
        assert_eq!(checkpoint(&p, 2 * WAD, WAD, 0), Err(MathError::TimeRegression));
        assert_eq!(checkpoint(&p, WAD - 1, WAD, 0), Err(MathError::OutOfRange));
        assert_eq!(
            checkpoint(&p, WAD, MAX_BORROW_INDEX + 1, 0),
            Err(MathError::OutOfRange)
        );
        assert_eq!(checkpoint(&p, WAD, WAD, 10_001), Err(MathError::InvalidRate));
        assert_eq!(
            checkpoint(&pool(0, MAX_SHARES + 1, 0), WAD, WAD, 0),
            Err(MathError::OutOfRange)
        );
        assert_eq!(
            checkpoint(&pool(MAX_POOL_VALUE + 1, 0, 0), WAD, WAD, 0),
            Err(MathError::OutOfRange)
        );
        assert_eq!(checkpoint(&pool(0, 0, 5), WAD, WAD, 0), Err(MathError::Underflow));
    }

    /// **Validates: Requirements 7.3, 7.4** - over random pools and index
    /// pairs: reserve + supplier interest equals total interest exactly; the
    /// NAV identity holds; debt rounds up and is monotone; reserves never
    /// decrease; supplier equity grows by exactly the supplier share of
    /// interest; a second checkpoint at the same index is the identity.
    #[test]
    fn prop_checkpoint_conserves_interest_and_nav() {
        let mut rng = Lcg(0x7301);
        for _ in 0..4_000 {
            let shares = rng.u128_below(MAX_SHARES + 1);
            let accounted = WAD + rng.u128_below(MAX_BORROW_INDEX / 4);
            let accounting = accounted + rng.u128_below(MAX_BORROW_INDEX / 4);
            let rf = rng.u128_below(10_001) as u16;
            let before_debt = debt_from_shares(shares, accounted).unwrap();
            let cash = rng.u128_below(1 << 80);
            let reserves = rng.u128_below((cash + before_debt).min(1 << 80) + 1);
            let p = pool(cash, shares, reserves);
            let before_equity = supplier_equity(cash, before_debt, reserves).unwrap();

            let c = checkpoint(&p, accounted, accounting, rf).unwrap();
            assert_eq!(c.reserve_add + c.supplier_interest, c.interest);
            assert_eq!(c.new_debt - c.old_debt, c.interest);
            assert!(c.pool.reserves >= reserves);
            assert_eq!(c.pool.cash, cash);
            assert_eq!(
                c.supplier_equity,
                cash + c.new_debt - c.pool.reserves,
                "NAV identity"
            );
            assert_eq!(c.supplier_equity, before_equity + c.supplier_interest);
            // Debt is rounded up: never below the exact quotient.
            assert!(c.new_debt * WAD >= shares * accounting);
            assert!(c.new_debt >= c.old_debt);
            // Idempotent at the same index.
            let again = checkpoint(&c.pool, accounting, accounting, rf).unwrap();
            assert_eq!(again.pool, c.pool);
            assert_eq!(again.supplier_equity, c.supplier_equity);
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
            // checkpoint <cash> <supply> <debt_shares> <reserves> <accounted> <accounting> <rf_bps> => <reserves'> <equity> <interest> <reserve_add> | err
            let (input, expected) = line.split_once("=>").expect("vector needs =>");
            let w: Vec<&str> = input.split_whitespace().collect();
            assert_eq!(w[0], "checkpoint", "unknown vector {line}");
            let n = |i: usize| w[i].parse::<u128>().unwrap();
            let p = PoolPlain {
                cash: n(1),
                total_supply_shares: n(2),
                total_debt_shares: n(3),
                reserves: n(4),
            };
            let result = checkpoint(&p, n(5), n(6), n(7) as u16);
            let expected = expected.trim();
            if expected == "err" {
                assert!(result.is_err(), "vector: {line}");
            } else {
                let e: Vec<u128> = expected.split_whitespace().map(|x| x.parse().unwrap()).collect();
                let c = result.unwrap_or_else(|error| panic!("{error:?} for {line}"));
                assert_eq!(
                    [c.pool.reserves, c.supplier_equity, c.interest, c.reserve_add],
                    [e[0], e[1], e[2], e[3]],
                    "vector: {line}"
                );
            }
            checked += 1;
        }
        assert!(checked >= 12, "only {checked} checkpoint vectors ran");
    }
}
