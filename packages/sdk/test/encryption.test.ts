// Unit and property-style tests for the encryption helpers (Task 3.4).
//
// The property-style tests use a fixed-seed generator, so they are
// deterministic and need no extra dependency.

import { test } from "node:test";
import assert from "node:assert/strict";
import { x25519 } from "@arcium-hq/client";
import {
  createSession,
  decryptOutput,
  encryptInputs,
  freshNonce,
  NONCE_BYTES,
  NonceLedger,
  NonceReuseError,
  nonceToU128,
  u128ToNonce,
} from "../src/index.ts";

/** mulberry32: a tiny deterministic PRNG. */
function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function randomBig(next: () => number, bits: number): bigint {
  let n = 0n;
  for (let i = 0; i < bits; i += 16) {
    n = (n << 16n) | BigInt(Math.floor(next() * 65536));
  }
  return n & ((1n << BigInt(bits)) - 1n);
}

const mxeKey = x25519.getPublicKey(
  Uint8Array.from({ length: 32 }, (_, i) => i + 1),
);

test("encrypt then decrypt round-trips random u64 and u128 vectors", () => {
  const next = rng(0xc0ffee);
  for (let round = 0; round < 25; round += 1) {
    const session = createSession(mxeKey);
    const count = 1 + Math.floor(next() * 5);
    const values = Array.from({ length: count }, (_, i) =>
      randomBig(next, i % 2 === 0 ? 64 : 128),
    );
    const sealed = encryptInputs(session.cipher, values);
    assert.equal(sealed.ciphertexts.length, count);
    assert.ok(sealed.ciphertexts.every((c) => c.length === 32));
    assert.deepEqual(
      decryptOutput(session.cipher, sealed.ciphertexts, sealed.nonce),
      values,
    );
    // The u128 form of the nonce decrypts identically.
    assert.deepEqual(
      decryptOutput(session.cipher, sealed.ciphertexts, sealed.nonceU128),
      values,
    );
  }
});

test("a fixed viewing key and nonce give a fixed ciphertext; another key cannot decrypt", () => {
  const secret = Uint8Array.from({ length: 32 }, (_, i) => 200 - i);
  const a = createSession(mxeKey, secret);
  const b = createSession(mxeKey, secret);
  const other = createSession(mxeKey);
  const nonce = u128ToNonce(12345n);
  assert.deepEqual(a.publicKey, b.publicKey);
  const sealed = encryptInputs(a.cipher, [7n, 35n], nonce);
  assert.deepEqual(
    sealed.ciphertexts,
    encryptInputs(b.cipher, [7n, 35n], nonce).ciphertexts,
  );
  assert.notDeepEqual(decryptOutput(other.cipher, sealed.ciphertexts, nonce), [
    7n,
    35n,
  ]);
});

test("nonces are fresh 16-byte values", () => {
  const seen = new Set<string>();
  for (let i = 0; i < 200; i += 1) {
    const nonce = freshNonce();
    assert.equal(nonce.length, NONCE_BYTES);
    seen.add(Buffer.from(nonce).toString("hex"));
  }
  assert.equal(seen.size, 200);
  const session = createSession(mxeKey);
  assert.throws(
    () => encryptInputs(session.cipher, [1n], new Uint8Array(12)),
    RangeError,
  );
  assert.throws(() => createSession(new Uint8Array(31)), RangeError);
});

test("u128 nonce conversion is little-endian and round-trips", () => {
  const next = rng(7);
  assert.deepEqual(
    [...u128ToNonce(0x0102n)],
    [2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  );
  for (let i = 0; i < 100; i += 1) {
    const value = randomBig(next, 128);
    assert.equal(nonceToU128(u128ToNonce(value)), value);
  }
  assert.equal(nonceToU128(u128ToNonce((1n << 128n) - 1n)), (1n << 128n) - 1n);
  assert.throws(() => u128ToNonce(1n << 128n), RangeError);
  assert.throws(() => u128ToNonce(-1n), RangeError);
  assert.throws(() => nonceToU128(new Uint8Array(15)), RangeError);
});

test("NonceLedger throws on any repeated (key, nonce) pair", () => {
  const next = rng(99);
  const ledger = new NonceLedger();
  const keyA = new Uint8Array(32).fill(1);
  const keyB = new Uint8Array(32).fill(2);
  const nonces = Array.from({ length: 50 }, () => randomBig(next, 128));
  for (const n of nonces) {
    ledger.record(keyA, n);
    // The same nonce under another key is a different effective pair.
    ledger.record(keyB, n);
  }
  assert.equal(ledger.size, 100);
  for (const n of nonces) {
    assert.throws(() => ledger.record(keyA, n), NonceReuseError);
    assert.throws(() => ledger.record(keyB, u128ToNonce(n)), NonceReuseError);
  }
  assert.equal(ledger.size, 100);
});

test("recordInput also reserves the output nonce (input + 1)", () => {
  const ledger = new NonceLedger();
  const key = new Uint8Array(32).fill(3);
  ledger.recordInput(key, 10n);
  assert.ok(ledger.has(key, 10n));
  assert.ok(ledger.has(key, 11n));
  // Sequential nonces under one key collide with a previous output nonce.
  assert.throws(() => ledger.recordInput(key, 11n), NonceReuseError);
  // The failed call recorded nothing: 12 is still free.
  assert.ok(!ledger.has(key, 12n));
  ledger.recordInput(key, 20n);
  // Wraps at 2^128 without throwing a range error.
  ledger.recordInput(key, (1n << 128n) - 1n);
  assert.ok(ledger.has(key, 0n));
});

test("encryptInputs with a ledger rejects a repeated nonce before encrypting", () => {
  const session = createSession(mxeKey);
  const ledger = new NonceLedger();
  const track = { ledger, publicKey: session.publicKey };
  const nonce = u128ToNonce(555n);
  encryptInputs(session.cipher, [1n], nonce, track);
  assert.throws(
    () => encryptInputs(session.cipher, [2n], nonce, track),
    NonceReuseError,
  );
  // Fresh random nonces never collide.
  for (let i = 0; i < 20; i += 1)
    encryptInputs(session.cipher, [BigInt(i)], undefined, track);
  assert.equal(ledger.size, 2 + 40);
});
