/**
 * Task 2.3 Arcium feasibility gate: nonce and ownership safety.
 *
 * Builds on the Task 2.2 persistent-Position spike and proves the two safety
 * properties the design's nonce-safety invariant depends on:
 *
 *   1. Two-owner output (Requirement 2.3): one `liquidation_receipts`
 *      computation seals an independent encrypted Receipt to a borrower key and
 *      a liquidator key. Each owner decrypts only the Receipt sealed to their
 *      own Viewing Key; the other owner's ciphertext does not recover under the
 *      wrong key. This is the "liquidation writes two owners" case.
 *
 *   2. Viewing Key rotation (Requirement 5.11): a Position stored under an old
 *      key is re-encrypted to a new key by `rotate_viewing_key`. The callback
 *      swaps the stored public key and increments `viewing_key_epoch` (from 0
 *      to 1) atomically. The stored payload then decrypts under the NEW key and
 *      not under the OLD one.
 *
 * Across the whole run every effective encryption-key identity (the x25519
 * public key actually used for each input and each output) is paired with the
 * nonce used, and `recordPair` fails loudly if any (key, nonce) pair repeats
 * (Requirement 2.4). Distinct owners always have distinct effective keys, so
 * their pairs never collide even when nonces coincide; within one key the
 * output nonce is the input nonce + 1, so fresh random nonces keep every pair
 * unique.
 *
 * This mirrors tests/walnut.ts and tests/position-state.ts conventions: x25519
 * key exchange with the MXE, RescueCipher encrypt/decrypt, the typed
 * `awaitEvent` helper, queue + awaitComputationFinalization, and local
 * decryption. No financial logic is exercised.
 */
import * as anchor from "@anchor-lang/core";
import { Program } from "@anchor-lang/core";
import { PublicKey } from "@solana/web3.js";
import { Walnut } from "../target/types/walnut";
import { randomBytes } from "crypto";
import {
  awaitComputationFinalization,
  getArciumEnv,
  getCompDefAccOffset,
  getArciumAccountBaseSeed,
  getArciumProgramId,
  uploadCircuit,
  RescueCipher,
  deserializeLE,
  getMXEAccAddress,
  getMempoolAccAddress,
  getCompDefAccAddress,
  getExecutingPoolAccAddress,
  x25519,
  getComputationAccAddress,
  getMXEPublicKey,
  getClusterAccAddress,
  getLookupTableAddress,
  getArciumProgram,
} from "@arcium-hq/client";
import * as fs from "fs";
import * as os from "os";
import { expect } from "chai";

const POSITION_SEED = Buffer.from("spike_position");

interface Owner {
  publicKey: Uint8Array;
  cipher: RescueCipher;
}

describe("Walnut Task 2.3: nonce and ownership safety", () => {
  anchor.setProvider(anchor.AnchorProvider.env());
  const program = anchor.workspace.Walnut as Program<Walnut>;
  const provider = anchor.getProvider() as anchor.AnchorProvider;

  type Event = anchor.IdlEvents<(typeof program)["idl"]>;
  const awaitEvent = async <E extends keyof Event>(
    eventName: E,
    timeoutMs = 120000,
  ): Promise<Event[E]> => {
    let listenerId: number;
    let timeoutId: NodeJS.Timeout;
    const event = await new Promise<Event[E]>((res, rej) => {
      listenerId = program.addEventListener(eventName, (ev) => {
        clearTimeout(timeoutId);
        res(ev);
      });
      timeoutId = setTimeout(() => {
        program.removeEventListener(listenerId);
        rej(
          new Error(
            `Event ${String(eventName)} timed out after ${timeoutMs}ms`,
          ),
        );
      }, timeoutMs);
    });
    await program.removeEventListener(listenerId);
    return event;
  };

  const arciumEnv = getArciumEnv();
  const clusterAccount = getClusterAccAddress(arciumEnv.arciumClusterOffset);

  // Every effective (encryption key, nonce) identity used anywhere in this run.
  // A fresh encryption (client input or MXE output) must never reuse a pair.
  const usedPairs = new Set<string>();
  const recordPair = (key: Uint8Array, nonce: Uint8Array) => {
    const id = `${Buffer.from(key).toString("hex")}:${Buffer.from(
      nonce,
    ).toString("hex")}`;
    expect(
      usedPairs.has(id),
      `effective (key, nonce) pair repeated: ${id}`,
    ).to.equal(false);
    usedPairs.add(id);
  };

  let mxePublicKey: Uint8Array;

  before(async () => {
    const payer = readKpJson(`${os.homedir()}/.config/solana/id.json`);
    mxePublicKey = await getMXEPublicKeyWithRetry(provider, program.programId);

    // Register the circuits used by this test. init_position is shared with the
    // Task 2.2 test, so tolerate it already being initialized.
    await initCompDef(
      payer,
      "liquidation_receipts",
      program.methods.initLiquidationReceiptsCompDef(),
    );
    await initCompDef(
      payer,
      "rotate_viewing_key",
      program.methods.initRotateViewingKeyCompDef(),
    );
    await initCompDef(
      payer,
      "init_position",
      program.methods.initPositionCompDef(),
    );
  });

  it("outputs encrypted results to two owners; each decrypts only its own", async () => {
    const payer = readKpJson(`${os.homedir()}/.config/solana/id.json`);

    // Two distinct owners produce two distinct effective keys.
    const borrower = makeOwner(mxePublicKey);
    const liquidator = makeOwner(mxePublicKey);

    // ReceiptInput = { amount, aux }. Opaque markers, not financial amounts.
    const borrowerInput = [1_234_000_000n, 9_000n];
    const liquidatorInput = [0n, 500_000n];

    const borrowerNonce = randomBytes(16);
    const liquidatorNonce = randomBytes(16);
    recordPair(borrower.publicKey, borrowerNonce);
    recordPair(liquidator.publicKey, liquidatorNonce);

    const borrowerCt = borrower.cipher.encrypt(borrowerInput, borrowerNonce);
    const liquidatorCt = liquidator.cipher.encrypt(
      liquidatorInput,
      liquidatorNonce,
    );

    const eventPromise = awaitEvent("liquidationReceiptsEmitted");
    const computationOffset = new anchor.BN(randomBytes(8), "hex");

    await program.methods
      .liquidationReceipts(
        computationOffset,
        borrowerCt.map((c) => Array.from(c)),
        Array.from(borrower.publicKey),
        new anchor.BN(deserializeLE(borrowerNonce).toString()),
        liquidatorCt.map((c) => Array.from(c)),
        Array.from(liquidator.publicKey),
        new anchor.BN(deserializeLE(liquidatorNonce).toString()),
      )
      .accountsPartial({
        payer: payer.publicKey,
        computationAccount: getComputationAccAddress(
          arciumEnv.arciumClusterOffset,
          computationOffset,
        ),
        clusterAccount,
        mxeAccount: getMXEAccAddress(program.programId),
        mempoolAccount: getMempoolAccAddress(arciumEnv.arciumClusterOffset),
        executingPool: getExecutingPoolAccAddress(
          arciumEnv.arciumClusterOffset,
        ),
        compDefAccount: getCompDefAccAddress(
          program.programId,
          Buffer.from(
            getCompDefAccOffset("liquidation_receipts"),
          ).readUInt32LE(),
        ),
      })
      .signers([payer])
      .rpc({ skipPreflight: true, commitment: "confirmed" });

    await awaitComputationFinalization(
      provider,
      computationOffset,
      program.programId,
      "confirmed",
    );

    const ev = await eventPromise;

    const borrowerOutKey = Uint8Array.from(ev.borrowerKey);
    const borrowerOutNonce = new Uint8Array(ev.borrowerNonce);
    const liquidatorOutKey = Uint8Array.from(ev.liquidatorKey);
    const liquidatorOutNonce = new Uint8Array(ev.liquidatorNonce);

    // The effective output key for each owner is that owner's Viewing key.
    expect(Buffer.from(borrowerOutKey)).to.deep.equal(
      Buffer.from(borrower.publicKey),
    );
    expect(Buffer.from(liquidatorOutKey)).to.deep.equal(
      Buffer.from(liquidator.publicKey),
    );

    recordPair(borrowerOutKey, borrowerOutNonce);
    recordPair(liquidatorOutKey, liquidatorOutNonce);

    const borrowerCtOut = ev.borrowerCiphertexts.map((c) => Array.from(c));
    const liquidatorCtOut = ev.liquidatorCiphertexts.map((c) => Array.from(c));

    // Expected plaintexts mirror the circuit: transfer = borrower.amount,
    // credited to the liquidator's remaining.
    const transfer = borrowerInput[0];
    const expectedBorrower = [transfer, borrowerInput[1]];
    const expectedLiquidator = [transfer, liquidatorInput[1] + transfer];

    const borrowerDecrypted = borrower.cipher.decrypt(
      borrowerCtOut,
      borrowerOutNonce,
    );
    const liquidatorDecrypted = liquidator.cipher.decrypt(
      liquidatorCtOut,
      liquidatorOutNonce,
    );
    expect(borrowerDecrypted).to.deep.equal(expectedBorrower);
    expect(liquidatorDecrypted).to.deep.equal(expectedLiquidator);

    // Ownership isolation: the borrower's key must not recover the liquidator's
    // Receipt. Decrypting under the wrong key yields garbage, never the plain.
    const crossAttempt = borrower.cipher.decrypt(
      liquidatorCtOut,
      liquidatorOutNonce,
    );
    expect(crossAttempt).to.not.deep.equal(expectedLiquidator);
  });

  it("rotates the Viewing Key: state decrypts under the new key, not the old", async () => {
    // A fresh owner avoids colliding with the Task 2.2 Position PDA.
    const owner = anchor.web3.Keypair.generate();
    const airdrop = await provider.connection.requestAirdrop(
      owner.publicKey,
      2 * anchor.web3.LAMPORTS_PER_SOL,
    );
    await provider.connection.confirmTransaction(airdrop, "confirmed");

    const oldKey = makeOwner(mxePublicKey);
    const positionPda = PublicKey.findProgramAddressSync(
      [POSITION_SEED, owner.publicKey.toBuffer()],
      program.programId,
    )[0];

    // ---- store an initial payload under the OLD Viewing Key (epoch 0) -----
    const payload = [
      3_000_000_000n,
      7_500_000n,
      11_000_000_000n,
      123_456_000_000_000_000_000n,
      0n,
    ];
    const initNonce = randomBytes(16);
    recordPair(oldKey.publicKey, initNonce);
    const initCt = oldKey.cipher.encrypt(payload, initNonce);
    const initOffset = new anchor.BN(randomBytes(8), "hex");

    await program.methods
      .initPosition(
        initOffset,
        initCt.map((c) => Array.from(c)),
        Array.from(oldKey.publicKey),
        new anchor.BN(deserializeLE(initNonce).toString()),
      )
      .accountsPartial({
        payer: owner.publicKey,
        computationAccount: getComputationAccAddress(
          arciumEnv.arciumClusterOffset,
          initOffset,
        ),
        clusterAccount,
        mxeAccount: getMXEAccAddress(program.programId),
        mempoolAccount: getMempoolAccAddress(arciumEnv.arciumClusterOffset),
        executingPool: getExecutingPoolAccAddress(
          arciumEnv.arciumClusterOffset,
        ),
        compDefAccount: getCompDefAccAddress(
          program.programId,
          Buffer.from(getCompDefAccOffset("init_position")).readUInt32LE(),
        ),
        position: positionPda,
      })
      .signers([owner])
      .rpc({ skipPreflight: true, commitment: "confirmed" });

    await awaitComputationFinalization(
      provider,
      initOffset,
      program.programId,
      "confirmed",
    );

    {
      const acct = await program.account.spikePosition.fetch(positionPda);
      expect(acct.viewingKeyEpoch.toNumber()).to.equal(0);
      expect(Buffer.from(acct.viewingPubkey)).to.deep.equal(
        Buffer.from(oldKey.publicKey),
      );
      const storedNonce = new Uint8Array(
        acct.state.nonce.toArrayLike(Buffer, "le", 16),
      );
      recordPair(oldKey.publicKey, storedNonce);
      const storedCt = acct.state.ciphertexts.map((c: number[]) =>
        Array.from(c),
      );
      expect(oldKey.cipher.decrypt(storedCt, storedNonce)).to.deep.equal(
        payload,
      );
    }

    // ---- rotate to a NEW Viewing Key --------------------------------------
    const newKey = makeOwner(mxePublicKey);
    const probeNonce = randomBytes(16);
    recordPair(newKey.publicKey, probeNonce);
    // The probe carries one throwaway scalar encrypted under the new key.
    const probeCt = newKey.cipher.encrypt([0n], probeNonce);
    const rotateOffset = new anchor.BN(randomBytes(8), "hex");

    await program.methods
      .rotateViewingKey(
        rotateOffset,
        Array.from(newKey.publicKey),
        new anchor.BN(deserializeLE(probeNonce).toString()),
        Array.from(probeCt[0]),
      )
      .accountsPartial({
        payer: owner.publicKey,
        computationAccount: getComputationAccAddress(
          arciumEnv.arciumClusterOffset,
          rotateOffset,
        ),
        clusterAccount,
        mxeAccount: getMXEAccAddress(program.programId),
        mempoolAccount: getMempoolAccAddress(arciumEnv.arciumClusterOffset),
        executingPool: getExecutingPoolAccAddress(
          arciumEnv.arciumClusterOffset,
        ),
        compDefAccount: getCompDefAccAddress(
          program.programId,
          Buffer.from(getCompDefAccOffset("rotate_viewing_key")).readUInt32LE(),
        ),
        position: positionPda,
      })
      .signers([owner])
      .rpc({ skipPreflight: true, commitment: "confirmed" });

    await awaitComputationFinalization(
      provider,
      rotateOffset,
      program.programId,
      "confirmed",
    );

    // ---- the Position now decrypts under the NEW key, not the OLD one -----
    const acct = await program.account.spikePosition.fetch(positionPda);
    expect(acct.viewingKeyEpoch.toNumber()).to.equal(1);
    expect(Buffer.from(acct.viewingPubkey)).to.deep.equal(
      Buffer.from(newKey.publicKey),
    );

    const storedNonce = new Uint8Array(
      acct.state.nonce.toArrayLike(Buffer, "le", 16),
    );
    recordPair(newKey.publicKey, storedNonce);
    const storedCt = acct.state.ciphertexts.map((c: number[]) => Array.from(c));

    // Decrypts under the NEW key to the original (unchanged) payload.
    expect(newKey.cipher.decrypt(storedCt, storedNonce)).to.deep.equal(payload);

    // Does NOT decrypt under the OLD key.
    expect(oldKey.cipher.decrypt(storedCt, storedNonce)).to.not.deep.equal(
      payload,
    );
  });

  it("recorded multiple unique effective (key, nonce) pairs with no repeats", () => {
    // The two-owner run contributes four pairs and rotation four more;
    // recordPair already fails loudly on any repeat, so reaching here with a
    // populated set is the nonce-safety evidence.
    expect(usedPairs.size).to.be.greaterThan(6);
  });

  function makeOwner(mxeKey: Uint8Array): Owner {
    const privateKey = x25519.utils.randomSecretKey();
    const publicKey = x25519.getPublicKey(privateKey);
    const sharedSecret = x25519.getSharedSecret(privateKey, mxeKey);
    return { publicKey, cipher: new RescueCipher(sharedSecret) };
  }

  async function initCompDef(
    owner: anchor.web3.Keypair,
    circuitName: string,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    method: any,
  ): Promise<void> {
    const baseSeedCompDefAcc = getArciumAccountBaseSeed(
      "ComputationDefinitionAccount",
    );
    const offset = getCompDefAccOffset(circuitName);
    const compDefPDA = PublicKey.findProgramAddressSync(
      [baseSeedCompDefAcc, program.programId.toBuffer(), offset],
      getArciumProgramId(),
    )[0];

    // init_position is shared with the Task 2.2 test and the test files run in
    // one process, so skip initialization if another test already registered
    // this computation definition.
    const existing = await provider.connection.getAccountInfo(compDefPDA);
    if (existing) return;

    const arciumProgram = getArciumProgram(provider);
    const mxeAccount = getMXEAccAddress(program.programId);
    const mxeAcc = await arciumProgram.account.mxeAccount.fetch(mxeAccount);
    const lutAddress = getLookupTableAddress(
      program.programId,
      mxeAcc.lutOffsetSlot,
    );

    await method
      .accounts({
        compDefAccount: compDefPDA,
        payer: owner.publicKey,
        mxeAccount,
        addressLookupTable: lutAddress,
      })
      .signers([owner])
      .rpc({ preflightCommitment: "confirmed", commitment: "confirmed" });

    const rawCircuit = fs.readFileSync(`build/${circuitName}.arcis`);
    await uploadCircuit(
      provider,
      circuitName,
      program.programId,
      rawCircuit,
      true,
    );
  }
});

async function getMXEPublicKeyWithRetry(
  provider: anchor.AnchorProvider,
  programId: PublicKey,
  maxRetries = 20,
  retryDelayMs = 500,
): Promise<Uint8Array> {
  for (let attempt = 1; attempt <= maxRetries; attempt++) {
    try {
      const mxePublicKey = await getMXEPublicKey(provider, programId);
      if (mxePublicKey) return mxePublicKey;
    } catch (error) {
      console.log(`Attempt ${attempt} failed to fetch MXE public key:`, error);
    }
    if (attempt < maxRetries) {
      await new Promise((resolve) => setTimeout(resolve, retryDelayMs));
    }
  }
  throw new Error(
    `Failed to fetch MXE public key after ${maxRetries} attempts`,
  );
}

function readKpJson(path: string): anchor.web3.Keypair {
  const file = fs.readFileSync(path);
  return anchor.web3.Keypair.fromSecretKey(
    new Uint8Array(JSON.parse(file.toString())),
  );
}
