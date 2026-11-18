// Supply, withdraw supply, and recovery on the TypeScript mirror (Task 8). The
// Rust counterparts are programs/walnut/src/math/supply/tests.rs and the
// circuit replay in encrypted-ixs/src/lib.rs; all three replay
// vectors/supply.vectors.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  INITIAL_SHARE_SCALE,
  RECOVERY_DUST_LIMIT,
  WAD,
  checkpoint,
  isCanonicalEmpty,
  recoverEmptyPool,
  reserveWithdrawal,
  supplyFlow,
  type PoolPlain,
  type SupplyRequest,
} from "../src/index.ts";

const RF = 1000;
const MAX64 = (1n << 64n) - 1n;
const VECTORS = readFileSync(
  new URL("../vectors/supply.vectors", import.meta.url),
  "utf8",
);

const pool = (
  cash: bigint,
  ts: bigint,
  ds: bigint,
  reserves: bigint,
): PoolPlain => ({
  cash,
  totalSupplyShares: ts,
  totalDebtShares: ds,
  reserves,
});
const supply = (requested: bigint): SupplyRequest => ({
  isWithdraw: false,
  requested,
  supplyCap: MAX64,
  claimsClear: true,
});
const withdraw = (requested: bigint): SupplyRequest => ({
  isWithdraw: true,
  requested,
  supplyCap: MAX64,
  claimsClear: true,
});
const equityAt = (p: PoolPlain, index: bigint): bigint =>
  checkpoint(p, index, index, BigInt(RF)).supplierEquity;

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

test("supply vectors", () => {
  let checked = 0;
  for (const raw of VECTORS.split("\n")) {
    const line = raw.trim();
    if (line === "" || line.startsWith("#")) continue;
    const [input, expected] = line.split("=>").map((s) => s.trim()) as [
      string,
      string,
    ];
    const w = input.split(/\s+/);
    const e = expected.split(/\s+/);
    const n = (i: number): bigint => BigInt(w[i] as string);
    const p = pool(n(1), n(2), n(3), n(4));
    if (w[0] === "supply") {
      const position = { freeUsdc: n(5), supplyShares: n(6) };
      const out = supplyFlow(p, position, n(7), n(8), Number(n(9)), {
        isWithdraw: w[10] === "1",
        requested: n(11),
        supplyCap: n(12),
        claimsClear: w[13] === "1",
      });
      if (e[0] === "zero") {
        assert.equal(out.applied, false, line);
        assert.deepEqual(out.pool, p, line);
        assert.deepEqual(out.position, position, line);
      } else {
        assert.equal(out.applied, true, line);
        assert.deepEqual(
          [
            out.pool.cash,
            out.pool.totalSupplyShares,
            out.pool.reserves,
            out.position.freeUsdc,
            out.position.supplyShares,
          ],
          e.map((x) => BigInt(x)),
          line,
        );
      }
    } else {
      const out = recoverEmptyPool(
        p,
        n(5),
        n(6),
        Number(n(7)),
        n(8),
        w[9] === "1",
      );
      if (e[0] === "zero") {
        assert.equal(out.applied, false, line);
        assert.deepEqual(out.pool, p, line);
      } else {
        assert.equal(out.applied, true, line);
        assert.equal(out.pool.reserves, BigInt(e[0] as string), line);
      }
    }
    checked += 1;
  }
  assert.ok(checked >= 39);
});

test("first supplier mints at the initial scale and the last exits whole", () => {
  const first = supplyFlow(
    pool(0n, 0n, 0n, 0n),
    { freeUsdc: 7_777n, supplyShares: 0n },
    WAD,
    WAD,
    RF,
    supply(7_777n),
  );
  assert.equal(first.position.supplyShares, 7_777n * INITIAL_SHARE_SCALE);
  const exit = supplyFlow(
    first.pool,
    first.position,
    WAD,
    WAD,
    RF,
    withdraw(first.position.supplyShares),
  );
  assert.equal(exit.position.freeUsdc, 7_777n);
  assert.ok(isCanonicalEmpty(exit.pool, true));
});

test("a supplier just after an accrual gets no earlier interest", () => {
  const p = pool(1_000_000n, 1_000_000_000n, 500_000n, 0n);
  const out = supplyFlow(
    p,
    { freeUsdc: 195_000n, supplyShares: 0n },
    WAD,
    2n * WAD,
    RF,
    supply(195_000n),
  );
  assert.equal(out.position.supplyShares, 100_000_000n);
  const exit = supplyFlow(
    out.pool,
    out.position,
    2n * WAD,
    2n * WAD,
    RF,
    withdraw(out.position.supplyShares),
  );
  assert.equal(exit.position.freeUsdc, 195_000n);
  // Interest accrued afterwards is shared pro rata.
  const later = supplyFlow(
    out.pool,
    out.position,
    2n * WAD,
    3n * WAD,
    RF,
    withdraw(out.position.supplyShares),
  );
  assert.equal(later.position.freeUsdc, 195_000n + 40_909n);
});

test("a supplier just before an accrual shares the later interest", () => {
  const p = pool(1_000_000n, 1_000_000_000n, 500_000n, 0n);
  const out = supplyFlow(
    p,
    { freeUsdc: 1_000_000n, supplyShares: 0n },
    WAD,
    WAD,
    RF,
    supply(1_000_000n),
  );
  const exit = supplyFlow(
    out.pool,
    out.position,
    WAD,
    2n * WAD,
    RF,
    withdraw(out.position.supplyShares),
  );
  assert.equal(exit.position.freeUsdc, 1_000_000n + 179_999n);
});

test("property: a new supplier never captures earlier interest or dilutes others", () => {
  const below = sampler(0x5104n);
  let withInterest = 0;
  for (let i = 0; i < 2_000; i += 1) {
    const cash = 1n + below(1_000_000_000_000n);
    const debt = below(3n) === 0n ? 0n : below(1_000_000_000_000n);
    const p = pool(
      cash,
      1n + below(1_000_000_000_000_000n),
      debt,
      below(1n + cash / 2n),
    );
    const a = WAD + below(2n * WAD);
    const b = a + below(WAD);
    const cp = checkpoint(p, a, b, BigInt(RF));
    const amount = 1n + below(2n * cp.supplierEquity + 1_000_000n);
    const out = supplyFlow(
      p,
      { freeUsdc: MAX64, supplyShares: 0n },
      a,
      b,
      RF,
      supply(amount),
    );
    if (!out.applied) continue;
    if (cp.interest > 0n) withInterest += 1;
    const shares = out.position.supplyShares;
    const ts = out.pool.totalSupplyShares;
    const eqB = equityAt(out.pool, b);
    assert.ok((shares * eqB) / ts <= amount, "captured earlier interest");
    // Existing suppliers: equity per share never falls.
    assert.ok(
      eqB * p.totalSupplyShares >= cp.supplierEquity * ts,
      "diluted existing suppliers",
    );
    const c = b + below(WAD);
    const later = checkpoint(out.pool, b, c, BigInt(RF));
    const claimC = (shares * later.supplierEquity) / ts;
    assert.ok(claimC * ts <= shares * (eqB + later.supplierInterest));
  }
  assert.ok(withInterest > 500, `only ${withInterest} accruals`);
});

test("recovery reclassifies dust and reserve withdrawal then empties the pool", () => {
  const out = recoverEmptyPool(
    pool(7n, 0n, 0n, 0n),
    WAD,
    WAD,
    RF,
    RECOVERY_DUST_LIMIT,
    true,
  );
  assert.equal(out.applied, true);
  assert.equal(out.pool.reserves, 7n);
  const drained = reserveWithdrawal(out.pool, WAD, WAD, BigInt(RF), 7n);
  assert.equal(drained.approved, 7n);
  assert.ok(isCanonicalEmpty(drained.pool, true));
  // Larger residue, shares, debt, or unsettled claims are never recovered.
  for (const [p, clear] of [
    [pool(RECOVERY_DUST_LIMIT + 1n, 0n, 0n, 0n), true],
    [pool(7n, 1n, 0n, 0n), true],
    [pool(7n, 0n, 1n, 0n), true],
    [pool(7n, 0n, 0n, 0n), false],
  ] as const) {
    const refused = recoverEmptyPool(
      p,
      WAD,
      WAD,
      RF,
      RECOVERY_DUST_LIMIT,
      clear,
    );
    assert.equal(refused.applied, false);
    assert.deepEqual(refused.pool, p);
  }
});
