// Walnut-Lend (task 4.1): `LendingPool` decoding and the public index maths.
// Mirror of programs/walnut/src/state/lending_pool.rs and math/rate.rs.
//
// Layout (offsets include the 8-byte Anchor discriminator, 253 bytes total):
//   market 8..40, bump 40, oracle 41..73, feed_id 73..105,
//   max_price_age_seconds u32 105..109, total_supply_shares u128 109..125,
//   total_debt_shares u128 125..141, supply_index_1e9 u128 141..157,
//   borrow_index_1e9 u128 157..173, last_accrual_ts i64 173..181,
//   reserves_usdc u64 181..189, rate_source u8 189, reserved 190..253.
import { PublicKey, type Connection } from "@solana/web3.js";
import {
  debtValue,
  INDEX_SCALE,
  MAX_INDEX_1E9,
  supplyValue,
} from "./lend-op.ts";

/** Account length of `LendingPool`. */
export const LENDING_POOL_ACCOUNT_LEN = 253;

const WAD = 1_000_000_000_000_000_000n;
const WAD_PER_INDEX = WAD / INDEX_SCALE;
const BPS = 10_000n;
const SECONDS_PER_YEAR = 31_536_000n;
const MAX_BORROW_RATE_BPS = 100_000n;
const MAX_BORROW_INDEX_WAD = 1_000n * WAD;
const MAX_ELAPSED_SECONDS = 10_000_000_000n;
const U64_MAX = (1n << 64n) - 1n;

/** Decoded `LendingPool`. */
export interface LendingPool {
  market: PublicKey;
  bump: number;
  oracle: PublicKey;
  feedId: Uint8Array;
  maxPriceAgeSeconds: number;
  totalSupplyShares: bigint;
  totalDebtShares: bigint;
  supplyIndex1e9: bigint;
  borrowIndex1e9: bigint;
  lastAccrualTs: bigint;
  reservesUsdc: bigint;
  /** {@link RATE_SOURCE_TOTALS} or {@link RATE_SOURCE_BAND}. */
  rateSource: number;
}

/** Rates follow the public totals. */
export const RATE_SOURCE_TOTALS = 0;
/** Rates follow the revealed utilization band of the encrypted Pool (ADR-0004). */
export const RATE_SOURCE_BAND = 1;

function getU128(view: DataView, at: number): bigint {
  return view.getBigUint64(at, true) | (view.getBigUint64(at + 8, true) << 64n);
}

/** Decodes raw `LendingPool` account data. */
export function decodeLendingPool(data: Uint8Array): LendingPool {
  if (data.length < LENDING_POOL_ACCOUNT_LEN) {
    throw new Error(
      `LendingPool account is ${data.length} bytes, expected ${LENDING_POOL_ACCOUNT_LEN}`,
    );
  }
  const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
  return {
    market: new PublicKey(data.slice(8, 40)),
    bump: data[40] as number,
    oracle: new PublicKey(data.slice(41, 73)),
    feedId: data.slice(73, 105),
    maxPriceAgeSeconds: view.getUint32(105, true),
    totalSupplyShares: getU128(view, 109),
    totalDebtShares: getU128(view, 125),
    supplyIndex1e9: getU128(view, 141),
    borrowIndex1e9: getU128(view, 157),
    lastAccrualTs: view.getBigInt64(173, true),
    reservesUsdc: view.getBigUint64(181, true),
    rateSource: data[189] as number,
  };
}

/** Fetches and decodes the `LendingPool` at `address`. */
export async function fetchLendingPool(
  connection: Pick<Connection, "getAccountInfo">,
  address: PublicKey,
): Promise<LendingPool> {
  const info = await connection.getAccountInfo(address);
  if (!info) throw new Error(`LendingPool ${address.toBase58()} not found`);
  return decodeLendingPool(info.data);
}

// ---- Rate model (math/rate.rs) -----------------------------------------------

/** Rust `RateParams` (annualized basis points). */
export interface RateParams {
  baseRateBps: bigint;
  slope1Bps: bigint;
  slope2Bps: bigint;
  optimalUtilizationBps: bigint;
}

/** Rust `RateParams::LAUNCH_DEFAULT`. */
export const LAUNCH_RATE_PARAMS: RateParams = {
  baseRateBps: 0n,
  slope1Bps: 400n,
  slope2Bps: 7_500n,
  optimalUtilizationBps: 8_000n,
};

const divCeil = (n: bigint, d: bigint): bigint => (n + d - 1n) / d;

/** Annual borrow rate in bps at `utilizationBps` (0..=10,000). */
export function borrowRateBps(
  rates: RateParams,
  utilizationBps: bigint,
): bigint {
  const optimal = rates.optimalUtilizationBps;
  if (
    utilizationBps < 0n ||
    utilizationBps > BPS ||
    optimal <= 0n ||
    optimal >= BPS
  ) {
    throw new RangeError("invalid rate parameters");
  }
  const rate =
    utilizationBps <= optimal
      ? rates.baseRateBps + divCeil(rates.slope1Bps * utilizationBps, optimal)
      : rates.baseRateBps +
        rates.slope1Bps +
        divCeil(rates.slope2Bps * (utilizationBps - optimal), BPS - optimal);
  return rate > MAX_BORROW_RATE_BPS ? MAX_BORROW_RATE_BPS : rate;
}

/** `ceil(rateBps * WAD / (1e4 * SECONDS_PER_YEAR))`. */
export function ratePerSecondWad(rateBps: bigint): bigint {
  if (rateBps > MAX_BORROW_RATE_BPS) throw new RangeError("rate too high");
  return divCeil(rateBps * WAD, BPS * SECONDS_PER_YEAR);
}

/** Mirror of `project_index` (WAD scale). */
export function projectIndexWad(
  index: bigint,
  indexTs: bigint,
  targetTs: bigint,
  ratePerSecond: bigint,
): bigint {
  if (index < WAD || index > MAX_BORROW_INDEX_WAD) {
    throw new RangeError("index out of range");
  }
  if (targetTs < indexTs) throw new RangeError("time regression");
  const elapsed = targetTs - indexTs;
  if (elapsed === 0n) return index;
  if (elapsed > MAX_ELAPSED_SECONDS) throw new RangeError("elapsed too long");
  const next = index + divCeil(index * ratePerSecond * elapsed, WAD);
  if (next > MAX_BORROW_INDEX_WAD) throw new RangeError("index out of range");
  return next;
}

// ---- Pool maths --------------------------------------------------------------

/** `min(1e4, debtValue * 1e4 / supplyValue)`; 0 when nothing is supplied. */
export function utilizationBps(pool: LendingPool): bigint {
  const supply = supplyValue(pool.totalSupplyShares, pool.supplyIndex1e9);
  if (supply === 0n) return 0n;
  const debt = debtValue(pool.totalDebtShares, pool.borrowIndex1e9);
  const u = (debt * BPS) / supply;
  return u > BPS ? BPS : u;
}

/** Options of {@link projectIndexes}. */
export interface AccrualOptions {
  /** From the market's asset state (`risk.reserve_factor_bps`). */
  reserveFactorBps: bigint;
  /** Defaults to {@link LAUNCH_RATE_PARAMS}. */
  rates?: RateParams;
}

/** Indexes and reserves after accrual. */
export interface ProjectedIndexes {
  borrowIndex1e9: bigint;
  supplyIndex1e9: bigint;
  reservesUsdc: bigint;
}

/**
 * Mirror of `LendingPool::accrue`: the indexes the program would store if it
 * accrued at `now` (unix seconds). Throws where the program rejects.
 */
export function projectIndexes(
  pool: LendingPool,
  now: bigint | number,
  options: AccrualOptions,
): ProjectedIndexes {
  const nowTs = BigInt(now);
  const rates = options.rates ?? LAUNCH_RATE_PARAMS;
  if (nowTs < pool.lastAccrualTs) throw new RangeError("time regression");
  if (options.reserveFactorBps < 0n || options.reserveFactorBps > BPS) {
    throw new RangeError("invalid reserve factor");
  }
  const unchanged: ProjectedIndexes = {
    borrowIndex1e9: pool.borrowIndex1e9,
    supplyIndex1e9: pool.supplyIndex1e9,
    reservesUsdc: pool.reservesUsdc,
  };
  for (const i of [pool.borrowIndex1e9, pool.supplyIndex1e9]) {
    if (i < INDEX_SCALE || i > MAX_INDEX_1E9) {
      throw new RangeError("index out of range");
    }
  }
  if (nowTs === pool.lastAccrualTs) return unchanged;

  const supplyVal = supplyValue(pool.totalSupplyShares, pool.supplyIndex1e9);
  const debtBefore = debtValue(pool.totalDebtShares, pool.borrowIndex1e9);
  const perSecond = ratePerSecondWad(
    borrowRateBps(rates, utilizationBps(pool)),
  );
  const projected = projectIndexWad(
    pool.borrowIndex1e9 * WAD_PER_INDEX,
    pool.lastAccrualTs,
    nowTs,
    perSecond,
  );
  let newBorrow = divCeil(projected, WAD_PER_INDEX);
  if (newBorrow < pool.borrowIndex1e9) newBorrow = pool.borrowIndex1e9;
  if (newBorrow > MAX_INDEX_1E9) throw new RangeError("index out of range");

  const debtAfter = debtValue(pool.totalDebtShares, newBorrow);
  const interest = debtAfter > debtBefore ? debtAfter - debtBefore : 0n;
  const reserveCredit = (interest * options.reserveFactorBps) / BPS;
  const supplierInterest = interest - reserveCredit;
  const newSupply =
    supplyVal === 0n
      ? pool.supplyIndex1e9
      : pool.supplyIndex1e9 +
        (pool.supplyIndex1e9 * supplierInterest) / supplyVal;
  if (newSupply > MAX_INDEX_1E9) throw new RangeError("index out of range");
  const newReserves = pool.reservesUsdc + reserveCredit;
  if (newReserves > U64_MAX) throw new RangeError("reserves overflow");
  return {
    borrowIndex1e9: newBorrow,
    supplyIndex1e9: newSupply,
    reservesUsdc: newReserves,
  };
}

/** The band's `[lower, upper]` utilization edges in bps (5% bands, 0..=19). */
export function bandEdgesBps(band: number): [bigint, bigint] {
  const b = BigInt(Math.min(Math.max(Math.trunc(band), 0), 19));
  const upper = (b + 1n) * 500n;
  return [b * 500n, upper > BPS ? BPS : upper];
}

/**
 * Mirror of `LendingPool::accrue_banded` (ADR-0004): the borrow index grows
 * at the rate of the band's upper edge (rounded up), the supply index at
 * `rate(lower) * lower * (1 - reserve factor)` (rounded down). Reserves are
 * implicit and stay unchanged. Throws where the program rejects.
 */
export function projectIndexesBanded(
  pool: LendingPool,
  now: bigint | number,
  options: AccrualOptions & { band: number },
): ProjectedIndexes {
  const nowTs = BigInt(now);
  const rates = options.rates ?? LAUNCH_RATE_PARAMS;
  const rf = options.reserveFactorBps;
  if (nowTs < pool.lastAccrualTs) throw new RangeError("time regression");
  if (rf < 0n || rf > BPS) throw new RangeError("invalid reserve factor");
  for (const i of [pool.borrowIndex1e9, pool.supplyIndex1e9]) {
    if (i < INDEX_SCALE || i > MAX_INDEX_1E9) {
      throw new RangeError("index out of range");
    }
  }
  const unchanged: ProjectedIndexes = {
    borrowIndex1e9: pool.borrowIndex1e9,
    supplyIndex1e9: pool.supplyIndex1e9,
    reservesUsdc: pool.reservesUsdc,
  };
  if (nowTs === pool.lastAccrualTs) return unchanged;
  const [lower, upper] = bandEdgesBps(options.band);
  const projectedBorrow = projectIndexWad(
    pool.borrowIndex1e9 * WAD_PER_INDEX,
    pool.lastAccrualTs,
    nowTs,
    ratePerSecondWad(borrowRateBps(rates, upper)),
  );
  let newBorrow = divCeil(projectedBorrow, WAD_PER_INDEX);
  if (newBorrow < pool.borrowIndex1e9) newBorrow = pool.borrowIndex1e9;
  const supplyRate =
    (((borrowRateBps(rates, lower) * lower) / BPS) * (BPS - rf)) / BPS;
  let newSupply = pool.supplyIndex1e9;
  if (supplyRate !== 0n) {
    const projected = projectIndexWad(
      pool.supplyIndex1e9 * WAD_PER_INDEX,
      pool.lastAccrualTs,
      nowTs,
      ratePerSecondWad(supplyRate),
    );
    const floor = projected / WAD_PER_INDEX;
    newSupply = floor > pool.supplyIndex1e9 ? floor : pool.supplyIndex1e9;
  }
  if (newBorrow > MAX_INDEX_1E9 || newSupply > MAX_INDEX_1E9) {
    throw new RangeError("index out of range");
  }
  return {
    borrowIndex1e9: newBorrow,
    supplyIndex1e9: newSupply,
    reservesUsdc: pool.reservesUsdc,
  };
}

/**
 * Mirror of `LendingPool::accrue_auto`: band mode when the pool's
 * `rateSource` is {@link RATE_SOURCE_BAND}, else the public-totals accrual.
 * `band` is the Asset State's last revealed utilization band.
 */
export function projectIndexesAuto(
  pool: LendingPool,
  now: bigint | number,
  options: AccrualOptions & { band: number },
): ProjectedIndexes {
  return pool.rateSource === RATE_SOURCE_BAND
    ? projectIndexesBanded(pool, now, options)
    : projectIndexes(pool, now, options);
}

/**
 * Annual borrow rate (bps) at the pool's current utilization.
 * Display value; the program accrues per second at the same rate.
 */
export function borrowAprBps(
  pool: LendingPool,
  rates: RateParams = LAUNCH_RATE_PARAMS,
): bigint {
  return borrowRateBps(rates, utilizationBps(pool));
}

/**
 * Supplier APY (bps, simple annual rate): `borrowApr * utilization *
 * (1e4 - reserveFactor) / 1e8`.
 */
export function supplyApyBps(
  pool: LendingPool,
  reserveFactorBps: bigint,
  rates: RateParams = LAUNCH_RATE_PARAMS,
): bigint {
  const apr = borrowAprBps(pool, rates);
  return (apr * utilizationBps(pool) * (BPS - reserveFactorBps)) / (BPS * BPS);
}

/** USDC owed for `shares` of debt: `ceil(shares * index / 1e9)` (what a full repay costs). */
export function accruedDebt(shares: bigint, borrowIndex1e9: bigint): bigint {
  return divCeil(shares * borrowIndex1e9, INDEX_SCALE);
}

/** USDC redeemable for `shares` of supply: `floor(shares * index / 1e9)`. */
export function accruedSupply(shares: bigint, supplyIndex1e9: bigint): bigint {
  return (shares * supplyIndex1e9) / INDEX_SCALE;
}
