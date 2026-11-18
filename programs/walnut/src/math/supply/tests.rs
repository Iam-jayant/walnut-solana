//! Supply, withdraw-supply, and recovery rules (Task 8.1 to 8.4; Requirements
//! 8.1 to 8.8, 7.13). Deterministic `prop_*` tests over the REAL
//! [`supply_flow`] and [`recover_empty_pool`]; the TypeScript mirror is tested
//! in `packages/reference-model/test/supply.test.ts`.
//!
//! | Property | Test |
//! | --- | --- |
//! | Supply never dilutes existing suppliers | `prop_supply_never_lowers_the_share_price_and_rounds_down` |
//! | Withdrawal never over-pays | `prop_withdraw_rounds_down_covers_cash_and_keeps_the_share_price` |
//! | Refusal moves nothing | `prop_refusal_returns_the_inputs_unchanged` |
//! | No historical-interest capture | `prop_a_new_supplier_never_captures_earlier_interest` |
//! | Round trip never profits | `prop_supply_then_withdraw_never_profits_without_interest` |
//! | First supplier needs the canonical empty state | `prop_first_supplier_needs_the_canonical_empty_state` |
//! | Dust | `prop_dust_supply_mints_zero_shares_and_moves_nothing` |
//! | Recovery | `prop_recovery_only_reclassifies_dust_as_reserves` |
//! | Overflow edges | `prop_domain_edges_are_refused_not_wrapped` |

use super::*;
use crate::math::{Lcg, WAD};

const CASES: u64 = 3_000;
const RF: u16 = 1_000;

fn pool(cash: u128, supply: u128, debt: u128, reserves: u128) -> PoolPlain {
    PoolPlain {
        cash,
        total_supply_shares: supply,
        total_debt_shares: debt,
        reserves,
    }
}

fn pos(free: u64, shares: u128) -> SupplyPosition {
    SupplyPosition {
        free_usdc: free,
        supply_shares: shares,
    }
}

fn supply_req(amount: u128) -> SupplyRequest {
    SupplyRequest {
        is_withdraw: false,
        requested: amount,
        supply_cap: u64::MAX,
        claims_clear: true,
    }
}

fn withdraw_req(shares: u128) -> SupplyRequest {
    SupplyRequest {
        is_withdraw: true,
        requested: shares,
        supply_cap: u64::MAX,
        claims_clear: true,
    }
}

fn run(p: &PoolPlain, s: &SupplyPosition, a: u128, b: u128, r: &SupplyRequest) -> SupplyOutcome {
    supply_flow(p, s, a, b, RF, r)
}

/// Equity of a Pool at one index (no interest pending).
fn equity_at(p: &PoolPlain, index: u128) -> u128 {
    checkpoint(p, index, index, RF).unwrap().supplier_equity
}

fn below(rng: &mut Lcg, n: u128) -> u128 {
    rng.u128_below(n)
}

/// A solvent, non-empty Pool with positive equity and a random index pair.
fn live_pool(rng: &mut Lcg) -> (PoolPlain, u128, u128) {
    let cash = 1 + below(rng, 1_000_000_000_000);
    let debt = if below(rng, 3) == 0 {
        0
    } else {
        below(rng, 1_000_000_000_000)
    };
    let supply = 1 + below(rng, 1_000_000_000_000_000);
    let reserves = below(rng, cash.min(1 + cash / 2));
    let accounted = WAD + below(rng, 2 * WAD);
    let accounting = accounted + below(rng, WAD);
    (pool(cash, supply, debt, reserves), accounted, accounting)
}

// ---- unit tests -----------------------------------------------------------

#[test]
fn first_supplier_mints_at_the_initial_scale() {
    let out = run(
        &pool(0, 0, 0, 0),
        &pos(5_000, 0),
        WAD,
        WAD,
        &supply_req(2_000),
    );
    assert!(out.applied);
    assert_eq!(out.position, pos(3_000, 2_000 * INITIAL_SHARE_SCALE));
    assert_eq!(out.pool, pool(2_000, 2_000 * INITIAL_SHARE_SCALE, 0, 0));
}

#[test]
fn first_supplier_is_refused_unless_the_pool_is_canonically_empty() {
    let p = pos(5_000, 0);
    for dirty in [
        pool(1, 0, 0, 0),
        pool(0, 0, 1, 0),
        pool(5, 0, 0, 5),
        pool(0, 0, 0, 1),
    ] {
        let out = run(&dirty, &p, WAD, WAD, &supply_req(100));
        assert!(!out.applied, "{dirty:?}");
        assert_eq!((out.pool, out.position), (dirty, p));
    }
    let blocked = SupplyRequest {
        claims_clear: false,
        ..supply_req(100)
    };
    let out = run(&pool(0, 0, 0, 0), &p, WAD, WAD, &blocked);
    assert!(!out.applied);
    assert_eq!(out.pool, pool(0, 0, 0, 0));
    assert!(run(&pool(0, 0, 0, 0), &p, WAD, WAD, &supply_req(100)).applied);
}

#[test]
fn supply_rejects_zero_shares_insufficient_balance_and_the_cap() {
    // One share is worth 1,000 base units: 999 mints floor(999 / 1000) = 0.
    let p = pool(1_000_000, 1_000, 0, 0);
    let s = pos(10_000, 0);
    assert!(!run(&p, &s, WAD, WAD, &supply_req(999)).applied);
    assert!(run(&p, &s, WAD, WAD, &supply_req(1_000)).applied);
    // Insufficient Free USDC.
    assert!(!run(&p, &pos(999, 0), WAD, WAD, &supply_req(1_000)).applied);
    // Zero request.
    assert!(!run(&p, &s, WAD, WAD, &supply_req(0)).applied);
    // Cap: equity 1,000,000 + amount must stay at or below 1,000,500.
    let q = pool(1_000_000, 1_000_000_000, 0, 0);
    let capped = |amount| SupplyRequest {
        supply_cap: 1_000_500,
        ..supply_req(amount)
    };
    assert!(run(&q, &s, WAD, WAD, &capped(500)).applied);
    assert!(!run(&q, &s, WAD, WAD, &capped(501)).applied);
}

#[test]
fn withdraw_rounds_down_and_needs_shares_and_complete_cash_coverage() {
    // equity 10, supply 3 shares: 1 share is worth floor(10 / 3) = 3.
    let p = pool(10, 3, 0, 0);
    let s = pos(0, 1);
    let out = run(&p, &s, WAD, WAD, &withdraw_req(1));
    assert!(out.applied);
    assert_eq!(out.position, pos(3, 0));
    assert_eq!(out.pool, pool(7, 2, 0, 0));
    // More shares than the owner holds.
    assert!(!run(&p, &s, WAD, WAD, &withdraw_req(2)).applied);
    // Claim above Cash: equity counts debt, Cash does not cover it.
    let lent = pool(2, 10, 8, 0);
    let out = run(&lent, &pos(0, 10), WAD, WAD, &withdraw_req(10));
    assert!(!out.applied, "claim 10 > cash 2");
    assert_eq!((out.pool, out.position), (lent, pos(0, 10)));
    // A partial claim that Cash covers applies.
    assert!(run(&lent, &pos(0, 10), WAD, WAD, &withdraw_req(2)).applied);
    // A claim that rounds to zero is refused.
    assert!(
        !run(
            &pool(1, 1_000, 0, 0),
            &pos(0, 1),
            WAD,
            WAD,
            &withdraw_req(1)
        )
        .applied
    );
}

#[test]
fn the_last_supplier_exits_with_exactly_the_supplied_amount() {
    let first = run(
        &pool(0, 0, 0, 0),
        &pos(7_777, 0),
        WAD,
        WAD,
        &supply_req(7_777),
    );
    let shares = first.position.supply_shares;
    let exit = run(
        &first.pool,
        &first.position,
        WAD,
        WAD,
        &withdraw_req(shares),
    );
    assert!(exit.applied);
    assert_eq!(exit.position, pos(7_777, 0));
    assert_eq!(exit.pool, pool(0, 0, 0, 0));
    assert!(is_canonical_empty(&exit.pool, true));
}

#[test]
fn a_supplier_just_after_an_accrual_gets_no_earlier_interest() {
    // Doubling the index doubles 500,000 debt shares' value: 500,000 interest,
    // 10% to reserves, so equity = 1,000,000 + 1,000,000 - 50,000.
    let p = pool(1_000_000, 1_000_000_000, 500_000, 0);
    let two = 2 * WAD;
    let out = run(&p, &pos(195_000, 0), WAD, two, &supply_req(195_000));
    assert!(out.applied);
    // Minted against the checkpointed equity 1,950,000: 195,000 / 1,950,000 = 10%.
    assert_eq!(out.position.supply_shares, 100_000_000);
    // Existing suppliers keep their whole 1,950,000.
    let after = checkpoint(&out.pool, two, two, RF).unwrap();
    assert_eq!(
        1_000_000_000 * after.supplier_equity / out.pool.total_supply_shares,
        1_950_000
    );
    // The new supplier can take out exactly what they put in.
    let exit = run(
        &out.pool,
        &out.position,
        two,
        two,
        &withdraw_req(out.position.supply_shares),
    );
    assert_eq!(exit.position.free_usdc, 195_000);
    // Interest that accrues afterwards is shared pro rata: index 2 to 3 adds
    // 500,000 of interest, 450,000 to suppliers; 1/11 of it is 40,909.
    let later = run(
        &out.pool,
        &out.position,
        two,
        3 * WAD,
        &withdraw_req(out.position.supply_shares),
    );
    assert_eq!(later.position.free_usdc, 195_000 + 40_909);
}

#[test]
fn a_supplier_just_before_an_accrual_shares_the_later_interest() {
    // Same pool, but the supply checkpoints at the unchanged index, then the
    // index doubles: the supplier participates in all of that interest.
    let p = pool(1_000_000, 1_000_000_000, 500_000, 0);
    let out = run(&p, &pos(1_000_000, 0), WAD, WAD, &supply_req(1_000_000));
    assert!(out.applied);
    let exit = run(
        &out.pool,
        &out.position,
        WAD,
        2 * WAD,
        &withdraw_req(out.position.supply_shares),
    );
    // Equity before = 1,000,000 + 500,000; the supplier owns 1,000,000 of
    // 2,500,000 and gets 40% of the 450,000 supplier interest (one unit of\n    // rounding dust stays behind).
    assert_eq!(exit.position.free_usdc, 1_000_000 + 179_999);
}

#[test]
fn a_refused_checkpoint_or_domain_violation_moves_nothing() {
    let p = pool(100, 100, 0, 0);
    let s = pos(100, 0);
    // The accounting index is below the accounted index.
    assert!(!run(&p, &s, 2 * WAD, WAD, &supply_req(10)).applied);
    // Amount above the domain.
    assert!(!run(&p, &pos(u64::MAX, 0), WAD, WAD, &supply_req(1 << 64)).applied);
    // Share total above the domain.
    let huge = pool(100, MAX_FLOW_SHARES + 1, 0, 0);
    assert!(!run(&huge, &s, WAD, WAD, &supply_req(10)).applied);
    // Minting would pass the share ceiling.
    let near = pool(1 << 62, MAX_FLOW_SHARES, 0, 0);
    assert!(!run(&near, &pos(u64::MAX, 0), WAD, WAD, &supply_req(1 << 62)).applied);
    // Withdraw into a Free USDC that would overflow u64.
    let p2 = pool(1_000, 1_000, 0, 0);
    assert!(!run(&p2, &pos(u64::MAX, 1_000), WAD, WAD, &withdraw_req(1_000)).applied);
    // Withdraw from an empty Pool.
    assert!(!run(&pool(0, 0, 0, 0), &pos(0, 5), WAD, WAD, &withdraw_req(5)).applied);
}

#[test]
fn recovery_moves_dust_into_reserves_and_nothing_else() {
    let p = pool(7, 0, 0, 0);
    let out = recover_empty_pool(&p, WAD, WAD, RF, RECOVERY_DUST_LIMIT, true);
    assert!(out.applied);
    assert_eq!(out.pool, pool(7, 0, 0, 7));
    // Reserve withdrawal then restores the canonical empty state.
    let drained = crate::math::checkpoint::reserve_withdrawal(&out.pool, WAD, WAD, RF, 7);
    assert_eq!(drained.approved, 7);
    assert!(is_canonical_empty(&drained.pool, true));
    // Supply reopens.
    assert!(run(&drained.pool, &pos(50, 0), WAD, WAD, &supply_req(50)).applied);
}

#[test]
fn recovery_refuses_everything_that_is_not_provable_dust() {
    let again = |p: PoolPlain, limit: u128, claims: bool| {
        let out = recover_empty_pool(&p, WAD, WAD, RF, limit, claims);
        assert!(!out.applied, "{p:?} {limit} {claims}");
        assert_eq!(out.pool, p);
    };
    again(pool(7, 1, 0, 0), RECOVERY_DUST_LIMIT, true); // shares outstanding
    again(pool(7, 0, 3, 0), RECOVERY_DUST_LIMIT, true); // debt outstanding
    again(pool(7, 0, 0, 0), RECOVERY_DUST_LIMIT, false); // unsettled claims
    again(
        pool(RECOVERY_DUST_LIMIT + 1, 0, 0, 0),
        RECOVERY_DUST_LIMIT,
        true,
    );
    again(pool(0, 0, 0, 0), RECOVERY_DUST_LIMIT, true); // nothing to recover
    again(pool(7, 0, 0, 7), RECOVERY_DUST_LIMIT, true); // already all reserves
}

// ---- property tests -------------------------------------------------------

#[test]
fn prop_supply_never_lowers_the_share_price_and_rounds_down() {
    let mut rng = Lcg(0x5101);
    let mut applied = 0;
    for _ in 0..CASES {
        let (p, a, b) = live_pool(&mut rng);
        let cp = checkpoint(&p, a, b, RF).unwrap();
        let amount = 1 + below(&mut rng, 10 * (cp.supplier_equity + 1_000));
        let free = below(&mut rng, u64::MAX as u128) as u64;
        let out = run(&p, &pos(free, 0), a, b, &supply_req(amount));
        if !out.applied {
            continue;
        }
        applied += 1;
        let minted = out.position.supply_shares;
        let ts0 = p.total_supply_shares;
        assert_eq!(out.pool.total_supply_shares, ts0 + minted);
        assert_eq!(out.pool.cash, p.cash + amount);
        assert_eq!(out.position.free_usdc as u128, free as u128 - amount);
        // Rounded down: the minted shares are worth at most the amount.
        assert!(minted * cp.supplier_equity <= amount * ts0);
        // Existing suppliers: equity per share never falls.
        let after = equity_at(&out.pool, b);
        assert_eq!(after, cp.supplier_equity + amount, "NAV identity");
        assert!(after * ts0 >= cp.supplier_equity * (ts0 + minted));
    }
    assert!(applied > CASES / 4, "only {applied} supplies applied");
}

#[test]
fn prop_withdraw_rounds_down_covers_cash_and_keeps_the_share_price() {
    let mut rng = Lcg(0x5102);
    let (mut applied, mut cash_refused) = (0, 0);
    for _ in 0..CASES {
        let (p, a, b) = live_pool(&mut rng);
        let cp = checkpoint(&p, a, b, RF).unwrap();
        let held = 1 + below(&mut rng, p.total_supply_shares);
        let shares = 1 + below(&mut rng, held + 2);
        let out = run(&p, &pos(0, held), a, b, &withdraw_req(shares));
        let claim = shares * cp.supplier_equity / p.total_supply_shares;
        if !out.applied {
            if shares <= held && claim > p.cash {
                cash_refused += 1;
            }
            continue;
        }
        applied += 1;
        assert!(shares <= held && claim > 0 && claim <= p.cash);
        assert_eq!(out.position.free_usdc as u128, claim);
        assert_eq!(out.position.supply_shares, held - shares);
        assert_eq!(out.pool.cash, p.cash - claim);
        assert_eq!(out.pool.total_supply_shares, p.total_supply_shares - shares);
        // Rounded down: never more than the pro-rata value.
        assert!(claim * p.total_supply_shares <= shares * cp.supplier_equity);
        // Remaining suppliers' equity per share never falls.
        if out.pool.total_supply_shares > 0 {
            let after = equity_at(&out.pool, b);
            assert_eq!(after, cp.supplier_equity - claim);
            assert!(
                after * p.total_supply_shares >= cp.supplier_equity * out.pool.total_supply_shares
            );
        }
    }
    assert!(applied > CASES / 10, "only {applied} withdrawals applied");
    assert!(cash_refused > 0, "no Cash-coverage refusal was generated");
}

#[test]
fn prop_refusal_returns_the_inputs_unchanged() {
    let mut rng = Lcg(0x5103);
    let mut refused = 0;
    for _ in 0..CASES {
        let (mut p, a, b) = live_pool(&mut rng);
        if below(&mut rng, 4) == 0 {
            p.total_supply_shares = 0;
        }
        let s = pos(
            below(&mut rng, 1_000_000) as u64,
            below(&mut rng, 1_000_000_000),
        );
        let req = SupplyRequest {
            is_withdraw: below(&mut rng, 2) == 0,
            requested: below(&mut rng, 2_000_000_000),
            supply_cap: below(&mut rng, 3_000_000_000_000) as u64,
            claims_clear: below(&mut rng, 5) != 0,
        };
        let out = run(&p, &s, a, b, &req);
        if !out.applied {
            refused += 1;
            assert_eq!((out.pool, out.position), (p, s), "{req:?}");
        }
    }
    assert!(refused > CASES / 4);
}

#[test]
fn prop_a_new_supplier_never_captures_earlier_interest() {
    let mut rng = Lcg(0x5104);
    let mut with_interest = 0;
    for _ in 0..CASES {
        let (p, a, b) = live_pool(&mut rng);
        let cp = checkpoint(&p, a, b, RF).unwrap();
        let amount = 1 + below(&mut rng, 2 * cp.supplier_equity + 1_000_000);
        let out = run(&p, &pos(u64::MAX, 0), a, b, &supply_req(amount));
        if !out.applied {
            continue;
        }
        let shares = out.position.supply_shares;
        let ts = out.pool.total_supply_shares;
        if cp.interest > 0 {
            with_interest += 1;
        }
        // Right after minting, the position is worth at most what was paid in,
        // even though the Pool accrued `cp.interest` before the supply.
        let eq_b = equity_at(&out.pool, b);
        assert!(shares * eq_b / ts <= amount, "captured earlier interest");
        // Later accrual b -> c is shared pro rata and never exceeds the
        // position's fraction of the later supplier interest.
        let c = b + below(&mut rng, WAD);
        let later = checkpoint(&out.pool, b, c, RF).unwrap();
        let claim_c = shares * later.supplier_equity / ts;
        assert!(claim_c * ts <= shares * (eq_b + later.supplier_interest));
        assert!(claim_c <= amount + later.supplier_interest);
    }
    assert!(with_interest > CASES / 4, "only {with_interest} accruals");
}

#[test]
fn prop_supply_then_withdraw_never_profits_without_interest() {
    let mut rng = Lcg(0x5105);
    let mut round_trips = 0;
    for _ in 0..CASES {
        let (p, _, b) = live_pool(&mut rng);
        let eq = equity_at(&p, b);
        let amount = 1 + below(&mut rng, 4 * eq + 1_000);
        let out = run(&p, &pos(u64::MAX, 0), b, b, &supply_req(amount));
        if !out.applied {
            continue;
        }
        let exit = run(
            &out.pool,
            &out.position,
            b,
            b,
            &withdraw_req(out.position.supply_shares),
        );
        if !exit.applied {
            continue; // the claim rounded to zero
        }
        round_trips += 1;
        let paid_in = amount;
        let refund = exit.position.free_usdc as u128 - out.position.free_usdc as u128;
        assert!(refund <= paid_in, "profit from a round trip");
        // Loss is bounded by one share's worth plus one unit.
        assert!(paid_in - refund <= eq / p.total_supply_shares + 2);
    }
    assert!(round_trips > CASES / 4);
}

#[test]
fn prop_first_supplier_needs_the_canonical_empty_state() {
    let mut rng = Lcg(0x5106);
    let (mut opened, mut blocked) = (0, 0);
    for _ in 0..CASES {
        let dirty = |rng: &mut Lcg| {
            if below(rng, 3) == 0 {
                1 + below(rng, 1_000)
            } else {
                0
            }
        };
        let (cash, debt) = (dirty(&mut rng), dirty(&mut rng));
        let reserves = dirty(&mut rng).min(cash);
        let p = pool(cash, 0, debt, reserves);
        let claims_clear = below(&mut rng, 4) != 0;
        let amount = 1 + below(&mut rng, 1_000_000_000);
        let req = SupplyRequest {
            claims_clear,
            ..supply_req(amount)
        };
        let out = run(&p, &pos(u64::MAX, 0), WAD, WAD, &req);
        let canonical = cash == 0 && debt == 0 && reserves == 0 && claims_clear;
        assert_eq!(out.applied, canonical, "{p:?} claims_clear={claims_clear}");
        if canonical {
            opened += 1;
            assert_eq!(out.position.supply_shares, amount * INITIAL_SHARE_SCALE);
            assert_eq!(out.pool, pool(amount, amount * INITIAL_SHARE_SCALE, 0, 0));
        } else {
            blocked += 1;
            assert_eq!((out.pool, out.position), (p, pos(u64::MAX, 0)));
        }
    }
    assert!(opened > 100 && blocked > 100);
}

#[test]
fn prop_dust_supply_mints_zero_shares_and_moves_nothing() {
    let mut rng = Lcg(0x5107);
    let mut dusty = 0;
    for _ in 0..CASES {
        // Equity far above the share count: one share is worth many units.
        let supply = 1 + below(&mut rng, 1_000);
        let cash = supply * (2 + below(&mut rng, 1_000_000));
        let p = pool(cash, supply, 0, 0);
        let worth = cash / supply;
        let amount = 1 + below(&mut rng, 2 * worth + 1);
        let s = pos(u64::MAX, 0);
        let out = run(&p, &s, WAD, WAD, &supply_req(amount));
        let minted = amount * supply / cash;
        assert_eq!(out.applied, minted > 0);
        if minted == 0 {
            dusty += 1;
            assert_eq!((out.pool, out.position), (p, s));
        }
    }
    assert!(dusty > CASES / 10, "only {dusty} dust cases");
}

#[test]
fn prop_recovery_only_reclassifies_dust_as_reserves() {
    let mut rng = Lcg(0x5108);
    let (mut applied, mut refused) = (0, 0);
    for _ in 0..CASES {
        let cash = below(&mut rng, 30_000);
        let reserves = below(&mut rng, cash + 1);
        let supply = if below(&mut rng, 4) == 0 { 1 } else { 0 };
        let debt = if below(&mut rng, 4) == 0 { 1 } else { 0 };
        let claims_clear = below(&mut rng, 4) != 0;
        let p = pool(cash, supply, debt, reserves);
        let out = recover_empty_pool(&p, WAD, WAD, RF, RECOVERY_DUST_LIMIT, claims_clear);
        let residue = cash - reserves;
        let expect = supply == 0
            && debt == 0
            && claims_clear
            && residue > 0
            && residue <= RECOVERY_DUST_LIMIT;
        assert_eq!(out.applied, expect, "{p:?}");
        if out.applied {
            applied += 1;
            assert_eq!(out.pool, pool(cash, 0, 0, cash));
            assert_eq!(equity_at(&out.pool, WAD), 0);
        } else {
            refused += 1;
            assert_eq!(out.pool, p);
        }
    }
    assert!(applied > 100 && refused > 100);
}

#[test]
fn prop_domain_edges_are_refused_not_wrapped() {
    let mut rng = Lcg(0x5109);
    for _ in 0..CASES {
        let edge = |rng: &mut Lcg| match below(rng, 6) {
            0 => 0,
            1 => u64::MAX as u128,
            2 => (u64::MAX as u128) + 1,
            3 => MAX_FLOW_SHARES,
            4 => MAX_FLOW_SHARES + 1,
            _ => u128::MAX >> below(rng, 100),
        };
        let p = pool(
            edge(&mut rng).min(1 << 96),
            edge(&mut rng),
            below(&mut rng, 2),
            0,
        );
        let s = pos(edge(&mut rng) as u64, edge(&mut rng));
        let req = SupplyRequest {
            is_withdraw: below(&mut rng, 2) == 0,
            requested: edge(&mut rng),
            supply_cap: edge(&mut rng) as u64,
            claims_clear: true,
        };
        // Must not panic (no overflow); a refusal returns the inputs.
        let out = run(&p, &s, WAD, WAD, &req);
        if !out.applied {
            assert_eq!((out.pool, out.position), (p, s));
        } else {
            assert!(out.pool.total_supply_shares <= MAX_FLOW_SHARES);
            assert!(out.pool.cash <= MAX_FLOW_VALUE);
        }
    }
}

// ---- shared golden vectors ------------------------------------------------

const VECTORS: &str =
    include_str!("../../../../../packages/reference-model/vectors/supply.vectors");

#[test]
fn golden_vectors_match_the_reference_model() {
    let n = |w: &str| -> u128 { w.parse().unwrap() };
    let (mut supplies, mut recoveries) = (0, 0);
    for line in VECTORS.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (input, expected) = line.split_once("=>").unwrap();
        let w: Vec<&str> = input.split_whitespace().collect();
        let e: Vec<&str> = expected.split_whitespace().collect();
        match w[0] {
            "supply" => {
                let p = pool(n(w[1]), n(w[2]), n(w[3]), n(w[4]));
                let s = pos(n(w[5]) as u64, n(w[6]));
                let req = SupplyRequest {
                    is_withdraw: w[10] == "1",
                    requested: n(w[11]),
                    supply_cap: n(w[12]) as u64,
                    claims_clear: w[13] == "1",
                };
                let out = supply_flow(&p, &s, n(w[7]), n(w[8]), n(w[9]) as u16, &req);
                if e[0] == "zero" {
                    assert!(!out.applied, "vector: {line}");
                    assert_eq!((out.pool, out.position), (p, s), "vector: {line}");
                } else {
                    assert!(out.applied, "vector: {line}");
                    let got = [
                        out.pool.cash,
                        out.pool.total_supply_shares,
                        out.pool.reserves,
                        out.position.free_usdc as u128,
                        out.position.supply_shares,
                    ];
                    let want: Vec<u128> = e.iter().map(|x| n(x)).collect();
                    assert_eq!(got.to_vec(), want, "vector: {line}");
                    assert_eq!(out.pool.total_debt_shares, p.total_debt_shares);
                }
                supplies += 1;
            }
            "recover" => {
                let p = pool(n(w[1]), n(w[2]), n(w[3]), n(w[4]));
                let out =
                    recover_empty_pool(&p, n(w[5]), n(w[6]), n(w[7]) as u16, n(w[8]), w[9] == "1");
                if e[0] == "zero" {
                    assert!(!out.applied, "vector: {line}");
                    assert_eq!(out.pool, p, "vector: {line}");
                } else {
                    assert!(out.applied, "vector: {line}");
                    assert_eq!(out.pool.reserves, n(e[0]), "vector: {line}");
                    assert_eq!(out.pool.cash, p.cash);
                }
                recoveries += 1;
            }
            other => panic!("unknown vector {other}"),
        }
    }
    assert!(supplies >= 30 && recoveries >= 9);
}
