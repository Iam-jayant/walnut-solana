// Rent and fee reclaim builders (Requirements 3.11, 14.7; ADR-0003).
//
// Two Arcium program paths recover what a queued computation locked:
//
//   claimComputationRent        closes a FINISHED computation account and
//                               returns its rent. @arcium-hq/client exposes it
//                               as the claimComputationRent helper, which signs
//                               with provider.publicKey. The Arcium program
//                               requires the ORIGINAL PAYER of the computation
//                               to sign, so the builders here check that the
//                               signer is the recorded payer (the Operation
//                               Intent `payer`, which equals `reclaim_authority`)
//                               before building anything.
//   reclaimExpiredComputationFee returns the fee of a computation that expired
//                               in the mempool and never ran. The client has no
//                               helper for it, so it is built directly from the
//                               Arcium IDL with the account wiring proven in
//                               tests/failure-matrix.ts (item 13). It uses the
//                               same payer check, because the fee returns to the
//                               payer.
//
// Building needs no RPC. Sending (and the live effect of either instruction)
// needs a cluster and is documented, not unit-tested.

import type { AnchorProvider } from "./anchor-shim.ts";
import {
  claimComputationRent,
  getArciumProgram,
  getClusterAccAddress,
  getComputationAccAddress,
  getExecutingPoolAccAddress,
  getFeePoolAccAddress,
  getMXEAccAddress,
} from "@arcium-hq/client";
import type { PublicKey, TransactionInstruction } from "@solana/web3.js";
import { toComputationOffsetBn, type ComputationOffsetInput } from "./pda.ts";

/** Raised when the signer is not the original payer of the computation. */
export class PayerMismatchError extends Error {
  readonly signer: string;
  readonly originalPayer: string;
  constructor(signer: PublicKey, originalPayer: PublicKey) {
    super(
      `reclaim must be signed by the original payer ${originalPayer.toBase58()}, not ${signer.toBase58()}`,
    );
    this.name = "PayerMismatchError";
    this.signer = signer.toBase58();
    this.originalPayer = originalPayer.toBase58();
  }
}

function requirePayer(signer: PublicKey, originalPayer: PublicKey): void {
  if (!signer.equals(originalPayer)) {
    throw new PayerMismatchError(signer, originalPayer);
  }
}

export interface ReclaimCommon {
  /** Used only to load the Arcium program IDL; nothing is sent. */
  readonly provider: AnchorProvider;
  /** The account that will sign the reclaim transaction. */
  readonly signer: PublicKey;
  /** The account that originally paid for the computation (Intent `payer`). */
  readonly originalPayer: PublicKey;
  readonly computationOffset: ComputationOffsetInput;
}

/**
 * Builds `claimComputationRent(compOffset, clusterOffset)` with the accounts
 * the client helper uses: the signer and the computation account.
 */
export async function buildClaimComputationRent(
  params: ReclaimCommon & { readonly clusterOffset: number },
): Promise<TransactionInstruction> {
  requirePayer(params.signer, params.originalPayer);
  const offset = toComputationOffsetBn(params.computationOffset);
  return getArciumProgram(params.provider)
    .methods.claimComputationRent(offset, params.clusterOffset)
    .accountsPartial({
      signer: params.signer,
      comp: getComputationAccAddress(params.clusterOffset, offset),
    })
    .instruction();
}

/**
 * Builds `reclaimExpiredComputationFee(compOffset, mxeProgram)` with the
 * account names used in tests/failure-matrix.ts.
 */
export async function buildReclaimExpiredComputationFee(
  params: ReclaimCommon & {
    readonly clusterOffset: number;
    /** The MXE (Walnut) program id that queued the computation. */
    readonly mxeProgramId: PublicKey;
  },
): Promise<TransactionInstruction> {
  requirePayer(params.signer, params.originalPayer);
  const offset = toComputationOffsetBn(params.computationOffset);
  return getArciumProgram(params.provider)
    .methods.reclaimExpiredComputationFee(offset, params.mxeProgramId)
    .accountsPartial({
      signer: params.signer,
      mxe: getMXEAccAddress(params.mxeProgramId),
      clusterAcc: getClusterAccAddress(params.clusterOffset),
      comp: getComputationAccAddress(params.clusterOffset, offset),
      executingPool: getExecutingPoolAccAddress(params.clusterOffset),
      poolAccount: getFeePoolAccAddress(),
    })
    .instruction();
}

/**
 * Sends the client's claimComputationRent after the payer check. The provider
 * wallet signs, so it must be the original payer. Needs a live cluster.
 */
export async function sendClaimComputationRent(
  params: Omit<ReclaimCommon, "signer"> & { readonly clusterOffset: number },
): Promise<string> {
  const signer = params.provider.publicKey;
  requirePayer(signer, params.originalPayer);
  return claimComputationRent(
    params.provider,
    params.clusterOffset,
    toComputationOffsetBn(params.computationOffset),
  );
}
