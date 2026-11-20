// Walnut-Lend (task 4.2): auditor disclosure and Viewing Key rotation helpers.
//
// Reconciled with target/idl/walnut.json: `disclose_position(computation_offset,
// auditor_pubkey, mask)` has no sequence argument (the program uses the
// signer's `operation_counter.next_sequence`, passed here as `sequence` only
// to derive the Intent PDA), and neither it nor `rotate_position_key` has a
// `market` account. The `Disclosure` layout matches state/disclosure.rs.
import type { Program } from "./anchor-shim.js";
import { PublicKey, type TransactionInstruction } from "@solana/web3.js";
import { createSession, decryptOutput } from "./encryption.ts";
import {
  deriveArciumQueueAccounts,
  deriveDisclosurePda,
  deriveIntentPda,
  deriveOperationCounterPda,
  toComputationOffsetBn,
  type ComputationOffsetInput,
} from "./pda.ts";
import {
  decryptPosition,
  rotationArgs,
  ViewingKeyDecryptError,
  type EncryptedPositionState,
  type PositionBalances,
  type ViewingKeyMaterial,
} from "./viewing-key.ts";

/** Disclosure mask bits (circuit `disclose_position`). */
export const DisclosureField = {
  FreeSol: 1,
  FreeUsdc: 2,
  CollateralSol: 4,
  SupplyShares: 8,
  DebtShares: 16,
} as const;
/** All five field bits. */
export const DISCLOSURE_MASK_ALL = 0b11111;

/** Anchor method names used by this module, for IDL reconciliation. */
export const DISCLOSURE_BUILDER_NAMES = {
  disclose: "disclosePosition",
  rotate: "rotatePositionKey",
} as const;

/** Builds a mask from field bits. */
export function disclosureMask(
  ...fields: Array<(typeof DisclosureField)[keyof typeof DisclosureField]>
): number {
  return fields.reduce((m, f) => m | f, 0);
}

type LooseMethods = Record<
  string,
  (...args: unknown[]) => {
    accountsPartial(a: Record<string, PublicKey>): {
      instruction(): Promise<TransactionInstruction>;
    };
  }
>;

function methods(program: Program): LooseMethods {
  return program.methods as unknown as LooseMethods;
}

/** Inputs of {@link buildDiscloseInstruction}. */
export interface DiscloseParams {
  program: Program;
  owner: PublicKey;
  position: PublicKey;
  /** The owner's current `operation_counter.next_sequence` (Intent PDA seed). */
  sequence: bigint | number;
  /** Auditor x25519 public key (32 bytes). Also seeds the Disclosure PDA. */
  auditorPubkey: Uint8Array;
  /** Field bits, 1..31. */
  mask: number;
  computationOffset: ComputationOffsetInput;
  clusterOffset: number;
  /** Extra Anchor accounts (merged last). */
  accounts?: Record<string, PublicKey>;
}

/** Builds `disclose_position(computation_offset, auditor_pubkey, mask)`. */
export function buildDiscloseInstruction(
  p: DiscloseParams,
): Promise<TransactionInstruction> {
  if (p.auditorPubkey.length !== 32) {
    throw new RangeError("auditor public key must be 32 bytes");
  }
  if (!Number.isInteger(p.mask) || p.mask < 1 || p.mask > DISCLOSURE_MASK_ALL) {
    throw new RangeError("mask must select 1 to 5 fields");
  }
  const programId = p.program.programId;
  const accounts: Record<string, PublicKey> = {
    ...deriveArciumQueueAccounts({
      programId,
      clusterOffset: p.clusterOffset,
      computationOffset: p.computationOffset,
      circuit: "disclose_position",
    }),
    payer: p.owner,
    position: p.position,
    operationIntent: deriveIntentPda(programId, p.owner, p.sequence),
    operationCounter: deriveOperationCounterPda(programId, p.owner),
    disclosure: deriveDisclosurePda(
      programId,
      p.position,
      new PublicKey(p.auditorPubkey),
    ),
    ...p.accounts,
  };
  return methods(p.program)[DISCLOSURE_BUILDER_NAMES.disclose]!(
    toComputationOffsetBn(p.computationOffset),
    Array.from(p.auditorPubkey),
    toComputationOffsetBn(p.mask), // IDL: mask is u64
  )
    .accountsPartial(accounts)
    .instruction();
}

// ---- Disclosure account ------------------------------------------------------

/** Decoded `Disclosure` account (design.md data model). */
export interface Disclosure {
  position: PublicKey;
  owner: PublicKey;
  auditor: PublicKey;
  mask: number;
  nonce: bigint;
  /** Five 32-byte ciphertexts: free_sol, free_usdc, collateral, supply, debt. */
  ciphertexts: Uint8Array[];
  positionGeneration: bigint;
  slot: bigint;
}

/** Decodes raw `Disclosure` account data (discriminator skipped, not checked). */
export function decodeDisclosure(data: Uint8Array): Disclosure {
  const len = 8 + 32 * 3 + 1 + 16 + 160 + 8 + 8 + 1;
  if (data.length < len) {
    throw new Error(
      `Disclosure account is ${data.length} bytes, expected ${len}`,
    );
  }
  const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
  const nonce =
    view.getBigUint64(105, true) | (view.getBigUint64(113, true) << 64n);
  return {
    position: new PublicKey(data.slice(8, 40)),
    owner: new PublicKey(data.slice(40, 72)),
    auditor: new PublicKey(data.slice(72, 104)),
    mask: data[104] as number,
    nonce,
    ciphertexts: Array.from({ length: 5 }, (_, i) =>
      data.slice(121 + 32 * i, 153 + 32 * i),
    ),
    positionGeneration: view.getBigUint64(281, true),
    slot: view.getBigUint64(289, true),
  };
}

/** Disclosed fields; unselected fields are `undefined`. */
export interface DisclosedFields {
  freeSol?: bigint;
  freeUsdc?: bigint;
  collateralSol?: bigint;
  supplySharesUsdc?: bigint;
  debtSharesUsdc?: bigint;
}

const FIELD_ORDER = [
  ["freeSol", DisclosureField.FreeSol, (1n << 64n) - 1n],
  ["freeUsdc", DisclosureField.FreeUsdc, (1n << 64n) - 1n],
  ["collateralSol", DisclosureField.CollateralSol, (1n << 64n) - 1n],
  ["supplySharesUsdc", DisclosureField.SupplyShares, (1n << 128n) - 1n],
  ["debtSharesUsdc", DisclosureField.DebtShares, (1n << 128n) - 1n],
] as const;

/**
 * Decrypts a Disclosure with the auditor secret and returns only the fields
 * selected by its mask. A wrong key or corrupted payload throws
 * {@link ViewingKeyDecryptError} (selected values are range-checked).
 */
export function decryptDisclosure(
  disclosure: Pick<Disclosure, "mask" | "nonce" | "ciphertexts">,
  auditorSecret: Uint8Array,
  mxePublicKey: Uint8Array,
): DisclosedFields {
  const session = createSession(mxePublicKey, auditorSecret);
  const values = decryptOutput(
    session.cipher,
    disclosure.ciphertexts,
    disclosure.nonce,
  );
  const out: DisclosedFields = {};
  FIELD_ORDER.forEach(([name, bit, max], i) => {
    if ((disclosure.mask & bit) === 0) return;
    const v = values[i];
    if (v === undefined || v < 0n || v > max)
      throw new ViewingKeyDecryptError();
    out[name] = v;
  });
  return out;
}

// ---- Rotation ----------------------------------------------------------------

/** Inputs of {@link buildRotatePositionKeyInstruction}. */
export interface RotateParams {
  program: Program;
  owner: PublicKey;
  position: PublicKey;
  sequence: bigint | number;
  oldEpoch: bigint | number;
  oldPublicKey: Uint8Array;
  newPublicKey: Uint8Array;
  computationOffset: ComputationOffsetInput;
  clusterOffset: number;
  accounts?: Record<string, PublicKey>;
}

/** Builds `rotate_position_key(...)` from {@link rotationArgs}. */
export function buildRotatePositionKeyInstruction(
  p: RotateParams,
): Promise<TransactionInstruction> {
  const args = rotationArgs({
    oldEpoch: p.oldEpoch,
    oldPublicKey: p.oldPublicKey,
    newPublicKey: p.newPublicKey,
  });
  const programId = p.program.programId;
  const accounts: Record<string, PublicKey> = {
    ...deriveArciumQueueAccounts({
      programId,
      clusterOffset: p.clusterOffset,
      computationOffset: p.computationOffset,
      circuit: "rotate_position_key",
    }),
    payer: p.owner,
    position: p.position,
    operationIntent: deriveIntentPda(programId, p.owner, p.sequence),
    operationCounter: deriveOperationCounterPda(programId, p.owner),
    ...p.accounts,
  };
  return methods(p.program)[DISCLOSURE_BUILDER_NAMES.rotate]!(
    toComputationOffsetBn(p.computationOffset),
    toComputationOffsetBn(p.sequence),
    toComputationOffsetBn(args.expectedOldEpoch),
    toComputationOffsetBn(args.expectedNewEpoch),
    Array.from(args.expectedOldPubkey),
    Array.from(args.newViewingPubkey),
  )
    .accountsPartial(accounts)
    .instruction();
}

/** Thrown by {@link rotateViewingKey} when the rotation did not verify. */
export class RotationFailedError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "RotationFailedError";
  }
}

/** Callbacks and data of {@link rotateViewingKey}. */
export interface RotateViewingKeyParams {
  /** The new key, already derived (epoch + 1 message) by the caller. */
  newKey: ViewingKeyMaterial;
  /** Balances decrypted with the old key before rotating. */
  expectedBalances: PositionBalances;
  mxePublicKey: Uint8Array;
  /** Sends `rotate_position_key` and resolves once the callback finalized. */
  submit(newPublicKey: Uint8Array): Promise<void>;
  /** Reads the Position payload after the rotation finalized. */
  fetchState(): Promise<EncryptedPositionState>;
  /** Persists the new key. Called only after the check below succeeds. */
  commitKey(newKey: ViewingKeyMaterial): Promise<void> | void;
}

const BALANCE_KEYS = [
  "freeSol",
  "freeUsdc",
  "collateralSol",
  "supplySharesUsdc",
  "debtSharesUsdc",
] as const;

/**
 * Rotates the Viewing Key: submit, decrypt the stored state with the new key,
 * compare with the expected balances, and only then call `commitKey`. On any
 * failure the old key stays in use (`commitKey` is not called) and a
 * {@link RotationFailedError} is thrown.
 */
export async function rotateViewingKey(
  p: RotateViewingKeyParams,
): Promise<PositionBalances> {
  try {
    await p.submit(p.newKey.publicKey);
  } catch (e) {
    throw new RotationFailedError(
      `rotation was not submitted: ${e instanceof Error ? e.message : String(e)}`,
    );
  }
  let after: PositionBalances;
  try {
    after = decryptPosition({
      encryptedState: await p.fetchState(),
      viewingSecret: p.newKey.secret,
      mxePublicKey: p.mxePublicKey,
    });
  } catch {
    throw new RotationFailedError(
      "the Position does not decrypt under the new key; the old key is kept",
    );
  }
  for (const k of BALANCE_KEYS) {
    if (after[k] !== p.expectedBalances[k]) {
      throw new RotationFailedError(
        `balance ${k} changed after rotation; the old key is kept`,
      );
    }
  }
  await p.commitKey(p.newKey);
  return after;
}
