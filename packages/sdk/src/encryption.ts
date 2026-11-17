// Encryption and decryption helpers (Requirement 14.3, ADR-0003).
//
// Wraps x25519 + RescueCipher from @arcium-hq/client 0.16.0 with the rules the
// ADR approved:
//   * a fresh random 16-byte nonce for every encryption, never sequential;
//   * the MXE derives each output nonce as its owning input's nonce + 1, so the
//     effective (key, nonce) pairs of an input and its output are both
//     recorded and must never repeat.
// Secrets stay in memory. Nothing here logs, stores, or returns the x25519
// private key or the shared secret.

import {
  deserializeLE,
  RescueCipher,
  serializeLE,
  x25519,
} from "@arcium-hq/client";

/** RescueCipher nonces are 16 bytes (a u128 on chain). */
export const NONCE_BYTES = 16;
const U128_MAX = (1n << 128n) - 1n;
const X25519_KEY_BYTES = 32;

/** A client x25519 key paired with the cipher derived with the MXE key. */
export interface EncryptionSession {
  /** The client x25519 public key. It is the effective key of the ciphertexts. */
  readonly publicKey: Uint8Array;
  readonly cipher: RescueCipher;
}

/** Fills `length` bytes from the platform CSPRNG (Node and browsers). */
export function randomBytes(length: number): Uint8Array {
  const bytes = new Uint8Array(length);
  globalThis.crypto.getRandomValues(bytes);
  return bytes;
}

/** Returns a fresh random 16-byte nonce. */
export function freshNonce(): Uint8Array {
  return randomBytes(NONCE_BYTES);
}

/**
 * Starts an encryption session with the MXE.
 *
 * `privateKey` is only for deterministic tests and key restoration (for
 * example a Viewing Key derived from a wallet signature). By default a new
 * random x25519 key is generated and never leaves this function.
 */
export function createSession(
  mxePublicKey: Uint8Array,
  privateKey?: Uint8Array,
): EncryptionSession {
  if (mxePublicKey.length !== X25519_KEY_BYTES) {
    throw new RangeError(
      `MXE public key must be ${X25519_KEY_BYTES} bytes, got ${mxePublicKey.length}`,
    );
  }
  const secret = privateKey ?? x25519.utils.randomSecretKey();
  const publicKey = x25519.getPublicKey(secret);
  const sharedSecret = x25519.getSharedSecret(secret, mxePublicKey);
  return { publicKey, cipher: new RescueCipher(sharedSecret) };
}

/** Converts a u128 nonce to 16 little-endian bytes. */
export function u128ToNonce(value: bigint): Uint8Array {
  if (value < 0n || value > U128_MAX) {
    throw new RangeError(`nonce does not fit in u128: ${value}`);
  }
  return serializeLE(value, NONCE_BYTES);
}

/** Converts 16 little-endian nonce bytes to a u128. */
export function nonceToU128(nonce: Uint8Array): bigint {
  assertNonce(nonce);
  return deserializeLE(nonce);
}

function assertNonce(nonce: Uint8Array): void {
  if (nonce.length !== NONCE_BYTES) {
    throw new RangeError(
      `nonce must be ${NONCE_BYTES} bytes, got ${nonce.length}`,
    );
  }
}

function toHex(bytes: Uint8Array): string {
  let out = "";
  for (const b of bytes) out += b.toString(16).padStart(2, "0");
  return out;
}

/** Thrown when an effective (key, nonce) pair is used a second time. */
export class NonceReuseError extends Error {
  constructor(keyHex: string, nonce: bigint) {
    super(`effective (key, nonce) pair repeated: ${keyHex}:${nonce}`);
    this.name = "NonceReuseError";
  }
}

/**
 * Records every effective (key, nonce) pair and throws on any repeat. It
 * mirrors the no-repeat guard of tests/nonce-ownership.ts (Task 2.3).
 */
export class NonceLedger {
  readonly #seen = new Set<string>();

  /** Number of recorded pairs. */
  get size(): number {
    return this.#seen.size;
  }

  /** Whether the pair has been recorded. */
  has(key: Uint8Array, nonce: Uint8Array | bigint): boolean {
    return this.#seen.has(pairId(key, toU128(nonce)));
  }

  /** Records one pair. Throws {@link NonceReuseError} if it was seen before. */
  record(key: Uint8Array, nonce: Uint8Array | bigint): void {
    this.#insert(key, [toU128(nonce)]);
  }

  /**
   * Records an input nonce and the output nonce the MXE derives from it
   * (input + 1, wrapping at 2^128). Nothing is recorded if either repeats.
   */
  recordInput(key: Uint8Array, nonce: Uint8Array | bigint): void {
    const input = toU128(nonce);
    this.#insert(key, [input, (input + 1n) & U128_MAX]);
  }

  #insert(key: Uint8Array, nonces: bigint[]): void {
    const ids = nonces.map((n) => pairId(key, n));
    ids.forEach((id, i) => {
      if (this.#seen.has(id) || ids.indexOf(id) !== i) {
        throw new NonceReuseError(toHex(key), nonces[i] as bigint);
      }
    });
    for (const id of ids) this.#seen.add(id);
  }
}

function toU128(nonce: Uint8Array | bigint): bigint {
  if (typeof nonce !== "bigint") return nonceToU128(nonce);
  if (nonce < 0n || nonce > U128_MAX) {
    throw new RangeError(`nonce does not fit in u128: ${nonce}`);
  }
  return nonce;
}

function pairId(key: Uint8Array, nonce: bigint): string {
  return `${toHex(key)}:${nonce}`;
}

/** The result of {@link encryptInputs}. */
export interface EncryptedInputs {
  /** One 32-byte ciphertext per input value, in input order. */
  readonly ciphertexts: number[][];
  /** The 16-byte nonce used. */
  readonly nonce: Uint8Array;
  /** The same nonce as a u128, the form the program instruction takes. */
  readonly nonceU128: bigint;
}

/**
 * Encrypts `values` with a fresh random 16-byte nonce.
 *
 * Pass `nonce` only to reproduce a vector in tests: a repeated nonce under one
 * key breaks the cipher. If `track` is given, the input nonce and the derived
 * output nonce are recorded first and a repeat throws before anything is
 * encrypted.
 */
export function encryptInputs(
  cipher: RescueCipher,
  values: bigint[],
  nonce?: Uint8Array,
  track?: { ledger: NonceLedger; publicKey: Uint8Array },
): EncryptedInputs {
  const used = nonce ?? freshNonce();
  assertNonce(used);
  track?.ledger.recordInput(track.publicKey, used);
  return {
    ciphertexts: cipher.encrypt(values, used),
    nonce: used,
    nonceU128: deserializeLE(used),
  };
}

/** Decrypts ciphertexts with the nonce they were sealed under. */
export function decryptOutput(
  cipher: RescueCipher,
  ciphertexts: ArrayLike<number>[],
  nonce: Uint8Array | bigint,
): bigint[] {
  const n = typeof nonce === "bigint" ? u128ToNonce(nonce) : nonce;
  assertNonce(n);
  return cipher.decrypt(
    ciphertexts.map((c) => Array.from(c)),
    n,
  );
}
