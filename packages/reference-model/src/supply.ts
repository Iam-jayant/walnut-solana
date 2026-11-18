// Supply and withdraw supply (Task 8): the bigint mirror of
// programs/walnut/src/math/supply.rs and the Arcis `supply_flow_values`.
// All-or-zero: a refusal returns the INPUT pool and position unchanged.
import { checkpoint, type PoolPlain } from "./checkpoint.ts";

/** Shares minted per USDC base unit by the first supplier of an empty Pool. */
export const INITIAL_SHARE_SCALE = 1_000n;
/** Largest equity, Cash, or request the flow accepts (u64::MAX). */
export const MAX_FLOW_VALUE = (1n << 64n) - 1n;
/** Largest total supply shares the flow accepts (2^63). */
export const MAX_FLOW_SHARES = 1n << 63n;

export interface SupplyPosition {
  freeUsdc: bigint;
  supplyShares: bigint;
}

export interface SupplyRequest {
  isWithdraw: boolean;
  /** Supply: USDC base units. Withdraw: supply shares to burn. */
  requested: bigint;
  supplyCap: bigint;
  claimsClear: boolean;
}

export interface SupplyOutcome {
  pool: PoolPlain;
  position: SupplyPosition;
  applied: boolean;
}

export function supplyFlow(
  pool: PoolPlain,
  position: SupplyPosition,
  accountedIndex: bigint,
  accountingIndex: bigint,
  reserveFactorBps: number,
  request: SupplyRequest,
): SupplyOutcome {
  const refused: SupplyOutcome = { pool, position, applied: false };
  let cp;
  try {
    cp = checkpoint(
      pool,
      accountedIndex,
      accountingIndex,
      BigInt(reserveFactorBps),
    );
  } catch {
    return refused;
  }
  const equity = cp.supplierEquity;
  const p = cp.pool;
  if (
    equity > MAX_FLOW_VALUE ||
    p.cash > MAX_FLOW_VALUE ||
    p.totalSupplyShares > MAX_FLOW_SHARES ||
    request.requested === 0n ||
    request.requested > MAX_FLOW_VALUE
  ) {
    return refused;
  }
  const empty = p.totalSupplyShares === 0n;
  if (request.isWithdraw) {
    const shares = request.requested;
    if (
      empty ||
      shares > position.supplyShares ||
      shares > p.totalSupplyShares
    ) {
      return refused;
    }
    const claim = (shares * equity) / p.totalSupplyShares;
    const newFree = position.freeUsdc + claim;
    if (claim === 0n || claim > p.cash || newFree > MAX_FLOW_VALUE)
      return refused;
    return {
      pool: {
        ...p,
        cash: p.cash - claim,
        totalSupplyShares: p.totalSupplyShares - shares,
      },
      position: {
        freeUsdc: newFree,
        supplyShares: position.supplyShares - shares,
      },
      applied: true,
    };
  }
  const amount = request.requested;
  if (amount > position.freeUsdc || equity + amount > request.supplyCap) {
    return refused;
  }
  let shares: bigint;
  if (empty) {
    if (
      p.cash !== 0n ||
      p.totalDebtShares !== 0n ||
      p.reserves !== 0n ||
      !request.claimsClear
    ) {
      return refused;
    }
    shares = amount * INITIAL_SHARE_SCALE;
  } else {
    if (equity === 0n) return refused;
    shares = (amount * p.totalSupplyShares) / equity;
  }
  if (shares === 0n || p.totalSupplyShares + shares > MAX_FLOW_SHARES) {
    return refused;
  }
  return {
    pool: {
      ...p,
      cash: p.cash + amount,
      totalSupplyShares: p.totalSupplyShares + shares,
    },
    position: {
      freeUsdc: position.freeUsdc - amount,
      supplyShares: position.supplyShares + shares,
    },
    applied: true,
  };
}

/** Largest residue `recoverEmptyPool` may reclassify as reserves (0.01 USDC). */
export const RECOVERY_DUST_LIMIT = 10_000n;

/**
 * The first-supplier precondition on the (checkpointed) Pool: Supply Shares,
 * Cash, Debt Shares, and Reserves are zero and no Recovery Claim is unsettled.
 */
export function isCanonicalEmpty(
  pool: PoolPlain,
  claimsClear: boolean,
): boolean {
  return (
    pool.totalSupplyShares === 0n &&
    pool.cash === 0n &&
    pool.totalDebtShares === 0n &&
    pool.reserves === 0n &&
    claimsClear
  );
}

export interface RecoveryOutcome {
  pool: PoolPlain;
  applied: boolean;
}

/**
 * Mirror of `math::supply::recover_empty_pool`: with no Supply or Debt Shares
 * and no unsettled claim, a nonzero residue `cash - reserves` of at most
 * `dustLimit` becomes reserves (`reserves := cash`). Otherwise the INPUT pool.
 */
export function recoverEmptyPool(
  pool: PoolPlain,
  accountedIndex: bigint,
  accountingIndex: bigint,
  reserveFactorBps: number,
  dustLimit: bigint,
  claimsClear: boolean,
): RecoveryOutcome {
  const refused: RecoveryOutcome = { pool, applied: false };
  let cp;
  try {
    cp = checkpoint(
      pool,
      accountedIndex,
      accountingIndex,
      BigInt(reserveFactorBps),
    );
  } catch {
    return refused;
  }
  const p = cp.pool;
  if (
    p.totalSupplyShares !== 0n ||
    p.totalDebtShares !== 0n ||
    !claimsClear ||
    cp.supplierEquity === 0n ||
    cp.supplierEquity > dustLimit
  ) {
    return refused;
  }
  return { pool: { ...p, reserves: p.cash }, applied: true };
}
