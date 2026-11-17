// Output nonces of Position-mutating circuits (Task 5.4; ADR-0003).
//
// Arcium seals an Enc<Shared, _> output under the owner key with nonce = the
// owning input's nonce + 1, and Rescue is stream-style: two outputs under one
// (key, nonce) leak the difference of their plaintexts. An attempt that does
// not apply leaves the stored nonce in place, so sealing under "stored nonce
// + 1" would repeat the pair on the next attempt (and the rejected output is
// still public in its callback transaction).
//
// Every Position-mutating circuit therefore takes a bare `Shared` input whose
// nonce the PROGRAM derives from the Position address and the unique Operation
// Intent sequence (programs/walnut/src/state/nonce.rs):
//
//   n = u128_le(sha256("walnut:position-output-nonce:v1" | position | seq_le)
//         [0..16]) & (2^127 - 1)
//
// The circuit output nonce is n + 1. The client never chooses it; this module
// mirrors the derivation so a client can predict, record, and check it.

import type { PublicKey } from "@solana/web3.js";
import { NonceLedger } from "./encryption.ts";
import { u64ToLeBytes } from "./pda.ts";

/** Domain separator of the derivation. */
export const OUTPUT_NONCE_DOMAIN = "walnut:position-output-nonce:v1";

const MASK_127 = (1n << 127n) - 1n;

/**
 * The output-nonce input `n` of the attempt with `intentSequence` on
 * `position` (the nonce the program passes to the circuit).
 */
export async function deriveOutputNonceInput(
  position: PublicKey | Uint8Array,
  intentSequence: bigint | number,
): Promise<bigint> {
  const positionBytes =
    position instanceof Uint8Array ? position : position.toBytes();
  if (positionBytes.length !== 32) {
    throw new RangeError(
      `position must be 32 bytes, got ${positionBytes.length}`,
    );
  }
  const domain = new TextEncoder().encode(OUTPUT_NONCE_DOMAIN);
  const seq = u64ToLeBytes(intentSequence);
  const data = new Uint8Array(domain.length + 32 + 8);
  data.set(domain, 0);
  data.set(positionBytes, domain.length);
  data.set(seq, domain.length + 32);
  const digest = new Uint8Array(
    await globalThis.crypto.subtle.digest("SHA-256", data),
  );
  let value = 0n;
  for (let i = 15; i >= 0; i--) {
    value = (value << 8n) | BigInt(digest[i] as number);
  }
  return value & MASK_127;
}

/** The nonce the circuit seals its output under: `n + 1`. */
export async function deriveOutputNonce(
  position: PublicKey | Uint8Array,
  intentSequence: bigint | number,
): Promise<bigint> {
  return (await deriveOutputNonceInput(position, intentSequence)) + 1n;
}

/**
 * Records the effective (key, nonce) pairs of one attempt in `ledger`: the
 * derived input nonce and its output (input + 1). Throws NonceReuseError if
 * either pair was seen before, so a repeated attempt output is caught.
 */
export async function recordAttemptOutput(
  ledger: NonceLedger,
  viewingPublicKey: Uint8Array,
  position: PublicKey | Uint8Array,
  intentSequence: bigint | number,
): Promise<bigint> {
  const input = await deriveOutputNonceInput(position, intentSequence);
  ledger.recordInput(viewingPublicKey, input);
  return input + 1n;
}
