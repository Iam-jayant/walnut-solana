//! Pool accounting property tests (Task 7.6; Requirements 7.3, 7.5, 7.10,
//! 7.11, 7.15, 7.14; design correctness properties 6, 7, 8, 13).
//!
//! Deterministic `prop_*` tests over the REAL production math
//! ([`rate`](super::rate), [`checkpoint`](super::checkpoint),
//! [`fixed`](super::fixed)). The same properties for the TypeScript mirror are
//! in `packages/reference-model/test/pool-properties.test.ts`.
//!
//! | Property | Test |
//! | --- | --- |
//! | Rate continuity and monotonicity | `prop_rate_curve_is_monotone_and_continuous_at_the_kink` |
//! | Band rates are conservative | `prop_band_rate_never_undercharges_the_true_utilization` |
//! | Interest split | `prop_interest_split_is_exact_and_rounds_toward_suppliers` |
//! | NAV identity | `prop_nav_identity_and_nav_grows_by_the_supplier_interest` |
//! | Reserve withdrawal neutrality | `prop_reserve_withdrawal_is_all_or_zero_and_equity_neutral` |
//! | Zero supply shares | `prop_zero_supply_shares_are_safe_and_irrelevant_to_pool_math` |
//! | Long elapsed time | `prop_long_elapsed_time_errors_instead_of_wrapping`, `prop_accrual_is_monotone_and_step_splitting_never_lowers_it` |
//! | Rounding directions and limits | `prop_debt_and_borrow_rounding_never_favor_the_borrower` and the interest tests above |

use super::checkpoint::{checkpoint, reserve_withdrawal, supplier_equity, PoolPlain};
use super::fixed::{debt_from_shares, shares_from_amount_ceil, MathError};
use super::rate::{
    band_rate_per_second_wad, band_upper_edge_bps, borrow_rate_bps, project_index,
    rate_per_second_wad, utilization_band, BAND_COUNT, MAX_BORROW_RATE_BPS, SECONDS_PER_YEAR,
};
use super::{Lcg, BPS_DENOMINATOR, MAX_BORROW_INDEX, MAX_ELAPSED_SECONDS, MAX_SHARES, WAD};
use crate::market::RateParams;

const CASES: u64 = 4_000;

fn below(rng: &mut Lcg, n: u128) -> u128 {
    rng.u128_below(n)
}

/// A valid rate curve (the governed bounds keep `base + slope1 + slope2` at
/// or below 57,000 bps and the kink strictly inside `(0, 10_000)`).
fn rates(rng: &mut Lcg) -> RateParams {
    RateParams {
        base_rate_bps: below(rng, 2_001) as u32,
        slope1_bps: below(rng, 3_001) as u32,
        slope2_bps: below(rng, 50_001) as u32,
        optimal_utilization_bps: 1 + below(rng, 9_999) as u16,
        band_max_age_slots: 9_000,
    }
}

/// A random index in `[WAD, MAX_BORROW_INDEX]`, biased toward small values.
fn index(rng: &mut Lcg) -> u128 {
    match below(rng, 4) {
        0 => WAD,
        1 => WAD + below(rng, WAD),
        2 => WAD + below(rng, 10 * WAD),
        _ => WAD + below(rng, MAX_BORROW_INDEX - WAD + 1),
    }
}

/// A random share count, biased toward small and boundary values.
fn shares(rng: &mut Lcg) -> u128 {
    match below(rng, 5) {
        0 => 0,
        1 => MAX_SHARES,
        2 => below(rng, 1_000),
        3 => below(rng, 1u128 << 53),
        _ => below(rng, MAX_SHARES + 1),
    }
}

fn amount(rng: &mut Lcg) -> u128 {
    match below(rng, 4) {
        0 => below(rng, 100),
        1 => below(rng, 1_000_000_000),
        2 => below(rng, 1u128 << 64),
        _ => below(rng, 1u128 << 90),
    }
}

/// A random Pool whose reserves never exceed `cash + debt` at `at_index`.
fn pool_at(rng: &mut Lcg, at_index: u128) -> PoolPlain {
    let debt_shares = shares(rng);
    let cash = amount(rng);
    let debt = debt_from_shares(debt_shares, at_index).unwrap();
    let ceiling = cash + debt;
    let reserves = if ceiling == 0 {
        0
    } else {
        below(rng, ceiling.min(1u128 << 95) + 1) / 2
    };
    PoolPlain {
        cash,
        total_supply_shares: shares(rng),
        total_debt_shares: debt_shares,
        reserves,
    }
}

/// **Validates: Requirements 7.5** (bounded kinked model, continuity,
/// monotonicity in utilization).
#[test]
fn prop_rate_curve_is_monotone_and_continuous_at_the_kink() {
    let mut rng = Lcg(0x7600_0001);
    for _ in 0..CASES / 4 {
        let r = rates(&mut rng);
        let optimal = r.optimal_utilization_bps as u128;
        let max_step = {
            let s1 = (r.slope1_bps as u128).div_ceil(optimal);
            let s2 = (r.slope2_bps as u128).div_ceil(BPS_DENOMINATOR - optimal);
            s1.max(s2)
        };
        let mut previous = borrow_rate_bps(&r, 0).unwrap();
        assert_eq!(previous, r.base_rate_bps as u128, "rate(0) = base");
        for u in 1..=BPS_DENOMINATOR {
            let rate = borrow_rate_bps(&r, u).unwrap();
            assert!(rate >= previous, "monotone at {u}: {r:?}");
            // Continuity: no step exceeds the steeper slope's per-bp increment.
            assert!(
                rate - previous <= max_step,
                "jump {} > {max_step} at {u}: {r:?}",
                rate - previous
            );
            assert!(rate <= MAX_BORROW_RATE_BPS);
            previous = rate;
        }
        // The kink is exact: base + slope1 at the optimal utilization.
        let at_kink = borrow_rate_bps(&r, optimal).unwrap();
        assert_eq!(at_kink, (r.base_rate_bps + r.slope1_bps) as u128);
        // The top of the curve is base + slope1 + slope2.
        assert_eq!(
            borrow_rate_bps(&r, BPS_DENOMINATOR).unwrap(),
            (r.base_rate_bps + r.slope1_bps + r.slope2_bps) as u128
        );
        // Per-second rates are monotone in the annual rate.
        let mut last = 0;
        for bps in (0..=MAX_BORROW_RATE_BPS).step_by(997) {
            let per_second = rate_per_second_wad(bps).unwrap();
            assert!(per_second >= last);
            last = per_second;
        }
    }
}

/// **Validates: Requirements 7.5, 7.6** (the accrual rate comes from the
/// coarse band and never undercharges the exact utilization).
#[test]
fn prop_band_rate_never_undercharges_the_true_utilization() {
    let mut rng = Lcg(0x7600_0002);
    for _ in 0..CASES {
        let r = rates(&mut rng);
        let cash = amount(&mut rng);
        let debt = amount(&mut rng);
        let band = utilization_band(cash, debt).unwrap();
        assert!(band < BAND_COUNT);
        let total = cash + debt;
        if total == 0 {
            assert_eq!(band, 0);
            continue;
        }
        // Exact utilization in bps, rounded up.
        let exact = (debt * BPS_DENOMINATOR).div_ceil(total);
        let upper = band_upper_edge_bps(band).unwrap();
        assert!(
            upper >= exact,
            "band edge {upper} below utilization {exact}"
        );
        let banded = band_rate_per_second_wad(&r, band).unwrap();
        let precise = rate_per_second_wad(borrow_rate_bps(&r, exact).unwrap()).unwrap();
        assert!(banded >= precise);
        // More debt never lowers the band; more cash never raises it.
        let more_debt = utilization_band(cash, debt + below(&mut rng, 1u128 << 20)).unwrap();
        assert!(more_debt >= band);
        let more_cash = utilization_band(cash + below(&mut rng, 1u128 << 20), debt).unwrap();
        assert!(more_cash <= band);
    }
}

/// **Validates: Requirements 7.10** (reserve plus supplier interest equals the
/// total interest exactly; the reserve share rounds down).
#[test]
fn prop_interest_split_is_exact_and_rounds_toward_suppliers() {
    let mut rng = Lcg(0x7600_0003);
    for _ in 0..CASES {
        let a = index(&mut rng);
        let b = a + below(&mut rng, (MAX_BORROW_INDEX - a).min(WAD * 3) + 1);
        let rf = below(&mut rng, 10_001) as u16;
        let pool = pool_at(&mut rng, a);
        let c = checkpoint(&pool, a, b, rf).unwrap();
        assert_eq!(c.reserve_add + c.supplier_interest, c.interest);
        assert_eq!(c.new_debt - c.old_debt, c.interest);
        // reserve_add = floor(interest * rf / 10_000).
        assert!(c.reserve_add * BPS_DENOMINATOR <= c.interest * rf as u128);
        assert!(c.interest * (rf as u128) < (c.reserve_add + 1) * BPS_DENOMINATOR);
        assert_eq!(c.pool.reserves, pool.reserves + c.reserve_add);
        // Only Reserves change; Cash and the share counts never do.
        assert_eq!(
            (
                c.pool.cash,
                c.pool.total_supply_shares,
                c.pool.total_debt_shares
            ),
            (pool.cash, pool.total_supply_shares, pool.total_debt_shares)
        );
        // Total interest is the exact interest within one unit (debt rounds up
        // at both ends).
        let exact_numerator = pool.total_debt_shares * (b - a);
        assert!(c.interest * WAD < exact_numerator + WAD);
        assert!(c.interest * WAD + WAD > exact_numerator);
        // A zero rate factor gives suppliers everything; 100% gives reserves
        // everything.
        let none = checkpoint(&pool, a, b, 0).unwrap();
        assert_eq!(none.reserve_add, 0);
        let all = checkpoint(&pool, a, b, 10_000).unwrap();
        assert_eq!(all.reserve_add, all.interest);
    }
}

/// **Validates: Requirements 7.11** (`equity = cash + debt - reserves`, and a
/// checkpoint raises it by exactly the supplier share of the interest).
#[test]
fn prop_nav_identity_and_nav_grows_by_the_supplier_interest() {
    let mut rng = Lcg(0x7600_0004);
    for _ in 0..CASES {
        let a = index(&mut rng);
        let b = a + below(&mut rng, (MAX_BORROW_INDEX - a).min(WAD * 3) + 1);
        let rf = below(&mut rng, 10_001) as u16;
        let pool = pool_at(&mut rng, a);
        let before = checkpoint(&pool, a, a, rf).unwrap();
        let after = checkpoint(&pool, a, b, rf).unwrap();
        // Identity.
        assert_eq!(
            after.supplier_equity,
            pool.cash + after.new_debt - after.pool.reserves
        );
        assert_eq!(
            after.supplier_equity,
            supplier_equity(after.pool.cash, after.new_debt, after.pool.reserves).unwrap()
        );
        // NAV before the interval is the stored state's equity.
        assert_eq!(
            before.supplier_equity,
            pool.cash + before.new_debt - pool.reserves
        );
        // NAV after = NAV before + supplier interest, exactly.
        assert_eq!(
            after.supplier_equity,
            before.supplier_equity + after.supplier_interest
        );
        // Reserves plus supplier equity equal Cash plus debt: value is only
        // split, never created.
        assert_eq!(
            after.supplier_equity + after.pool.reserves,
            pool.cash + after.new_debt
        );
    }
}

/// **Validates: Requirements 7.15** and design property 13 (reserve withdrawal
/// neutrality): the approval is the full request or zero, Cash and Reserves
/// drop by the same amount, and supplier equity is unchanged.
#[test]
fn prop_reserve_withdrawal_is_all_or_zero_and_equity_neutral() {
    let mut rng = Lcg(0x7600_0005);
    let mut approvals = 0;
    for _ in 0..CASES {
        let a = index(&mut rng);
        let b = a + below(&mut rng, (MAX_BORROW_INDEX - a).min(WAD) + 1);
        let rf = below(&mut rng, 10_001) as u16;
        let pool = pool_at(&mut rng, a);
        let cp = checkpoint(&pool, a, b, rf).unwrap();
        // Requests around the boundaries of Cash and Reserves.
        let limit = cp.pool.cash.min(cp.pool.reserves).min(u64::MAX as u128) as u64;
        let candidates = [
            0,
            1,
            limit,
            limit.saturating_add(1),
            limit / 2,
            u64::MAX,
            below(&mut rng, u64::MAX as u128) as u64,
        ];
        for requested in candidates {
            let out = reserve_withdrawal(&pool, a, b, rf, requested);
            assert!(
                out.approved == 0 || out.approved == requested,
                "all or zero"
            );
            if out.approved == 0 {
                assert_eq!(out.pool, pool, "a refusal commits nothing");
                let r = requested as u128;
                let refusable = r == 0 || r > cp.pool.cash || r > cp.pool.reserves;
                assert!(refusable, "refused a payable request {requested}: {cp:?}");
                continue;
            }
            approvals += 1;
            let r = out.approved as u128;
            assert!(r <= cp.pool.cash && r <= cp.pool.reserves);
            assert_eq!(out.pool.cash, cp.pool.cash - r);
            assert_eq!(out.pool.reserves, cp.pool.reserves - r);
            // Share counts untouched.
            assert_eq!(out.pool.total_supply_shares, pool.total_supply_shares);
            assert_eq!(out.pool.total_debt_shares, pool.total_debt_shares);
            // Neutrality: equity after equals equity of the checkpointed state.
            let equity_after =
                supplier_equity(out.pool.cash, cp.new_debt, out.pool.reserves).unwrap();
            assert_eq!(equity_after, cp.supplier_equity);
            // Value conservation: cash + reserves leaving the Pool equals the
            // transfer, twice counted once each.
            assert_eq!(
                (cp.pool.cash + cp.pool.reserves) - (out.pool.cash + out.pool.reserves),
                2 * r
            );
        }
    }
    assert!(approvals > 500, "only {approvals} approvals were exercised");
}

/// **Validates: Requirements 7.13** (zero Supply Shares): the checkpoint, the
/// NAV, and the reserve withdrawal never read or divide by the supply share
/// count, so they are defined and identical for zero and nonzero supply.
#[test]
fn prop_zero_supply_shares_are_safe_and_irrelevant_to_pool_math() {
    let mut rng = Lcg(0x7600_0006);
    for _ in 0..CASES {
        let a = index(&mut rng);
        let b = a + below(&mut rng, (MAX_BORROW_INDEX - a).min(WAD) + 1);
        let rf = below(&mut rng, 10_001) as u16;
        let pool = pool_at(&mut rng, a);
        let empty = PoolPlain {
            total_supply_shares: 0,
            ..pool
        };
        let full = PoolPlain {
            total_supply_shares: MAX_SHARES,
            ..pool
        };
        let (c0, c1) = (
            checkpoint(&empty, a, b, rf).unwrap(),
            checkpoint(&full, a, b, rf).unwrap(),
        );
        assert_eq!(c0.supplier_equity, c1.supplier_equity);
        assert_eq!(c0.interest, c1.interest);
        assert_eq!(c0.pool.reserves, c1.pool.reserves);
        let request = below(&mut rng, 1u128 << 60) as u64;
        let (w0, w1) = (
            reserve_withdrawal(&empty, a, b, rf, request),
            reserve_withdrawal(&full, a, b, rf, request),
        );
        assert_eq!(w0.approved, w1.approved);
        assert_eq!(w0.pool.cash, w1.pool.cash);
        assert_eq!(w0.pool.total_supply_shares, 0);
    }
    // The canonical empty Pool: everything is zero and stays zero.
    let zero = PoolPlain::default();
    let c = checkpoint(&zero, WAD, 5 * WAD, 1_000).unwrap();
    assert_eq!((c.interest, c.reserve_add, c.supplier_equity), (0, 0, 0));
    assert_eq!(reserve_withdrawal(&zero, WAD, WAD, 1_000, 1).approved, 0);
    assert_eq!(utilization_band(0, 0), Ok(0));
}

/// **Validates: Requirements 7.3** (long elapsed time): an interval beyond
/// the maximum, or one that pushes the index past its ceiling, is an error,
/// never a wrap, a clamp, or a panic.
#[test]
fn prop_long_elapsed_time_errors_instead_of_wrapping() {
    let mut rng = Lcg(0x7600_0007);
    for _ in 0..CASES {
        let idx = index(&mut rng);
        let rate = rate_per_second_wad(below(&mut rng, MAX_BORROW_RATE_BPS + 1)).unwrap();
        let start = below(&mut rng, 1u128 << 40) as i64;
        let elapsed = match below(&mut rng, 5) {
            0 => MAX_ELAPSED_SECONDS,
            1 => MAX_ELAPSED_SECONDS + 1,
            2 => below(&mut rng, MAX_ELAPSED_SECONDS),
            3 => SECONDS_PER_YEAR * below(&mut rng, 400),
            _ => u32::MAX as u128 * 4,
        };
        let result = project_index(idx, start, start + elapsed as i64, rate);
        match result {
            Ok(next) => {
                assert!(elapsed <= MAX_ELAPSED_SECONDS);
                assert!(next >= idx && next <= MAX_BORROW_INDEX);
                if elapsed == 0 || rate == 0 {
                    assert_eq!(next, idx);
                }
            }
            Err(MathError::ElapsedTooLong) => assert!(elapsed > MAX_ELAPSED_SECONDS),
            Err(MathError::OutOfRange) => {
                // The ceiling was hit (or the input was not a valid index).
                assert!(elapsed > 0);
            }
            Err(other) => panic!("unexpected {other:?} for {idx} {rate} {elapsed}"),
        }
    }
    // Pool values at the domain ceilings never overflow the checkpoint.
    let top = PoolPlain {
        cash: 1 << 96,
        total_supply_shares: MAX_SHARES,
        total_debt_shares: MAX_SHARES,
        reserves: 0,
    };
    let c = checkpoint(&top, WAD, MAX_BORROW_INDEX, 10_000).unwrap();
    assert_eq!(c.new_debt, MAX_SHARES * 1_000);
    assert!(c.supplier_equity > 0);
    // A time regression is an error too.
    assert_eq!(project_index(WAD, 10, 9, 1), Err(MathError::TimeRegression));
}

/// **Validates: Requirements 7.3** (the index never decreases; splitting an
/// interval into steps never accrues less than one step, because every step
/// rounds up).
#[test]
fn prop_accrual_is_monotone_and_step_splitting_never_lowers_it() {
    let mut rng = Lcg(0x7600_0008);
    for _ in 0..CASES {
        let idx = WAD + below(&mut rng, 5 * WAD);
        let rate = rate_per_second_wad(below(&mut rng, 20_000)).unwrap();
        let t1 = below(&mut rng, 100_000_000) as i64;
        let t2 = t1 + below(&mut rng, 100_000_000) as i64;
        let t3 = t2 + below(&mut rng, 100_000_000) as i64;
        let one = project_index(idx, 0, t3, rate);
        let step = project_index(idx, 0, t1, rate)
            .and_then(|i1| project_index(i1, t1, t2, rate))
            .and_then(|i2| project_index(i2, t2, t3, rate));
        let (Ok(one), Ok(step)) = (one, step) else {
            continue;
        };
        assert!(one >= idx && step >= one, "steps {step} < single {one}");
        // Never under-accrues: next * WAD >= idx * WAD + idx * rate * dt.
        let growth = rate * t3 as u128;
        assert!(one * WAD >= idx * WAD + idx * growth || growth > (1u128 << 60));
        // Monotone in time.
        let later = project_index(idx, 0, t3 + 1, rate).unwrap();
        assert!(later >= one);
    }
}

/// **Validates: Requirements 7.4, 9.5** (rounding directions): debt rounds up,
/// borrowed shares round up, so the booked debt is never below the amount
/// delivered and exceeds it by less than one index unit.
#[test]
fn prop_debt_and_borrow_rounding_never_favor_the_borrower() {
    let mut rng = Lcg(0x7600_0009);
    for _ in 0..CASES {
        let idx = index(&mut rng);
        let s = shares(&mut rng);
        let debt = debt_from_shares(s, idx).unwrap();
        // debt = ceil(s * idx / WAD).
        assert!(debt * WAD >= s * idx);
        assert!(debt == 0 || (debt - 1) * WAD < s * idx);

        let amt = amount(&mut rng) % (MAX_SHARES + 1);
        let issued = shares_from_amount_ceil(amt, idx).unwrap();
        // The borrower is booked at least what they received.
        let booked = debt_from_shares(issued, idx).unwrap();
        assert!(booked >= amt, "booked {booked} < delivered {amt}");
        // The surplus is below one whole index multiple (ceil(idx / WAD)) and
        // goes to Reserves (Requirement 9.5).
        let surplus = booked - amt;
        assert!(surplus <= idx.div_ceil(WAD), "surplus {surplus} at {idx}");
        // At index WAD there is no rounding at all.
        assert_eq!(shares_from_amount_ceil(amt, WAD).unwrap(), amt);
    }
}
