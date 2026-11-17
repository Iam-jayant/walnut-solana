// Tests for Viewing Key derivation, local view, and custody (Task 5.5;
// Requirements 5.9, 5.10, 5.12, 5.14). Pure Node, no network.
import { test } from "node:test";
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { readFileSync } from "node:fs";
import { inspect } from "node:util";
import { PublicKey } from "@solana/web3.js";
import { x25519 } from "@arcium-hq/client";
import {
  createSession,
  decryptPosition,
  deriveViewingKey,
  encryptInputs,
  rotationArgs,
  ViewingKeyCustodyError,
  ViewingKeyDecryptError,
  ViewingKeyMaterial,
  ViewingKeySession,
  viewingKeyMessage,
  viewingKeyMessageBytes,
  WALLET_SIGNATURE_BYTES,
} from "../src/index.ts";

const PROGRAM = "5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea";
const POSITION = "11111111111111111111111111111111";
const toHex = (bytes: Uint8Array): string => Buffer.from(bytes).toString("hex");

/** A deterministic stand-in for a wallet: signature = SHA-512(seed | message). */
async function fakeWalletSign(
  seed: string,
  message: Uint8Array,
): Promise<Uint8Array> {
  const data = new Uint8Array([...new TextEncoder().encode(seed), ...message]);
  const digest = new Uint8Array(
    await globalThis.crypto.subtle.digest("SHA-512", data),
  );
  assert.equal(digest.length, WALLET_SIGNATURE_BYTES);
  return digest;
}

const messageFor = (epoch: bigint | number, network = "devnet") =>
  viewingKeyMessageBytes({
    programId: PROGRAM,
    network,
    positionPda: POSITION,
    epoch,
  });

test("message matches the golden vectors", () => {
  const base = { programId: PROGRAM, network: "devnet", positionPda: POSITION };
  assert.equal(
    viewingKeyMessage({ ...base, epoch: 0 }),
    "walnut-solana viewing key v1 | program=5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea | network=devnet | position=11111111111111111111111111111111 | epoch=0",
  );
  assert.equal(
    viewingKeyMessage({
      programId: new PublicKey(PROGRAM),
      network: "mainnet-beta",
      positionPda: new PublicKey(POSITION),
      epoch: 18446744073709551615n,
    }),
    "walnut-solana viewing key v1 | program=5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea | network=mainnet-beta | position=11111111111111111111111111111111 | epoch=18446744073709551615",
  );
  assert.equal(
    new TextDecoder().decode(viewingKeyMessageBytes({ ...base, epoch: 7n })),
    viewingKeyMessage({ ...base, epoch: 7n }),
  );
});

test("message rejects malformed fields", () => {
  const base = { programId: PROGRAM, network: "devnet", positionPda: POSITION };
  assert.throws(() => viewingKeyMessage({ ...base, epoch: -1 }), RangeError);
  assert.throws(() => viewingKeyMessage({ ...base, epoch: 1.5 }), RangeError);
  assert.throws(
    () => viewingKeyMessage({ ...base, epoch: 1n << 64n }),
    RangeError,
  );
  for (const network of ["", "Dev", "a | b", "dev net", "x".repeat(40)]) {
    assert.throws(
      () => viewingKeyMessage({ ...base, network, epoch: 0 }),
      RangeError,
    );
  }
  assert.throws(
    () => viewingKeyMessage({ ...base, programId: "not-a-key", epoch: 0 }),
    RangeError,
  );
});

test("derivation is deterministic and golden", async () => {
  const signature = Uint8Array.from({ length: 64 }, (_, i) => i);
  const a = await deriveViewingKey(signature);
  const b = await deriveViewingKey(signature);
  assert.deepEqual(a.secret, b.secret);
  assert.deepEqual(a.publicKey, b.publicKey);
  assert.equal(a.secret.length, 32);
  assert.equal(a.publicKey.length, 32);
  assert.deepEqual(a.publicKey, x25519.getPublicKey(a.secret));
  assert.equal(
    toHex(a.publicKey),
    "9059185a67964d63cdcf481b570deebcc9baac7fbf9e570a41ba8b8f8b182a7c",
  );
  await assert.rejects(deriveViewingKey(new Uint8Array(63)), RangeError);
});

test("the epoch, position, network, and program change the key", async () => {
  const keyOf = async (message: Uint8Array) =>
    toHex(
      (await deriveViewingKey(await fakeWalletSign("wallet", message)))
        .publicKey,
    );
  const k0 = await keyOf(messageFor(0));
  assert.equal(k0, await keyOf(messageFor(0)), "same inputs, same key");
  assert.notEqual(k0, await keyOf(messageFor(1)), "epoch");
  assert.notEqual(k0, await keyOf(messageFor(0, "mainnet-beta")), "network");
  assert.notEqual(
    k0,
    await keyOf(
      viewingKeyMessageBytes({
        programId: PROGRAM,
        network: "devnet",
        positionPda: PROGRAM,
        epoch: 0,
      }),
    ),
    "position",
  );
  const other = toHex(
    (await deriveViewingKey(await fakeWalletSign("other", messageFor(0))))
      .publicKey,
  );
  assert.notEqual(k0, other, "a different wallet");
});

// One encrypted Position payload under a known Viewing Key.
async function sealedPosition() {
  const mxe = x25519.getPublicKey(x25519.utils.randomSecretKey());
  const material = await deriveViewingKey(
    await fakeWalletSign("wallet", messageFor(0)),
  );
  const values = [11n, 22n, 33n, (1n << 100n) + 5n, 44n];
  const sealed = encryptInputs(
    createSession(mxe, material.secret).cipher,
    values,
  );
  return {
    mxe,
    material,
    values,
    encryptedState: {
      nonce: sealed.nonceU128,
      ciphertexts: sealed.ciphertexts,
    },
  };
}

test("local decrypt returns the five balances", async () => {
  const { mxe, material, encryptedState } = await sealedPosition();
  assert.deepEqual(
    decryptPosition({
      encryptedState,
      viewingSecret: material.secret,
      mxePublicKey: mxe,
    }),
    {
      freeSol: 11n,
      freeUsdc: 22n,
      collateralSol: 33n,
      supplySharesUsdc: (1n << 100n) + 5n,
      debtSharesUsdc: 44n,
    },
  );
});

test("a wrong Viewing Key does not decrypt", async () => {
  const { mxe, encryptedState } = await sealedPosition();
  const wrong = await deriveViewingKey(
    await fakeWalletSign("wallet", messageFor(1)),
  );
  assert.throws(
    () =>
      decryptPosition({
        encryptedState,
        viewingSecret: wrong.secret,
        mxePublicKey: mxe,
      }),
    ViewingKeyDecryptError,
  );
  assert.throws(
    () =>
      decryptPosition({
        encryptedState: { ...encryptedState, ciphertexts: [] },
        viewingSecret: wrong.secret,
        mxePublicKey: mxe,
      }),
    RangeError,
  );
});

test("a session decrypts, then clear zeroizes and blocks every use", async () => {
  const { mxe, material, encryptedState } = await sealedPosition();
  const buffer = material.secret; // the session adopts and zeroes this buffer
  assert.ok(buffer.some((byte) => byte !== 0));
  const session = new ViewingKeySession(material);
  assert.equal(session.isUnlocked, true);
  assert.equal(session.decryptPosition(encryptedState, mxe).freeUsdc, 22n);
  assert.deepEqual(session.publicKey, material.publicKey);

  session.clear();
  assert.equal(session.isUnlocked, false);
  assert.ok(
    buffer.every((byte) => byte === 0),
    "secret buffer is zeroed",
  );
  assert.throws(
    () => session.decryptPosition(encryptedState, mxe),
    ViewingKeyCustodyError,
  );
  assert.throws(() => session.publicKey, ViewingKeyCustodyError);
  assert.throws(() => session.attachPage({} as never), ViewingKeyCustodyError);
  session.clear(); // idempotent
});

test("unlock wipes the wallet signature by default", async () => {
  const signature = await fakeWalletSign("wallet", messageFor(0));
  const keep = new Uint8Array(signature);
  const kept = await ViewingKeySession.unlock(new Uint8Array(keep), {
    wipeSignature: false,
  });
  const session = await ViewingKeySession.unlock(signature);
  assert.ok(signature.every((byte) => byte === 0));
  assert.deepEqual(session.publicKey, kept.publicKey);
  session.clear();
  kept.clear();
});

test("secrets never serialize or print", async () => {
  const material = await deriveViewingKey(
    await fakeWalletSign("wallet", messageFor(0)),
  );
  const secretHex = toHex(material.secret);
  const session = new ViewingKeySession(
    new ViewingKeyMaterial(new Uint8Array(material.secret)),
  );
  for (const value of [material, session, { nested: [session] }]) {
    assert.throws(() => JSON.stringify(value), ViewingKeyCustodyError);
  }
  assert.ok(!Object.keys(material).includes("secret"));
  assert.ok(!JSON.stringify({ ...material }).includes(secretHex));
  for (const text of [
    inspect(material, { showHidden: true, depth: 5 }),
    inspect(session, { showHidden: true, depth: 5 }),
    String(material),
    String(session),
    `${session}`,
  ]) {
    assert.ok(!text.includes(secretHex));
    assert.ok(!text.includes(Buffer.from(material.secret).toString("base64")));
  }
  assert.equal(String(session), "[ViewingKeySession unlocked]");
  session.clear();
  assert.equal(String(session), "[ViewingKeySession locked]");
});

test("the source never logs, stores, or sends key material", () => {
  const source = readFileSync(
    new URL("../src/viewing-key.ts", import.meta.url),
    "utf8",
  );
  for (const forbidden of [
    "console.",
    "localStorage",
    "sessionStorage",
    "indexedDB",
    "fetch(",
    "XMLHttpRequest",
    "document.cookie",
    "sendBeacon",
  ]) {
    assert.ok(!source.includes(forbidden), `source must not use ${forbidden}`);
  }
});

test("lock, disconnect, and account change clear the session", async () => {
  const fresh = async () => {
    const material = await deriveViewingKey(
      await fakeWalletSign("wallet", messageFor(0)),
    );
    return {
      session: new ViewingKeySession(material),
      buffer: material.secret,
    };
  };

  const locked = await fresh();
  locked.session.lock();
  assert.equal(locked.session.isUnlocked, false);
  assert.ok(locked.buffer.every((byte) => byte === 0));

  const disconnected = await fresh();
  disconnected.session.onDisconnect();
  assert.equal(disconnected.session.isUnlocked, false);

  for (const event of ["disconnect", "accountChanged"]) {
    const { session, buffer } = await fresh();
    const wallet = new EventEmitter();
    session.attachWallet(wallet);
    wallet.emit("connect"); // unrelated events do nothing
    assert.equal(session.isUnlocked, true);
    wallet.emit(event);
    assert.equal(session.isUnlocked, false, event);
    assert.ok(buffer.every((byte) => byte === 0));
    assert.equal(wallet.listenerCount(event), 0, "listeners are detached");
  }

  // A removeListener-only source is supported, and detach works early.
  const { session } = await fresh();
  const wallet = new EventEmitter();
  const detach = session.attachWallet({
    on: (event, listener) => wallet.on(event, listener),
    removeListener: (event, listener) => wallet.off(event, listener),
  });
  detach();
  wallet.emit("disconnect");
  assert.equal(session.isUnlocked, true);
  session.clear();
});

test("pagehide and a hidden tab clear the session", async () => {
  const makeTarget = () => {
    const emitter = new EventEmitter();
    const document = { visibilityState: "visible" };
    return {
      emitter,
      document,
      addEventListener: (e: string, l: () => void) => emitter.on(e, l),
      removeEventListener: (e: string, l: () => void) => emitter.off(e, l),
    };
  };
  const fresh = async () =>
    new ViewingKeySession(
      await deriveViewingKey(await fakeWalletSign("wallet", messageFor(0))),
    );

  const a = await fresh();
  const pageA = makeTarget();
  a.attachPage(pageA);
  pageA.emitter.emit("visibilitychange"); // still visible
  assert.equal(a.isUnlocked, true);
  pageA.document.visibilityState = "hidden";
  pageA.emitter.emit("visibilitychange");
  assert.equal(a.isUnlocked, false);
  assert.equal(pageA.emitter.listenerCount("pagehide"), 0);

  const b = await fresh();
  const pageB = makeTarget();
  b.attachPage(pageB);
  pageB.emitter.emit("pagehide");
  assert.equal(b.isUnlocked, false);

  // No window (Node): attaching is a no-op.
  const c = await fresh();
  c.attachPage(undefined);
  assert.equal(c.isUnlocked, true);
  c.clear();
});

test("rotation arguments bind old and new epochs and keys", () => {
  const oldKey = new Uint8Array(32).fill(1);
  const newKey = new Uint8Array(32).fill(2);
  const args = rotationArgs({
    oldEpoch: 4,
    oldPublicKey: oldKey,
    newPublicKey: newKey,
  });
  assert.equal(args.expectedOldEpoch, 4n);
  assert.equal(args.expectedNewEpoch, 5n);
  assert.deepEqual(args.expectedOldPubkey, oldKey);
  assert.deepEqual(args.newViewingPubkey, newKey);
  const bad = (
    oldPublicKey: Uint8Array,
    newPublicKey: Uint8Array,
    oldEpoch = 0,
  ) =>
    assert.throws(
      () => rotationArgs({ oldEpoch, oldPublicKey, newPublicKey }),
      RangeError,
    );
  bad(oldKey, oldKey);
  bad(oldKey, new Uint8Array(32));
  bad(oldKey, new Uint8Array(31).fill(3));
  assert.throws(
    () =>
      rotationArgs({
        oldEpoch: (1n << 64n) - 1n,
        oldPublicKey: oldKey,
        newPublicKey: newKey,
      }),
    RangeError,
  );
});
