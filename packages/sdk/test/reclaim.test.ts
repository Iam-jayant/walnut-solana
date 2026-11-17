// Unit tests for the rent and fee reclaim builders (Task 3.4).
//
// The provider is a fake: it points at an unreachable endpoint and never sends
// anything, because building an instruction needs only the Arcium IDL. The
// live effect of either instruction (rent returned, fee returned) needs a
// cluster and is not tested here; it is covered by tests/failure-matrix.ts
// (items 12 and 13) and the devnet runs.

import { test } from "node:test";
import assert from "node:assert/strict";
import { AnchorProvider } from "@anchor-lang/core";
import {
  getArciumProgramId,
  getClusterAccAddress,
  getComputationAccAddress,
  getExecutingPoolAccAddress,
  getFeePoolAccAddress,
  getMXEAccAddress,
} from "@arcium-hq/client";
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import {
  buildClaimComputationRent,
  buildReclaimExpiredComputationFee,
  PayerMismatchError,
  sendClaimComputationRent,
  toComputationOffsetBn,
} from "../src/index.ts";

const payer = Keypair.generate();
const stranger = Keypair.generate();
const MXE_PROGRAM = new PublicKey(
  "5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea",
);
const CLUSTER = 456;
const OFFSET = 0x0102030405060708n;

function fakeProvider(signer: Keypair): AnchorProvider {
  const wallet = {
    publicKey: signer.publicKey,
    signTransaction: () => Promise.reject(new Error("fake wallet cannot sign")),
    signAllTransactions: () =>
      Promise.reject(new Error("fake wallet cannot sign")),
  };
  return new AnchorProvider(new Connection("http://127.0.0.1:1"), wallet, {
    commitment: "confirmed",
  });
}

const discriminator = (data: Uint8Array): number[] => [...data.subarray(0, 8)];

test("buildClaimComputationRent builds the Arcium instruction", async () => {
  const ix = await buildClaimComputationRent({
    provider: fakeProvider(payer),
    signer: payer.publicKey,
    originalPayer: payer.publicKey,
    clusterOffset: CLUSTER,
    computationOffset: OFFSET,
  });
  assert.ok(ix.programId.equals(getArciumProgramId()));
  // Discriminator from the Arcium 0.16.0 IDL.
  assert.deepEqual(
    discriminator(ix.data),
    [215, 218, 1, 166, 81, 218, 16, 151],
  );
  // Args: u64 computation offset (LE), then u32 cluster offset (LE).
  assert.deepEqual([...ix.data.subarray(8, 16)], [8, 7, 6, 5, 4, 3, 2, 1]);
  assert.equal(
    new DataView(ix.data.buffer, ix.data.byteOffset).getUint32(16, true),
    CLUSTER,
  );
  assert.equal(ix.data.length, 20);
  const [signerMeta, compMeta] = ix.keys;
  assert.ok(signerMeta?.pubkey.equals(payer.publicKey));
  assert.equal(signerMeta?.isSigner, true);
  assert.equal(signerMeta?.isWritable, true);
  assert.ok(
    compMeta?.pubkey.equals(
      getComputationAccAddress(CLUSTER, toComputationOffsetBn(OFFSET)),
    ),
  );
  assert.equal(compMeta?.isWritable, true);
});

test("buildReclaimExpiredComputationFee uses the failure-matrix account wiring", async () => {
  const ix = await buildReclaimExpiredComputationFee({
    provider: fakeProvider(payer),
    signer: payer.publicKey,
    originalPayer: payer.publicKey,
    clusterOffset: CLUSTER,
    computationOffset: OFFSET,
    mxeProgramId: MXE_PROGRAM,
  });
  assert.ok(ix.programId.equals(getArciumProgramId()));
  assert.deepEqual(
    discriminator(ix.data),
    [146, 230, 72, 249, 217, 144, 70, 33],
  );
  // Args: u64 computation offset (LE), then the 32-byte MXE program id.
  assert.deepEqual([...ix.data.subarray(8, 16)], [8, 7, 6, 5, 4, 3, 2, 1]);
  assert.deepEqual([...ix.data.subarray(16, 48)], [...MXE_PROGRAM.toBytes()]);
  const keys = ix.keys.map((k) => k.pubkey.toBase58());
  for (const expected of [
    payer.publicKey,
    getMXEAccAddress(MXE_PROGRAM),
    getClusterAccAddress(CLUSTER),
    getComputationAccAddress(CLUSTER, toComputationOffsetBn(OFFSET)),
    getExecutingPoolAccAddress(CLUSTER),
    getFeePoolAccAddress(),
  ]) {
    assert.ok(keys.includes(expected.toBase58()), expected.toBase58());
  }
  const signer = ix.keys.find((k) => k.pubkey.equals(payer.publicKey));
  assert.equal(signer?.isSigner, true);
  assert.equal(signer?.isWritable, true);
});

test("a signer other than the original payer is rejected", async () => {
  const provider = fakeProvider(stranger);
  await assert.rejects(
    buildClaimComputationRent({
      provider,
      signer: stranger.publicKey,
      originalPayer: payer.publicKey,
      clusterOffset: CLUSTER,
      computationOffset: OFFSET,
    }),
    PayerMismatchError,
  );
  await assert.rejects(
    buildReclaimExpiredComputationFee({
      provider,
      signer: stranger.publicKey,
      originalPayer: payer.publicKey,
      clusterOffset: CLUSTER,
      computationOffset: OFFSET,
      mxeProgramId: MXE_PROGRAM,
    }),
    PayerMismatchError,
  );
  await assert.rejects(
    sendClaimComputationRent({
      provider,
      originalPayer: payer.publicKey,
      clusterOffset: CLUSTER,
      computationOffset: OFFSET,
    }),
    PayerMismatchError,
  );
});

test("a computation offset outside u64 is rejected", async () => {
  await assert.rejects(
    buildClaimComputationRent({
      provider: fakeProvider(payer),
      signer: payer.publicKey,
      originalPayer: payer.publicKey,
      clusterOffset: CLUSTER,
      computationOffset: 1n << 64n,
    }),
    RangeError,
  );
});
