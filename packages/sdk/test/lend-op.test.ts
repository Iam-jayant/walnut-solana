// Golden vectors copied from the Rust tests in programs/walnut/src/math/lend.rs
// and state/lending_pool.rs, plus deterministic LCG property checks
// (Properties 3, 4, 5 of walnut-lend).
import { test } from "node:test";
import assert from "node:assert/strict";
import { PublicKey } from "@solana/web3.js";
import {
  accruedDebt,
  accruedSupply,
  applyLendOp,
  borrowAprBps,
  borrowMint,
  decodeLendingPool,
  fullRepayAmount,
  INDEX_SCALE,
  LendOp,
  liquidationSeize,
  MAX_INDEX_1E9,
  projectIndexes,
  repayBurn,
  supplyApyBps,
  supplySharesFor,
  supplyValue,
  utilizationBps,
  withdrawSupplyBurn,
  LENDING_POOL_ACCOUNT_LEN,
  type LendCtx,
  type LendingPool,
} from "../src/index.ts";
import type { PositionBalances } from "../src/index.ts";

/** 64-bit LCG (Knuth MMIX), deterministic. */
class Lcg {
  #s: bigint;
  constructor(seed: bigint) {
    this.#s = seed;
  }
  next(): bigint {
    this.#s =
      (this.#s * 6364136223846793005n + 1442695040888963407n) &
      ((1n << 64n) - 1n);
    return this.#s >> 11n;
  }
  below(n: bigint): bigint {
    return this.next() % n;
  }
}

const bal = (o: Partial<PositionBalances> = {}): PositionBalances => ({
  freeSol: 0n,
  freeUsdc: 0n,
  collateralSol: 0n,
  supplySharesUsdc: 0n,
  debtSharesUsdc: 0n,
  ...o,
});
const ctx = (index: bigint, price: bigint): LendCtx => ({
  borrowIndex1e9: index,
  price,
  ltvBps: 7_500n,
  liqThresholdBps: 8_000n,
  positionDebtCap: 1_000_000_000_000n,
});

test("share rounding directions (Rust share_rounding_directions)", () => {
  const idx = 1_500_000_000n;
  assert.equal(borrowMint(10n, idx), 7n);
  assert.equal(repayBurn(10n, idx), 6n);
  assert.equal(supplySharesFor(10n, idx), 6n);
  assert.equal(withdrawSupplyBurn(10n, idx), 7n);
  assert.throws(() => borrowMint(10n, 500n), RangeError);
});

test("seize matches formula (Rust seize_matches_formula)", () => {
  assert.equal(
    liquidationSeize(100_000_000n, 500n, 150_000_000n),
    700_000_000n,
  );
  assert.throws(() => liquidationSeize(1n, 0n, 0n), RangeError);
});

test("LTV boundary for borrow (Rust ltv_boundary_for_borrow)", () => {
  const s = bal({ collateralSol: 1_000_000_000n });
  const c = ctx(1_000_000_000n, 100_000_000n);
  const ok = applyLendOp(s, LendOp.Borrow, 75_000_000n, 75_000_000n, c);
  assert.equal(ok.applied, true);
  assert.equal(ok.state.debtSharesUsdc, 75_000_000n);
  assert.equal(ok.state.freeUsdc, 75_000_000n);
  const no = applyLendOp(s, LendOp.Borrow, 75_000_001n, 75_000_001n, c);
  assert.equal(no.applied, false);
  assert.deepEqual(no.state, s);
});

test("health check never mutates (Rust health_check_never_mutates)", () => {
  const s = bal({ collateralSol: 1_000_000_000n, debtSharesUsdc: 90_000_000n });
  const r = applyLendOp(
    s,
    LendOp.HealthCheck,
    5n,
    5n,
    ctx(1_000_000_000n, 100_000_000n),
  );
  assert.equal(r.applied, true);
  assert.deepEqual(r.state, s);
});

test("liquidation applies on unhealthy, within close factor", () => {
  const s = bal({ collateralSol: 1_000_000_000n, debtSharesUsdc: 90_000_000n });
  const c = ctx(1_000_000_000n, 100_000_000n);
  const ok = applyLendOp(s, LendOp.Liquidate, 500_000_000n, 45_000_000n, c);
  assert.equal(ok.applied, true);
  assert.equal(ok.state.collateralSol, 500_000_000n);
  assert.equal(ok.state.debtSharesUsdc, 45_000_000n);
  // above half and not full close
  assert.equal(
    applyLendOp(s, LendOp.Liquidate, 1n, 45_000_001n, c).applied,
    false,
  );
  // healthy position refuses
  const healthy = bal({ collateralSol: 1_000_000_000n, debtSharesUsdc: 10n });
  assert.equal(
    applyLendOp(healthy, LendOp.Liquidate, 1n, 5n, c).applied,
    false,
  );
});

test("supply and withdraw supply move free USDC and shares", () => {
  const c = ctx(1_000_000_000n, 100_000_000n);
  const s = bal({ freeUsdc: 100n });
  const sup = applyLendOp(s, LendOp.Supply, 100n, 100n, c);
  assert.equal(sup.applied, true);
  assert.deepEqual(sup.state, bal({ supplySharesUsdc: 100n }));
  const wd = applyLendOp(sup.state, LendOp.WithdrawSupply, 100n, 100n, c);
  assert.deepEqual(wd.state, bal({ freeUsdc: 100n }));
  assert.equal(
    applyLendOp(sup.state, LendOp.WithdrawSupply, 101n, 101n, c).applied,
    false,
  );
});

// Property 3: full repay clears debt exactly.
test("Property 3: full repay burns exactly the debt shares", () => {
  const rng = new Lcg(33n);
  for (let i = 0; i < 2_000; i++) {
    const shares = rng.below(1_000_000_000_000n);
    const idx = INDEX_SCALE + rng.below(MAX_INDEX_1E9 - INDEX_SCALE);
    assert.equal(repayBurn(fullRepayAmount(shares, idx), idx), shares);
  }
});

// Property 4: supply then withdraw never creates value.
test("Property 4: supply then withdraw never creates value", () => {
  const rng = new Lcg(44n);
  for (let i = 0; i < 2_000; i++) {
    const amount = rng.below(1_000_000_000_000n);
    const idx = INDEX_SCALE + rng.below(MAX_INDEX_1E9 - INDEX_SCALE);
    const minted = supplySharesFor(amount, idx);
    assert.ok(supplyValue(minted, idx) <= amount);
    const grown =
      idx + idx / 1_000n > MAX_INDEX_1E9 ? MAX_INDEX_1E9 : idx + idx / 1_000n;
    assert.ok(supplyValue(minted, grown) >= supplyValue(minted, idx));
    assert.ok(withdrawSupplyBurn(amount, idx) >= minted);
    assert.ok(accruedSupply(minted, idx) <= amount);
  }
});

// Property 5: indexes monotonic, interest conserved.
function pool(supply: bigint, debt: bigint): LendingPool {
  return {
    market: PublicKey.default,
    bump: 255,
    oracle: PublicKey.default,
    feedId: new Uint8Array(32),
    maxPriceAgeSeconds: 86_400,
    totalSupplyShares: supply,
    totalDebtShares: debt,
    supplyIndex1e9: INDEX_SCALE,
    borrowIndex1e9: INDEX_SCALE,
    lastAccrualTs: 0n,
    reservesUsdc: 0n,
  };
}

test("empty pool does not move (Rust empty_pool_does_not_move)", () => {
  const p = pool(0n, 0n);
  const r = projectIndexes(p, 1_000_000n, { reserveFactorBps: 1_000n });
  assert.equal(r.borrowIndex1e9, INDEX_SCALE);
  assert.equal(r.supplyIndex1e9, INDEX_SCALE);
  assert.equal(utilizationBps(p), 0n);
});

test("interest accrues and splits (Rust interest_accrues_and_splits)", () => {
  const p = pool(1_000_000_000_000n, 800_000_000_000n);
  const r = projectIndexes(p, 31_536_000n, { reserveFactorBps: 1_000n });
  assert.ok(r.borrowIndex1e9 > INDEX_SCALE);
  assert.ok(r.supplyIndex1e9 > INDEX_SCALE);
  assert.ok(r.reservesUsdc > 0n);
  assert.throws(() =>
    projectIndexes({ ...p, lastAccrualTs: 5n }, 1n, {
      reserveFactorBps: 1_000n,
    }),
  );
  assert.ok(borrowAprBps(p) > 0n);
  assert.ok(supplyApyBps(p, 1_000n) > 0n);
  assert.ok(accruedDebt(1_000n, r.borrowIndex1e9) >= 1_000n);
});

test("Property 5: indexes monotonic and interest conserved", () => {
  const rng = new Lcg(55n);
  for (let i = 0; i < 1_000; i++) {
    const supply = 1n + rng.below(1_000_000_000_000_000n);
    const debt = rng.below(supply + 1n);
    const rf = rng.below(10_001n);
    const dt = rng.below(100_000_000n);
    const p = pool(supply, debt);
    const r = projectIndexes(p, dt, { reserveFactorBps: rf });
    assert.ok(
      r.borrowIndex1e9 >= INDEX_SCALE && r.supplyIndex1e9 >= INDEX_SCALE,
    );
    const interest =
      (debt * r.borrowIndex1e9) / INDEX_SCALE -
      (debt * INDEX_SCALE) / INDEX_SCALE;
    const reserve = r.reservesUsdc;
    const gain =
      supplyValue(supply, r.supplyIndex1e9) - supplyValue(supply, INDEX_SCALE);
    assert.ok(gain + reserve <= interest + 1n);
    assert.equal(reserve, (interest * rf) / 10_000n);
  }
});

test("decodeLendingPool reads the 253-byte layout", () => {
  const data = new Uint8Array(LENDING_POOL_ACCOUNT_LEN);
  const view = new DataView(data.buffer);
  data[40] = 254;
  view.setUint32(105, 86_400, true);
  view.setBigUint64(109, 7n, true); // total_supply_shares low
  view.setBigUint64(157, INDEX_SCALE, true); // borrow index low
  view.setBigInt64(173, 42n, true);
  view.setBigUint64(181, 9n, true);
  const d = decodeLendingPool(data);
  assert.equal(d.bump, 254);
  assert.equal(d.maxPriceAgeSeconds, 86_400);
  assert.equal(d.totalSupplyShares, 7n);
  assert.equal(d.borrowIndex1e9, INDEX_SCALE);
  assert.equal(d.lastAccrualTs, 42n);
  assert.equal(d.reservesUsdc, 9n);
});
