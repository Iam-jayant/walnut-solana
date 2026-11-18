// Lending display maths: projected indexes, market rates and the value of the
// decrypted shares. Positions store shares; USDC values are shares times the
// public index (shares * index / 1e9), projected to the current time so the
// numbers keep accruing between transactions.
import {
  accruedDebt,
  accruedSupply,
  borrowAprBps,
  INDEX_SCALE,
  projectIndexes,
  supplyApyBps,
  utilizationBps,
  type PositionBalances,
} from "@walnut/sdk";
import type { LendingView, Protocol } from "./chain.ts";

export interface Indexes {
  borrow: bigint;
  supply: bigint;
}

/** Indexes at `nowSeconds`; the stored ones when projection is not possible. */
export function indexesAt(
  lending: LendingView | null,
  protocol: Protocol | null,
  nowSeconds: number,
): Indexes {
  if (lending === null) return { borrow: INDEX_SCALE, supply: INDEX_SCALE };
  const { pool } = lending;
  const stored = {
    borrow: pool.borrowIndex1e9,
    supply: pool.supplyIndex1e9,
  };
  if (protocol === null) return stored;
  try {
    const now = Math.max(nowSeconds, Number(pool.lastAccrualTs));
    const p = projectIndexes(pool, now, {
      reserveFactorBps: protocol.reserveFactorBps,
      rates: protocol.rates,
    });
    return { borrow: p.borrowIndex1e9, supply: p.supplyIndex1e9 };
  } catch {
    return stored;
  }
}

export interface MarketStats {
  utilizationBps: bigint;
  borrowAprBps: bigint;
  supplyApyBps: bigint;
  totalSupplied: bigint;
  totalBorrowed: bigint;
  liquidity: bigint;
}

export function marketStats(
  lending: LendingView,
  protocol: Protocol,
  indexes: Indexes,
): MarketStats {
  const pool = {
    ...lending.pool,
    borrowIndex1e9: indexes.borrow,
    supplyIndex1e9: indexes.supply,
  };
  return {
    utilizationBps: utilizationBps(pool),
    borrowAprBps: borrowAprBps(pool, protocol.rates),
    supplyApyBps: supplyApyBps(pool, protocol.reserveFactorBps, protocol.rates),
    totalSupplied: accruedSupply(pool.totalSupplyShares, indexes.supply),
    totalBorrowed: accruedDebt(pool.totalDebtShares, indexes.borrow),
    liquidity: lending.liquidity,
  };
}

/** USDC values of the stored shares (debt rounds up: the cost of a full repay). */
export interface PositionValues {
  supply: bigint;
  debt: bigint;
}

export function positionValues(
  balances: PositionBalances,
  indexes: Indexes,
): PositionValues {
  return {
    supply: accruedSupply(balances.supplySharesUsdc, indexes.supply),
    debt: accruedDebt(balances.debtSharesUsdc, indexes.borrow),
  };
}

/** Basis points as a percentage string, e.g. 425 -> "4.25%". */
export function formatBps(bps: bigint): string {
  const whole = bps / 100n;
  const frac = (bps % 100n).toString().padStart(2, "0");
  return `${whole}.${frac}%`;
}
