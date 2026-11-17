// Unit tests for the Pyth PriceUpdateV2 layout and the TypeScript mirror of the
// Rust oracle validation (Task 6.5, Requirement 6). Golden vectors here match
// programs/walnut/src/oracle/mod.rs tests value for value.

import { test } from "node:test";
import assert from "node:assert/strict";
import { Keypair, PublicKey } from "@solana/web3.js";
import {
  decodeAccumulatorUpdate,
  deviationWithin,
  encodePriceUpdateV2,
  hexToBytes,
  LAUNCH_ORACLE_PARAMS,
  MAX_NORMALIZED_PRICE,
  normalizePrice,
  parsePriceUpdateV2,
  PRICE_UPDATE_V2_LEN,
  PYTH_FEED_IDS,
  PYTH_RECEIVER_PROGRAM_ID,
  toPriceView,
  validatePrice,
  type PriceUpdateV2,
  type PythPriceView,
} from "../src/index.ts";

/** Golden `PriceUpdateV2` bytes, shared with tests/oracle-fixture.ts. */
export const GOLDEN_PRICE_UPDATE_HEX =
  "22f123639d7ef4cd0707070707070707070707070707070707070707070707070707070707070707" +
  "01" +
  "1111111111111111111111111111111111111111111111111111111111111111" +
  "00d6117e0300000020a1070000000000f8ffffffe803000000000000e703000000000000" +
  "00d6117e0300000020a10700000000002a0000000000000000";

const SOL_FEED = new Uint8Array(32).fill(0x11);
const USDC_FEED = new Uint8Array(32).fill(0x22);
const NOW = 1_000_030n;

const sol: PythPriceView = {
  owner: PYTH_RECEIVER_PROGRAM_ID,
  feedId: SOL_FEED,
  price: 150_00000000n,
  conf: 5_00000n,
  exponent: -8,
  publishTime: 1_000n,
  emaPrice: 150_00000000n,
  emaConf: 5_00000n,
};
const tenDollars: PythPriceView = {
  ...sol,
  price: 10_00000000n,
  conf: 1n,
  emaPrice: 10_00000000n,
  publishTime: NOW - 1n,
};

function run(view: PythPriceView, use: "collateral" | "debt" = "collateral") {
  return validatePrice(view, SOL_FEED, LAUNCH_ORACLE_PARAMS, NOW, use);
}

function errorOf(
  view: PythPriceView,
  use: "collateral" | "debt" = "collateral",
) {
  const result = run(view, use);
  assert.equal(result.ok, false);
  return result.ok ? undefined : result.error;
}

const golden: PriceUpdateV2 = {
  writeAuthority: new PublicKey(new Uint8Array(32).fill(7)),
  verificationLevel: "full",
  feedId: SOL_FEED,
  price: 15000000000n,
  conf: 500000n,
  exponent: -8,
  publishTime: 1000n,
  prevPublishTime: 999n,
  emaPrice: 15000000000n,
  emaConf: 500000n,
  postedSlot: 42n,
};

test("PriceUpdateV2 encodes to the golden bytes and round-trips", () => {
  const bytes = encodePriceUpdateV2(golden);
  assert.equal(bytes.length, PRICE_UPDATE_V2_LEN);
  assert.equal(Buffer.from(bytes).toString("hex"), GOLDEN_PRICE_UPDATE_HEX);
  const parsed = parsePriceUpdateV2(bytes);
  assert.deepEqual(parsed.price, golden.price);
  assert.equal(parsed.exponent, -8);
  assert.equal(parsed.verificationLevel, "full");
  assert.equal(parsed.postedSlot, 42n);
  assert.ok(parsed.writeAuthority.equals(golden.writeAuthority));
});

test("a partial verification level round-trips and shifts the body by one byte", () => {
  const bytes = encodePriceUpdateV2({ ...golden, verificationLevel: 3 });
  const parsed = parsePriceUpdateV2(bytes);
  assert.equal(parsed.verificationLevel, 3);
  assert.equal(parsed.price, golden.price);
  assert.equal(parsed.postedSlot, 42n);
});

test("parsing rejects a short account and a bad discriminator", () => {
  const bytes = encodePriceUpdateV2(golden);
  assert.throws(() => parsePriceUpdateV2(bytes.slice(0, 100)), /expected 134/);
  const bad = bytes.slice();
  bad[0] = 0;
  assert.throws(() => parsePriceUpdateV2(bad), /discriminator/);
});

test("golden: SOL collateral 149.995 and USDC debt 1.0001", () => {
  const solResult = run({ ...sol, publishTime: NOW - 30n }, "collateral");
  assert.ok(solResult.ok);
  assert.equal(solResult.normalized, 149_99500000n);
  const usdc = validatePrice(
    {
      ...sol,
      feedId: USDC_FEED,
      price: 1_00000000n,
      conf: 10_000n,
      emaPrice: 1_00000000n,
    },
    USDC_FEED,
    LAUNCH_ORACLE_PARAMS,
    1_030n,
    "debt",
  );
  assert.ok(usdc.ok);
  assert.equal(usdc.normalized, 1_00010000n);
});

test("golden: collateral rounds down (99) and debt rounds up (101)", () => {
  const view = {
    ...tenDollars,
    price: 1_000n,
    conf: 1n,
    exponent: -9,
    emaPrice: 1_000n,
  };
  const low = run(view, "collateral");
  const high = run(view, "debt");
  assert.ok(low.ok && high.ok);
  assert.equal(low.normalized, 99n);
  assert.equal(high.normalized, 101n);
});

test("wrong owner and wrong feed id are rejected", () => {
  assert.equal(
    errorOf({ ...tenDollars, owner: Keypair.generate().publicKey }),
    "WrongReceiverProgram",
  );
  const feed = SOL_FEED.slice();
  feed[31] = 0x80;
  assert.equal(errorOf({ ...tenDollars, feedId: feed }), "FeedMismatch");
});

test("age boundary: max passes, max+1 and future fail", () => {
  assert.ok(run({ ...tenDollars, publishTime: NOW - 60n }).ok);
  assert.equal(
    errorOf({ ...tenDollars, publishTime: NOW - 61n }),
    "StalePrice",
  );
  assert.ok(run({ ...tenDollars, publishTime: NOW }).ok);
  assert.equal(errorOf({ ...tenDollars, publishTime: NOW + 1n }), "StalePrice");
  assert.equal(
    errorOf({ ...tenDollars, publishTime: -1n }),
    "NegativeTimestamp",
  );
});

test("zero and negative prices are rejected", () => {
  for (const price of [0n, -1n, -(1n << 63n)]) {
    assert.equal(
      errorOf({ ...tenDollars, price }),
      "NonPositivePrice",
      String(price),
    );
  }
  assert.equal(errorOf({ ...tenDollars, emaPrice: 0n }), "NonPositivePrice");
});

test("confidence boundary is exact at 200 bps", () => {
  assert.ok(run({ ...tenDollars, conf: 20_000_000n }).ok);
  assert.equal(
    errorOf({ ...tenDollars, conf: 20_000_001n }),
    "ConfidenceTooWide",
  );
});

test("exponent outside -12..12 is rejected", () => {
  for (const exponent of [-13, 13, -2147483648, 2147483647]) {
    assert.equal(
      errorOf({ ...tenDollars, exponent }),
      "ExponentOutOfRange",
      String(exponent),
    );
  }
});

test("normalization overflow and domain edges", () => {
  const u64Max = (1n << 64n) - 1n;
  const overflow = normalizePrice(u64Max, 12, false);
  assert.deepEqual(overflow, { ok: false, error: "ArithmeticOverflow" });
  assert.deepEqual(normalizePrice(u64Max, 0, true), {
    ok: false,
    error: "PriceOutOfRange",
  });
  assert.deepEqual(normalizePrice(MAX_NORMALIZED_PRICE, -8, false), {
    ok: true,
    value: MAX_NORMALIZED_PRICE,
  });
  assert.deepEqual(normalizePrice(MAX_NORMALIZED_PRICE + 1n, -8, false), {
    ok: false,
    error: "PriceOutOfRange",
  });
  assert.deepEqual(normalizePrice(1n, -9, false), {
    ok: false,
    error: "PriceOutOfRange",
  });
  assert.deepEqual(normalizePrice(1n, -9, true), { ok: true, value: 1n });
  assert.deepEqual(normalizePrice(1n, 12, false), {
    ok: false,
    error: "PriceOutOfRange",
  });
});

test("price plus confidence beyond u64 never reaches normalization", () => {
  const view = {
    ...tenDollars,
    price: (1n << 63n) - 1n,
    conf: (1n << 63n) + 1n,
  };
  // The confidence band fires first through the pipeline; the overflow is
  // reached with the widest allowed band disabled by a huge price/conf pair.
  assert.equal(errorOf(view, "debt"), "ConfidenceTooWide");
});

test("spot-versus-EMA deviation boundary is exact in both directions", () => {
  const at = (price: bigint) => run({ ...tenDollars, price });
  assert.ok(at(10_50000000n).ok);
  assert.ok(!at(10_50000001n).ok);
  assert.ok(at(9_50000000n).ok);
  assert.ok(!at(9_49999999n).ok);
  assert.equal(deviationWithin(10_500n, 10_000n, 500), null);
  assert.equal(deviationWithin(10_501n, 10_000n, 500), "DeviationTooHigh");
  assert.equal(deviationWithin(1n, 0n, 500), "NonPositivePrice");
});

test("invalid parameters are rejected", () => {
  const bad = { ...LAUNCH_ORACLE_PARAMS, maxAgeSeconds: 4 };
  const result = validatePrice(tenDollars, SOL_FEED, bad, NOW, "collateral");
  assert.deepEqual(result, { ok: false, error: "InvalidOracleParams" });
});

test("Hermes accumulator decode extracts the price message", () => {
  // Build a minimal PNAU payload: header, empty VAA, one message, no proof.
  const msg = Buffer.alloc(85);
  msg[0] = 0;
  Buffer.from(SOL_FEED).copy(msg, 1);
  msg.writeBigInt64BE(15000000000n, 33);
  msg.writeBigUInt64BE(500000n, 41);
  msg.writeInt32BE(-8, 49);
  msg.writeBigInt64BE(1000n, 53);
  msg.writeBigInt64BE(999n, 61);
  msg.writeBigInt64BE(15000000000n, 69);
  msg.writeBigUInt64BE(500000n, 77);
  const size = Buffer.alloc(2);
  size.writeUInt16BE(85);
  const vaaLen = Buffer.alloc(2);
  vaaLen.writeUInt16BE(3);
  const payload = Buffer.concat([
    Buffer.from("PNAU"),
    Buffer.from([1, 0, 0, 0]), // major, minor, trailing size 0, type 0
    vaaLen,
    Buffer.from([9, 9, 9]),
    Buffer.from([1]),
    size,
    msg,
    Buffer.from([0]),
  ]);
  const [decoded] = decodeAccumulatorUpdate(payload);
  assert.ok(decoded);
  assert.equal(decoded.price, 15000000000n);
  assert.equal(decoded.exponent, -8);
  assert.equal(decoded.publishTime, 1000n);
  assert.deepEqual(Array.from(decoded.feedId), Array.from(SOL_FEED));
  assert.throws(() => decodeAccumulatorUpdate(payload.slice(0, 20)));
  assert.throws(() => decodeAccumulatorUpdate(Buffer.from("XXXXXXXXXX")));
});

test("feed ids are 32 bytes and a parsed account validates end to end", () => {
  for (const id of Object.values(PYTH_FEED_IDS)) {
    assert.equal(hexToBytes(id).length, 32);
  }
  const bytes = encodePriceUpdateV2({
    ...golden,
    feedId: hexToBytes(PYTH_FEED_IDS["SOL/USD"]),
    publishTime: NOW - 5n,
  });
  const view = toPriceView(parsePriceUpdateV2(bytes), PYTH_RECEIVER_PROGRAM_ID);
  const result = validatePrice(
    view,
    hexToBytes(PYTH_FEED_IDS["SOL/USD"]),
    LAUNCH_ORACLE_PARAMS,
    NOW,
    "collateral",
  );
  assert.ok(result.ok);
  assert.equal(result.normalized, 149_99500000n);
});
