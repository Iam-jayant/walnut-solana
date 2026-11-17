// Unit tests for PDA helpers (Task 3.4).
//
// Golden vectors were computed independently with
// PublicKey.findProgramAddressSync over the raw seed bytes of the Rust scheme
// in programs/walnut/src/operation/seeds.rs and programs/walnut/src/lib.rs,
// without using the SDK helpers. Owners are 32 copies of one byte, as in the
// Rust tests (`key(n)`).

import { test } from "node:test";
import assert from "node:assert/strict";
import { PublicKey } from "@solana/web3.js";
import {
  compDefOffsetOf,
  deriveArciumQueueAccounts,
  deriveIntentPda,
  deriveIntentPdaWithBump,
  deriveOperationCounterPda,
  derivePositionPda,
  getClusterAccAddress,
  getComputationAccAddress,
  getMXEAccAddress,
  toComputationOffsetBn,
  u64ToLeBytes,
} from "../src/index.ts";

const PROGRAM = new PublicKey("5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea");
const owner = (byte: number): PublicKey =>
  new PublicKey(new Uint8Array(32).fill(byte));

const INTENT_GOLDEN: [number, bigint, string, number][] = [
  [1, 0n, "GrJXYWq17gyiF8LDBJakHU6MeqYRLihQzcd5NiBApWQa", 254],
  [1, 1n, "FvmTMaL4DdoyDV3247SEyonANq5pJz4WzZTKrWzHjJHq", 252],
  [2, 0n, "GnpaPkhkDfCWhffewYv5dfMT4y7UYnrX9oKRQAWomYz1", 253],
  [7, 0x0102030405060708n, "CeuZ7mfRRvinqgjqujXnhEpfefhC4FWtb52hANv2h2My", 255],
  [1, 256n, "5kij7NdJZz8jVx6QLC3Dce4gSAMWTKoK3QcmSXep3oNK", 253],
  [
    1,
    18446744073709551615n,
    "T7zFaUSPHkZeNNygBPqh7bARtpfFvZE2aFo9GwzgX3N",
    255,
  ],
];

test("Operation Intent PDAs match the golden vectors", () => {
  for (const [byte, sequence, address, bump] of INTENT_GOLDEN) {
    const derived = deriveIntentPdaWithBump(PROGRAM, owner(byte), sequence);
    assert.equal(derived.address.toBase58(), address);
    assert.equal(derived.bump, bump);
    assert.ok(
      deriveIntentPda(PROGRAM, owner(byte), sequence).equals(derived.address),
    );
  }
});

test("Operation Counter and Position PDAs match the golden vectors", () => {
  assert.equal(
    deriveOperationCounterPda(PROGRAM, owner(1)).toBase58(),
    "HCCY4zHMV2QgPri7P4mqZMn1QRC7dsAtSCQpoRjEG6hz",
  );
  assert.equal(
    deriveOperationCounterPda(PROGRAM, owner(2)).toBase58(),
    "FjdoG8gPbdKSbWSSMGo9zfaDLLjpDiaKWq3K3fD7bUK3",
  );
  assert.equal(
    derivePositionPda(PROGRAM, owner(1)).toBase58(),
    "HvLTu1wWDWLXK3xZ24hgwm87KhQcELmQm3rEzQjhtRE2",
  );
  assert.equal(
    derivePositionPda(PROGRAM, owner(2)).toBase58(),
    "3ZiET1KEH23ufrZQ9iYEFb5CaPyjNzNasjZNUzSvax5G",
  );
});

test("the sequence seed is a little-endian u64", () => {
  assert.deepEqual(
    [...u64ToLeBytes(0x0102030405060708n)],
    [8, 7, 6, 5, 4, 3, 2, 1],
  );
  // Same derivation by hand, with a big-endian sequence for contrast.
  const seeds = (sequence: Uint8Array): Uint8Array[] => [
    new TextEncoder().encode("operation_intent"),
    owner(7).toBytes(),
    sequence,
  ];
  const little = PublicKey.findProgramAddressSync(
    seeds(Uint8Array.of(8, 7, 6, 5, 4, 3, 2, 1)),
    PROGRAM,
  )[0];
  const big = PublicKey.findProgramAddressSync(
    seeds(Uint8Array.of(1, 2, 3, 4, 5, 6, 7, 8)),
    PROGRAM,
  )[0];
  assert.ok(
    deriveIntentPda(PROGRAM, owner(7), 0x0102030405060708n).equals(little),
  );
  assert.ok(!little.equals(big));
});

test("distinct owners, sequences, and PDA kinds give distinct addresses", () => {
  const seen = new Set<string>();
  for (const byte of [1, 2, 3]) {
    for (const sequence of [0, 1, 255, 256]) {
      seen.add(deriveIntentPda(PROGRAM, owner(byte), sequence).toBase58());
    }
    seen.add(deriveOperationCounterPda(PROGRAM, owner(byte)).toBase58());
    seen.add(derivePositionPda(PROGRAM, owner(byte)).toBase58());
  }
  assert.equal(seen.size, 3 * (4 + 2));
  // number and bigint sequences agree.
  assert.ok(
    deriveIntentPda(PROGRAM, owner(1), 5).equals(
      deriveIntentPda(PROGRAM, owner(1), 5n),
    ),
  );
});

test("sequences outside u64 are rejected", () => {
  assert.throws(() => deriveIntentPda(PROGRAM, owner(1), -1n), RangeError);
  assert.throws(
    () => deriveIntentPda(PROGRAM, owner(1), 1n << 64n),
    RangeError,
  );
  assert.throws(() => deriveIntentPda(PROGRAM, owner(1), 1.5), RangeError);
});

test("Arcium account helpers match the client derivations", () => {
  const clusterOffset = 456;
  const accounts = deriveArciumQueueAccounts({
    programId: PROGRAM,
    clusterOffset,
    computationOffset: 42n,
    circuit: "add_together",
  });
  assert.ok(
    accounts.clusterAccount.equals(getClusterAccAddress(clusterOffset)),
  );
  assert.ok(accounts.mxeAccount.equals(getMXEAccAddress(PROGRAM)));
  assert.ok(
    accounts.computationAccount.equals(
      getComputationAccAddress(clusterOffset, toComputationOffsetBn(42)),
    ),
  );
  assert.equal(
    new Set(Object.values(accounts).map((k) => k.toBase58())).size,
    6,
  );
  assert.ok(Number.isInteger(compDefOffsetOf("add_together")));
});
