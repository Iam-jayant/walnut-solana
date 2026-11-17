// Rate model, Borrow Index projection, and utilization bands (Task 7.2): the
// bigint mirror of programs/walnut/src/math/rate.rs. Replays vectors/rate.vectors.
import {
  BPS_DENOMINATOR,
  FixedPointError,
  MAX_BORROW_INDEX,
  WAD,
  debtFromShares,
  mulWadCeilWide,
} from "./fixed.ts";

export const SECONDS_PER_YEAR = 31_536_000n;
export const BAND_COUNT = 20;
export const BAND_WIDTH_BPS = 500n;
export const MAX_BORROW_RATE_BPS = 100_000n;
export const MAX_ELAPSED_SECONDS = 10_000_000_000n;
/** Conservative seconds per slot used to project through Decision Expiry. */
export const SECONDS_PER_SLOT_UPPER_BOUND = 1n;

export interface RateParams {
  baseRateBps: bigint;
  slope1Bps: bigint;
  slope2Bps: bigint;
  optimalUtilizationBps: bigint;
}

const ceilDiv = (a: bigint, b: bigint): bigint => (a + b - 1n) / b;

/** Upper utilization edge of `band` in bps: (band + 1) x 500. */
export function bandUpperEdgeBps(band: number): bigint {
  if (!Number.isInteger(band) || band < 0 || band >= BAND_COUNT) {
    throw new FixedPointError("invalid");
  }
  return (BigInt(band) + 1n) * BAND_WIDTH_BPS;
}

/** Annual borrow rate in bps at `utilizationBps` (kinked, capped). */
export function borrowRateBps(
  rates: RateParams,
  utilizationBps: bigint,
): bigint {
  const opt = rates.optimalUtilizationBps;
  if (
    utilizationBps > BPS_DENOMINATOR ||
    opt === 0n ||
    opt >= BPS_DENOMINATOR
  ) {
    throw new FixedPointError("invalid");
  }
  const rate =
    utilizationBps <= opt
      ? rates.baseRateBps + ceilDiv(rates.slope1Bps * utilizationBps, opt)
      : rates.baseRateBps +
        rates.slope1Bps +
        ceilDiv(
          rates.slope2Bps * (utilizationBps - opt),
          BPS_DENOMINATOR - opt,
        );
  return rate > MAX_BORROW_RATE_BPS ? MAX_BORROW_RATE_BPS : rate;
}

/** ceil(rateBps x WAD / (10_000 x SECONDS_PER_YEAR)). */
export function ratePerSecondWad(rateBps: bigint): bigint {
  if (rateBps < 0n || rateBps > MAX_BORROW_RATE_BPS)
    throw new FixedPointError("invalid");
  return ceilDiv(rateBps * WAD, BPS_DENOMINATOR * SECONDS_PER_YEAR);
}

/** The per-second WAD rate accrual uses for `band`: the rate at its upper edge. */
export function bandRatePerSecondWad(rates: RateParams, band: number): bigint {
  return ratePerSecondWad(borrowRateBps(rates, bandUpperEdgeBps(band)));
}

/** index + ceil(index x rate x elapsed / WAD); never decreases, identity at zero elapsed. */
export function projectIndex(
  index: bigint,
  indexTs: bigint,
  targetTs: bigint,
  ratePerSecond: bigint,
): bigint {
  if (index < WAD || index > MAX_BORROW_INDEX)
    throw new FixedPointError("range");
  if (targetTs < indexTs) throw new FixedPointError("regress");
  const elapsed = targetTs - indexTs;
  if (elapsed === 0n) return index;
  if (elapsed > MAX_ELAPSED_SECONDS) throw new FixedPointError("elapsed");
  const next = index + mulWadCeilWide(index, ratePerSecond * elapsed);
  if (next > MAX_BORROW_INDEX) throw new FixedPointError("range");
  return next;
}

/** Timestamp a decision `slotsToExpiry` slots after `nowTs` can still commit at. */
export function expiryTimestamp(nowTs: bigint, slotsToExpiry: bigint): bigint {
  return nowTs + slotsToExpiry * SECONDS_PER_SLOT_UPPER_BOUND;
}

/** Band of debt / (cash + debt): the count of edges 5k% (k = 1..19) reached. */
export function utilizationBand(cash: bigint, debt: bigint): number {
  const total = cash + debt;
  if (total === 0n) return 0;
  let band = 0;
  for (let k = 1n; k < BigInt(BAND_COUNT); k++) {
    if (debt * BPS_DENOMINATOR >= total * k * BAND_WIDTH_BPS) band++;
  }
  return band;
}

/** Band from Pool aggregates: debt = ceil(debtShares x index / WAD). */
export function bandFromPool(
  cash: bigint,
  debtShares: bigint,
  index: bigint,
): number {
  return utilizationBand(cash, debtFromShares(debtShares, index));
}

/** Whether a band observation is usable (stale, never-observed, and future bands block new risk). */
export function isBandFresh(
  observationSlot: bigint,
  currentSlot: bigint,
  maxAgeSlots: bigint,
): boolean {
  if (observationSlot === 0n || observationSlot > currentSlot) return false;
  return currentSlot - observationSlot <= maxAgeSlots;
}
