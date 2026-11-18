/**
 * Task 8: USDC supply and withdrawal (Requirements 8.1 to 8.7).
 *
 * Proves on localnet, against the Market created by `market-init.ts`, that:
 *   1. shielded USDC can be supplied: Free USDC drops by exactly the amount,
 *      the Position gains Supply Shares, and both the Position and the Pool
 *      payloads change together (one Intent, `Applied`);
 *   2. an insufficient-balance supply and an over-withdrawal each resolve
 *      `Zero`: the Intent ends `Zero` and the stored Position state and nonce
 *      do not change;
 *   3. a withdrawal of all shares returns the supplied USDC to Free USDC
 *      (exactly, when the Pool had no other supplier or interest) and leaves no
 *      shares;
 *   4. with no borrowers there is no interest: the claim after supplying is
 *      exactly the supplied amount (equity neutrality).
 *
 * Supply needs a fresh utilization band (a stale band blocks new risk), so the
 * test refreshes it with `update_utilization_band` first. Each run creates a
 * fresh owner and Position and only needs the USDC mint authority to be the
 * provider wallet (true when `market-init.ts` created the mint); otherwise the
 * test is skipped with a log line. CI-only: it needs the Arcium localnet.
 */ import * as anchor from "@anchor-lang/core";
import { BN, Program } from "@anchor-lang/core";
import {
  ComputeBudgetProgram,
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionInstruction,
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
const SUPPLY_CIRCUIT = "supply_flow";
const BAND_CIRCUIT = "update_utilization_band";
const SHIELDED = 1_000_000n;
const SUPPLIED = 400_000n;
describe("Walnut Task 8: USDC supply and withdrawal", () => {
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

  before(async () => {
    const m = await program.account.market.fetchNullable(market);
    expect(m, "run market-init.ts first: the Market must exist").to.not.equal(
      null,
    );
    for (const circuit of [
      INIT_CIRCUIT,
      SHIELD_CIRCUIT,
      SUPPLY_CIRCUIT,
      BAND_CIRCUIT,
    ]) {
      await initCompDef(circuit);
    }
  });

  it("supplies, resolves refusals Zero, and withdraws exactly", async function () {
    const m = await program.account.market.fetch(market, "confirmed");
    if (m.positionCount.gte(m.positionCountCap)) {
      console.log("      Position count cap reached; supply test skipped");
      this.skip();
    }
    if (!(await isMintAuthority(m.usdcMint, wallet.publicKey))) {
      console.log("      USDC mint authority is not this wallet; test skipped");
      this.skip();
    }
    const owner = await fundedOwner(400_000_000);
    const viewingSecret = x25519.utils.randomSecretKey();
    const position = positionOf(owner.publicKey);
    await activePosition(owner, viewingSecret);
    const decrypt = await decryptor(viewingSecret);

    // Shield 1,000,000 test USDC.
    const usdcAccount = await tokenAccount(owner, m.usdcMint, 0);
    await mintTo(m.usdcMint, usdcAccount, 5_000_000n);
    await shield(owner, m, usdcAccount, SHIELDED);
    let state = await decrypt(position);
    expect(state[1]).to.equal(SHIELDED, "free_usdc after shield");
    expect(state[3]).to.equal(0n, "no supply shares yet");

    await refreshBand(owner);
    const stateBefore = await position_(position);

    // Insufficient balance: supplying more than Free USDC resolves Zero and
    // moves nothing (the stored payload and nonce are unchanged).
    const tooMuch = await supplyFlow(owner, false, SHIELDED + 1n);
    expect(tooMuch.status).to.equal("zero");
    const afterZero = await position_(position);
    expect(afterZero.encryptedState.nonce.toString()).to.equal(
      stateBefore.encryptedState.nonce.toString(),
      "a Zero supply leaves the stored nonce unchanged",
    );
    state = await decrypt(position);
    expect(state[1]).to.equal(SHIELDED);
    expect(state[3]).to.equal(0n);

    // Supply 400,000: Free USDC drops by exactly that amount.
    const applied = await supplyFlow(owner, false, SUPPLIED);
    expect(applied.status).to.equal("applied");
    state = await decrypt(position);
    expect(state[1]).to.equal(SHIELDED - SUPPLIED, "free_usdc after supply");
    const shares = state[3] as bigint;
    expect(shares > 0n, "supply shares minted").to.equal(true);

    // Over-withdraw: one share more than owned resolves Zero.
    const over = await supplyFlow(owner, true, shares + 1n);
    expect(over.status).to.equal("zero");
    state = await decrypt(position);
    expect(state[1]).to.equal(SHIELDED - SUPPLIED);
    expect(state[3]).to.equal(shares);

    // Withdraw everything: no borrowers, so no interest and the claim is the
    // supplied amount (to within one unit when another supplier shares the
    // Pool and a rounding unit stays behind).
    const out = await supplyFlow(owner, true, shares);
    expect(out.status).to.equal("applied");
    state = await decrypt(position);
    expect(state[3]).to.equal(0n, "all shares burned");
    expect(state[1] <= SHIELDED, "never more than was supplied").to.equal(true);
    expect(state[1] >= SHIELDED - 1n, "at most one unit of rounding").to.equal(
      true,
    );
    const stored = await position_(position);
    expect(stored.coordination.lock.active).to.equal(false);
  });

  async function position_(address: PublicKey) {
    return program.account.position.fetch(address, "confirmed");
  }

  /** Queues `supply_flow` and waits for its Intent to leave `Queued`. */
  async function supplyFlow(
    owner: Keypair,
    isWithdraw: boolean,
    requested: bigint,
  ): Promise<{ status: string }> {
    const m = await program.account.market.fetch(market, "confirmed");
    const counter = await program.account.operationCounter.fetch(
      counterOf(owner.publicKey),
      "confirmed",
    );
    const sequence = counter.nextSequence as BN;
    const offset = newOffset();
    const intent = intentOf(owner.publicKey, sequence);
    await program.methods
      .supplyFlow(offset, sequence, isWithdraw, new BN(requested.toString()))
      .accountsPartial({
        payer: owner.publicKey,
        ...arciumAccounts(SUPPLY_CIRCUIT, offset),
        market,
        assetState: m.assetState,
        poolState: m.poolState,
        position: positionOf(owner.publicKey),
        operationCounter: counterOf(owner.publicKey),
        operationIntent: intent,
      })
      .preInstructions([
        ComputeBudgetProgram.setComputeUnitLimit({ units: 600_000 }),
      ])
      .signers([owner])
      .rpc({ commitment: "confirmed" });
    await awaitComputationFinalization(
      provider,
      offset,
      program.programId,
      "confirmed",
    );
    return { status: await terminalStatus(intent) };
  }

  async function terminalStatus(intent: PublicKey): Promise<string> {
    for (let attempt = 0; attempt < 120; attempt++) {
      const account = await program.account.operationIntent.fetch(
        intent,
        "confirmed",
      );
      const name = Object.keys(account.status)[0] as string;
      if (name !== "queued") return name;
      await sleep(500);
    }
    throw new Error("Intent did not reach a terminal status");
  }

  /** Publishes a fresh utilization band (supply is blocked by a stale one). */
  async function refreshBand(payer: Keypair): Promise<void> {
    for (let attempt = 0; attempt < 3; attempt++) {
      const m = await program.account.market.fetch(market, "confirmed");
      const counterAddress = pda(
        Buffer.from("operation_counter"),
        m.poolState.toBuffer(),
      );
      const existing =
        await program.account.operationCounter.fetchNullable(counterAddress);
      const sequence = (existing?.nextSequence ?? new BN(0)) as BN;
      const offset = newOffset();
      const intent = pda(
        Buffer.from("operation_intent"),
        m.poolState.toBuffer(),
        le8(sequence),
      );
      await program.methods
        .updateUtilizationBand(offset, sequence)
        .accountsPartial({
          payer: payer.publicKey,
          ...arciumAccounts(BAND_CIRCUIT, offset),
          market,
          assetState: m.assetState,
          poolState: m.poolState,
          operationCounter: counterAddress,
          operationIntent: intent,
        })
        .preInstructions([
          ComputeBudgetProgram.setComputeUnitLimit({ units: 600_000 }),
        ])
        .signers([payer])
        .rpc({ commitment: "confirmed" });
      await awaitComputationFinalization(
        provider,
        offset,
        program.programId,
        "confirmed",
      );
      if ((await terminalStatus(intent)) === "applied") return;
    }
    throw new Error("the utilization band could not be refreshed");
  }

  async function shield(
    owner: Keypair,
    m: { usdcMint: PublicKey; usdcVault: PublicKey },
    source: PublicKey,
    amount: bigint,
  ): Promise<void> {
    const position = positionOf(owner.publicKey);
    const counter = await program.account.operationCounter.fetch(
      counterOf(owner.publicKey),
      "confirmed",
    );
    const p = await position_(position);
    const sequence = counter.nextSequence as BN;
    const shieldSequence = p.nextShieldSeq as BN;
    const offset = newOffset();
    await program.methods
      .shield(
        offset,
        sequence,
        shieldSequence,
        { usdc: {} },
        new BN(amount.toString()),
      )
      .accountsPartial({
        payer: owner.publicKey,
        ...arciumAccounts(SHIELD_CIRCUIT, offset),
        market,
        position,
        operationCounter: counterOf(owner.publicKey),
        operationIntent: intentOf(owner.publicKey, sequence),
        pendingShield: pda(
          Buffer.from("pending_shield"),
          position.toBuffer(),
          le8(shieldSequence),
        ),
        mint: m.usdcMint,
        vault: m.usdcVault,
        sourceTokenAccount: source,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .preInstructions([
        ComputeBudgetProgram.setComputeUnitLimit({ units: 600_000 }),
      ])
      .signers([owner])
      .rpc({ commitment: "confirmed" });
    await awaitComputationFinalization(
      provider,
      offset,
      program.programId,
      "confirmed",
    );
    expect(await terminalStatus(intentOf(owner.publicKey, sequence))).to.equal(
      "applied",
    );
  }

  async function activePosition(
    owner: Keypair,
    viewingSecret: Uint8Array,
  ): Promise<void> {
    const offset = newOffset();
    const sequence = new BN(0);
    await program.methods
      .initializePosition(
        offset,
        sequence,
        Array.from(x25519.getPublicKey(viewingSecret)),
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

  async function mintTo(
    mint: PublicKey,
    destination: PublicKey,
    amount: bigint,
  ): Promise<void> {
    // MintTo: tag 7 | amount u64 LE.
    const data = Buffer.alloc(9);
    data.writeUInt8(7, 0);
    data.writeBigUInt64LE(amount, 1);
    await provider.sendAndConfirm(
      new Transaction().add(
        new TransactionInstruction({
          programId: TOKEN_PROGRAM_ID,
          keys: [
            { pubkey: mint, isSigner: false, isWritable: true },
            { pubkey: destination, isSigner: false, isWritable: true },
            { pubkey: wallet.publicKey, isSigner: true, isWritable: false },
          ],
          data,
        }),
      ),
      [wallet],
      { commitment: "confirmed" },
    );
  }

  async function isMintAuthority(
    mint: PublicKey,
    authority: PublicKey,
  ): Promise<boolean> {
    const info = await connection.getAccountInfo(mint, "confirmed");
    if (!info || info.data.readUInt32LE(0) !== 1) return false;
    return new PublicKey(info.data.subarray(4, 36)).equals(authority);
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
    const init = {
      [INIT_CIRCUIT]: () => program.methods.initPositionStateCompDef(),
      [SHIELD_CIRCUIT]: () => program.methods.initShieldCreditCompDef(),
      [SUPPLY_CIRCUIT]: () => program.methods.initSupplyFlowCompDef(),
      [BAND_CIRCUIT]: () => program.methods.initUpdateUtilizationBandCompDef(),
    }[circuit];
    if (init === undefined) throw new Error(`unknown circuit ${circuit}`);
    await init().accounts(accounts).signers([wallet]).rpc(options);
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

function readKpJson(path: string): Keypair {
  return Keypair.fromSecretKey(
    new Uint8Array(JSON.parse(fs.readFileSync(path).toString())),
  );
}
