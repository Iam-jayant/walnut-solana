// Walnut-Lend (task 4.1): bigint mirror of the `lend_op` circuit rules and the
// public share maths in programs/walnut/src/math/lend.rs, plus instruction
// builders for lend_op, liquidate and check_health.
//
// Builders take the Anchor `Program` (IDL) from the caller and use
// `program.methods.<name>(...)`; names, arguments and accounts follow
// target/idl/walnut.json.
import type { Program } from "./anchor-shim.ts";
import {
  PublicKey,
  type AccountMeta,
  type TransactionInstruction,
} from "@solana/web3.js";
import {
  deriveArciumQueueAccounts,
  deriveIntentPda,
  deriveLendingPoolPda,
  deriveOperationCounterPda,
  toComputationOffsetBn,
  type ComputationOffsetInput,
} from "./pda.ts";
import type { PositionBalances } from "./viewing-key.ts";

/** Circuit op tags (math/lend.rs OP_*). */
export const LendOp = {
  AddCollateral: 0,
  WithdrawCollateral: 1,
  Borrow: 2,
  Repay: 3,
  Supply: 4,
  WithdrawSupply: 5,
  HealthCheck: 6,
  Liquidate: 7,
} as const;
export type LendOp = (typeof LendOp)[keyof typeof LendOp];

/** Index scale: 1.0 == 1e9. */
export const INDEX_SCALE = 1_000_000_000n;
/** Largest accepted index (10.0). */
export const MAX_INDEX_1E9 = 10_000_000_000n;
/** Largest price passed to the circuit (USDC base units per SOL). */
export const MAX_LEND_PRICE = 10_000_000_000n;
/** Largest collateral per position in lamports. */
export const MAX_COLLATERAL_LAMPORTS = 1_000_000_000_000_000n;

const BPS = 10_000n;
const U64_MAX = (1n << 64n) - 1n;

// ---- Share maths (public values; mirror of math/lend.rs) --------------------

function checkIndex(index: bigint): bigint {
  if (index < INDEX_SCALE || index > MAX_INDEX_1E9) {
    throw new RangeError(`index out of range: ${index}`);
  }
  return index;
}

function toU64(value: bigint): bigint {
  if (value < 0n || value > U64_MAX) {
    throw new RangeError(`value does not fit in u64: ${value}`);
  }
  return value;
}

const mulDivFloor = (a: bigint, b: bigint, d: bigint): bigint => (a * b) / d;
const mulDivCeil = (a: bigint, b: bigint, d: bigint): bigint =>
  (a * b + d - 1n) / d;

/** Clamps a normalized price to {@link MAX_LEND_PRICE}; rejects zero. */
export function clampPrice(price: bigint): bigint {
  if (price <= 0n) throw new RangeError("price must be positive");
  return price > MAX_LEND_PRICE ? MAX_LEND_PRICE : price;
}

/** Borrow: `ceil(amount * 1e9 / borrowIndex)`. */
export function borrowMint(amount: bigint, borrowIndex: bigint): bigint {
  return toU64(mulDivCeil(amount, INDEX_SCALE, checkIndex(borrowIndex)));
}

/** Repay: `floor(amount * 1e9 / borrowIndex)`. */
export function repayBurn(amount: bigint, borrowIndex: bigint): bigint {
  return toU64(mulDivFloor(amount, INDEX_SCALE, checkIndex(borrowIndex)));
}

/** Amount that clears `debtShares`: `ceil(shares * index / 1e9)`. */
export function fullRepayAmount(
  debtShares: bigint,
  borrowIndex: bigint,
): bigint {
  return toU64(mulDivCeil(debtShares, checkIndex(borrowIndex), INDEX_SCALE));
}

/** Supply: `floor(amount * 1e9 / supplyIndex)`. */
export function supplySharesFor(amount: bigint, supplyIndex: bigint): bigint {
  return toU64(mulDivFloor(amount, INDEX_SCALE, checkIndex(supplyIndex)));
}

/** Withdraw supply: `ceil(amount * 1e9 / supplyIndex)`. */
export function withdrawSupplyBurn(
  amount: bigint,
  supplyIndex: bigint,
): bigint {
  return toU64(mulDivCeil(amount, INDEX_SCALE, checkIndex(supplyIndex)));
}

/** `floor(shares * supplyIndex / 1e9)`. */
export function supplyValue(shares: bigint, supplyIndex: bigint): bigint {
  return mulDivFloor(shares, supplyIndex, INDEX_SCALE);
}

/** `floor(shares * borrowIndex / 1e9)`. */
export function debtValue(shares: bigint, borrowIndex: bigint): bigint {
  return mulDivFloor(shares, borrowIndex, INDEX_SCALE);
}

/** Liquidation debt burn: `floor(repay * 1e9 / borrowIndex)`. */
export function liquidationBurn(repay: bigint, borrowIndex: bigint): bigint {
  return repayBurn(repay, borrowIndex);
}

/** Liquidation seize in lamports: `floor(repay * 1e9 * (1e4 + bonus) / (price * 1e4))`. */
export function liquidationSeize(
  repay: bigint,
  bonusBps: bigint,
  price: bigint,
): bigint {
  if (price <= 0n) throw new RangeError("price must be positive");
  return toU64((repay * INDEX_SCALE * (BPS + bonusBps)) / (price * BPS));
}

// ---- Circuit mirror ----------------------------------------------------------

/** Public circuit arguments besides op, amount and shares. */
export interface LendCtx {
  /** Borrow index (1e9 scale). */
  borrowIndex1e9: bigint;
  /** USDC base units per 1 SOL. */
  price: bigint;
  ltvBps: bigint;
  liqThresholdBps: bigint;
  /** Per-position debt cap in USDC base units. */
  positionDebtCap: bigint;
}

/** Result of {@link applyLendOp}; for op 6 `applied` means unhealthy. */
export interface LendOpResult {
  state: PositionBalances;
  applied: boolean;
}

/**
 * Mirror of `lend_op_rules`. A refusal (and the health check) returns the input
 * state unchanged. Callers must respect the bounds from the design.
 */
export function applyLendOp(
  state: PositionBalances,
  op: LendOp | number,
  amount: bigint,
  shares: bigint,
  ctx: LendCtx,
): LendOpResult {
  const a = amount;
  const sh = shares;
  const idx = ctx.borrowIndex1e9;
  const px = ctx.price;
  const { freeSol, freeUsdc, collateralSol: coll } = state;
  const sup = state.supplySharesUsdc;
  const debt = state.debtSharesUsdc;

  const debtScaled = debt * idx * BPS;
  const unhealthy = debtScaled > coll * px * ctx.liqThresholdBps;

  let applied: boolean;
  switch (op) {
    case LendOp.AddCollateral:
      applied = a <= freeSol;
      break;
    case LendOp.WithdrawCollateral:
      applied = a <= coll && debtScaled <= (coll - a) * px * ctx.ltvBps;
      break;
    case LendOp.Borrow: {
      const newDebt = debt + sh;
      applied =
        newDebt * idx * BPS <= coll * px * ctx.ltvBps &&
        newDebt * idx <= ctx.positionDebtCap * INDEX_SCALE &&
        freeUsdc + a <= U64_MAX;
      break;
    }
    case LendOp.Repay:
      applied = sh <= debt && a <= freeUsdc;
      break;
    case LendOp.Supply:
      applied = a <= freeUsdc && sup + sh <= U64_MAX;
      break;
    case LendOp.WithdrawSupply:
      applied = sh <= sup && freeUsdc + a <= U64_MAX;
      break;
    case LendOp.HealthCheck:
      return { state, applied: unhealthy };
    case LendOp.Liquidate:
      applied =
        unhealthy &&
        sh <= debt &&
        (sh * 2n <= debt || sh === debt) &&
        a <= coll;
      break;
    default:
      applied = false;
  }
  if (!applied) return { state, applied: false };

  const s = { ...state };
  switch (op) {
    case LendOp.AddCollateral:
      s.freeSol -= a;
      s.collateralSol += a;
      break;
    case LendOp.WithdrawCollateral:
      s.collateralSol -= a;
      s.freeSol += a;
      break;
    case LendOp.Borrow:
      s.debtSharesUsdc += sh;
      s.freeUsdc += a;
      break;
    case LendOp.Repay:
      s.debtSharesUsdc -= sh;
      s.freeUsdc -= a;
      break;
    case LendOp.Supply:
      s.freeUsdc -= a;
      s.supplySharesUsdc += sh;
      break;
    case LendOp.WithdrawSupply:
      s.supplySharesUsdc -= sh;
      s.freeUsdc += a;
      break;
    case LendOp.Liquidate:
      s.collateralSol -= a;
      s.debtSharesUsdc -= sh;
      break;
  }
  return { state: s, applied: true };
}

// ---- Instruction builders ----------------------------------------------------
//
// Reconciled with target/idl/walnut.json: `lend_op`, `liquidate` and
// `check_health` all seed the Operation Counter and Operation Intent with the
// transaction signer (`payer`), never with the Position owner. `lend_op`
// requires the signer to own the Position (Position PDA seeds `[market,
// payer]`); `liquidate` and `check_health` take any Position account. Every
// queue instruction requires `price_update`; it is read for ops 1 and 2,
// health checks and liquidations, and may be the pool oracle otherwise.

/** Anchor method names used by this module. */
export const LEND_BUILDER_NAMES = {
  lendOp: "lendOp",
  liquidate: "liquidate",
  checkHealth: "checkHealth",
} as const;

/** Inputs shared by the three queue builders. */
export interface LendQueueParams {
  /** Anchor program built from the final IDL. */
  program: Program;
  /** Transaction signer: the Position owner (lend_op), liquidator or anyone. */
  signer: PublicKey;
  market: PublicKey;
  /** The Position account (the signer's own for `lend_op`). */
  position: PublicKey;
  /** The signer's next Operation sequence (Intent PDA seed `[signer, sequence]`). */
  sequence: bigint | number;
  computationOffset: ComputationOffsetInput;
  clusterOffset: number;
  /** `AssetState` of the Market (`market.asset_state`). */
  assetState: PublicKey;
  /** The pool oracle; always required, read only when the op needs a price. */
  priceUpdate: PublicKey;
  /** Extra Anchor accounts (merged last, override derived ones). */
  accounts?: Record<string, PublicKey>;
}

function queueAccounts(
  p: LendQueueParams,
  circuit: string,
): Record<string, PublicKey> {
  const programId = p.program.programId;
  return {
    ...deriveArciumQueueAccounts({
      programId,
      clusterOffset: p.clusterOffset,
      computationOffset: p.computationOffset,
      circuit,
    }),
    payer: p.signer,
    market: p.market,
    assetState: p.assetState,
    position: p.position,
    lendingPool: deriveLendingPoolPda(programId, p.market),
    operationIntent: deriveIntentPda(programId, p.signer, p.sequence),
    operationCounter: deriveOperationCounterPda(programId, p.signer),
    priceUpdate: p.priceUpdate,
    ...p.accounts,
  };
}

// The Program type is loose, so method access is untyped.
type LooseMethods = Record<
  string,
  (...args: unknown[]) => {
    accountsPartial(a: Record<string, PublicKey>): {
      remainingAccounts(m: AccountMeta[]): {
        instruction(): Promise<TransactionInstruction>;
      };
      instruction(): Promise<TransactionInstruction>;
    };
  }
>;

function methods(program: Program): LooseMethods {
  return program.methods as unknown as LooseMethods;
}

/** Builds `lend_op(computation_offset, sequence, op, amount)` for ops 0-5. */
export function buildLendOpInstruction(
  p: LendQueueParams & {
    op: LendOp | number;
    amount: bigint;
    /** The Market's USDC vault (`market.usdc_vault`). */
    usdcVault: PublicKey;
  },
): Promise<TransactionInstruction> {
  if (p.op < LendOp.AddCollateral || p.op > LendOp.WithdrawSupply) {
    throw new RangeError("lend_op queues ops 0 to 5 only");
  }
  return methods(p.program)[LEND_BUILDER_NAMES.lendOp]!(
    toComputationOffsetBn(p.computationOffset),
    toComputationOffsetBn(p.sequence),
    p.op,
    toComputationOffsetBn(p.amount),
  )
    .accountsPartial({
      ...queueAccounts(p, "lend_op"),
      usdcVault: p.usdcVault,
      ...p.accounts,
    })
    .instruction();
}

/**
 * Builds `check_health(computation_offset, sequence)`. Anyone may sign; the
 * signer's own counter and Intent are used and `position` is the target.
 */
export function buildCheckHealthInstruction(
  p: LendQueueParams,
): Promise<TransactionInstruction> {
  return methods(p.program)[LEND_BUILDER_NAMES.checkHealth]!(
    toComputationOffsetBn(p.computationOffset),
    toComputationOffsetBn(p.sequence),
  )
    .accountsPartial(queueAccounts(p, "lend_op"))
    .instruction();
}

/**
 * Builds `liquidate(computation_offset, sequence, repay_amount)`. The signer
 * is the liquidator (its own counter and Intent) and `position` is the
 * borrower's Position. `remaining_accounts` order is `[liquidator_usdc,
 * liquidator_wsol, token_program, wsol_vault]`.
 */
export function buildLiquidateInstruction(
  p: LendQueueParams & {
    repayAmount: bigint;
    /** The Market's USDC vault (`market.usdc_vault`). */
    usdcVault: PublicKey;
    /** The Market's wSOL vault (`market.wsol_vault`). */
    wsolVault: PublicKey;
    liquidatorUsdc: PublicKey;
    liquidatorWsol: PublicKey;
    tokenProgram: PublicKey;
  },
): Promise<TransactionInstruction> {
  const remaining: AccountMeta[] = [
    { pubkey: p.liquidatorUsdc, isSigner: false, isWritable: true },
    { pubkey: p.liquidatorWsol, isSigner: false, isWritable: true },
    { pubkey: p.tokenProgram, isSigner: false, isWritable: false },
    { pubkey: p.wsolVault, isSigner: false, isWritable: false },
  ];
  return methods(p.program)[LEND_BUILDER_NAMES.liquidate]!(
    toComputationOffsetBn(p.computationOffset),
    toComputationOffsetBn(p.sequence),
    toComputationOffsetBn(p.repayAmount),
  )
    .accountsPartial({
      ...queueAccounts(p, "lend_op"),
      usdcVault: p.usdcVault,
      ...p.accounts,
    })
    .remainingAccounts(remaining)
    .instruction();
}

/** Accounts of `abandon_lend_op` (liquidation refund accounts optional). */
export interface AbandonLendOpParams {
  program: Program;
  signer: PublicKey;
  market: PublicKey;
  position: PublicKey;
  operationIntent: PublicKey;
  /**
   * Required when the queued op is a liquidation, in this order:
   * `[usdc_vault, wsol_vault, vault_authority, token_program,
   * liquidator_usdc, liquidator_wsol]`.
   */
  liquidationAccounts?: [
    PublicKey,
    PublicKey,
    PublicKey,
    PublicKey,
    PublicKey,
    PublicKey,
  ];
}

/** Builds `abandon_lend_op()`. */
export function buildAbandonLendOpInstruction(
  p: AbandonLendOpParams,
): Promise<TransactionInstruction> {
  const writable = [true, true, false, false, true, true];
  const remaining: AccountMeta[] = (p.liquidationAccounts ?? []).map(
    (pubkey, i) => ({
      pubkey,
      isSigner: false,
      isWritable: writable[i] as boolean,
    }),
  );
  return methods(p.program)["abandonLendOp"]!()
    .accountsPartial({
      signer: p.signer,
      market: p.market,
      lendingPool: deriveLendingPoolPda(p.program.programId, p.market),
      position: p.position,
      operationIntent: p.operationIntent,
    })
    .remainingAccounts(remaining)
    .instruction();
}
