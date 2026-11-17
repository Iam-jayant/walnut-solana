// Transaction-size check (Requirement 14.9, ADR-0003).
//
// Computes the exact wire length of a transaction from its compiled message,
// the same approach as packages/sdk/tx-size/measure.ts. web3.js serializes into
// fixed 1,232-byte buffers and throws on an oversized transaction, so the
// length is computed analytically from the Solana wire format instead: the
// signatures, the version prefix, the header, the static account keys, the
// blockhash, the instructions, and any address-table lookups. An oversized
// transaction is reported, never thrown.

import {
  Message,
  MessageV0,
  Transaction,
  VersionedTransaction,
  type VersionedMessage,
} from "@solana/web3.js";

/** Solana's packet limit for a serialized transaction. */
export const TX_HARD_LIMIT = 1232;
/** Walnut's design target; above it an address lookup table is expected. */
export const TX_TARGET = 1150;

export interface TransactionSizeReport {
  /** Exact serialized length including signature slots. */
  readonly bytes: number;
  readonly hardLimit: number;
  readonly target: number;
  /**
   * True when `bytes` is strictly under the hard limit. This matches the
   * Task 2.5 gate (tx-size/measure.ts), which treats 1,232 as a failure.
   */
  readonly ok: boolean;
  /** True when `bytes` is above the 1,150-byte target. */
  readonly overTarget: boolean;
}

/** Anything the checker can size. */
export type SizableTransaction =
  Transaction | VersionedTransaction | Message | MessageV0 | VersionedMessage;

/** Byte length of a shortvec (compact-u16) length prefix. */
export function compactLength(n: number): number {
  if (n < 0x80) return 1;
  if (n < 0x4000) return 2;
  return 3;
}

const BASE58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/** Number of bytes a base58 string decodes to (legacy instruction data). */
function base58DecodedLength(text: string): number {
  let leadingZeros = 0;
  while (leadingZeros < text.length && text[leadingZeros] === "1") {
    leadingZeros += 1;
  }
  let value = 0n;
  for (const char of text) {
    const digit = BASE58.indexOf(char);
    if (digit < 0) throw new Error(`invalid base58 character: ${char}`);
    value = value * 58n + BigInt(digit);
  }
  let bytes = 0;
  while (value > 0n) {
    value >>= 8n;
    bytes += 1;
  }
  return leadingZeros + bytes;
}

/** Exact length of a compiled v0 message plus its signature slots. */
export function v0TransactionByteLength(message: MessageV0): number {
  const numSigs = message.header.numRequiredSignatures;
  let len = 1 + 3; // version prefix + 3-byte header
  len +=
    compactLength(message.staticAccountKeys.length) +
    32 * message.staticAccountKeys.length;
  len += 32; // recent blockhash
  len += compactLength(message.compiledInstructions.length);
  for (const ix of message.compiledInstructions) {
    len += 1; // program id index
    len +=
      compactLength(ix.accountKeyIndexes.length) + ix.accountKeyIndexes.length;
    len += compactLength(ix.data.length) + ix.data.length;
  }
  len += compactLength(message.addressTableLookups.length);
  for (const lut of message.addressTableLookups) {
    len += 32; // lookup table account key
    len +=
      compactLength(lut.writableIndexes.length) + lut.writableIndexes.length;
    len +=
      compactLength(lut.readonlyIndexes.length) + lut.readonlyIndexes.length;
  }
  return compactLength(numSigs) + 64 * numSigs + len;
}

/** Exact length of a compiled legacy message plus its signature slots. */
export function legacyTransactionByteLength(message: Message): number {
  const numSigs = message.header.numRequiredSignatures;
  let len = 3; // header
  len +=
    compactLength(message.accountKeys.length) + 32 * message.accountKeys.length;
  len += 32; // recent blockhash
  len += compactLength(message.instructions.length);
  for (const ix of message.instructions) {
    const dataLength = base58DecodedLength(ix.data);
    len += 1; // program id index
    len += compactLength(ix.accounts.length) + ix.accounts.length;
    len += compactLength(dataLength) + dataLength;
  }
  return compactLength(numSigs) + 64 * numSigs + len;
}

function messageByteLength(message: unknown): number {
  if (message instanceof MessageV0) return v0TransactionByteLength(message);
  if (message instanceof Message) return legacyTransactionByteLength(message);
  throw new Error("unsupported transaction message version");
}

function byteLength(input: SizableTransaction): number {
  if (input instanceof VersionedTransaction) {
    return messageByteLength(input.message);
  }
  if (input instanceof Transaction) {
    return legacyTransactionByteLength(input.compileMessage());
  }
  return messageByteLength(input);
}
/**
 * Reports the serialized size of a transaction against the hard limit and the
 * target. Accepts a legacy Transaction (blockhash and fee payer set), a
 * VersionedTransaction, or a compiled Message or MessageV0. Never throws for
 * an oversized transaction.
 */
export function checkTransactionSize(
  input: SizableTransaction,
): TransactionSizeReport {
  const bytes = byteLength(input);
  return {
    bytes,
    hardLimit: TX_HARD_LIMIT,
    target: TX_TARGET,
    ok: bytes < TX_HARD_LIMIT,
    overTarget: bytes > TX_TARGET,
  };
}
