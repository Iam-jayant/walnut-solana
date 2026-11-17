// Pyth PriceUpdateV2 layout and the TypeScript mirror of the Rust oracle
// validation (Task 6.5, Requirement 6).
//
// The on-chain authority is `programs/walnut/src/oracle/mod.rs`
// (`validate_price`). This module ports the same rules with `bigint` so the SDK,
// the demo scripts, and the localnet fixture can pre-check a price update the
// way the program will. Parity is pinned by golden vectors that appear in both
// the Rust tests and `packages/sdk/test/oracle.test.ts`; if one side changes,
// the other test breaks.
//
// `PriceUpdateV2` account layout (pyth-solana-receiver-sdk, Anchor account):
//
//   offset  size  field
//   0       8     Anchor discriminator  [34, 241, 35, 99, 157, 126, 244, 205]
//   8       32    write_authority
//   40      1|2   verification_level: 1 = Full, 0 + num_signatures(u8) = Partial
//   ..      32    price_message.feed_id
//   ..      8     price (i64 LE)
//   ..      8     conf (u64 LE)
//   ..      4     exponent (i32 LE)
//   ..      8     publish_time (i64 LE)
//   ..      8     prev_publish_time (i64 LE)
//   ..      8     ema_price (i64 LE)
//   ..      8     ema_conf (u64 LE)
//   ..      8     posted_slot (u64 LE)
//
// The receiver allocates the account at its maximum size (134 bytes); a Full
// verification level serializes one byte shorter than Partial, so the tail is
// zero padding.

import { PublicKey } from "@solana/web3.js";

/** The Pyth Solana receiver program id (devnet and mainnet). */
export const PYTH_RECEIVER_PROGRAM_ID = new PublicKey(
  "rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ",
);

/** Anchor discriminator of `PriceUpdateV2`. */
export const PRICE_UPDATE_V2_DISCRIMINATOR = Uint8Array.from([
  34, 241, 35, 99, 157, 126, 244, 205,
]);

/** Allocated size of a `PriceUpdateV2` account. */
export const PRICE_UPDATE_V2_LEN = 134;

/** Pyth feed ids (hex, no 0x). Identical on devnet and mainnet. */
export const PYTH_FEED_IDS = {
  "SOL/USD": "ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d",
  "USDC/USD":
    "eaa020c61cc479712813461ce153894a96a6c00b21ed0cfc2798d1f9a9e9c94a",
} as const;

// ---- Constants mirrored from the Rust program --------------------------------

/** `crate::math::USD_SCALE`. */
export const USD_SCALE = 100_000_000n;
/** `crate::math::MIN_NORMALIZED_PRICE`. */
export const MIN_NORMALIZED_PRICE = 1n;
/** `crate::math::MAX_NORMALIZED_PRICE`. */
export const MAX_NORMALIZED_PRICE = 1_000_000_000_000_000_000n;
/** `crate::math::MIN_PYTH_EXPONENT`. */
export const MIN_PYTH_EXPONENT = -12;
/** `crate::math::MAX_PYTH_EXPONENT`. */
export const MAX_PYTH_EXPONENT = 12;
const BPS_DENOMINATOR = 10_000n;
const U64_MAX = (1n << 64n) - 1n;
const U128_MAX = (1n << 128n) - 1n;

/** Oracle parameter bounds (Requirements 6.2 and 6.3). */
export const ORACLE_BOUNDS = {
  maxAgeSeconds: [5, 300],
  maxConfidenceBps: [10, 1_000],
  maxDeviationBps: [50, 5_000],
} as const;

/** Rust `OracleError`, by variant name. */
export type OracleError =
  | "WrongReceiverProgram"
  | "FeedMismatch"
  | "StalePrice"
  | "NonPositivePrice"
  | "ConfidenceTooWide"
  | "ExponentOutOfRange"
  | "PriceOutOfRange"
  | "DeviationTooHigh"
  | "InvalidOracleParams"
  | "NegativeTimestamp"
  | "ArithmeticOverflow";

/** Rust `OracleParams`. */
export interface OracleParams {
  readonly maxAgeSeconds: number;
  readonly maxConfidenceBps: number;
  readonly maxDeviationBps: number;
}

/** Rust `OracleParams::LAUNCH_DEFAULT`. */
export const LAUNCH_ORACLE_PARAMS: OracleParams = {
  maxAgeSeconds: 60,
  maxConfidenceBps: 200,
  maxDeviationBps: 500,
};

/** Rust `PriceUse`. */
export type PriceUse = "collateral" | "debt";

/** The price fields the checks read (Rust `PythPriceView`). */
export interface PythPriceView {
  readonly owner: PublicKey;
  readonly feedId: Uint8Array;
  readonly price: bigint;
  readonly conf: bigint;
  readonly exponent: number;
  readonly publishTime: bigint;
  readonly emaPrice: bigint;
  readonly emaConf: bigint;
}

/** Result of {@link validatePrice}. */
export type ValidationResult =
  | {
      readonly ok: true;
      readonly normalized: bigint;
      readonly publishTime: bigint;
    }
  | { readonly ok: false; readonly error: OracleError };

const fail = (error: OracleError): ValidationResult => ({ ok: false, error });

// ---- Account encoding --------------------------------------------------------

/** The decoded body of a `PriceUpdateV2` account. */
export interface PriceUpdateV2 {
  readonly writeAuthority: PublicKey;
  /** `"full"`, or the number of verified signatures for a partial update. */
  readonly verificationLevel: "full" | number;
  readonly feedId: Uint8Array;
  readonly price: bigint;
  readonly conf: bigint;
  readonly exponent: number;
  readonly publishTime: bigint;
  readonly prevPublishTime: bigint;
  readonly emaPrice: bigint;
  readonly emaConf: bigint;
  readonly postedSlot: bigint;
}

/** Serializes a `PriceUpdateV2` account body (134 bytes, zero padded). */
export function encodePriceUpdateV2(update: PriceUpdateV2): Uint8Array {
  if (update.feedId.length !== 32) {
    throw new Error("feedId must be 32 bytes");
  }
  const out = new Uint8Array(PRICE_UPDATE_V2_LEN);
  const view = new DataView(out.buffer);
  out.set(PRICE_UPDATE_V2_DISCRIMINATOR, 0);
  out.set(update.writeAuthority.toBytes(), 8);
  let at = 40;
  if (update.verificationLevel === "full") {
    out[at] = 1;
    at += 1;
  } else {
    out[at] = 0;
    out[at + 1] = update.verificationLevel;
    at += 2;
  }
  out.set(update.feedId, at);
  at += 32;
  view.setBigInt64(at, update.price, true);
  view.setBigUint64(at + 8, update.conf, true);
  view.setInt32(at + 16, update.exponent, true);
  view.setBigInt64(at + 20, update.publishTime, true);
  view.setBigInt64(at + 28, update.prevPublishTime, true);
  view.setBigInt64(at + 36, update.emaPrice, true);
  view.setBigUint64(at + 44, update.emaConf, true);
  view.setBigUint64(at + 52, update.postedSlot, true);
  return out;
}

/** Parses a `PriceUpdateV2` account body. Throws on a malformed account. */
export function parsePriceUpdateV2(data: Uint8Array): PriceUpdateV2 {
  if (data.length < PRICE_UPDATE_V2_LEN) {
    throw new Error(
      `PriceUpdateV2 account is ${data.length} bytes, expected ${PRICE_UPDATE_V2_LEN}`,
    );
  }
  for (let i = 0; i < 8; i++) {
    if (data[i] !== PRICE_UPDATE_V2_DISCRIMINATOR[i]) {
      throw new Error("not a PriceUpdateV2 account (bad discriminator)");
    }
  }
  const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
  const writeAuthority = new PublicKey(data.slice(8, 40));
  let at = 40;
  let verificationLevel: "full" | number;
  if (data[at] === 1) {
    verificationLevel = "full";
    at += 1;
  } else if (data[at] === 0) {
    verificationLevel = data[at + 1] ?? 0;
    at += 2;
  } else {
    throw new Error("unknown verification level tag");
  }
  return {
    writeAuthority,
    verificationLevel,
    feedId: data.slice(at, at + 32),
    price: view.getBigInt64(at + 32, true),
    conf: view.getBigUint64(at + 40, true),
    exponent: view.getInt32(at + 48, true),
    publishTime: view.getBigInt64(at + 52, true),
    prevPublishTime: view.getBigInt64(at + 60, true),
    emaPrice: view.getBigInt64(at + 68, true),
    emaConf: view.getBigUint64(at + 76, true),
    postedSlot: view.getBigUint64(at + 84, true),
  };
}

/** Lifts a parsed account into the view the checks read. */
export function toPriceView(
  update: PriceUpdateV2,
  owner: PublicKey,
): PythPriceView {
  return {
    owner,
    feedId: update.feedId,
    price: update.price,
    conf: update.conf,
    exponent: update.exponent,
    publishTime: update.publishTime,
    emaPrice: update.emaPrice,
    emaConf: update.emaConf,
  };
}

// ---- Validation (mirror of oracle/mod.rs) ------------------------------------

function pow10(exp: number): bigint {
  return 10n ** BigInt(exp);
}

/** Mirror of `OracleParams::validate`. */
export function validateOracleParams(params: OracleParams): OracleError | null {
  const within = (v: number, [lo, hi]: readonly [number, number]) =>
    Number.isInteger(v) && v >= lo && v <= hi;
  if (
    !within(params.maxAgeSeconds, ORACLE_BOUNDS.maxAgeSeconds) ||
    !within(params.maxConfidenceBps, ORACLE_BOUNDS.maxConfidenceBps) ||
    !within(params.maxDeviationBps, ORACLE_BOUNDS.maxDeviationBps)
  ) {
    return "InvalidOracleParams";
  }
  return null;
}

/** Mirror of `normalize_price`. Returns the value or an error name. */
export function normalizePrice(
  mantissa: bigint,
  exponent: number,
  roundUp: boolean,
): { ok: true; value: bigint } | { ok: false; error: OracleError } {
  if (exponent < MIN_PYTH_EXPONENT || exponent > MAX_PYTH_EXPONENT) {
    return { ok: false, error: "ExponentOutOfRange" };
  }
  const scaled = mantissa * USD_SCALE;
  if (scaled > U128_MAX) return { ok: false, error: "ArithmeticOverflow" };
  let value: bigint;
  if (exponent >= 0) {
    value = scaled * pow10(exponent);
    if (value > U128_MAX) return { ok: false, error: "ArithmeticOverflow" };
  } else {
    const den = pow10(-exponent);
    value = roundUp ? (scaled + den - 1n) / den : scaled / den;
  }
  if (value < MIN_NORMALIZED_PRICE || value > MAX_NORMALIZED_PRICE) {
    return { ok: false, error: "PriceOutOfRange" };
  }
  return { ok: true, value };
}

/** Mirror of `deviation_within`: returns an error name or null when within. */
export function deviationWithin(
  spot: bigint,
  reference: bigint,
  maxDeviationBps: number,
): OracleError | null {
  if (reference === 0n) return "NonPositivePrice";
  const diff = spot > reference ? spot - reference : reference - spot;
  const lhs = diff * BPS_DENOMINATOR;
  const rhs = reference * BigInt(maxDeviationBps);
  if (lhs > U128_MAX || rhs > U128_MAX) return "ArithmeticOverflow";
  return lhs > rhs ? "DeviationTooHigh" : null;
}

/**
 * Mirror of `validate_price`: receiver and feed id, parameter bounds, age,
 * confidence, positivity, exponent, conservative normalization, and the
 * spot-versus-EMA breaker, in the same order as the Rust function.
 */
export function validatePrice(
  view: PythPriceView,
  expectedFeedId: Uint8Array,
  params: OracleParams,
  now: bigint,
  priceUse: PriceUse,
): ValidationResult {
  const paramError = validateOracleParams(params);
  if (paramError) return fail(paramError);

  if (!view.owner.equals(PYTH_RECEIVER_PROGRAM_ID)) {
    return fail("WrongReceiverProgram");
  }
  if (
    view.feedId.length !== expectedFeedId.length ||
    view.feedId.some((byte, i) => byte !== expectedFeedId[i])
  ) {
    return fail("FeedMismatch");
  }

  // check_age
  if (view.publishTime < 0n || now < 0n) return fail("NegativeTimestamp");
  if (view.publishTime > now) return fail("StalePrice");
  if (now - view.publishTime > BigInt(params.maxAgeSeconds)) {
    return fail("StalePrice");
  }

  // check_confidence
  if (view.price <= 0n) return fail("NonPositivePrice");
  if (
    view.conf * BPS_DENOMINATOR >
    view.price * BigInt(params.maxConfidenceBps)
  ) {
    return fail("ConfidenceTooWide");
  }

  // conservative_mantissa
  let mantissa: bigint;
  if (priceUse === "collateral") {
    mantissa = view.price - view.conf;
    if (mantissa <= 0n) return fail("NonPositivePrice");
  } else {
    mantissa = view.price + view.conf;
    if (mantissa > U64_MAX) return fail("ArithmeticOverflow");
  }
  const normalized = normalizePrice(
    mantissa,
    view.exponent,
    priceUse === "debt",
  );
  if (!normalized.ok) return fail(normalized.error);

  // Spot-versus-EMA breaker.
  if (view.emaPrice <= 0n) return fail("NonPositivePrice");
  const spot = normalizePrice(view.price, view.exponent, false);
  if (!spot.ok) return fail(spot.error);
  const ema = normalizePrice(view.emaPrice, view.exponent, false);
  if (!ema.ok) return fail(ema.error);
  const deviation = deviationWithin(
    spot.value,
    ema.value,
    params.maxDeviationBps,
  );
  if (deviation) return fail(deviation);

  return {
    ok: true,
    normalized: normalized.value,
    publishTime: view.publishTime,
  };
}

// ---- Hermes accumulator updates ----------------------------------------------

/** A price feed message decoded from a signed Hermes accumulator update. */
export interface PriceFeedMessage {
  readonly feedId: Uint8Array;
  readonly price: bigint;
  readonly conf: bigint;
  readonly exponent: number;
  readonly publishTime: bigint;
  readonly prevPublishTime: bigint;
  readonly emaPrice: bigint;
  readonly emaConf: bigint;
}

/**
 * Decodes the price feed messages inside a Hermes accumulator update
 * ("PNAU" payload, as returned by `/v2/updates/price/latest?encoding=hex|base64`).
 * This reads the messages only; it does not verify the Wormhole signatures
 * (the Pyth receiver program does that on chain).
 */
export function decodeAccumulatorUpdate(data: Uint8Array): PriceFeedMessage[] {
  const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
  const need = (at: number, n: number) => {
    if (at + n > data.length) throw new Error("truncated accumulator update");
  };
  need(0, 6);
  if (
    data[0] !== 0x50 ||
    data[1] !== 0x4e ||
    data[2] !== 0x41 ||
    data[3] !== 0x55
  ) {
    throw new Error("not a PNAU accumulator update");
  }
  let at = 4 + 2; // magic, major, minor
  need(at, 1);
  at += 1 + data[at]!; // trailing header
  need(at, 1);
  if (data[at] !== 0) throw new Error("unsupported update type");
  at += 1;
  need(at, 2);
  const vaaLen = view.getUint16(at, false);
  at += 2 + vaaLen;
  need(at, 1);
  const count = data[at]!;
  at += 1;
  const messages: PriceFeedMessage[] = [];
  for (let i = 0; i < count; i++) {
    need(at, 2);
    const size = view.getUint16(at, false);
    at += 2;
    need(at, size);
    const m = at;
    if (data[m] !== 0 || size < 85) {
      throw new Error("unsupported accumulator message type");
    }
    messages.push({
      feedId: data.slice(m + 1, m + 33),
      price: view.getBigInt64(m + 33, false),
      conf: view.getBigUint64(m + 41, false),
      exponent: view.getInt32(m + 49, false),
      publishTime: view.getBigInt64(m + 53, false),
      prevPublishTime: view.getBigInt64(m + 61, false),
      emaPrice: view.getBigInt64(m + 69, false),
      emaConf: view.getBigUint64(m + 77, false),
    });
    at += size;
    need(at, 1);
    at += 1 + data[at]! * 20; // merkle proof
  }
  return messages;
}

/** Hex (optionally 0x-prefixed) to bytes. */
export function hexToBytes(hex: string): Uint8Array {
  const clean = hex.startsWith("0x") ? hex.slice(2) : hex;
  if (clean.length % 2 !== 0 || /[^0-9a-fA-F]/.test(clean)) {
    throw new Error("invalid hex");
  }
  const out = new Uint8Array(clean.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(clean.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

/** Bytes to lowercase hex. */
export function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}
