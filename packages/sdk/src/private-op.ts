// Walnut private operations (ADR-0004, task P.6): encrypted-amount supply,
// withdraw supply, borrow, repay, collateral and liquidation.
//
// The client computes the share conversion from the public LendingPool
// indexes, encrypts `(amount, shares)` (liquidation: `(amount, burnShares,
// seize)`) to the MXE and submits only ciphertexts. The circuits in
// encrypted-ixs/src/lib.rs (`pool_op`, `position_private`, `liquidate_pool`)
// re-check the conversion with multiplications and comparisons; this module
// mirrors those checks so a client can verify its inputs before sending.
//
// Builders take the Anchor `Program` (IDL) from the caller; names, arguments
// and accounts follow target/idl/walnut.json.
import type { Program } from "./anchor-shim.ts";
import { BN } from "./anchor-shim.ts";
import { PublicKey, type TransactionInstruction } from "@solana/web3.js";
import { createSession, encryptInputs } from "./encryption.ts";
import { INDEX_SCALE } from "./lend-op.ts";
import {
  deriveArciumQueueAccounts,
  deriveIntentPda,
  deriveLendingPoolPda,
  deriveOperationCounterPda,
  toComputationOffsetBn,
  type ComputationOffsetInput,
} from "./pda.ts";

/** `pool_op` codes. */
export const PoolOp = {
  Supply: 0,
  WithdrawSupply: 1,
  Borrow: 2,
  Repay: 3,
} as const;
export type PoolOp = (typeof PoolOp)[keyof typeof PoolOp];

/** `position_private` codes. */
export const CollateralOp = {
  Add: 0,
  Withdraw: 1,
} as const;
export type CollateralOp = (typeof CollateralOp)[keyof typeof CollateralOp];

/** Oldest accrual timestamp the program accepts, relative to its clock. */
export const MAX_ACCRUAL_LAG_SECONDS = 120n;

const BPS = 10_000n;
const U64_MAX = (1n << 64n) - 1n;
const divCeil = (n: bigint, d: bigint): bigint => (n + d - 1n) / d;

function checkU64(name: string, v: bigint): bigint {
  if (v < 0n || v > U64_MAX) throw new RangeError(`${name} is not a u64: ${v}`);
  return v;
}

// ---- Conversions (what the circuits verify) ---------------------------------

/**
 * The share count `pool_op` accepts for `amount` at the given indexes:
 * supply and repay `floor(a * 1e9 / idx)`, withdraw supply and borrow
 * `ceil(a * 1e9 / idx)` (rounding against the user).
 */
export function poolOpShares(
  op: PoolOp | number,
  amount: bigint,
  supplyIndex1e9: bigint,
  borrowIndex1e9: bigint,
): bigint {
  const scaled = checkU64("amount", amount) * INDEX_SCALE;
  switch (op) {
    case PoolOp.Supply:
      return scaled / supplyIndex1e9;
    case PoolOp.WithdrawSupply:
      return divCeil(scaled, supplyIndex1e9);
    case PoolOp.Borrow:
      return divCeil(scaled, borrowIndex1e9);
    case PoolOp.Repay:
      return scaled / borrowIndex1e9;
    default:
      throw new RangeError(`unknown pool op ${op}`);
  }
}

/** Mirror of the circuit's conversion check for `pool_op`. */
export function poolOpConversionHolds(
  op: PoolOp | number,
  amount: bigint,
  shares: bigint,
  supplyIndex1e9: bigint,
  borrowIndex1e9: bigint,
): boolean {
  const a = amount * INDEX_SCALE;
  const shm1 = shares > 0n ? shares - 1n : 0n;
  const floorOk = (idx: bigint) =>
    shares > 0n && shares * idx <= a && a < (shares + 1n) * idx;
  const ceilOk = (idx: bigint) =>
    shares > 0n && amount > 0n && shares * idx >= a && shm1 * idx < a;
  switch (op) {
    case PoolOp.Supply:
      return floorOk(supplyIndex1e9);
    case PoolOp.WithdrawSupply:
      return ceilOk(supplyIndex1e9);
    case PoolOp.Borrow:
      return ceilOk(borrowIndex1e9);
    case PoolOp.Repay:
      return floorOk(borrowIndex1e9);
    default:
      return false;
  }
}

/**
 * The smallest repay amount whose conversion burns exactly `debtShares`:
 * `ceil(debtShares * idx / 1e9)`, for which `floor(a * 1e9 / idx) ==
 * debtShares` whenever `idx >= 1e9`.
 */
export function fullRepayAmountPrivate(
  debtShares: bigint,
  borrowIndex1e9: bigint,
): bigint {
  return divCeil(debtShares * borrowIndex1e9, INDEX_SCALE);
}

/** Liquidation inputs `liquidate_pool` accepts for a repay `amount`. */
export interface LiquidationInputs {
  amount: bigint;
  burnShares: bigint;
  seize: bigint;
}

/**
 * `burnShares = floor(a * 1e9 / bidx)`, `seize = floor(a * 1e9 * (1e4 +
 * bonus) / (price * 1e4))` lamports.
 */
export function liquidationInputs(
  amount: bigint,
  borrowIndex1e9: bigint,
  price: bigint,
  bonusBps: bigint,
): LiquidationInputs {
  if (price <= 0n) throw new RangeError("price must be positive");
  const a = checkU64("amount", amount) * INDEX_SCALE;
  return {
    amount,
    burnShares: a / borrowIndex1e9,
    seize: (a * (BPS + bonusBps)) / (price * BPS),
  };
}

/** Mirror of the circuit's conversion checks for `liquidate_pool`. */
export function liquidationConversionHolds(
  inputs: LiquidationInputs,
  borrowIndex1e9: bigint,
  price: bigint,
  bonusBps: bigint,
): boolean {
  const a = inputs.amount * INDEX_SCALE;
  const sh = inputs.burnShares;
  const z = inputs.seize;
  const num = a * (BPS + bonusBps);
  return (
    sh > 0n &&
    sh * borrowIndex1e9 <= a &&
    a < (sh + 1n) * borrowIndex1e9 &&
    z > 0n &&
    z * price * BPS <= num &&
    num < (z + 1n) * price * BPS
  );
}

// ---- Encryption -------------------------------------------------------------

/** The encrypted input of a private instruction. */
export interface PrivateInput {
  /** x25519 public key of the session (`input_pubkey`). */
  publicKey: number[];
  /** `input_nonce` (u128). */
  nonce: bigint;
  /** One 32-byte ciphertext per value (`input_ciphertexts`). */
  ciphertexts: number[][];
}

/**
 * Encrypts `values` (u64 each) to the MXE with a fresh ephemeral session key
 * and a fresh nonce. The secret key never leaves this function.
 */
export function encryptPrivateInput(
  mxePublicKey: Uint8Array,
  values: bigint[],
): PrivateInput {
  values.forEach((v, i) => checkU64(`value ${i}`, v));
  const session = createSession(mxePublicKey);
  const enc = encryptInputs(session.cipher, values);
  return {
    publicKey: Array.from(session.publicKey),
    nonce: enc.nonceU128,
    ciphertexts: enc.ciphertexts,
  };
}

/**
 * A safe `accrue_to_ts`: the newest of `lastAccrualTs` and `now - margin`.
 * The program accepts `last_accrual_ts <= t <= now` and `now - t <= 120`.
 * Project the indexes to the returned time (`projectIndexesAuto`) and use
 * them for the conversion.
 */
export function chooseAccrualTs(
  lastAccrualTs: bigint,
  nowSeconds: bigint,
  marginSeconds = 15n,
): bigint {
  const t = nowSeconds - marginSeconds;
  if (t >= lastAccrualTs) return t;
  if (nowSeconds - lastAccrualTs > MAX_ACCRUAL_LAG_SECONDS) {
    throw new RangeError("last accrual is older than the accepted lag");
  }
  return lastAccrualTs;
}

// ---- Builders ---------------------------------------------------------------

/** Anchor method names used by this module. */
export const PRIVATE_BUILDER_NAMES = {
  poolOp: "poolOp",
  positionPrivate: "positionPrivate",
  liquidatePool: "liquidatePool",
  abandonPrivateOp: "abandonPrivateOp",
  setLendingRateSource: "setLendingRateSource",
} as const;

/** PDA of the encrypted Pool State (`["pool_state", market]`). */
export function derivePoolStatePda(
  programId: PublicKey,
  market: PublicKey,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("pool_state"), market.toBuffer()],
    programId,
  )[0];
}

/** PDA of a production Position (`["position", market, owner]`). */
export function derivePrivatePositionPda(
  programId: PublicKey,
  market: PublicKey,
  owner: PublicKey,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("position"), market.toBuffer(), owner.toBuffer()],
    programId,
  )[0];
}

/** Inputs shared by the private queue builders. */
export interface PrivateQueueParams {
  program: Program;
  /** The signer: the Position owner, or the liquidator. */
  signer: PublicKey;
  market: PublicKey;
  assetState: PublicKey;
  /** `market.pool_state` (defaults to the PDA). */
  poolState?: PublicKey;
  /** The signer's next Operation sequence. */
  sequence: bigint | number;
  computationOffset: ComputationOffsetInput;
  clusterOffset: number;
  /** The pool oracle; read only by borrow, collateral withdrawal and liquidation. */
  priceUpdate: PublicKey;
  /** The accrual timestamp the indexes were projected to. */
  accrueToTs: bigint;
  /** The encrypted input. */
  input: PrivateInput;
  /** Extra Anchor accounts (merged last). */
  accounts?: Record<string, PublicKey>;
}

type LooseMethods = Record<
  string,
  (...args: unknown[]) => {
    accountsPartial(a: Record<string, PublicKey | null>): {
      instruction(): Promise<TransactionInstruction>;
    };
  }
>;

function methods(program: Program): LooseMethods {
  return program.methods as unknown as LooseMethods;
}

function baseAccounts(
  p: PrivateQueueParams,
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
    lendingPool: deriveLendingPoolPda(programId, p.market),
    operationIntent: deriveIntentPda(programId, p.signer, p.sequence),
    operationCounter: deriveOperationCounterPda(programId, p.signer),
    priceUpdate: p.priceUpdate,
  };
}

function inputArgs(p: PrivateQueueParams, count: number): unknown[] {
  if (p.input.ciphertexts.length !== count) {
    throw new RangeError(`expected ${count} ciphertexts`);
  }
  return [
    new BN(p.accrueToTs.toString()),
    p.input.publicKey,
    new BN(p.input.nonce.toString()),
    p.input.ciphertexts,
  ];
}

/**
 * Builds `pool_op(computation_offset, sequence, op, accrue_to_ts,
 * input_pubkey, input_nonce, input_ciphertexts[2])`. The input encrypts
 * `[amount, shares]`.
 */
export function buildPoolOpInstruction(
  p: PrivateQueueParams & { op: PoolOp | number },
): Promise<TransactionInstruction> {
  if (p.op < PoolOp.Supply || p.op > PoolOp.Repay) {
    throw new RangeError("pool_op codes are 0 to 3");
  }
  const programId = p.program.programId;
  return methods(p.program)[PRIVATE_BUILDER_NAMES.poolOp]!(
    toComputationOffsetBn(p.computationOffset),
    toComputationOffsetBn(p.sequence),
    p.op,
    ...inputArgs(p, 2),
  )
    .accountsPartial({
      ...baseAccounts(p, "pool_op"),
      poolState: p.poolState ?? derivePoolStatePda(programId, p.market),
      position: derivePrivatePositionPda(programId, p.market, p.signer),
      ...p.accounts,
    })
    .instruction();
}

/**
 * Builds `position_private(computation_offset, sequence, op, accrue_to_ts,
 * input_pubkey, input_nonce, input_ciphertexts[2])`. The input encrypts
 * `[lamports, 0]`.
 */
export function buildPositionPrivateInstruction(
  p: PrivateQueueParams & { op: CollateralOp | number },
): Promise<TransactionInstruction> {
  if (p.op !== CollateralOp.Add && p.op !== CollateralOp.Withdraw) {
    throw new RangeError("position_private codes are 0 and 1");
  }
  const programId = p.program.programId;
  return methods(p.program)[PRIVATE_BUILDER_NAMES.positionPrivate]!(
    toComputationOffsetBn(p.computationOffset),
    toComputationOffsetBn(p.sequence),
    p.op,
    ...inputArgs(p, 2),
  )
    .accountsPartial({
      ...baseAccounts(p, "position_private"),
      position: derivePrivatePositionPda(programId, p.market, p.signer),
      ...p.accounts,
    })
    .instruction();
}

/**
 * Builds `liquidate_pool(computation_offset, sequence, accrue_to_ts,
 * input_pubkey, input_nonce, input_ciphertexts[3])`. The signer pays from
 * and is paid into its own Position; `target` is the borrower's Position.
 * The input encrypts `[amount, burnShares, seize]`.
 */
export function buildLiquidatePoolInstruction(
  p: PrivateQueueParams & { target: PublicKey },
): Promise<TransactionInstruction> {
  const programId = p.program.programId;
  return methods(p.program)[PRIVATE_BUILDER_NAMES.liquidatePool]!(
    toComputationOffsetBn(p.computationOffset),
    toComputationOffsetBn(p.sequence),
    ...inputArgs(p, 3),
  )
    .accountsPartial({
      ...baseAccounts(p, "liquidate_pool"),
      poolState: p.poolState ?? derivePoolStatePda(programId, p.market),
      position: p.target,
      liquidatorPosition: derivePrivatePositionPda(
        programId,
        p.market,
        p.signer,
      ),
      ...p.accounts,
    })
    .instruction();
}

/** Builds `abandon_private_op()` for a queued private operation. */
export function buildAbandonPrivateOpInstruction(p: {
  program: Program;
  signer: PublicKey;
  market: PublicKey;
  /** The primary Position (owner's, or the liquidation target). */
  position: PublicKey;
  operationIntent: PublicKey;
  /** Required for pool ops and liquidations. */
  poolState?: PublicKey;
  /** The liquidator's Position, for a liquidation. */
  secondPosition?: PublicKey;
}): Promise<TransactionInstruction> {
  return methods(p.program)[PRIVATE_BUILDER_NAMES.abandonPrivateOp]!()
    .accountsPartial({
      signer: p.signer,
      market: p.market,
      lendingPool: deriveLendingPoolPda(p.program.programId, p.market),
      poolState: p.poolState ?? null,
      position: p.position,
      secondPosition: p.secondPosition ?? null,
      operationIntent: p.operationIntent,
    })
    .instruction();
}

/** Pool-owned Operation counter PDA (`["operation_counter", pool_state]`). */
export function derivePoolCounterPda(
  programId: PublicKey,
  poolState: PublicKey,
): PublicKey {
  return deriveOperationCounterPda(programId, poolState);
}

/**
 * Builds `update_pool_band(computation_offset, sequence)`: the light band
 * circuit over the private Pool at the LendingPool borrow index. Anyone may
 * sign; `sequence` is the Pool-owned counter's next sequence (0 when the
 * counter does not exist yet).
 */
export function buildUpdatePoolBandInstruction(p: {
  program: Program;
  payer: PublicKey;
  market: PublicKey;
  assetState: PublicKey;
  poolState?: PublicKey;
  sequence: bigint | number;
  computationOffset: ComputationOffsetInput;
  clusterOffset: number;
}): Promise<TransactionInstruction> {
  const programId = p.program.programId;
  const poolState = p.poolState ?? derivePoolStatePda(programId, p.market);
  return methods(p.program)["updatePoolBand"]!(
    toComputationOffsetBn(p.computationOffset),
    toComputationOffsetBn(p.sequence),
  )
    .accountsPartial({
      ...deriveArciumQueueAccounts({
        programId,
        clusterOffset: p.clusterOffset,
        computationOffset: p.computationOffset,
        circuit: "pool_band",
      }),
      payer: p.payer,
      market: p.market,
      assetState: p.assetState,
      poolState,
      lendingPool: deriveLendingPoolPda(programId, p.market),
      operationCounter: derivePoolCounterPda(programId, poolState),
      operationIntent: deriveIntentPda(programId, poolState, p.sequence),
    })
    .instruction();
}

/** Builds `set_lending_rate_source(rate_source)` (admin). */
export function buildSetLendingRateSourceInstruction(p: {
  program: Program;
  admin: PublicKey;
  market: PublicKey;
  assetState: PublicKey;
  rateSource: number;
}): Promise<TransactionInstruction> {
  return methods(p.program)[PRIVATE_BUILDER_NAMES.setLendingRateSource]!(
    p.rateSource,
  )
    .accountsPartial({
      admin: p.admin,
      market: p.market,
      assetState: p.assetState,
      lendingPool: deriveLendingPoolPda(p.program.programId, p.market),
    })
    .instruction();
}
