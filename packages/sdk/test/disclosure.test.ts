import { test } from "node:test";
import assert from "node:assert/strict";
import {
  DisclosureField,
  decodeDisclosure,
  disclosureMask,
  rotateViewingKey,
  RotationFailedError,
  ViewingKeyMaterial,
} from "../src/index.ts";

test("disclosureMask combines field bits", () => {
  assert.equal(
    disclosureMask(DisclosureField.FreeSol, DisclosureField.DebtShares),
    0b10001,
  );
});

test("decodeDisclosure reads mask, nonce, ciphertexts and generation", () => {
  const data = new Uint8Array(8 + 96 + 1 + 16 + 160 + 8 + 8 + 1);
  const view = new DataView(data.buffer);
  data[104] = 0b101;
  view.setBigUint64(105, 9n, true);
  data[121] = 7;
  data[153 + 32] = 8;
  view.setBigUint64(281, 3n, true);
  view.setBigUint64(289, 11n, true);
  const d = decodeDisclosure(data);
  assert.equal(d.mask, 5);
  assert.equal(d.nonce, 9n);
  assert.equal(d.ciphertexts.length, 5);
  assert.equal(d.ciphertexts[0]?.[0], 7);
  assert.equal(d.ciphertexts[2]?.[0], 8);
  assert.equal(d.positionGeneration, 3n);
  assert.equal(d.slot, 11n);
});

test("rotateViewingKey keeps the old key when submit fails", async () => {
  let committed = false;
  const key = new ViewingKeyMaterial(new Uint8Array(32).fill(1));
  await assert.rejects(
    rotateViewingKey({
      newKey: key,
      expectedBalances: {
        freeSol: 0n,
        freeUsdc: 0n,
        collateralSol: 0n,
        supplySharesUsdc: 0n,
        debtSharesUsdc: 0n,
      },
      mxePublicKey: new Uint8Array(32),
      submit: () => Promise.reject(new Error("boom")),
      fetchState: () => Promise.reject(new Error("unused")),
      commitKey: () => {
        committed = true;
      },
    }),
    RotationFailedError,
  );
  assert.equal(committed, false);
});
