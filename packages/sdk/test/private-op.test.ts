// ADR-0004 private operations: the client conversions must always pass the
// circuit checks (mirrored in src/private-op.ts), and the band-mode index
// projection must mirror `LendingPool::accrue_banded`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { PublicKey } from "@solana/web3.js";
import { x25519 } from "@arcium-hq/client";
import {
  chooseAccrualTs,
  decodeLendingPool,
  encryptPrivateInput,
  fullRepayAmountPrivate,
  INDEX_SCALE,
  LENDING_POOL_ACCOUNT_LEN,
  liquidationConversionHolds,
  liquidationInputs,
  PoolOp,
  poolOpConversionHolds,
  poolOpShares,
  projectIndexes,
  projectIndexesAuto,
  projectIndexesBanded,
  RATE_SOURCE_BAND,
  RATE_SOURCE_TOTALS,
  type LendingPool,
} from "../src/index.ts";

class Lcg {
  #s: bigint;
  constructor(seed: bigint) {
    this.#s = seed;
  }
  next(): bigint {
    this.#s =
      (this.#s * 6364136223846793005n + 1442695040888963407n) &
      ((1n << 64n) - 1n);
    return this.#s;
  }
  below(n: bigint): bigint {
    return n === 0n ? 0n : this.next() % n;
  }
}

function pool(rateSource = RATE_SOURCE_TOTALS): LendingPool {
  return {
    market: PublicKey.default,
    bump: 255,
    oracle: PublicKey.default,
    feedId: new Uint8Array(32),
    maxPriceAgeSeconds: 86_400,
    totalSupplyShares: 0n,
    totalDebtShares: 0n,
    supplyIndex1e9: INDEX_SCALE,
    borrowIndex1e9: INDEX_SCALE,
    lastAccrualTs: 0n,
    reservesUsdc: 0n,
    rateSource,
  };
}

test("client share conversions always pass the circuit checks", () => {
  const rng = new Lcg(11n);
  for (let i = 0; i < 5_000; i++) {
    const sidx = INDEX_SCALE + rng.below(9n * INDEX_SCALE);
    const bidx = INDEX_SCALE + rng.below(9n * INDEX_SCALE);
    const amount = 1n + rng.below(1_000_000_000_000n);
    for (const op of [
      PoolOp.Supply,
      PoolOp.WithdrawSupply,
      PoolOp.Borrow,
      PoolOp.Repay,
    ]) {
      const shares = poolOpShares(op, amount, sidx, bidx);
      if (shares === 0n) continue;
      assert.ok(
        poolOpConversionHolds(op, amount, shares, sidx, bidx),
        `op ${op} a ${amount} s ${shares}`,
      );
      // Any other share count is refused.
      assert.ok(!poolOpConversionHolds(op, amount, shares + 1n, sidx, bidx));
      if (shares > 1n) {
        assert.ok(!poolOpConversionHolds(op, amount, shares - 1n, sidx, bidx));
      }
    }
  }
});

test("conversions round against the user", () => {
  const idx = 1_500_000_000n; // 1.5
  // supply 10 units: floor(10e9 / 1.5e9) = 6 shares, worth 9 <= 10
  assert.equal(poolOpShares(PoolOp.Supply, 10n, idx, idx), 6n);
  // borrow 10 units: ceil = 7 debt shares, owed 10.5 >= 10
  assert.equal(poolOpShares(PoolOp.Borrow, 10n, idx, idx), 7n);
  // withdraw 10 units burns ceil = 7 supply shares
  assert.equal(poolOpShares(PoolOp.WithdrawSupply, 10n, idx, idx), 7n);
  // repay 10 units burns floor = 6 debt shares
  assert.equal(poolOpShares(PoolOp.Repay, 10n, idx, idx), 6n);
});

test("full private repay amount burns exactly the debt shares", () => {
  const rng = new Lcg(5n);
  for (let i = 0; i < 2_000; i++) {
    const idx = INDEX_SCALE + rng.below(9n * INDEX_SCALE);
    const debt = 1n + rng.below(1_000_000_000_000n);
    const a = fullRepayAmountPrivate(debt, idx);
    assert.equal(poolOpShares(PoolOp.Repay, a, idx, idx), debt);
  }
});

test("liquidation inputs pass the circuit checks", () => {
  const rng = new Lcg(9n);
  let checked = 0;
  for (let i = 0; i < 3_000; i++) {
    const bidx = INDEX_SCALE + rng.below(9n * INDEX_SCALE);
    const price = 1n + rng.below(1_000_000_000n);
    const bonus = rng.below(2_000n);
    const amount = 1n + rng.below(1_000_000_000n);
    const li = liquidationInputs(amount, bidx, price, bonus);
    if (li.burnShares === 0n || li.seize === 0n) continue;
    checked++;
    assert.ok(liquidationConversionHolds(li, bidx, price, bonus));
    assert.ok(
      !liquidationConversionHolds(
        { ...li, seize: li.seize + 1n },
        bidx,
        price,
        bonus,
      ),
    );
  }
  assert.ok(checked > 1_000);
});

test("encrypted input carries a fresh key, nonce and one ciphertext per value", () => {
  const mxe = x25519.getPublicKey(x25519.utils.randomSecretKey());
  const a = encryptPrivateInput(mxe, [5_000_000n, 5_000_000n]);
  const b = encryptPrivateInput(mxe, [5_000_000n, 5_000_000n]);
  assert.equal(a.publicKey.length, 32);
  assert.equal(a.ciphertexts.length, 2);
  assert.ok(a.ciphertexts.every((c) => c.length === 32));
  assert.notDeepEqual(a.publicKey, b.publicKey);
  assert.notEqual(a.nonce, b.nonce);
  assert.notDeepEqual(a.ciphertexts, b.ciphertexts);
  assert.throws(() => encryptPrivateInput(mxe, [-1n]));
});

test("accrual timestamp stays within the accepted window", () => {
  assert.equal(chooseAccrualTs(100n, 1_000n), 985n);
  assert.equal(chooseAccrualTs(995n, 1_000n), 995n);
  assert.throws(() => chooseAccrualTs(800n, 1_000n, 500n));
});

test("rate source byte decodes at offset 189", () => {
  const data = new Uint8Array(LENDING_POOL_ACCOUNT_LEN);
  const view = new DataView(data.buffer);
  view.setBigUint64(141, INDEX_SCALE, true);
  view.setBigUint64(157, INDEX_SCALE, true);
  data[189] = RATE_SOURCE_BAND;
  assert.equal(decodeLendingPool(data).rateSource, RATE_SOURCE_BAND);
});

test("band 0 grows only the borrow index (Rust band_zero_grows_only_the_borrow_index)", () => {
  const r = projectIndexesBanded(pool(), 31_536_000n, {
    reserveFactorBps: 1_000n,
    band: 0,
  });
  assert.ok(r.borrowIndex1e9 > INDEX_SCALE);
  assert.equal(r.supplyIndex1e9, INDEX_SCALE);
});

test("banded suppliers are never credited more than borrowers pay", () => {
  const rng = new Lcg(77n);
  for (let i = 0; i < 2_000; i++) {
    const band = Number(rng.below(20n));
    const supply = 1_000_000n + rng.below(1_000_000_000_000_000n);
    const lo = (supply * BigInt(band) * 500n) / 10_000n;
    let hi = (supply * (BigInt(band) + 1n) * 500n) / 10_000n;
    if (hi > supply) hi = supply;
    const debt = lo + rng.below(hi - lo + 1n);
    const rf = rng.below(10_001n);
    const dt = 1n + rng.below(100_000_000n);
    const r = projectIndexesBanded(pool(), dt, { reserveFactorBps: rf, band });
    const paid = (debt * r.borrowIndex1e9) / INDEX_SCALE - debt;
    const credited = (supply * r.supplyIndex1e9) / INDEX_SCALE - supply;
    assert.ok(credited <= paid + 1n, `band ${band}`);
  }
});

test("banded accrual golden vectors (Rust banded_accrual_golden_vectors)", () => {
  const vectors: [number, bigint, bigint, bigint, bigint][] = [
    [15, 1_000n, 31_536_000n, 1_040_000_001n, 1_025_200_000n],
    [3, 500n, 86_400n, 1_000_027_398n, 1_000_002_739n],
    [19, 0n, 3_600n, 1_000_090_183n, 1_000_065_331n],
  ];
  for (const [band, rf, dt, borrow, supply] of vectors) {
    const r = projectIndexesBanded(pool(), dt, { reserveFactorBps: rf, band });
    assert.equal(r.borrowIndex1e9, borrow);
    assert.equal(r.supplyIndex1e9, supply);
  }
});

test("auto projection follows the rate source", () => {
  const opts = { reserveFactorBps: 1_000n, band: 15 };
  const totals = { ...pool(RATE_SOURCE_TOTALS), totalSupplyShares: 1_000_000n };
  assert.deepEqual(
    projectIndexesAuto(totals, 31_536_000n, opts),
    projectIndexes(totals, 31_536_000n, opts),
  );
  const banded = pool(RATE_SOURCE_BAND);
  assert.deepEqual(
    projectIndexesAuto(banded, 31_536_000n, opts),
    projectIndexesBanded(banded, 31_536_000n, opts),
  );
  assert.ok(
    projectIndexesAuto(banded, 31_536_000n, opts).supplyIndex1e9 > INDEX_SCALE,
  );
});
