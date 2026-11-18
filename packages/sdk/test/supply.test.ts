import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  INITIAL_SHARE_SCALE,
  claimForShares,
  sharesForSupply,
} from "../src/supply.ts";

// The vectors are generated from the reference model and replayed by the Rust
// program math and the Arcis circuit helpers (Task 8).
const VECTORS = readFileSync(
  new URL("../../reference-model/vectors/supply.vectors", import.meta.url),
  "utf8",
);
const WAD = 10n ** 18n;
const ceilDiv = (a: bigint, b: bigint): bigint => (a + b - 1n) / b;

test("first-supplier scale and rounding directions", () => {
  assert.equal(INITIAL_SHARE_SCALE, 1_000n);
  assert.equal(sharesForSupply(2_000n, 0n, 0n), 2_000_000n);
  // One share is worth 10 units here, so 9 units mint floor(0.9) = 0 shares.
  assert.equal(sharesForSupply(9n, 1_000n, 10_000n), 0n);
  assert.equal(sharesForSupply(10n, 1_000n, 10_000n), 1n);
  // A claim rounds down: 1 share of 3 over equity 10 pays 3.
  assert.equal(claimForShares(1n, 10n, 3n), 3n);
  assert.throws(() => claimForShares(1n, 10n, 0n));
  assert.throws(() => sharesForSupply(1n, 5n, 0n));
});

test("matches the golden supply and withdraw vectors without interest", () => {
  let checked = 0;
  for (const raw of VECTORS.split("\n")) {
    const line = raw.trim();
    if (!line.startsWith("supply ")) continue;
    const [input, expected] = line.split("=>").map((s) => s.trim()) as [
      string,
      string,
    ];
    const w = input
      .split(/\s+/)
      .slice(1)
      .map((x) => BigInt(x));
    const [cash, ts, ds, reserves, , positionShares, accounted, accounting] =
      w as [bigint, bigint, bigint, bigint, bigint, bigint, bigint, bigint];
    const isWithdraw = w[9] === 1n;
    const requested = w[10] as bigint;
    if (accounted !== accounting || expected === "zero") continue;
    const equity = cash + ceilDiv(ds * accounting, WAD) - reserves;
    const out = expected.split(/\s+/).map((x) => BigInt(x));
    if (isWithdraw) {
      const claim = claimForShares(requested, equity, ts);
      assert.equal(out[0], cash - claim, line);
      assert.ok(positionShares >= requested);
    } else {
      const minted = sharesForSupply(requested, ts, equity);
      assert.equal(out[1], ts + minted, line);
    }
    checked += 1;
  }
  assert.ok(checked >= 4, `only ${checked} vectors replayed`);
});
