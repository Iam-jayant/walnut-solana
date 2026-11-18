// PDA helpers.
//
// The Walnut seed scheme mirrors the Rust program byte for byte:
//   OperationIntent  = PDA["operation_intent" | owner (32) | sequence (u64 LE)]
//     programs/walnut/src/operation/seeds.rs (OPERATION_INTENT_SEED)
//   OperationCounter = PDA["operation_counter" | owner (32)]
//     programs/walnut/src/operation/seeds.rs (OPERATION_COUNTER_SEED)
//   SpikePosition    = PDA["spike_position" | owner (32)]
//     programs/walnut/src/lib.rs (POSITION_SEED; Task 2.2 feasibility account,
//     replaced by the production Position in Task 5)
// The operation kind is deliberately not a seed: one sequence maps to one
// address whatever the kind.
//
// The Arcium helpers wrap @arcium-hq/client 0.16.0 so callers pass Walnut
// concepts (program id, cluster offset, circuit name) instead of raw seeds.

import { BN } from "@anchor-lang/core";
import {
  getClusterAccAddress,
  getCompDefAccAddress,
  getCompDefAccOffset,
  getComputationAccAddress,
  getExecutingPoolAccAddress,
  getFeePoolAccAddress,
  getMempoolAccAddress,
  getMXEAccAddress,
} from "@arcium-hq/client";
import { PublicKey } from "@solana/web3.js";

export {
  getClusterAccAddress,
  getCompDefAccAddress,
  getCompDefAccOffset,
  getComputationAccAddress,
  getExecutingPoolAccAddress,
  getFeePoolAccAddress,
  getMempoolAccAddress,
  getMXEAccAddress,
};

/** Seed constant of an Operation Intent PDA. */
export const OPERATION_INTENT_SEED = "operation_intent";
/** Seed constant of an Operation Counter PDA. */
export const OPERATION_COUNTER_SEED = "operation_counter";
/** Seed constant of the Task 2.2 SpikePosition PDA. */
export const POSITION_SEED = "spike_position";

const U64_MAX = (1n << 64n) - 1n;
const textEncoder = new TextEncoder();

/** A derived address and its bump seed. */
export interface DerivedAddress {
  readonly address: PublicKey;
  readonly bump: number;
}

/** Validates a u64 given as bigint or safe integer and returns it as bigint. */
export function parseU64(value: bigint | number): bigint {
  let n: bigint;
  if (typeof value === "number") {
    if (!Number.isSafeInteger(value)) {
      throw new RangeError(`not a safe integer: ${value}`);
    }
    n = BigInt(value);
  } else {
    n = value;
  }
  if (n < 0n || n > U64_MAX) {
    throw new RangeError(`value does not fit in u64: ${n}`);
  }
  return n;
}

/** Encodes `value` as 8 little-endian bytes. Rejects anything outside u64. */
export function u64ToLeBytes(value: bigint | number): Uint8Array {
  const bytes = new Uint8Array(8);
  new DataView(bytes.buffer).setBigUint64(0, parseU64(value), true);
  return bytes;
}

function derive(seeds: Uint8Array[], programId: PublicKey): DerivedAddress {
  const [address, bump] = PublicKey.findProgramAddressSync(seeds, programId);
  return { address, bump };
}

/** Derives the Operation Intent PDA and bump for `owner` and `sequence`. */
export function deriveIntentPdaWithBump(
  programId: PublicKey,
  owner: PublicKey,
  sequence: bigint | number,
): DerivedAddress {
  return derive(
    [
      textEncoder.encode(OPERATION_INTENT_SEED),
      owner.toBytes(),
      u64ToLeBytes(sequence),
    ],
    programId,
  );
}

/** Derives the Operation Intent address for `owner` and `sequence`. */
export function deriveIntentPda(
  programId: PublicKey,
  owner: PublicKey,
  sequence: bigint | number,
): PublicKey {
  return deriveIntentPdaWithBump(programId, owner, sequence).address;
}

/** Derives the per-owner Operation Counter address. */
export function deriveOperationCounterPda(
  programId: PublicKey,
  owner: PublicKey,
): PublicKey {
  return derive(
    [textEncoder.encode(OPERATION_COUNTER_SEED), owner.toBytes()],
    programId,
  ).address;
}

/** Derives the owner's SpikePosition PDA (Task 2.2 feasibility account). */
export function derivePositionPda(
  programId: PublicKey,
  owner: PublicKey,
): PublicKey {
  return derive([textEncoder.encode(POSITION_SEED), owner.toBytes()], programId)
    .address;
}

/** Accepts a u64 as bigint, safe integer, or BN. */
export type ComputationOffsetInput = bigint | number | BN;

/** Converts a computation offset to the BN the Arcium client expects. */
export function toComputationOffsetBn(offset: ComputationOffsetInput): BN {
  if (BN.isBN(offset)) return offset;
  return new BN(parseU64(offset).toString());
}

/** The u32 computation-definition offset of a circuit name. */
export function compDefOffsetOf(circuit: string): number {
  const raw = getCompDefAccOffset(circuit);
  return new DataView(raw.buffer, raw.byteOffset, raw.byteLength).getUint32(
    0,
    true,
  );
}

/** The Arcium accounts every queue instruction needs, minus the payer. */
export interface ArciumQueueAccounts {
  computationAccount: PublicKey;
  clusterAccount: PublicKey;
  mxeAccount: PublicKey;
  mempoolAccount: PublicKey;
  executingPool: PublicKey;
  compDefAccount: PublicKey;
}

/**
 * Derives the standard Arcium queue accounts for one computation, using the
 * same names as the Anchor `accountsPartial` calls in tests/ and bench/.
 */
export function deriveArciumQueueAccounts(params: {
  programId: PublicKey;
  clusterOffset: number;
  computationOffset: ComputationOffsetInput;
  /** Circuit name, e.g. "add_together". */
  circuit: string;
}): ArciumQueueAccounts {
  const { programId, clusterOffset, circuit } = params;
  return {
    computationAccount: getComputationAccAddress(
      clusterOffset,
      toComputationOffsetBn(params.computationOffset),
    ),
    clusterAccount: getClusterAccAddress(clusterOffset),
    mxeAccount: getMXEAccAddress(programId),
    mempoolAccount: getMempoolAccAddress(clusterOffset),
    executingPool: getExecutingPoolAccAddress(clusterOffset),
    compDefAccount: getCompDefAccAddress(programId, compDefOffsetOf(circuit)),
  };
}

/** Seed constant of the Walnut-Lend `LendingPool` PDA. */
export const LENDING_POOL_SEED = "lending_pool";
/** Seed constant of a Walnut-Lend `Disclosure` PDA. */
export const DISCLOSURE_SEED = "disclosure";

/** Derives the `LendingPool` PDA: `["lending_pool", market]`. */
export function deriveLendingPoolPda(
  programId: PublicKey,
  market: PublicKey,
): PublicKey {
  return derive(
    [textEncoder.encode(LENDING_POOL_SEED), market.toBytes()],
    programId,
  ).address;
}

/** Derives the `Disclosure` PDA: `["disclosure", position, auditor]`. */
export function deriveDisclosurePda(
  programId: PublicKey,
  position: PublicKey,
  auditor: PublicKey,
): PublicKey {
  return derive(
    [
      textEncoder.encode(DISCLOSURE_SEED),
      position.toBytes(),
      auditor.toBytes(),
    ],
    programId,
  ).address;
}
