/**
 * Task 5.5: Viewing Key rotation (Requirements 5.10-5.12).
 *
 * Proves on localnet, against the Market created by `market-init.ts`, that:
 *   1. a fresh Position (epoch 0) holds a shielded SOL balance the owner
 *      decrypts locally with the epoch-0 Viewing Key;
 *   2. `rotate_position_key` (owner-signed, not pausable) commits the old and
 *      new epochs and public keys; the callback re-encrypts, stores the new
 *      public key, and increments the epoch to 1 together;
 *   3. the same balances decrypt under the new key and NOT under the old key;
 *   4. queue-time checks reject an unchanged key, a zero key, and wrong
 *      expected epochs or old key, and nobody but the owner can rotate;
 *   5. a later operation (another SOL shield) works under the new key.
 *
 * The Viewing Keys are derived from a wallet-style signature exactly as
 * `packages/sdk/src/viewing-key.ts` does (HKDF-SHA-256 over the signature; the
 * epoch is part of the signed message), so epoch 0 and epoch 1 give different
 * keys. Each run creates a fresh owner and Position, like `position-shield.ts`.
 * It is CI-only (needs the local Arcium cluster) and sorts after
 * `position-init.ts`, `position-shield.ts`, and `position-unshield.ts`.
 * Abandonment after Lock expiry (1,800 slots) is covered by the Rust tests in
 * `programs/walnut/src/state/rotation/tests.rs`.
 */
import * as anchor from "@anchor-lang/core";
import { BN, Program } from "@anchor-lang/core";
import {
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";
import { Walnut } from "../target/types/walnut";
import { createHash, hkdfSync, randomBytes } from "crypto";
import {
  awaitComputationFinalization,
  deserializeLE,
  getArciumAccountBaseSeed,
  getArciumEnv,
  getArciumProgram,
  getArciumProgramId,
  getClusterAccAddress,
  getCompDefAccAddress,
  getCompDefAccOffset,
  getComputationAccAddress,
  getExecutingPoolAccAddress,
  getLookupTableAddress,
  getMempoolAccAddress,
  getMXEAccAddress,
  getMXEPublicKey,
  RescueCipher,
  x25519,
  uploadCircuit,
} from "@arcium-hq/client";
import * as fs from "fs";
import * as os from "os";
import { expect } from "chai";

const TOKEN_PROGRAM_ID = new PublicKey(
  "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
);
const TOKEN_ACCOUNT_SIZE = 165;
const INIT_CIRCUIT = "init_position_state";
const SHIELD_CIRCUIT = "shield_credit";
const ROTATE_CIRCUIT = "rotate_position_key";
const SOL_AMOUNT = 54_321_000n;
const OUTPUT_NONCE_DOMAIN = "walnut:position-output-nonce:v1";
// Mirrors packages/sdk/src/viewing-key.ts.
const VIEWING_KEY_DOMAIN = "walnut-solana viewing key v1";
const VIEWING_KEY_INFO = "walnut-solana viewing key v1 x25519 secret";

describe("Walnut Task 5.5: Viewing Key rotation", () => {
  anchor.setProvider(anchor.AnchorProvider.env());
  const program = anchor.workspace.Walnut as Program<Walnut>;
  const provider = anchor.getProvider() as anchor.AnchorProvider;
  const connection = provider.connection;
  const wallet = readKpJson(`${os.homedir()}/.config/solana/id.json`);
  const arciumEnv = getArciumEnv();
  const clusterAccount = getClusterAccAddress(arciumEnv.arciumClusterOffset);

  const pda = (...seeds: Buffer[]) =>
    PublicKey.findProgramAddressSync(seeds, program.programId)[0];
  const le8 = (n: BN) => n.toArrayLike(Buffer, "le", 8);
  const market = pda(Buffer.from("market"));
  const positionOf = (owner: PublicKey) =>
    pda(Buffer.from("position"), market.toBuffer(), owner.toBuffer());
  const counterOf = (owner: PublicKey) =>
    pda(Buffer.from("operation_counter"), owner.toBuffer());
  const intentOf = (owner: PublicKey, sequence: BN) =>
    pda(Buffer.from("operation_intent"), owner.toBuffer(), le8(sequence));
  const pendingShieldOf = (position: PublicKey, sequence: BN) =>
    pda(Buffer.from("pending_shield"), position.toBuffer(), le8(sequence));

  const arciumAccounts = (circuit: string, offset: BN) => ({
    computationAccount: getComputationAccAddress(
      arciumEnv.arciumClusterOffset,
      offset,
    ),
    clusterAccount,
    mxeAccount: getMXEAccAddress(program.programId),
    mempoolAccount: getMempoolAccAddress(arciumEnv.arciumClusterOffset),
    executingPool: getExecutingPoolAccAddress(arciumEnv.arciumClusterOffset),
    compDefAccount: getCompDefAccAddress(
      program.programId,
      Buffer.from(getCompDefAccOffset(circuit)).readUInt32LE(),
    ),
  });

  const newOffset = () => new BN(randomBytes(8), "hex");

  /** The Viewing Key of `epoch` from a deterministic wallet-style signature. */
  const viewingKey = (
    wallet: Keypair,
    position: PublicKey,
    epoch: number,
  ): { secret: Uint8Array; publicKey: Uint8Array } => {
    const message =
      `${VIEWING_KEY_DOMAIN} | program=${program.programId.toBase58()}` +
      ` | network=localnet | position=${position.toBase58()} | epoch=${epoch}`;
    const signature = createHash("sha512")
      .update(wallet.secretKey)
      .update(message)
      .digest();
    const secret = new Uint8Array(
      hkdfSync("sha256", signature, VIEWING_KEY_DOMAIN, VIEWING_KEY_INFO, 32),
    );
    return { secret, publicKey: x25519.getPublicKey(secret) };
  };

  /** The output nonce the program derives for an attempt (`state::nonce`). */
  const derivedOutputNonce = (position: PublicKey, sequence: BN): bigint => {
    const digest = createHash("sha256")
      .update(Buffer.from(OUTPUT_NONCE_DOMAIN))
      .update(position.toBuffer())
      .update(le8(sequence))
      .digest();
    let n = 0n;
    for (let i = 15; i >= 0; i--) n = (n << 8n) | BigInt(digest[i]);
    return (n & ((1n << 127n) - 1n)) + 1n;
  };

  before(async () => {
    const m = await program.account.market.fetchNullable(market);
    expect(m, "run market-init.ts first: the Market must exist").to.not.equal(
      null,
    );
    await initCompDef(INIT_CIRCUIT);
    await initCompDef(SHIELD_CIRCUIT);
    await initCompDef(ROTATE_CIRCUIT);
  });

  it("rotates the Viewing Key and re-encrypts atomically", async function () {
    const m = await program.account.market.fetch(market, "confirmed");
    if (m.positionCount.gte(m.positionCountCap)) {
      console.log("      Position count cap reached; rotation test skipped");
      this.skip();
    }
    const owner = await fundedOwner(400_000_000);
    const position = positionOf(owner.publicKey);
    const key0 = viewingKey(owner, position, 0);
    const key1 = viewingKey(owner, position, 1);
    expect(
      Buffer.from(key0.publicKey).equals(Buffer.from(key1.publicKey)),
    ).to.equal(false);
    await activePosition(owner, key0.publicKey);
    const wsolAccount = await tokenAccount(owner, m.wsolMint, 0);
    await shieldSol(owner, m, wsolAccount, SOL_AMOUNT);

    const decrypt0 = await decryptor(key0.secret);
    expect(await decrypt0(position)).to.deep.equal([
      SOL_AMOUNT,
      0n,
      0n,
      0n,
      0n,
    ]);
    const before = await program.account.position.fetch(position, "confirmed");
    expect(before.viewingKeyEpoch.toNumber()).to.equal(0);
    expect(Buffer.from(before.viewingPubkey)).to.deep.equal(
      Buffer.from(key0.publicKey),
    );

    // Queue-time rejections change nothing and need no computation.
    const lie = async (
      overrides: Partial<{
        oldEpoch: number;
        newEpoch: number;
        oldPubkey: Uint8Array;
        newPubkey: Uint8Array;
      }>,
      signer: Keypair = owner,
    ) => {
      const { instruction } = await rotateInstruction(owner, signer, {
        oldEpoch: 0,
        newEpoch: 1,
        oldPubkey: key0.publicKey,
        newPubkey: key1.publicKey,
        ...overrides,
      });
      return provider.sendAndConfirm(
        new Transaction().add(instruction),
        [signer],
        { commitment: "confirmed" },
      );
    };
    await expectRejected(
      lie({ newPubkey: key0.publicKey }),
      "RotationKeyUnchanged",
    );
    await expectRejected(
      lie({ newPubkey: new Uint8Array(32) }),
      "InvalidViewingKey",
    );
    await expectRejected(lie({ newEpoch: 2 }), "RotationEpochMismatch");
    await expectRejected(lie({ oldEpoch: 1 }), "RotationEpochMismatch");
    await expectRejected(
      lie({ oldPubkey: key1.publicKey }),
      "RotationOldKeyMismatch",
    );
    const stranger = await fundedOwner(50_000_000);
    await expectRejected(
      lie({}, stranger),
      "ConstraintSeeds",
      "AccountNotInitialized",
      "2006",
    );
    const unchanged = await program.account.position.fetch(
      position,
      "confirmed",
    );
    expect(unchanged.viewingKeyEpoch.toNumber()).to.equal(0);
    expect(unchanged.coordination.lock.active).to.equal(false);

    // The real rotation.
    const { instruction, offset, sequence } = await rotateInstruction(
      owner,
      owner,
      {
        oldEpoch: 0,
        newEpoch: 1,
        oldPubkey: key0.publicKey,
        newPubkey: key1.publicKey,
      },
    );
    await provider.sendAndConfirm(new Transaction().add(instruction), [owner], {
      commitment: "confirmed",
    });
    const queued = await program.account.position.fetch(position, "confirmed");
    expect(queued.coordination.lock.active).to.equal(true, "Lock while queued");
    expect(queued.viewingKeyEpoch.toNumber()).to.equal(
      0,
      "old key until Applied",
    );
    await awaitComputationFinalization(
      provider,
      offset,
      program.programId,
      "confirmed",
    );
    const rotated = await waitForEpoch(position, 1);

    // Key, epoch, and payload changed together.
    expect(Buffer.from(rotated.viewingPubkey)).to.deep.equal(
      Buffer.from(key1.publicKey),
    );
    expect(rotated.coordination.lock.active).to.equal(false);
    expect(BigInt((rotated.encryptedState.nonce as BN).toString())).to.equal(
      derivedOutputNonce(position, sequence),
      "stored output nonce is derived from the Intent sequence",
    );
    expect(
      rotated.coordination.generation.gt(before.coordination.generation),
    ).to.equal(true);

    // Decrypts under the new key, not under the old key.
    const decrypt1 = await decryptor(key1.secret);
    expect(await decrypt1(position)).to.deep.equal([
      SOL_AMOUNT,
      0n,
      0n,
      0n,
      0n,
    ]);
    let underOld: bigint[] | undefined;
    try {
      underOld = await decrypt0(position);
    } catch {
      underOld = undefined;
    }
    expect(underOld).to.not.deep.equal([SOL_AMOUNT, 0n, 0n, 0n, 0n]);

    // A subsequent operation works under the new key.
    await shieldSol(owner, m, wsolAccount, SOL_AMOUNT);
    expect(await decrypt1(position)).to.deep.equal([
      SOL_AMOUNT * 2n,
      0n,
      0n,
      0n,
      0n,
    ]);
    const after = await program.account.position.fetch(position, "confirmed");
    expect(after.viewingKeyEpoch.toNumber()).to.equal(1);
    expect(Buffer.from(after.viewingPubkey)).to.deep.equal(
      Buffer.from(key1.publicKey),
    );
  });

  async function rotateInstruction(
    owner: Keypair,
    signer: Keypair,
    args: {
      oldEpoch: number;
      newEpoch: number;
      oldPubkey: Uint8Array;
      newPubkey: Uint8Array;
    },
  ) {
    const position = positionOf(owner.publicKey);
    const counter = await program.account.operationCounter.fetch(
      counterOf(owner.publicKey),
      "confirmed",
    );
    const sequence = counter.nextSequence as BN;
    const offset = newOffset();
    // The seeds name the signer: a stranger signing for the owner's Position
    // fails the Position seeds, so only the owner can rotate.
    const instruction = await program.methods
      .rotatePositionKey(
        offset,
        sequence,
        new BN(args.oldEpoch),
        new BN(args.newEpoch),
        Array.from(args.oldPubkey),
        Array.from(args.newPubkey),
      )
      .accountsPartial({
        payer: signer.publicKey,
        ...arciumAccounts(ROTATE_CIRCUIT, offset),
        position,
        operationCounter: counterOf(owner.publicKey),
        operationIntent: intentOf(signer.publicKey, sequence),
      })
      .instruction();
    return { offset, instruction, sequence };
  }

  async function waitForEpoch(position: PublicKey, epoch: number) {
    for (let attempt = 0; attempt < 60; attempt++) {
      const p = await program.account.position.fetch(position, "confirmed");
      if (p.viewingKeyEpoch.toNumber() === epoch) return p;
      await sleep(500);
    }
    throw new Error(`Position did not reach epoch ${epoch}`);
  }

  async function shieldSol(
    owner: Keypair,
    m: Awaited<ReturnType<typeof program.account.market.fetch>>,
    wsolAccount: PublicKey,
    amount: bigint,
  ): Promise<void> {
    const position = positionOf(owner.publicKey);
    const counter = await program.account.operationCounter.fetch(
      counterOf(owner.publicKey),
      "confirmed",
    );
    const p = await program.account.position.fetch(position, "confirmed");
    const sequence = counter.nextSequence as BN;
    const shieldSequence = p.nextShieldSeq as BN;
    const offset = newOffset();
    const pendingShield = pendingShieldOf(position, shieldSequence);
    const instruction = await program.methods
      .shield(
        offset,
        sequence,
        shieldSequence,
        { sol: {} },
        new BN(amount.toString()),
      )
      .accountsPartial({
        payer: owner.publicKey,
        ...arciumAccounts(SHIELD_CIRCUIT, offset),
        market,
        position,
        operationCounter: counterOf(owner.publicKey),
        operationIntent: intentOf(owner.publicKey, sequence),
        pendingShield,
        mint: m.wsolMint,
        vault: m.wsolVault,
        sourceTokenAccount: wsolAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .instruction();
    await provider.sendAndConfirm(
      new Transaction().add(
        SystemProgram.transfer({
          fromPubkey: owner.publicKey,
          toPubkey: wsolAccount,
          lamports: Number(amount),
        }),
        new TransactionInstruction({
          programId: TOKEN_PROGRAM_ID,
          keys: [{ pubkey: wsolAccount, isSigner: false, isWritable: true }],
          data: Buffer.from([17]), // SyncNative
        }),
        instruction,
      ),
      [owner],
      { commitment: "confirmed" },
    );
    await awaitComputationFinalization(
      provider,
      offset,
      program.programId,
      "confirmed",
    );
    for (let attempt = 0; attempt < 60; attempt++) {
      const account = await program.account.pendingShield.fetch(
        pendingShield,
        "confirmed",
      );
      if ("credited" in account.status) return;
      await sleep(500);
    }
    throw new Error("Pending Shield was not credited");
  }

  async function activePosition(
    owner: Keypair,
    viewingPublicKey: Uint8Array,
  ): Promise<void> {
    const offset = newOffset();
    const sequence = new BN(0);
    await program.methods
      .initializePosition(
        offset,
        sequence,
        Array.from(viewingPublicKey),
        new BN(deserializeLE(randomBytes(16)).toString()),
      )
      .accountsPartial({
        payer: owner.publicKey,
        ...arciumAccounts(INIT_CIRCUIT, offset),
        market,
        position: positionOf(owner.publicKey),
        operationCounter: counterOf(owner.publicKey),
        operationIntent: intentOf(owner.publicKey, sequence),
      })
      .signers([owner])
      .rpc({ commitment: "confirmed" });
    await awaitComputationFinalization(
      provider,
      offset,
      program.programId,
      "confirmed",
    );
    for (let attempt = 0; attempt < 60; attempt++) {
      const p = await program.account.position.fetch(
        positionOf(owner.publicKey),
        "confirmed",
      );
      if ("active" in p.status) return;
      await sleep(500);
    }
    throw new Error("Position did not become active");
  }

  /** Returns a local decryptor of the stored Position state. */
  async function decryptor(viewingSecret: Uint8Array) {
    const mxePublicKey = await getMXEPublicKeyWithRetry();
    const cipher = new RescueCipher(
      x25519.getSharedSecret(viewingSecret, mxePublicKey),
    );
    return async (position: PublicKey): Promise<bigint[]> => {
      const p = await program.account.position.fetch(position, "confirmed");
      const nonce = p.encryptedState.nonce as BN;
      return cipher.decrypt(
        p.encryptedState.ciphertexts.map((c: number[]) => Array.from(c)),
        new Uint8Array(nonce.toArrayLike(Buffer, "le", 16)),
      );
    };
  }

  async function fundedOwner(lamports: number): Promise<Keypair> {
    const owner = Keypair.generate();
    await provider.sendAndConfirm(
      new Transaction().add(
        SystemProgram.transfer({
          fromPubkey: wallet.publicKey,
          toPubkey: owner.publicKey,
          lamports,
        }),
      ),
      [wallet],
      { commitment: "confirmed" },
    );
    return owner;
  }

  /** Creates a classic SPL token account of `mint` owned by `owner`. */
  async function tokenAccount(
    owner: Keypair,
    mint: PublicKey,
    extraLamports: number,
  ): Promise<PublicKey> {
    const account = Keypair.generate();
    const rent =
      await connection.getMinimumBalanceForRentExemption(TOKEN_ACCOUNT_SIZE);
    // InitializeAccount3: tag 18 | owner.
    const data = Buffer.alloc(33);
    data.writeUInt8(18, 0);
    owner.publicKey.toBuffer().copy(data, 1);
    await provider.sendAndConfirm(
      new Transaction().add(
        SystemProgram.createAccount({
          fromPubkey: owner.publicKey,
          newAccountPubkey: account.publicKey,
          lamports: rent + extraLamports,
          space: TOKEN_ACCOUNT_SIZE,
          programId: TOKEN_PROGRAM_ID,
        }),
        new TransactionInstruction({
          programId: TOKEN_PROGRAM_ID,
          keys: [
            { pubkey: account.publicKey, isSigner: false, isWritable: true },
            { pubkey: mint, isSigner: false, isWritable: false },
          ],
          data,
        }),
      ),
      [owner, account],
      { commitment: "confirmed" },
    );
    return account.publicKey;
  }

  async function getMXEPublicKeyWithRetry(): Promise<Uint8Array> {
    for (let attempt = 0; attempt < 20; attempt++) {
      try {
        const key = await getMXEPublicKey(provider, program.programId);
        if (key) return key;
      } catch {
        // Retry below.
      }
      await sleep(500);
    }
    throw new Error("Failed to fetch the MXE public key");
  }

  async function initCompDef(circuit: string): Promise<void> {
    const offset = getCompDefAccOffset(circuit);
    const compDefPDA = PublicKey.findProgramAddressSync(
      [
        getArciumAccountBaseSeed("ComputationDefinitionAccount"),
        program.programId.toBuffer(),
        offset,
      ],
      getArciumProgramId(),
    )[0];
    if (await connection.getAccountInfo(compDefPDA)) return;
    const arciumProgram = getArciumProgram(provider);
    const mxeAccount = getMXEAccAddress(program.programId);
    const mxeAcc = await arciumProgram.account.mxeAccount.fetch(mxeAccount);
    const accounts = {
      compDefAccount: compDefPDA,
      payer: wallet.publicKey,
      mxeAccount,
      addressLookupTable: getLookupTableAddress(
        program.programId,
        mxeAcc.lutOffsetSlot,
      ),
    };
    const options = {
      preflightCommitment: "confirmed",
      commitment: "confirmed",
    } as const;
    if (circuit === INIT_CIRCUIT) {
      await program.methods
        .initPositionStateCompDef()
        .accounts(accounts)
        .signers([wallet])
        .rpc(options);
    } else if (circuit === SHIELD_CIRCUIT) {
      await program.methods
        .initShieldCreditCompDef()
        .accounts(accounts)
        .signers([wallet])
        .rpc(options);
    } else {
      await program.methods
        .initRotatePositionKeyCompDef()
        .accounts(accounts)
        .signers([wallet])
        .rpc(options);
    }
    await uploadCircuit(
      provider,
      circuit,
      program.programId,
      fs.readFileSync(`build/${circuit}.arcis`),
      true,
    );
  }
});

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Asserts that `promise` rejects and that the error names one of `codes`. */
async function expectRejected(
  promise: Promise<unknown>,
  ...codes: string[]
): Promise<void> {
  let error: unknown;
  try {
    await promise;
  } catch (caught) {
    error = caught;
  }
  const expected = codes.join(" or ");
  expect(error, `expected rejection with ${expected}`).to.not.equal(undefined);
  const e = error as {
    message?: string;
    logs?: string[];
    transactionLogs?: string[];
    error?: { errorCode?: { code?: string } };
  };
  const text = [
    e.message ?? "",
    ...(e.logs ?? []),
    ...(e.transactionLogs ?? []),
    e.error?.errorCode?.code,
  ]
    .filter(Boolean)
    .join("\n");
  expect(
    codes.some((code) => text.includes(code)),
    `rejection should name ${expected}, got: ${text}`,
  ).to.equal(true);
}

function readKpJson(path: string): Keypair {
  return Keypair.fromSecretKey(
    new Uint8Array(JSON.parse(fs.readFileSync(path).toString())),
  );
}
