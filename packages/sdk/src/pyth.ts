// Pyth price-update transaction flows (Task 6.4, Requirement 6.10).
//
// A Walnut price-sensitive operation (borrow, health, liquidation, …) needs a
// fresh, verified Pyth `PriceUpdateV2` account on chain before the queue
// instruction can reference it. There are two ways to arrange that, and the
// design fixes the choice by measured serialized size:
//
//   * same-transaction ("post-and-use"): the Pyth receiver post instructions
//     and the Walnut queue instruction ride in one transaction, so the update
//     and its use are atomic.
//   * post-then-use: when the combined transaction would exceed the size
//     target, the SDK posts the update in one transaction and immediately sends
//     the queue transaction referencing the verified PriceUpdate account
//     (design "Oracle design"; Requirement 6.10).
//
// This module assembles both and selects between them against the same
// `checkTransactionSize` gate the rest of the SDK uses (packages/sdk/src/
// tx-size.ts). It is transport-agnostic: the caller supplies the Pyth post
// instructions (from `@pythnetwork/pyth-solana-receiver`) and the Walnut queue
// instruction, so the selector is unit-testable without a cluster. The on-chain
// validation of the posted account (receiver owner, feed id, age, confidence,
// exponent, deviation) lives in the Rust `oracle` module
// (programs/walnut/src/oracle/mod.rs); this is only the transaction plumbing.

import {
  type AddressLookupTableAccount,
  PublicKey,
  TransactionInstruction,
  TransactionMessage,
  VersionedTransaction,
} from "@solana/web3.js";

import {
  checkTransactionSize,
  TX_TARGET,
  type TransactionSizeReport,
} from "./tx-size.ts";

/** Which Pyth update flow the SDK uses for a price-sensitive operation. */
export type PythUpdateMode = "same-transaction" | "post-then-use";

/** Inputs shared by both flows. */
export interface PythUpdateFlowInput {
  /** Fee payer for every transaction in the flow. */
  readonly payer: PublicKey;
  /** A recent blockhash for the transaction message(s). */
  readonly recentBlockhash: string;
  /**
   * The Pyth receiver instructions that post (and verify) the price update(s),
   * in order. Produced by `@pythnetwork/pyth-solana-receiver`.
   */
  readonly postInstructions: readonly TransactionInstruction[];
  /**
   * The Walnut price-sensitive queue instruction, which references the posted
   * PriceUpdate account(s) by address.
   */
  readonly queueInstruction: TransactionInstruction;
  /**
   * Optional instructions that close ephemeral PriceUpdate accounts after the
   * price has been consumed. They ride with the queue instruction so the close
   * never happens before the use.
   */
  readonly closeInstructions?: readonly TransactionInstruction[];
  /** Optional address lookup tables for the queue (and combined) transaction. */
  readonly lookupTables?: readonly AddressLookupTableAccount[];
}

/** The assembled flow plus the size evidence behind the choice. */
export interface PythUpdateFlow {
  readonly mode: PythUpdateMode;
  /** One transaction for same-transaction, two (post, then queue) otherwise. */
  readonly transactions: readonly VersionedTransaction[];
  /** The size of the single combined transaction that was attempted first. */
  readonly combinedSize: TransactionSizeReport;
  /** A short, human-readable reason for the selected mode. */
  readonly reason: string;
}

/** Options for {@link selectPythUpdateFlow}. */
export interface SelectPythUpdateFlowOptions {
  /**
   * Largest combined size (bytes) that still selects the same-transaction
   * flow. Defaults to the 1,150-byte design target so the combined path leaves
   * head-room below the 1,232-byte hard limit. A combined transaction above
   * this falls back to post-then-use.
   */
  readonly maxCombinedBytes?: number;
}

function toV0(
  payer: PublicKey,
  recentBlockhash: string,
  instructions: readonly TransactionInstruction[],
  lookupTables: readonly AddressLookupTableAccount[] = [],
): VersionedTransaction {
  const message = new TransactionMessage({
    payerKey: payer,
    recentBlockhash,
    instructions: [...instructions],
  }).compileToV0Message([...lookupTables]);
  return new VersionedTransaction(message);
}

/**
 * Builds the single "post-and-use" transaction: every Pyth post instruction,
 * then the Walnut queue instruction, then any close instructions.
 */
export function buildSameTransactionUpdate(
  input: PythUpdateFlowInput,
): VersionedTransaction {
  return toV0(
    input.payer,
    input.recentBlockhash,
    [
      ...input.postInstructions,
      input.queueInstruction,
      ...(input.closeInstructions ?? []),
    ],
    input.lookupTables,
  );
}

/**
 * Builds the two-transaction "post-then-use" fallback: one transaction that
 * posts (and verifies) the price update, and one that queues the Walnut
 * operation referencing the posted account and then closes it.
 */
export function buildPostThenUseUpdate(input: PythUpdateFlowInput): {
  readonly post: VersionedTransaction;
  readonly queue: VersionedTransaction;
} {
  const post = toV0(input.payer, input.recentBlockhash, input.postInstructions);
  const queue = toV0(
    input.payer,
    input.recentBlockhash,
    [input.queueInstruction, ...(input.closeInstructions ?? [])],
    input.lookupTables,
  );
  return { post, queue };
}

/**
 * Selects the production Pyth update flow by measured serialized size. The
 * combined "post-and-use" transaction is assembled and measured first; it is
 * used when it is within the hard limit and no larger than
 * `maxCombinedBytes` (the 1,150-byte target by default). Otherwise the
 * post-then-use fallback is returned. The combined size report is always
 * included as the evidence behind the choice (Requirement 6.10).
 */
export function selectPythUpdateFlow(
  input: PythUpdateFlowInput,
  options: SelectPythUpdateFlowOptions = {},
): PythUpdateFlow {
  const maxCombinedBytes = options.maxCombinedBytes ?? TX_TARGET;
  const combined = buildSameTransactionUpdate(input);
  const combinedSize = checkTransactionSize(combined);

  if (combinedSize.ok && combinedSize.bytes <= maxCombinedBytes) {
    return {
      mode: "same-transaction",
      transactions: [combined],
      combinedSize,
      reason:
        `combined post-and-use transaction is ${combinedSize.bytes} bytes, ` +
        `within the ${maxCombinedBytes}-byte selection limit`,
    };
  }

  const { post, queue } = buildPostThenUseUpdate(input);
  return {
    mode: "post-then-use",
    transactions: [post, queue],
    combinedSize,
    reason: combinedSize.ok
      ? `combined transaction is ${combinedSize.bytes} bytes, above the ` +
        `${maxCombinedBytes}-byte target; posting then using instead`
      : `combined transaction is ${combinedSize.bytes} bytes, over the ` +
        `${combinedSize.hardLimit}-byte hard limit; posting then using instead`,
  };
}
