// Viewing Key derivation, local view, and browser custody (Task 5.5;
// Requirements 5.9, 5.10, 5.12, 5.14; design: "Viewing Key and rotation").
//
// The Viewing Key is an x25519 key derived on the client from a wallet
// signature over a domain-separated message that binds the program, the
// network, the Position PDA, and the key epoch:
//
//   walnut-solana viewing key v1 | program=<ID> | network=<NET> | position=<PDA> | epoch=<EPOCH>
//
// The wallet signature is turned into the x25519 secret with HKDF-SHA-256
// (RFC 5869; salt = the message domain, info = a fixed label, 32 bytes out).
// Ed25519 signatures are deterministic, so the same wallet, message, and
// epoch always give the same key; a different epoch gives a different key.
//
// Custody rules (enforced here, tested in test/viewing-key.test.ts):
//   * the signature and the secret never leave the client: no function here
//     takes them to a transaction, a URL, storage, or a log;
//   * the secret lives in one Uint8Array owned by the session and is zeroed
//     by `clear()`; `unlock()` zeroes the caller's signature buffer by default;
//   * secret-bearing objects refuse JSON serialization (`toJSON` throws) and
//     print a redacted form to `util.inspect` and the Node console;
//   * `lock()`, a wallet `disconnect`/`accountChanged` event, `pagehide`, and
//     a hidden `visibilitychange` clear the session.
// The transient RescueCipher built inside `decryptPosition` holds a derived
// shared secret in library-owned memory for the duration of the call; JavaScript
// gives no way to wipe it, so it is dropped (not retained) when the call ends.

import { PublicKey } from "@solana/web3.js";
import { createSession, decryptOutput } from "./encryption.ts";
import { x25519 } from "@arcium-hq/client";

/** First line of every Viewing Key message (also the HKDF salt). */
export const VIEWING_KEY_MESSAGE_DOMAIN = "walnut-solana viewing key v1";
/** HKDF info label of the x25519 secret. */
export const VIEWING_KEY_KDF_INFO =
  "walnut-solana viewing key v1 x25519 secret";
/** Ed25519 signature length in bytes. */
export const WALLET_SIGNATURE_BYTES = 64;
/** x25519 secret and public key length in bytes. */
export const VIEWING_KEY_BYTES = 32;

const U64_MAX = (1n << 64n) - 1n;
const U128_MAX = (1n << 128n) - 1n;
const NETWORK_PATTERN = /^[a-z0-9][a-z0-9-]{0,31}$/;
const INSPECT = Symbol.for("nodejs.util.inspect.custom");

/** Thrown for custody violations: use after clear, serialization attempts. */
export class ViewingKeyCustodyError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ViewingKeyCustodyError";
  }
}

/** Thrown when a Position payload does not decrypt to valid balances. */
export class ViewingKeyDecryptError extends Error {
  constructor() {
    super("Position payload did not decrypt under this Viewing Key");
    this.name = "ViewingKeyDecryptError";
  }
}

/** The inputs of the signed message. */
export interface ViewingKeyMessageParams {
  /** The Walnut program id. */
  programId: PublicKey | string;
  /** A lowercase network name from the Network Profile, such as `devnet`. */
  network: string;
  /** The Position PDA. */
  positionPda: PublicKey | string;
  /** The Viewing Key epoch (`Position.viewing_key_epoch`), a u64. */
  epoch: bigint | number;
}

function base58Of(value: PublicKey | string, name: string): string {
  try {
    return (
      typeof value === "string" ? new PublicKey(value) : value
    ).toBase58();
  } catch {
    throw new RangeError(`${name} is not a valid public key`);
  }
}

function epochOf(epoch: bigint | number): bigint {
  const value = typeof epoch === "number" ? BigInt(epoch) : epoch;
  if (
    (typeof epoch === "number" && !Number.isSafeInteger(epoch)) ||
    value < 0n ||
    value > U64_MAX
  ) {
    throw new RangeError("epoch must be a u64");
  }
  return value;
}

/**
 * The exact text the wallet signs:
 * `walnut-solana viewing key v1 | program=<ID> | network=<NET> | position=<PDA> | epoch=<EPOCH>`.
 *
 * Fields are separated by ` | `. Public keys are canonical base58 and the epoch
 * is a decimal u64. The network must be a lowercase `[a-z0-9-]` name so no
 * field can contain the separator.
 */
export function viewingKeyMessage(params: ViewingKeyMessageParams): string {
  if (!NETWORK_PATTERN.test(params.network)) {
    throw new RangeError("network must be a lowercase [a-z0-9-] name");
  }
  const program = base58Of(params.programId, "programId");
  const position = base58Of(params.positionPda, "positionPda");
  const epoch = epochOf(params.epoch);
  return [
    VIEWING_KEY_MESSAGE_DOMAIN,
    `program=${program}`,
    `network=${params.network}`,
    `position=${position}`,
    `epoch=${epoch}`,
  ].join(" | ");
}

/** The message as the UTF-8 bytes passed to the wallet's `signMessage`. */
export function viewingKeyMessageBytes(
  params: ViewingKeyMessageParams,
): Uint8Array {
  return new TextEncoder().encode(viewingKeyMessage(params));
}

/**
 * A derived Viewing Key. The secret is readable through `secret` (not
 * enumerable, so spreading or `Object.keys` cannot expose it); JSON
 * serialization throws and printing it is redacted.
 */
export class ViewingKeyMaterial {
  /** The x25519 public key, registered on chain as `viewing_pubkey`. */
  readonly publicKey: Uint8Array;
  readonly #secret: Uint8Array;

  constructor(secret: Uint8Array) {
    if (secret.length !== VIEWING_KEY_BYTES) {
      throw new RangeError(`secret must be ${VIEWING_KEY_BYTES} bytes`);
    }
    this.#secret = secret;
    this.publicKey = x25519.getPublicKey(secret);
  }

  /** The x25519 secret. The session that adopts this material zeroes it. */
  get secret(): Uint8Array {
    return this.#secret;
  }

  /** Zeroes the secret buffer. */
  wipe(): void {
    this.#secret.fill(0);
  }

  toJSON(): never {
    throw new ViewingKeyCustodyError("a Viewing Key must not be serialized");
  }

  toString(): string {
    return "[ViewingKeyMaterial redacted]";
  }

  [INSPECT](): string {
    return this.toString();
  }
}

/**
 * Derives the Viewing Key from a wallet signature over
 * {@link viewingKeyMessage}.
 *
 * `signature` is the 64-byte Ed25519 signature. It is only read here; pass
 * `wipeSignature` to zero the caller's buffer once the key is derived.
 */
export async function deriveViewingKey(
  signature: Uint8Array,
  options: { wipeSignature?: boolean } = {},
): Promise<ViewingKeyMaterial> {
  if (signature.length !== WALLET_SIGNATURE_BYTES) {
    throw new RangeError(
      `wallet signature must be ${WALLET_SIGNATURE_BYTES} bytes`,
    );
  }
  const subtle = globalThis.crypto.subtle;
  const encoder = new TextEncoder();
  const base = await subtle.importKey(
    "raw",
    signature as Uint8Array<ArrayBuffer>,
    "HKDF",
    false,
    ["deriveBits"],
  );
  const bits = await subtle.deriveBits(
    {
      name: "HKDF",
      hash: "SHA-256",
      salt: encoder.encode(VIEWING_KEY_MESSAGE_DOMAIN),
      info: encoder.encode(VIEWING_KEY_KDF_INFO),
    },
    base,
    VIEWING_KEY_BYTES * 8,
  );
  if (options.wipeSignature === true) signature.fill(0);
  return new ViewingKeyMaterial(new Uint8Array(bits));
}

/** The encrypted Position payload as stored on chain. */
export interface EncryptedPositionState {
  /** The output nonce of the stored revision (u128). */
  nonce: bigint | Uint8Array;
  /** Five 32-byte ciphertexts in `PositionSecret` order. */
  ciphertexts: ArrayLike<number>[];
}

/** The five decrypted Position balances (`PositionSecret` order). */
export interface PositionBalances {
  freeSol: bigint;
  freeUsdc: bigint;
  collateralSol: bigint;
  supplySharesUsdc: bigint;
  debtSharesUsdc: bigint;
}

/** Field widths of `PositionSecret`: three u64 then two u128. */
const FIELD_MAX = [U64_MAX, U64_MAX, U64_MAX, U128_MAX, U128_MAX] as const;

/**
 * Decrypts a stored Position payload locally (Requirement 5.9). No network.
 *
 * A wrong key does not make Rescue decryption fail by itself: it yields
 * uniformly random field elements. The values are range-checked against the
 * `u64`/`u128` field widths, so a wrong key or a corrupted payload throws
 * {@link ViewingKeyDecryptError} (a wrong key passes with probability
 * about 2^-190).
 */
export function decryptPosition(params: {
  encryptedState: EncryptedPositionState;
  viewingSecret: Uint8Array;
  mxePublicKey: Uint8Array;
}): PositionBalances {
  const { encryptedState, viewingSecret, mxePublicKey } = params;
  if (encryptedState.ciphertexts.length !== FIELD_MAX.length) {
    throw new RangeError(
      `a Position payload has ${FIELD_MAX.length} ciphertexts`,
    );
  }
  const session = createSession(mxePublicKey, viewingSecret);
  const values = decryptOutput(
    session.cipher,
    encryptedState.ciphertexts,
    encryptedState.nonce,
  );
  for (let i = 0; i < FIELD_MAX.length; i++) {
    const value = values[i];
    if (value === undefined || value < 0n || value > (FIELD_MAX[i] as bigint)) {
      throw new ViewingKeyDecryptError();
    }
  }
  const [freeSol, freeUsdc, collateralSol, supplySharesUsdc, debtSharesUsdc] =
    values as [bigint, bigint, bigint, bigint, bigint];
  return { freeSol, freeUsdc, collateralSol, supplySharesUsdc, debtSharesUsdc };
}

/** The wallet-adapter-like event source a session listens to. */
export interface WalletEventSource {
  on(event: string, listener: () => void): unknown;
  off?(event: string, listener: () => void): unknown;
  removeListener?(event: string, listener: () => void): unknown;
}

/** The wallet events that end a session (the wallet or account went away). */
export const WALLET_CLEAR_EVENTS: readonly string[] = [
  "disconnect",
  "accountChanged",
];

/** The structural subset of `window` the page hooks use. */
export interface PageEventTarget {
  addEventListener(event: string, listener: () => void): void;
  removeEventListener(event: string, listener: () => void): void;
  document?: { visibilityState?: string };
}

/**
 * An in-memory Viewing Key session. The secret is held in one buffer, zeroed
 * by {@link ViewingKeySession.clear}; any use after that throws. A session is
 * single-use: unlock again by deriving a new one.
 */
export class ViewingKeySession {
  #secret: Uint8Array | null;
  readonly #publicKey: Uint8Array;
  readonly #detachers: Array<() => void> = [];

  /**
   * Adopts `material`: the session now owns its secret buffer and zeroes it
   * on `clear()`. Prefer {@link ViewingKeySession.unlock}.
   */
  constructor(material: ViewingKeyMaterial) {
    this.#secret = material.secret;
    this.#publicKey = material.publicKey;
  }

  /** Derives the key from a wallet signature and wraps it in a session. */
  static async unlock(
    signature: Uint8Array,
    options: { wipeSignature?: boolean } = { wipeSignature: true },
  ): Promise<ViewingKeySession> {
    return new ViewingKeySession(await deriveViewingKey(signature, options));
  }

  /** Whether the secret is still held. */
  get isUnlocked(): boolean {
    return this.#secret !== null;
  }

  /** A copy of the public key. Throws after `clear()`. */
  get publicKey(): Uint8Array {
    this.#live();
    return new Uint8Array(this.#publicKey);
  }

  /** Decrypts a Position payload with the session key. Throws after `clear()`. */
  decryptPosition(
    encryptedState: EncryptedPositionState,
    mxePublicKey: Uint8Array,
  ): PositionBalances {
    return decryptPosition({
      encryptedState,
      viewingSecret: this.#live(),
      mxePublicKey,
    });
  }

  /**
   * Clears the session on any event of `events` from `source` (a wallet
   * adapter). Returns a function that detaches the listeners early.
   */
  attachWallet(
    source: WalletEventSource,
    events: readonly string[] = WALLET_CLEAR_EVENTS,
  ): () => void {
    this.#live();
    const listener = (): void => this.clear();
    for (const event of events) source.on(event, listener);
    const detach = (): void => {
      for (const event of events) {
        if (typeof source.off === "function") source.off(event, listener);
        else source.removeListener?.(event, listener);
      }
    };
    this.#detachers.push(detach);
    return detach;
  }

  /**
   * Clears the session on `pagehide` and when `visibilitychange` leaves the
   * page hidden. Does nothing when there is no page (`window` is absent).
   * Returns the detach function.
   */
  attachPage(
    target: PageEventTarget | undefined = (
      globalThis as { window?: PageEventTarget }
    ).window,
  ): () => void {
    this.#live();
    if (target === undefined || typeof target.addEventListener !== "function") {
      return () => {};
    }
    const onHide = (): void => this.clear();
    const onVisibility = (): void => {
      if (target.document?.visibilityState === "hidden") this.clear();
    };
    target.addEventListener("pagehide", onHide);
    target.addEventListener("visibilitychange", onVisibility);
    const detach = (): void => {
      target.removeEventListener("pagehide", onHide);
      target.removeEventListener("visibilitychange", onVisibility);
    };
    this.#detachers.push(detach);
    return detach;
  }

  /** The user locked the app: clears the session. */
  lock(): void {
    this.clear();
  }

  /** The wallet disconnected: clears the session. */
  onDisconnect(): void {
    this.clear();
  }

  /** Zeroes the secret, drops it, and detaches every listener. Idempotent. */
  clear(): void {
    if (this.#secret !== null) {
      this.#secret.fill(0);
      this.#secret = null;
    }
    for (const detach of this.#detachers.splice(0)) detach();
  }

  toJSON(): never {
    throw new ViewingKeyCustodyError(
      "a Viewing Key session must not be serialized",
    );
  }

  toString(): string {
    return `[ViewingKeySession ${this.isUnlocked ? "unlocked" : "locked"}]`;
  }

  [INSPECT](): string {
    return this.toString();
  }

  #live(): Uint8Array {
    if (this.#secret === null) {
      throw new ViewingKeyCustodyError("the Viewing Key session is cleared");
    }
    return this.#secret;
  }
}

/** The arguments of the owner-signed rotation instruction (Requirement 5.11). */
export interface RotationArgs {
  /** The epoch the Position has now. */
  expectedOldEpoch: bigint;
  /** The epoch after rotation: always `expectedOldEpoch + 1`. */
  expectedNewEpoch: bigint;
  /** The Viewing public key the Position has now. */
  expectedOldPubkey: Uint8Array;
  /** The Viewing public key to rotate to. */
  newViewingPubkey: Uint8Array;
}

/**
 * Builds the rotation arguments. `newMaterial` must be derived from a wallet
 * signature over the message for epoch `oldEpoch + 1` (see
 * {@link viewingKeyMessage}); only its public key is returned.
 */
export function rotationArgs(params: {
  oldEpoch: bigint | number;
  oldPublicKey: Uint8Array;
  newPublicKey: Uint8Array;
}): RotationArgs {
  const oldEpoch = epochOf(params.oldEpoch);
  if (oldEpoch === U64_MAX) {
    throw new RangeError("the Viewing Key epoch cannot be incremented");
  }
  if (
    params.oldPublicKey.length !== VIEWING_KEY_BYTES ||
    params.newPublicKey.length !== VIEWING_KEY_BYTES
  ) {
    throw new RangeError(`public keys must be ${VIEWING_KEY_BYTES} bytes`);
  }
  if (params.newPublicKey.every((byte) => byte === 0)) {
    throw new RangeError("the new Viewing public key must be nonzero");
  }
  if (params.newPublicKey.every((byte, i) => byte === params.oldPublicKey[i])) {
    throw new RangeError("the new Viewing public key must differ from the old");
  }
  return {
    expectedOldEpoch: oldEpoch,
    expectedNewEpoch: oldEpoch + 1n,
    expectedOldPubkey: new Uint8Array(params.oldPublicKey),
    newViewingPubkey: new Uint8Array(params.newPublicKey),
  };
}
