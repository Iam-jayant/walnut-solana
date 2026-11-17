// Tests for the output-nonce derivation (Task 5.4; ADR-0003 nonce safety).
//
// The vectors are asserted identically by the Rust tests in
// programs/walnut/src/state/nonce.rs, so the client and the program agree.
import { test } from "node:test";
import assert from "node:assert/strict";
import { PublicKey } from "@solana/web3.js";
import {
  deriveOutputNonce,
  deriveOutputNonceInput,
  NonceLedger,
  NonceReuseError,
  recordAttemptOutput,
} from "../src/index.ts";

const position = new PublicKey(new Uint8Array(32).fill(0x23));
const viewingKey = new Uint8Array(32).fill(0x33);

test("derivation matches the Rust vectors", async () => {
  assert.equal(
    await deriveOutputNonceInput(position, 0),
    0x763669ecad9ce051199b4389985c02ccn,
  );
  assert.equal(
    await deriveOutputNonceInput(position, 1n),
    0x5e39c77fc4f24515e0f25a9d90214f9dn,
  );
  assert.equal(
    await deriveOutputNonceInput(position.toBytes(), 7),
    0x08b16e4633c91b0d9b42baa8e2b81605n,
  );
  assert.equal(
    await deriveOutputNonce(position, 0),
    0x763669ecad9ce051199b4389985c02ccn + 1n,
  );
});

test("retries never reuse an output (key, nonce) in the ledger", async () => {
  const ledger = new NonceLedger();
  // The stored state's nonce from a previous applied attempt.
  const stored = 1_000n;
  ledger.record(viewingKey, stored);
  const seen = new Set<bigint>([stored]);
  // Many attempts, most of which "do not apply": each gets a fresh sequence.
  for (let sequence = 0; sequence < 200; sequence++) {
    const output = await recordAttemptOutput(
      ledger,
      viewingKey,
      position,
      sequence,
    );
    assert.ok(!seen.has(output), `output nonce repeated at ${sequence}`);
    seen.add(output);
    assert.notEqual(output, stored + 1n);
  }
  assert.equal(ledger.size, 1 + 200 * 2);
});

test("the legacy stored-nonce + 1 scheme repeats and the ledger catches it", () => {
  const ledger = new NonceLedger();
  ledger.recordInput(viewingKey, 1_000n);
  assert.throws(() => ledger.recordInput(viewingKey, 1_000n), NonceReuseError);
});

test("distinct positions and sequences give distinct nonces", async () => {
  const other = new PublicKey(new Uint8Array(32).fill(0x24));
  const values = new Set<bigint>();
  for (const p of [position, other]) {
    for (let sequence = 0; sequence < 50; sequence++) {
      values.add(await deriveOutputNonceInput(p, sequence));
    }
  }
  assert.equal(values.size, 100);
  await assert.rejects(() => deriveOutputNonceInput(new Uint8Array(31), 0));
});
