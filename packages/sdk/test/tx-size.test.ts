// Unit tests for the transaction-size check (Task 3.4).

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  AddressLookupTableAccount,
  ComputeBudgetProgram,
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionInstruction,
  TransactionMessage,
  VersionedTransaction,
  type AccountMeta,
} from "@solana/web3.js";
import {
  arciumCallbackStd,
  arciumQueueStd,
  OPERATIONS,
  type AccountRef,
  type Envelope,
} from "../tx-size/accounts.ts";
import {
  checkTransactionSize,
  TX_HARD_LIMIT,
  TX_TARGET,
} from "../src/index.ts";

const BLOCKHASH = PublicKey.default.toBase58();

function transfer(payer: PublicKey, to: PublicKey): TransactionInstruction {
  return SystemProgram.transfer({
    fromPubkey: payer,
    toPubkey: to,
    lamports: 1,
  });
}

function v0(
  payer: PublicKey,
  instructions: TransactionInstruction[],
  lookupTables: AddressLookupTableAccount[] = [],
): VersionedTransaction {
  return new VersionedTransaction(
    new TransactionMessage({
      payerKey: payer,
      recentBlockhash: BLOCKHASH,
      instructions,
    }).compileToV0Message(lookupTables),
  );
}

test("a small transfer matches the exact serialized length", () => {
  const payer = Keypair.generate().publicKey;
  const ix = transfer(payer, Keypair.generate().publicKey);

  const versioned = v0(payer, [ix]);
  const report = checkTransactionSize(versioned);
  assert.equal(report.bytes, versioned.serialize().length);
  assert.equal(report.hardLimit, 1232);
  assert.equal(report.target, 1150);
  assert.equal(report.ok, true);
  assert.equal(report.overTarget, false);
  // The compiled message alone reports the same length.
  assert.equal(checkTransactionSize(versioned.message).bytes, report.bytes);

  const legacy = new Transaction({
    feePayer: payer,
    recentBlockhash: BLOCKHASH,
  });
  legacy.add(ix);
  const legacyReport = checkTransactionSize(legacy);
  assert.equal(
    legacyReport.bytes,
    legacy.serialize({ requireAllSignatures: false }).length,
  );
  assert.equal(
    checkTransactionSize(legacy.compileMessage()).bytes,
    legacyReport.bytes,
  );
  assert.equal(legacyReport.ok, true);
});

test("legacy instruction data length is counted exactly, including leading zero bytes", () => {
  const payer = Keypair.generate().publicKey;
  const legacy = new Transaction({
    feePayer: payer,
    recentBlockhash: BLOCKHASH,
  });
  legacy.add(
    new TransactionInstruction({
      programId: SystemProgram.programId,
      keys: [{ pubkey: payer, isSigner: true, isWritable: true }],
      data: Buffer.concat([Buffer.alloc(5), Buffer.from([1, 2, 3])]),
    }),
  );
  assert.equal(
    checkTransactionSize(legacy).bytes,
    legacy.serialize({ requireAllSignatures: false }).length,
  );
});

test("an oversized transaction reports over-limit and does not throw", () => {
  const payer = Keypair.generate().publicKey;
  const big = new TransactionInstruction({
    programId: Keypair.generate().publicKey,
    keys: [],
    data: Buffer.alloc(1300),
  });
  const tx = v0(payer, [big]);
  assert.throws(() => tx.serialize());
  const report = checkTransactionSize(tx);
  assert.ok(report.bytes > TX_HARD_LIMIT);
  assert.equal(report.ok, false);
  assert.equal(report.overTarget, true);

  const legacy = new Transaction({
    feePayer: payer,
    recentBlockhash: BLOCKHASH,
  });
  legacy.add(big);
  const legacyReport = checkTransactionSize(legacy);
  assert.ok(legacyReport.bytes > TX_HARD_LIMIT);
  assert.equal(legacyReport.ok, false);
});

test("a transaction between the target and the hard limit is ok but over target", () => {
  const payer = Keypair.generate().publicKey;
  const mid = (size: number): TransactionInstruction =>
    new TransactionInstruction({
      programId: Keypair.generate().publicKey,
      keys: [],
      data: Buffer.alloc(size),
    });
  // Probe the fixed overhead, then add the one extra byte the data-length
  // prefix needs once the data reaches 128 bytes.
  const base = checkTransactionSize(v0(payer, [mid(0)])).bytes;
  const report = checkTransactionSize(v0(payer, [mid(1200 - base - 1)]));
  assert.equal(report.bytes, 1200);
  assert.equal(report.ok, true);
  assert.equal(report.overTarget, true);
  assert.ok(report.bytes > TX_TARGET && report.bytes < TX_HARD_LIMIT);
});

// ---- Task 2.5 envelopes (tx-size/accounts.ts) ------------------------------

const keys = new Map<string, PublicKey>();
function keyFor(name: string): PublicKey {
  if (name === "compute-budget") return ComputeBudgetProgram.programId;
  if (name === "system-program") return SystemProgram.programId;
  let key = keys.get(name);
  if (!key) {
    const bytes = new Uint8Array(32);
    new DataView(bytes.buffer).setUint32(28, keys.size + 1);
    key = new PublicKey(bytes);
    keys.set(name, key);
  }
  return key;
}

function meta(ref: AccountRef): AccountMeta {
  return {
    pubkey: keyFor(ref.name),
    isSigner: ref.signer,
    isWritable: ref.writable,
  };
}

/** Assembles an envelope like tx-size/measure.ts, without a lookup table. */
function envelopeSize(payerName: string, std: AccountRef[], env: Envelope) {
  const payerRef: AccountRef = {
    name: payerName,
    signer: true,
    writable: true,
  };
  const main = new TransactionInstruction({
    programId: keyFor("walnut-program"),
    keys: [payerRef, ...std, ...env.accounts].map(meta),
    data: Buffer.alloc(env.dataLen),
  });
  const extras = env.extraIxs.map(
    (e) =>
      new TransactionInstruction({
        programId: keyFor(e.programName),
        keys: e.accounts.map(meta),
        data: Buffer.alloc(e.dataLen),
      }),
  );
  return checkTransactionSize(v0(keyFor(payerName), [main, ...extras]));
}

test("Position initialization queue envelope is within the target without a lookup table", () => {
  const op = OPERATIONS.find((o) => o.key === "position-init");
  assert.ok(op);
  const report = envelopeSize("queue-payer", arciumQueueStd(), op.queue);
  assert.equal(report.ok, true);
  assert.equal(report.overTarget, false);
});

test("liquidation callback envelope needs a lookup table to meet the target", () => {
  const op = OPERATIONS.find((o) => o.key === "liquidation");
  assert.ok(op);
  const report = envelopeSize(
    "callback-node",
    arciumCallbackStd(),
    op.callback,
  );
  assert.equal(report.overTarget, true);
});
