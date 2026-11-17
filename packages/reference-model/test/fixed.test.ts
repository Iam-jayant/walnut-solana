// Replays the shared golden vectors (vectors/fixed.vectors, rate.vectors,
// checkpoint.vectors) on the TypeScript mirrors. The Rust program
// (programs/walnut/src/math) and the Arcis helpers (encrypted-ixs) replay the
// same files, so all three implementations agree (Tasks 7.1 to 7.3).
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  FixedPointError,
  MAX_BORROW_INDEX,
  MAX_SHARES,
  WAD,
  bandFromPool,
  borrowRateBps,
  checkpoint,
  debtFromShares,
  divWadCeil,
  divWadFloor,
  mulDivCeil,
  mulDivFloor,
  mulWadCeil,
  mulWadCeilWide,
  mulWadFloor,
  mulWadFloorWide,
  projectIndex,
  ratePerSecondWad,
  sharesFromAmountCeil,
  sharesFromAmountFloor,
  utilizationBand,
  isBandFresh,
  type PoolPlain,
} from "../src/index.ts";

function vectors(name: string): string[] {
  return readFileSync(new URL(`../vectors/${name}`, import.meta.url), "utf8")
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l !== "" && !l.startsWith("#"));
}

/** Runs `fn`, returning the decimal result or the error code, like the vector files. */
function outcome(fn: () => bigint | number): string {
  try {
    return fn().toString();
  } catch (error) {
    if (error instanceof FixedPointError) return error.code;
    throw error;
  }
}

test("fixed-point vectors", () => {
  let checked = 0;
  for (const line of vectors("fixed.vectors")) {
    const w = line.split(/\s+/);
    const n = (i: number): bigint => BigInt(w[i] as string);
    const last = w[w.length - 1] as string;
    const got = ((): string => {
      switch (w[0]) {
        case "mul_div_floor": return outcome(() => mulDivFloor(n(1), n(2), n(3)));
        case "mul_div_ceil": return outcome(() => mulDivCeil(n(1), n(2), n(3)));
        case "mul_wad_floor": return outcome(() => mulWadFloor(n(1), n(2)));
        case "mul_wad_ceil": return outcome(() => mulWadCeil(n(1), n(2)));
        case "div_wad_floor": return outcome(() => divWadFloor(n(1), n(2)));
        case "div_wad_ceil": return outcome(() => divWadCeil(n(1), n(2)));
        case "mul_wad_floor_wide": return outcome(() => mulWadFloorWide(n(1), n(2)));
        case "mul_wad_ceil_wide": return outcome(() => mulWadCeilWide(n(1), n(2)));
        case "debt_from_shares": return outcome(() => debtFromShares(n(1), n(2)));
        case "shares_from_amount_ceil": return outcome(() => sharesFromAmountCeil(n(1), n(2)));
        case "shares_from_amount_floor": return outcome(() => sharesFromAmountFloor(n(1), n(2)));
        default: throw new Error(`unknown vector ${line}`);
      }
    })();
    assert.equal(got, last, line);
    checked++;
  }
  assert.ok(checked >= 100);
});

test("rate, band, and projection vectors", () => {
  let checked = 0;
  for (const line of vectors("rate.vectors")) {
    const w = line.split(/\s+/);
    const n = (i: number): bigint => BigInt(w[i] as string);
    const last = w[w.length - 1] as string;
    let got: string;
    switch (w[0]) {
      case "band": got = outcome(() => utilizationBand(n(1), n(2))); break;
      case "band_pool": got = outcome(() => bandFromPool(n(1), n(2), n(3))); break;
      case "rate_bps":
        got = outcome(() =>
          borrowRateBps(
            { baseRateBps: n(1), slope1Bps: n(2), slope2Bps: n(3), optimalUtilizationBps: n(4) },
            n(5),
          ),
        );
        break;
      case "rate_per_second": got = outcome(() => ratePerSecondWad(n(1))); break;
      case "project": got = outcome(() => projectIndex(n(1), n(2), n(3), n(4))); break;
      default: throw new Error(`unknown vector ${line}`);
    }
    assert.equal(got, last, line);
    checked++;
  }
  assert.ok(checked >= 80);
});

test("checkpoint vectors", () => {
  let checked = 0;
  for (const line of vectors("checkpoint.vectors")) {
    const [input, expected] = line.split("=>").map((s) => s.trim()) as [string, string];
    const w = input.split(/\s+/).map((x, i) => (i === 0 ? 0n : BigInt(x)));
    const pool: PoolPlain = {
      cash: w[1] as bigint,
      totalSupplyShares: w[2] as bigint,
      totalDebtShares: w[3] as bigint,
      reserves: w[4] as bigint,
    };
    let got: string;
    try {
      const c = checkpoint(pool, w[5] as bigint, w[6] as bigint, w[7] as bigint);
      got = [c.pool.reserves, c.supplierEquity, c.interest, c.reserveAdd].join(" ");
    } catch (error) {
      if (!(error instanceof FixedPointError)) throw error;
      got = "err";
    }
    assert.equal(got, expected, line);
    checked++;
  }
  assert.ok(checked >= 30);
});

test("checkpoint conserves interest and the NAV identity (property run)", () => {
  let seed = 0x7301n;
  const next = (): bigint => {
    seed = (seed * 6364136223846793005n + 1442695040888963407n) & ((1n << 64n) - 1n);
    return seed >> 11n;
  };
  const below = (n: bigint): bigint => ((next() << 64n) ^ next()) % n;
  for (let i = 0; i < 2000; i++) {
    const shares = below(MAX_SHARES + 1n);
    const accounted = WAD + below(MAX_BORROW_INDEX / 4n);
    const accounting = accounted + below(MAX_BORROW_INDEX / 4n);
    const rf = below(10_001n);
    const cash = below(1n << 80n);
    const reserves = below(cash + debtFromShares(shares, accounted) + 1n);
    const pool: PoolPlain = { cash, totalSupplyShares: 1n, totalDebtShares: shares, reserves };
    const c = checkpoint(pool, accounted, accounting, rf);
    assert.equal(c.reserveAdd + c.supplierInterest, c.interest);
    assert.equal(c.supplierEquity, cash + c.newDebt - c.pool.reserves);
    assert.ok(c.newDebt >= c.oldDebt && c.pool.reserves >= reserves);
    const again = checkpoint(c.pool, accounting, accounting, rf);
    assert.deepEqual(again.pool, c.pool);
  }
});

test("band freshness blocks stale, never-observed, and future observations", () => {
  assert.equal(isBandFresh(0n, 10n, 100n), false);
  assert.equal(isBandFresh(11n, 10n, 100n), false);
  assert.equal(isBandFresh(10n, 110n, 100n), true);
  assert.equal(isBandFresh(10n, 111n, 100n), false);
});