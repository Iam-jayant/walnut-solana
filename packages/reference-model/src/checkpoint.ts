// Pool interest checkpoint (Task 7.3): the bigint mirror of
// programs/walnut/src/math/checkpoint.rs and the Arcis `checkpoint_pool`.
// Replays vectors/checkpoint.vectors.
import {
  BPS_DENOMINATOR,
  FixedPointError,
  checkIndex,
  debtFromShares,
  mulDivFloor,
} from "./fixed.ts";

/** Largest Pool cash or reserves value the checkpoint accepts (2^96). */
export const MAX_POOL_VALUE = 1n << 96n;

export interface PoolPlain {
  cash: bigint;
  totalSupplyShares: bigint;
  totalDebtShares: bigint;
  reserves: bigint;
}

export interface Checkpoint {
  pool: PoolPlain;
  oldDebt: bigint;
  newDebt: bigint;
  interest: bigint;
  reserveAdd: bigint;
  supplierInterest: bigint;
  supplierEquity: bigint;
}

/** cash + debt - reserves; throws when reserves exceed cash + debt. */
export function supplierEquity(
  cash: bigint,
  debt: bigint,
  reserves: bigint,
): bigint {
  const value = cash + debt - reserves;
  if (value < 0n) throw new FixedPointError("underflow");
  return value;
}

/**
 * First step of every Pool-mutating operation:
 * interest = ceil(shares x accounting / WAD) - ceil(shares x accounted / WAD);
 * reserves += floor(interest x reserveFactorBps / 10_000);
 * equity = cash + newDebt - reserves.
 */
export function checkpoint(
  pool: PoolPlain,
  accountedIndex: bigint,
  accountingIndex: bigint,
  reserveFactorBps: bigint,
): Checkpoint {
  checkIndex(accountedIndex);
  checkIndex(accountingIndex);
  if (accountingIndex < accountedIndex) throw new FixedPointError("regress");
  if (reserveFactorBps < 0n || reserveFactorBps > BPS_DENOMINATOR) {
    throw new FixedPointError("invalid");
  }
  if (pool.cash > MAX_POOL_VALUE || pool.reserves > MAX_POOL_VALUE) {
    throw new FixedPointError("range");
  }
  const oldDebt = debtFromShares(pool.totalDebtShares, accountedIndex);
  const newDebt = debtFromShares(pool.totalDebtShares, accountingIndex);
  const interest = newDebt - oldDebt;
  const reserveAdd = mulDivFloor(interest, reserveFactorBps, BPS_DENOMINATOR);
  const reserves = pool.reserves + reserveAdd;
  return {
    pool: { ...pool, reserves },
    oldDebt,
    newDebt,
    interest,
    reserveAdd,
    supplierInterest: interest - reserveAdd,
    supplierEquity: supplierEquity(pool.cash, newDebt, reserves),
  };
}
/** Result of a reserve withdrawal (Task 7.5); mirrors `math::checkpoint::reserve_withdrawal`. */
export interface ReserveWithdrawal {
  /** The payload to store: checkpointed with Cash and Reserves reduced on approval, the input unchanged on refusal. */
  pool: PoolPlain;
  /** The full request or zero. */
  approved: bigint;
}
/**
 * Checkpoint first, then approve the full request only if it is nonzero and no
 * greater than Cash and Reserves after the checkpoint. A refusal (or a failed
 * checkpoint) commits nothing.
 */
export function reserveWithdrawal(
  pool: PoolPlain,
  accountedIndex: bigint,
  accountingIndex: bigint,
  reserveFactorBps: bigint,
  requested: bigint,
): ReserveWithdrawal {
  const refused = { pool, approved: 0n };
  let c: Checkpoint;
  try {
    c = checkpoint(pool, accountedIndex, accountingIndex, reserveFactorBps);
  } catch (error) {
    if (error instanceof FixedPointError) return refused;
    throw error;
  }
  if (
    requested === 0n ||
    requested > c.pool.cash ||
    requested > c.pool.reserves
  )
    return refused;
  return {
    pool: {
      ...c.pool,
      cash: c.pool.cash - requested,
      reserves: c.pool.reserves - requested,
    },
    approved: requested,
  };
}
