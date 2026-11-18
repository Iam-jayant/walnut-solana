import type { PositionBalances } from "@walnut/sdk";

const LAMPORTS = 1_000_000_000n;
const SCALE = LAMPORTS * 10_000n;

export interface BorrowMetrics {
  /** Borrow limit in USDC base units: collateral x price x LTV. */
  limit: bigint;
  debt: bigint;
  available: bigint;
  /** Debt as a percentage of the limit, 0 to 100. */
  usedPercent: number;
}

/** debt is the USDC value of the debt shares at the current index. */
export function borrowMetrics(
  balances: PositionBalances,
  price: bigint,
  ltvBps: bigint,
  debt: bigint,
): BorrowMetrics {
  const limit = (balances.collateralSol * price * ltvBps) / 10_000n / LAMPORTS;
  const available = limit > debt ? limit - debt : 0n;
  let usedPercent = 0;
  if (limit > 0n)
    usedPercent = Math.min(100, Number((debt * 10_000n) / limit) / 100);
  else if (debt > 0n) usedPercent = 100;
  return { limit, debt, available, usedPercent };
}

/** The largest collateral amount that can be removed with the debt covered. */
export function maxRemovableCollateral(
  balances: PositionBalances,
  price: bigint,
  ltvBps: bigint,
  debt: bigint,
): bigint {
  if (debt === 0n) return balances.collateralSol;
  const perLamport = price * ltvBps;
  if (perLamport === 0n) return 0n;
  const required = (debt * SCALE + perLamport - 1n) / perLamport;
  return balances.collateralSol > required
    ? balances.collateralSol - required
    : 0n;
}

export type Health = "none" | "healthy" | "moderate" | "high";

export function healthOf(usedPercent: number, debt: bigint): Health {
  if (debt === 0n) return "none";
  if (usedPercent < 60) return "healthy";
  if (usedPercent < 85) return "moderate";
  return "high";
}
