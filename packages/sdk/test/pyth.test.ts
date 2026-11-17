// Unit tests for the Pyth update-flow selector (Task 6.4, Requirement 6.10).

import { test } from "node:test";
import assert from "node:assert/strict";
import { Keypair, PublicKey, TransactionInstruction } from "@solana/web3.js";
import {
  buildPostThenUseUpdate,
  buildSameTransactionUpdate,
  checkTransactionSize,
  selectPythUpdateFlow,
  TX_HARD_LIMIT,
  type PythUpdateFlowInput,
} from "../src/index.ts";

const BLOCKHASH = PublicKey.default.toBase58();

function ix(
  dataLen: number,
  accounts: PublicKey[] = [],
): TransactionInstruction {
  return new TransactionInstruction({
    programId: Keypair.generate().publicKey,
    keys: accounts.map((pubkey) => ({
      pubkey,
      isSigner: false,
      isWritable: false,
    })),
    data: Buffer.alloc(dataLen),
  });
}

function input(
  overrides: Partial<PythUpdateFlowInput> = {},
): PythUpdateFlowInput {
  const priceAccount = Keypair.generate().publicKey;
  return {
    payer: Keypair.generate().publicKey,
    recentBlockhash: BLOCKHASH,
    postInstructions: [ix(200, [priceAccount])],
    queueInstruction: ix(100, [priceAccount]),
    ...overrides,
  };
}

test("a small combined update selects the same-transaction flow", () => {
  const flow = selectPythUpdateFlow(input());
  assert.equal(flow.mode, "same-transaction");
  assert.equal(flow.transactions.length, 1);
  assert.equal(flow.combinedSize.ok, true);
  // The single transaction is exactly the combined builder's output.
  assert.equal(
    checkTransactionSize(flow.transactions[0]!).bytes,
    checkTransactionSize(buildSameTransactionUpdate(input())).bytes,
  );
});

test("an oversized combined update falls back to post-then-use", () => {
  // A huge post instruction pushes the combined transaction over the limit.
  const flow = selectPythUpdateFlow(input({ postInstructions: [ix(1100)] }));
  assert.equal(flow.mode, "post-then-use");
  assert.equal(flow.transactions.length, 2);
  // The combined attempt was measured and recorded as the reason.
  assert.ok(flow.combinedSize.bytes > TX_HARD_LIMIT);
  assert.ok(flow.reason.includes("hard limit"));
});

test("a combined update over the target but under the hard limit still splits", () => {
  // Choose a data size that lands between the 1,150 target and 1,232 limit.
  const probe = checkTransactionSize(
    buildSameTransactionUpdate(input({ postInstructions: [ix(0)] })),
  ).bytes;
  const dataLen = 1180 - probe; // combined ≈ 1180 bytes: over target, under limit
  const flow = selectPythUpdateFlow(input({ postInstructions: [ix(dataLen)] }));
  assert.equal(flow.combinedSize.ok, true);
  assert.equal(flow.combinedSize.overTarget, true);
  assert.equal(flow.mode, "post-then-use");
  assert.ok(flow.reason.includes("target"));
});

test("a higher maxCombinedBytes keeps the same-transaction flow", () => {
  const over = input({ postInstructions: [ix(300)] });
  const tight = selectPythUpdateFlow(over, { maxCombinedBytes: 100 });
  assert.equal(tight.mode, "post-then-use");
  const loose = selectPythUpdateFlow(over, {
    maxCombinedBytes: TX_HARD_LIMIT - 1,
  });
  assert.equal(loose.mode, "same-transaction");
});

test("post-then-use posts first, then queues with any close instructions", () => {
  const closeIx = ix(20);
  const { post, queue } = buildPostThenUseUpdate(
    input({ closeInstructions: [closeIx] }),
  );
  // Each transaction is independently within the hard limit.
  assert.equal(checkTransactionSize(post).ok, true);
  assert.equal(checkTransactionSize(queue).ok, true);
  // The queue transaction carries the queue instruction plus the close.
  assert.equal(queue.message.compiledInstructions.length, 2);
  // The post transaction carries only the post instruction.
  assert.equal(post.message.compiledInstructions.length, 1);
});
