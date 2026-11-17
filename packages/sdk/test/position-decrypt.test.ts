// Local decryption and wrong-key rejection of the Position payload (Task 5.6;
// Requirements 5.9, 5.10, 5.12). Pure Node, no network, no cluster: the MXE is a
// locally generated x25519 keypair and the payload is sealed with the same
// RescueCipher the program's circuits use.
import { test } from "node:test";
import assert from "node:assert/strict";
import { RescueCipher, x25519 } from "@arcium-hq/client";
import {
  createSession,
  decryptPosition,
  deriveViewingKey,
  encryptInputs,
  ViewingKeyDecryptError,
  ViewingKeySession,
  viewingKeyMessageBytes,
  nonceToU128,
  u128ToNonce,
  type EncryptedPositionState,
  type ViewingKeyMaterial,
} from "../src/index.ts";

const PROGRAM = "5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea";
const OTHER_PROGRAM = "11111111111111111111111111111112";
const POSITION = "11111111111111111111111111111111";
const OTHER_POSITION = "11111111111111111111111111111113";

/** A deterministic stand-in for a wallet: signature = SHA-512(seed | message). */
async function sign(seed: string, message: Uint8Array): Promise<Uint8Array> {
  const data = new Uint8Array([...new TextEncoder().encode(seed), ...message]);
  return new Uint8Array(await globalThis.crypto.subtle.digest("SHA-512", data));
}

async function keyFor(
  options: {
    wallet?: string;
    program?: string;
    network?: string;
    position?: string;
    epoch?: number;
  } = {},
): Promise<ViewingKeyMaterial> {
  const message = viewingKeyMessageBytes({
    programId: options.program ?? PROGRAM,
    network: options.network ?? "devnet",
    positionPda: options.position ?? POSITION,
    epoch: options.epoch ?? 0,
  });
  return deriveViewingKey(await sign(options.wallet ?? "wallet", message));
}

/** A fake MXE: the secret stays here, the public key goes to the client. */
function fakeMxe(): { secret: Uint8Array; publicKey: Uint8Array } {
  const secret = x25519.utils.randomSecretKey();
  return { secret, publicKey: x25519.getPublicKey(secret) };
}

const BALANCES = [123_456n, 7n, 0n, (1n << 100n) + 9n, 18_446_744_073_709n];

/** What the MXE does when it seals a Position payload to a Viewing public key. */
function sealFor(
  mxeSecret: Uint8Array,
  viewingPublicKey: Uint8Array,
  values: bigint[],
  nonce: Uint8Array,
): EncryptedPositionState {
  const cipher = new RescueCipher(
    x25519.getSharedSecret(mxeSecret, viewingPublicKey),
  );
  const sealed = cipher.encrypt(values, nonce);
  return {
    nonce: nonceToU128(nonce),
    ciphertexts: sealed,
  };
}

const nonceOf = (n: bigint): Uint8Array => u128ToNonce(n);

test("a payload sealed to key A decrypts under A to the exact balances", async () => {
  const mxe = fakeMxe();
  const a = await keyFor();
  const state = sealFor(mxe.secret, a.publicKey, BALANCES, nonceOf(41n));
  assert.deepEqual(
    decryptPosition({
      encryptedState: state,
      viewingSecret: a.secret,
      mxePublicKey: mxe.publicKey,
    }),
    {
      freeSol: 123_456n,
      freeUsdc: 7n,
      collateralSol: 0n,
      supplySharesUsdc: (1n << 100n) + 9n,
      debtSharesUsdc: 18_446_744_073_709n,
    },
  );
});

test("the client-side encryption path round-trips through the same decryptor", async () => {
  const mxe = fakeMxe();
  const a = await keyFor();
  const sealed = encryptInputs(
    createSession(mxe.publicKey, a.secret).cipher,
    BALANCES,
  );
  const balances = decryptPosition({
    encryptedState: {
      nonce: sealed.nonceU128,
      ciphertexts: sealed.ciphertexts,
    },
    viewingSecret: a.secret,
    mxePublicKey: mxe.publicKey,
  });
  assert.equal(balances.freeSol, BALANCES[0]);
  assert.equal(balances.debtSharesUsdc, BALANCES[4]);
});

test("a wrong key is rejected: other wallet, other position, other network, other program, other epoch", async () => {
  const mxe = fakeMxe();
  const a = await keyFor();
  const state = sealFor(mxe.secret, a.publicKey, BALANCES, nonceOf(42n));
  const wrongKeys: Array<[string, ViewingKeyMaterial]> = [
    ["a different wallet", await keyFor({ wallet: "attacker" })],
    ["a different position", await keyFor({ position: OTHER_POSITION })],
    ["a different network", await keyFor({ network: "mainnet-beta" })],
    ["a different program", await keyFor({ program: OTHER_PROGRAM })],
    ["a different epoch", await keyFor({ epoch: 1 })],
  ];
  for (const [label, wrong] of wrongKeys) {
    assert.notDeepEqual(wrong.publicKey, a.publicKey, label);
    assert.throws(
      () =>
        decryptPosition({
          encryptedState: state,
          viewingSecret: wrong.secret,
          mxePublicKey: mxe.publicKey,
        }),
      ViewingKeyDecryptError,
      label,
    );
  }
});

test("after a rotation the stale epoch key is rejected and the new key reads the same balances", async () => {
  const mxe = fakeMxe();
  const epoch0 = await keyFor({ epoch: 0 });
  const epoch1 = await keyFor({ epoch: 1 });
  const before = sealFor(mxe.secret, epoch0.publicKey, BALANCES, nonceOf(100n));
  // The rotation circuit re-seals the same five values to the new key under a
  // new nonce (the program derives it; any distinct nonce shows the same).
  const after = sealFor(mxe.secret, epoch1.publicKey, BALANCES, nonceOf(200n));
  const read = (state: EncryptedPositionState, key: ViewingKeyMaterial) =>
    decryptPosition({
      encryptedState: state,
      viewingSecret: key.secret,
      mxePublicKey: mxe.publicKey,
    });
  const expected = read(before, epoch0);
  assert.deepEqual(read(after, epoch1), expected);
  // The old key can no longer read the stored payload.
  assert.throws(() => read(after, epoch0), ViewingKeyDecryptError);
  // The new key cannot read a payload from before the rotation.
  assert.throws(() => read(before, epoch1), ViewingKeyDecryptError);
});

test("a wrong nonce or a wrong MXE key is rejected", async () => {
  const mxe = fakeMxe();
  const a = await keyFor();
  const state = sealFor(mxe.secret, a.publicKey, BALANCES, nonceOf(7n));
  const decrypt = (
    encryptedState: EncryptedPositionState,
    mxePublicKey: Uint8Array,
  ) =>
    decryptPosition({
      encryptedState,
      viewingSecret: a.secret,
      mxePublicKey,
    });
  assert.equal(decrypt(state, mxe.publicKey).freeUsdc, 7n);
  assert.throws(
    () =>
      decrypt({ ...state, nonce: (state.nonce as bigint) + 1n }, mxe.publicKey),
    ViewingKeyDecryptError,
  );
  assert.throws(
    () => decrypt(state, fakeMxe().publicKey),
    ViewingKeyDecryptError,
  );
});

test("a session unlocked from the right wallet signature reads; one from another epoch is rejected", async () => {
  const mxe = fakeMxe();
  const a = await keyFor();
  const state = sealFor(mxe.secret, a.publicKey, BALANCES, nonceOf(9n));
  const message = (epoch: number) =>
    viewingKeyMessageBytes({
      programId: PROGRAM,
      network: "devnet",
      positionPda: POSITION,
      epoch,
    });
  const right = await ViewingKeySession.unlock(
    await sign("wallet", message(0)),
  );
  assert.equal(right.decryptPosition(state, mxe.publicKey).freeSol, 123_456n);
  const stale = await ViewingKeySession.unlock(
    await sign("wallet", message(1)),
  );
  assert.throws(
    () => stale.decryptPosition(state, mxe.publicKey),
    ViewingKeyDecryptError,
  );
  right.clear();
  stale.clear();
});

test("the zero state of a new Position decrypts to zero balances", async () => {
  const mxe = fakeMxe();
  const a = await keyFor();
  const zero = sealFor(
    mxe.secret,
    a.publicKey,
    [0n, 0n, 0n, 0n, 0n],
    nonceOf(1n),
  );
  assert.deepEqual(
    decryptPosition({
      encryptedState: zero,
      viewingSecret: a.secret,
      mxePublicKey: mxe.publicKey,
    }),
    {
      freeSol: 0n,
      freeUsdc: 0n,
      collateralSol: 0n,
      supplySharesUsdc: 0n,
      debtSharesUsdc: 0n,
    },
  );
});
