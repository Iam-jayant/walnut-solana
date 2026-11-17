// Tests for the wSOL unwrap builder (Task 5.4, Requirement 5.8).
import { test } from "node:test";
import assert from "node:assert/strict";
import { Keypair } from "@solana/web3.js";
import {
  buildUnwrapWsolInstruction,
  buildUnwrapWsolTransaction,
  SPL_TOKEN_PROGRAM_ID,
} from "../src/index.ts";

test("unwrap is an owner-signed SPL CloseAccount to the owner", () => {
  const owner = Keypair.generate().publicKey;
  const wsol = Keypair.generate().publicKey;
  const ix = buildUnwrapWsolInstruction(owner, wsol);
  assert.ok(ix.programId.equals(SPL_TOKEN_PROGRAM_ID));
  assert.deepEqual([...ix.data], [9]);
  assert.equal(ix.keys.length, 3);
  assert.ok(ix.keys[0]?.pubkey.equals(wsol));
  assert.ok(ix.keys[0]?.isWritable && !ix.keys[0].isSigner);
  // The lamports go to the owner, who is the authority and the only signer.
  assert.ok(ix.keys[1]?.pubkey.equals(owner) && ix.keys[1].isWritable);
  assert.ok(ix.keys[2]?.pubkey.equals(owner) && ix.keys[2].isSigner);
});

test("unwrap transaction is standalone: one instruction, owner pays", () => {
  const owner = Keypair.generate();
  const wsol = Keypair.generate().publicKey;
  const tx = buildUnwrapWsolTransaction(owner.publicKey, wsol);
  assert.ok(tx.feePayer?.equals(owner.publicKey));
  assert.equal(tx.instructions.length, 1);
  assert.ok(tx.instructions[0]?.programId.equals(SPL_TOKEN_PROGRAM_ID));
  tx.recentBlockhash = "11111111111111111111111111111111";
  tx.sign(owner);
  assert.equal(tx.signatures.length, 1);
  assert.ok(tx.verifySignatures());
});
