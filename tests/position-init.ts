/**
 * Task 5.1: production Position initialization (Requirements 3.1-3.6, 4.10,
 * 4.11, 5.1-5.3, 5.9, 5.10).
 *
 * Proves on localnet, against the Market created by `market-init.ts`, that:
 *   1. `initialize_position` creates the owner's Position in `Initializing`,
 *      registers the Viewing Key at epoch 0, increments `position_count`,
 *      creates the Operation Intent, and Locks the Position;
 *   2. the `init_position_state` callback stores the encrypted zero state,
 *      marks the Intent `Applied`, bumps the generation, releases the Lock, and
 *      activates the Position; the owner decrypts the zero state locally and
 *      the output nonce is the input nonce + 1;
 *   3. a second Position for the same owner is rejected, an `Active` Position
 *      cannot retry or close, and a zero Viewing Key is rejected;
 *   4. when the Market is already at its Position count cap, initialization
 *      fails with `PositionCountCapReached`.
 *
 * Retry and close of an `InitFailed` Position need a failed attempt, which on
 * localnet means waiting past Lock expiry (1,800 slots) for abandonment. They
 * are covered by the Rust tests in `programs/walnut/src/state/position.rs`.
 *
 * Idempotent: every run uses fresh owner keypairs funded by the provider
 * wallet. The file name sorts after `market-*.ts`, so the Market exists.
 */
import * as anchor from "@anchor-lang/core";
import { BN, Program } from "@anchor-lang/core";
import {
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
} from "@solana/web3.js";
import { Walnut } from "../target/types/walnut";
import { randomBytes } from "crypto";
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
  uploadCircuit,
  x25519,
} from "@arcium-hq/client";
import * as fs from "fs";
import * as os from "os";
import { expect } from "chai";

const CIRCUIT = "init_position_state";
const FIELD_COUNT = 5;

type ParsedEvent = { name: string; data: Record<string, unknown> };

describe("Walnut Task 5.1: Position initialization", () => {
  anchor.setProvider(anchor.AnchorProvider.env());
  const program = anchor.workspace.Walnut as Program<Walnut>;
  const provider = anchor.getProvider() as anchor.AnchorProvider;
  const connection = provider.connection;
  const wallet = readKpJson(`${os.homedir()}/.config/solana/id.json`);
  const arciumEnv = getArciumEnv();
  const clusterAccount = getClusterAccAddress(arciumEnv.arciumClusterOffset);
  const parser = new anchor.EventParser(program.programId, program.coder);

  const pda = (...seeds: Buffer[]) =>
    PublicKey.findProgramAddressSync(seeds, program.programId)[0];
  const market = pda(Buffer.from("market"));
  const positionOf = (owner: PublicKey) =>
    pda(Buffer.from("position"), market.toBuffer(), owner.toBuffer());
  const counterOf = (owner: PublicKey) =>
    pda(Buffer.from("operation_counter"), owner.toBuffer());
  const intentOf = (owner: PublicKey, sequence: BN) =>
    pda(
      Buffer.from("operation_intent"),
      owner.toBuffer(),
      sequence.toArrayLike(Buffer, "le", 8),
    );

  const arciumAccounts = (offset: BN) => ({
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
      Buffer.from(getCompDefAccOffset(CIRCUIT)).readUInt32LE(),
    ),
  });

  const initialize = (
    owner: Keypair,
    sequence: BN,
    viewingPubkey: Uint8Array,
    nonce: Uint8Array,
    offset = new BN(randomBytes(8), "hex"),
  ) =>
    program.methods
      .initializePosition(
        offset,
        sequence,
        Array.from(viewingPubkey),
        new BN(deserializeLE(nonce).toString()),
      )
      .accountsPartial({
        payer: owner.publicKey,
        ...arciumAccounts(offset),
        market,
        position: positionOf(owner.publicKey),
        operationCounter: counterOf(owner.publicKey),
        operationIntent: intentOf(owner.publicKey, sequence),
      })
      .signers([owner])
      .rpc({ commitment: "confirmed" });

  async function fundedOwner(): Promise<Keypair> {
    const owner = Keypair.generate();
    const tx = new Transaction().add(
      SystemProgram.transfer({
        fromPubkey: wallet.publicKey,
        toPubkey: owner.publicKey,
        lamports: 200_000_000,
      }),
    );
    await provider.sendAndConfirm(tx, [wallet], { commitment: "confirmed" });
    return owner;
  }

  async function events(signature: string): Promise<ParsedEvent[]> {
    const tx = await connection.getTransaction(signature, {
      commitment: "confirmed",
      maxSupportedTransactionVersion: 0,
    });
    expect(tx?.meta?.logMessages, "transaction logs").to.not.equal(undefined);
    return [...parser.parseLogs(tx!.meta!.logMessages!)] as ParsedEvent[];
  }

  const marketAccount = () => program.account.market.fetch(market, "confirmed");

  before(async () => {
    const m = await program.account.market.fetchNullable(market);
    expect(m, "run market-init.ts first: the Market must exist").to.not.equal(
      null,
    );
    await initCompDef();
  });

  it("initializes, activates, and decrypts the zero state locally", async () => {
    const owner = await fundedOwner();
    const before = await marketAccount();
    const viewingSecret = x25519.utils.randomSecretKey();
    const viewingPubkey = x25519.getPublicKey(viewingSecret);
    const nonce = randomBytes(16);
    const sequence = new BN(0);
    const position = positionOf(owner.publicKey);

    if (before.positionCount.gte(before.positionCountCap)) {
      await expectRejected(
        initialize(owner, sequence, viewingPubkey, nonce),
        "PositionCountCapReached",
      );
      return;
    }

    const offset = new BN(randomBytes(8), "hex");
    const signature = await initialize(
      owner,
      sequence,
      viewingPubkey,
      nonce,
      offset,
    );
    const queued = await events(signature);
    expect(queued.map((e) => e.name)).to.deep.equal([
      "positionCountChanged",
      "positionInitQueued",
    ]);
    const count = queued[0].data;
    expect(
      new BN(count.newCount as BN).eq(before.positionCount.addn(1)),
    ).to.equal(true);
    expect(queued[1].data.retry).to.equal(false);

    const initializing = await program.account.position.fetch(
      position,
      "confirmed",
    );
    expect("initializing" in initializing.status).to.equal(true);
    expect(initializing.owner.equals(owner.publicKey)).to.equal(true);
    expect(initializing.market.equals(market)).to.equal(true);
    expect(Buffer.from(initializing.viewingPubkey)).to.deep.equal(
      Buffer.from(viewingPubkey),
    );
    expect(initializing.viewingKeyEpoch.toNumber()).to.equal(0);
    expect(initializing.hasDebt).to.equal(false);
    expect(initializing.coordination.lock.active).to.equal(true);
    const intentAddress = intentOf(owner.publicKey, sequence);
    expect(
      initializing.coordination.lock.operationIntent.equals(intentAddress),
    ).to.equal(true);
    expect(
      (await marketAccount()).positionCount.eq(before.positionCount.addn(1)),
    ).to.equal(true);

    await awaitComputationFinalization(
      provider,
      offset,
      program.programId,
      "confirmed",
    );

    const active = await waitForStatus(position, "active");
    expect(active.coordination.lock.active).to.equal(false);
    expect(active.coordination.generation.toNumber()).to.equal(1);
    const intent = await program.account.operationIntent.fetch(
      intentAddress,
      "confirmed",
    );
    expect("applied" in intent.status).to.equal(true);
    expect("positionInit" in intent.kind).to.equal(true);

    // Local decryption with the Viewing Key only; no Walnut server involved.
    const mxePublicKey = await getMXEPublicKeyWithRetry();
    const cipher = new RescueCipher(
      x25519.getSharedSecret(viewingSecret, mxePublicKey),
    );
    const outputNonce = active.encryptedState.nonce as BN;
    expect(
      outputNonce.eq(new BN(deserializeLE(nonce).toString()).addn(1)),
      "output nonce = input nonce + 1",
    ).to.equal(true);
    const ciphertexts = active.encryptedState.ciphertexts.map((c: number[]) =>
      Array.from(c),
    );
    expect(ciphertexts.length).to.equal(FIELD_COUNT);
    const plain = cipher.decrypt(
      ciphertexts,
      new Uint8Array(outputNonce.toArrayLike(Buffer, "le", 16)),
    );
    expect(plain).to.deep.equal([0n, 0n, 0n, 0n, 0n]);

    // One Position per owner; an Active Position cannot retry or close.
    await expectRejected(
      initialize(owner, new BN(1), viewingPubkey, randomBytes(16)),
      "already in use",
    );
    const retryOffset = new BN(randomBytes(8), "hex");
    await expectRejected(
      program.methods
        .retryPositionInit(
          retryOffset,
          new BN(1),
          new BN(deserializeLE(randomBytes(16)).toString()),
        )
        .accountsPartial({
          payer: owner.publicKey,
          ...arciumAccounts(retryOffset),
          market,
          position,
          operationCounter: counterOf(owner.publicKey),
          operationIntent: intentOf(owner.publicKey, new BN(1)),
        })
        .signers([owner])
        .rpc({ commitment: "confirmed" }),
      "PositionNotRetryable",
    );
    await expectRejected(
      program.methods
        .closePosition()
        .accountsPartial({ owner: owner.publicKey, market, position })
        .signers([owner])
        .rpc({ commitment: "confirmed" }),
      "PositionNotClosable",
    );
    const after = await program.account.position.fetch(position, "confirmed");
    expect("active" in after.status).to.equal(true);
  });

  it("rejects a zero Viewing Key", async () => {
    const owner = await fundedOwner();
    await expectRejected(
      initialize(owner, new BN(0), new Uint8Array(32), randomBytes(16)),
      "InvalidViewingKey",
    );
    const missing = await program.account.position.fetchNullable(
      positionOf(owner.publicKey),
      "confirmed",
    );
    expect(missing).to.equal(null);
  });

  async function waitForStatus(position: PublicKey, status: string) {
    for (let attempt = 0; attempt < 60; attempt++) {
      const account = await program.account.position.fetch(
        position,
        "confirmed",
      );
      if (status in account.status) return account;
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
    throw new Error(`Position did not reach ${status}`);
  }

  async function getMXEPublicKeyWithRetry(): Promise<Uint8Array> {
    for (let attempt = 0; attempt < 20; attempt++) {
      try {
        const key = await getMXEPublicKey(provider, program.programId);
        if (key) return key;
      } catch {
        // Retry below.
      }
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
    throw new Error("Failed to fetch the MXE public key");
  }

  async function initCompDef(): Promise<void> {
    const offset = getCompDefAccOffset(CIRCUIT);
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
    await program.methods
      .initPositionStateCompDef()
      .accounts({
        compDefAccount: compDefPDA,
        payer: wallet.publicKey,
        mxeAccount,
        addressLookupTable: getLookupTableAddress(
          program.programId,
          mxeAcc.lutOffsetSlot,
        ),
      })
      .signers([wallet])
      .rpc({ preflightCommitment: "confirmed", commitment: "confirmed" });
    await uploadCircuit(
      provider,
      CIRCUIT,
      program.programId,
      fs.readFileSync(`build/${CIRCUIT}.arcis`),
      true,
    );
  }
});

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
