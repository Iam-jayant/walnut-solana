// Pool accounting properties on the TypeScript mirror (Task 7.6). The Rust
// counterparts are in programs/walnut/src/math/properties.rs; the same
// definitions are replayed there against the production functions.
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  MAX_BORROW_INDEX,
  MAX_SHARES,
  WAD,
  borrowRateBps,
  checkpoint,
  debtFromShares,
  reserveWithdrawal,
  supplierEquity,
  type PoolPlain,
  type RateParams,
} from "../src/index.ts";

/** Deterministic xorshift64 sampler (no property-testing dependency). */
function sampler(seed: bigint) {
  let state = seed;
  const next = (): bigint => {
    state ^= (state << 13n) & 0xffff_ffff_ffff_ffffn;
    state ^= state >> 7n;
    state ^= (state << 17n) & 0xffff_ffff_ffff_ffffn;
    return state;
  };
  return (below: bigint): bigint => ((next() << 64n) ^ next()) % below;
}

function poolAt(below: (n: bigint) => bigint, index: bigint): PoolPlain {
  const debtShares = below(MAX_SHARES + 1n);
  const cash = below(1n << 90n);
  const debt = debtFromShares(debtShares, index);
  return {
    cash,
    totalSupplyShares: below(MAX_SHARES + 1n),
    totalDebtShares: debtShares,
    reserves: below(cash + debt + 1n) / 2n,
  };
}

test("rate curve is monotone and its steps are bounded by the steeper slope", () => {
  const below = sampler(0x7601n);
  for (let n = 0; n < 40; n++) {
    const optimal = 1n + below(9_999n);
    const rates: RateParams = {
      baseRateBps: below(2_001n),
      slope1Bps: below(3_001n),
      slope2Bps: below(50_001n),
      optimalUtilizationBps: optimal,
    };
    const ceil = (a: bigint, b: bigint): bigint => (a + b - 1n) / b;
    const maxStep = [
      ceil(rates.slope1Bps, optimal),
      ceil(rates.slope2Bps, 10_000n - optimal),
    ].reduce((a, b) => (a > b ? a : b));
    let previous = borrowRateBps(rates, 0n);
    for (let u = 1n; u <= 10_000n; u++) {
      const rate = borrowRateBps(rates, u);
      assert.ok(rate >= previous);
      assert.ok(rate - previous <= maxStep);
      previous = rate;
    }
    assert.equal(
      borrowRateBps(rates, optimal),
      rates.baseRateBps + rates.slope1Bps,
    );
  }
});

test("interest split, NAV identity, and reserve withdrawal neutrality", () => {
  const below = sampler(0x7602n);
  let approvals = 0;
  for (let n = 0; n < 1_500; n++) {
    const a = WAD + below(MAX_BORROW_INDEX - WAD + 1n);
    const room =
      MAX_BORROW_INDEX - a < 3n * WAD ? MAX_BORROW_INDEX - a : 3n * WAD;
    const b = a + below(room + 1n);
    const rf = below(10_001n);
    const pool = poolAt(below, a);
    const before = checkpoint(pool, a, a, rf);
    const after = checkpoint(pool, a, b, rf);
    // Reserve plus supplier interest equals the interest exactly.
    assert.equal(after.reserveAdd + after.supplierInterest, after.interest);
    assert.ok(after.reserveAdd * 10_000n <= after.interest * rf);
    // NAV identity and growth by exactly the supplier interest.
    assert.equal(
      after.supplierEquity,
      pool.cash + after.newDebt - after.pool.reserves,
    );
    assert.equal(
      after.supplierEquity,
      before.supplierEquity + after.supplierInterest,
    );
    // Reserve withdrawal: all or zero, equity neutral.
    const limit =
      after.pool.cash < after.pool.reserves
        ? after.pool.cash
        : after.pool.reserves;
    for (const requested of [0n, 1n, limit, limit + 1n, below(1n << 62n)]) {
      const out = reserveWithdrawal(pool, a, b, rf, requested);
      assert.ok(out.approved === 0n || out.approved === requested);
      if (out.approved === 0n) {
        assert.deepEqual(out.pool, pool);
        continue;
      }
      approvals++;
      assert.equal(out.pool.cash, after.pool.cash - requested);
      assert.equal(out.pool.reserves, after.pool.reserves - requested);
      assert.equal(
        supplierEquity(out.pool.cash, after.newDebt, out.pool.reserves),
        after.supplierEquity,
      );
    }
  }
  assert.ok(approvals > 500, `only ${approvals} approvals`);
});

test("zero supply shares do not enter the Pool math", () => {
  const below = sampler(0x7603n);
  for (let n = 0; n < 300; n++) {
    const a = WAD + below(WAD);
    const b = a + below(WAD);
    const pool = poolAt(below, a);
    const empty = { ...pool, totalSupplyShares: 0n };
    const full = { ...pool, totalSupplyShares: MAX_SHARES };
    const c0 = checkpoint(empty, a, b, 1_000n);
    const c1 = checkpoint(full, a, b, 1_000n);
    assert.equal(c0.supplierEquity, c1.supplierEquity);
    assert.equal(c0.interest, c1.interest);
  }
  const zero = checkpoint(
    { cash: 0n, totalSupplyShares: 0n, totalDebtShares: 0n, reserves: 0n },
    WAD,
    5n * WAD,
    1_000n,
  );
  assert.equal(zero.supplierEquity, 0n);
});
