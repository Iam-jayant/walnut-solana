/**
 * Task 5.4: unshield USDC and wSOL (Requirements 5.7, 5.8, 5.9).
 *
 * Proves on localnet, against the Market created by `market-init.ts`, that:
 *   1. after a shield, `unshield` approves the requested amount and the
 *      callback transfers exactly that amount from the vault to the owner's
 *      destination token account, signed by the vault-authority PDA; the owner
 *      decrypts the new Position state locally and the free balance dropped by
 *      exactly the unshielded amount (USDC, then SOL as wSOL);
 *   2. an oversized request (more than the free balance) approves zero: the
 *      Intent ends `Zero`, no tokens move, and the stored state and nonce are
 *      unchanged;
 *   3. a destination that is not the owner's account of the asset's mint is
 *      rejected at queue time;
 *   4. the output nonce stored after an unshield is the one derived from the
 *      Position and the Intent sequence (no (key, nonce) repeats across
 *      attempts, `state::nonce`);
 *   5. native SOL comes back only through the separate owner-signed unwrap
 *      transaction built by the SDK (`buildUnwrapWsolTransaction`).
 *
 * Each run creates a fresh owner and Position, so it is idempotent. The USDC
 * part needs the provider wallet to be the USDC mint authority, which holds
 * when `market-init.ts` created the mint; otherwise it is skipped with a log
 * line. Abandonment after Lock expiry (1,800 slots) is covered by the Rust
 * tests in `programs/walnut/src/state/unshield/tests.rs`.
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
import { createHash, randomBytes } from "crypto";
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
import { buildUnwrapWsolTransaction } from "../packages/sdk/src/unwrap";

const TOKEN_PROGRAM_ID = new PublicKey(
  "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
);
const TOKEN_ACCOUNT_SIZE = 165;
const INIT_CIRCUIT = "init_position_state";
const SHIELD_CIRCUIT = "shield_credit";
const UNSHIELD_CIRCUIT = "unshield";
const USDC_SHIELD = 1_234_567n;
const USDC_UNSHIELD = 1_000_000n;
const SOL_SHIELD = 54_321_000n;
const SOL_UNSHIELD = 20_000_000n;
const OUTPUT_NONCE_DOMAIN = "walnut:position-output-nonce:v1";

type Asset = { sol: Record<string, never> } | { usdc: Record<string, never> };
type ParsedEvent = { name: string; data: Record<string, unknown> };

describe("Walnut Task 5.4: unshield USDC and wSOL", () => {
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
    await initCompDef(UNSHIELD_CIRCUIT);
  });

  it("unshields USDC and wSOL, approves zero when oversized, and unwraps", async function () {
    const m = await program.account.market.fetch(market, "confirmed");
    if (m.positionCount.gte(m.positionCountCap)) {
      console.log("      Position count cap reached; unshield test skipped");
      this.skip();
    }
    const owner = await fundedOwner(400_000_000);
    const viewingSecret = x25519.utils.randomSecretKey();
    const position = positionOf(owner.publicKey);
    await activePosition(owner, viewingSecret);
    const decrypt = await decryptor(viewingSecret);

    // ---- USDC -----------------------------------------------------------
    let freeUsdc = 0n;
    if (await isMintAuthority(m.usdcMint, wallet.publicKey)) {
      const usdcAccount = await tokenAccount(owner, m.usdcMint, 0);
      const usdcOut = await tokenAccount(owner, m.usdcMint, 0);
      await mintTo(m.usdcMint, usdcAccount, 5_000_000n);
      await shield(owner, {
        asset: { usdc: {} },
        mint: m.usdcMint,
        vault: m.usdcVault,
        source: usdcAccount,
        amount: USDC_SHIELD,
      });
      freeUsdc = USDC_SHIELD;
      expect((await decrypt(position))[1]).to.equal(freeUsdc, "free_usdc");

      // An unshield to someone else's token account is rejected.
      const strangerAccount = await tokenAccount(
        await fundedOwner(10_000_000),
        m.usdcMint,
        0,
      );
      await expectRejected(
        unshieldTx(owner, {
          asset: { usdc: {} },
          destination: strangerAccount,
          amount: 1n,
        }).then((u) => send(owner, u.instruction)),
        "UnshieldDestinationInvalid",
        "0x17e6",
      );

      // Approved: exactly the requested amount reaches the owner's account.
      const vaultBefore = await tokenBalance(m.usdcVault);
      const approved = await unshield(owner, {
        asset: { usdc: {} },
        destination: usdcOut,
        amount: USDC_UNSHIELD,
      });
      expect(approved.queuedEvents.map((e) => e.name)).to.deep.equal([
        "unshieldQueued",
      ]);
      await waitForIntentStatus(approved.intent, "applied");
      expect(await tokenBalance(usdcOut)).to.equal(USDC_UNSHIELD);
      expect(await tokenBalance(m.usdcVault)).to.equal(
        vaultBefore - USDC_UNSHIELD,
      );
      freeUsdc -= USDC_UNSHIELD;
      const afterUnshield = await decrypt(position);
      expect(afterUnshield[0]).to.equal(0n, "free_sol");
      expect(afterUnshield[1]).to.equal(freeUsdc, "free_usdc");
      expect(await storedNonce(position)).to.equal(
        derivedOutputNonce(position, approved.sequence),
        "output nonce is derived from the Intent sequence",
      );

      // Oversized: approves zero, nothing moves, state and nonce unchanged.
      const nonceBefore = await storedNonce(position);
      const vaultMid = await tokenBalance(m.usdcVault);
      const zero = await unshield(owner, {
        asset: { usdc: {} },
        destination: usdcOut,
        amount: freeUsdc + 1n,
      });
      await waitForIntentStatus(zero.intent, "zero");
      expect(await tokenBalance(usdcOut)).to.equal(USDC_UNSHIELD);
      expect(await tokenBalance(m.usdcVault)).to.equal(vaultMid);
      expect(await storedNonce(position)).to.equal(nonceBefore);
      expect((await decrypt(position))[1]).to.equal(freeUsdc, "free_usdc");
      const p = await program.account.position.fetch(position, "confirmed");
      expect(p.coordination.lock.active).to.equal(false);
    } else {
      console.log(
        "      USDC mint authority is not this wallet; USDC unshield skipped",
      );
    }

    // ---- SOL (wSOL out, then a separate unwrap) ---------------------------
    const wsolAccount = await tokenAccount(owner, m.wsolMint, 0);
    const wsolOut = await tokenAccount(owner, m.wsolMint, 0);
    const shielded = await shieldInstruction(owner, {
      asset: { sol: {} },
      mint: m.wsolMint,
      vault: m.wsolVault,
      source: wsolAccount,
      amount: SOL_SHIELD,
    });
    await provider.sendAndConfirm(
      new Transaction().add(
        SystemProgram.transfer({
          fromPubkey: owner.publicKey,
          toPubkey: wsolAccount,
          lamports: Number(SOL_SHIELD),
        }),
        syncNative(wsolAccount),
        shielded.instruction,
      ),
      [owner],
      { commitment: "confirmed" },
    );
    await awaitComputationFinalization(
      provider,
      shielded.offset,
      program.programId,
      "confirmed",
    );
    await waitForShieldStatus(shielded.pendingShield, "credited");
    expect((await decrypt(position))[0]).to.equal(SOL_SHIELD, "free_sol");

    const wsolVaultBefore = await tokenBalance(m.wsolVault);
    const sol = await unshield(owner, {
      asset: { sol: {} },
      destination: wsolOut,
      amount: SOL_UNSHIELD,
    });
    await waitForIntentStatus(sol.intent, "applied");
    expect(await tokenBalance(wsolOut)).to.equal(SOL_UNSHIELD);
    expect(await tokenBalance(m.wsolVault)).to.equal(
      wsolVaultBefore - SOL_UNSHIELD,
    );
    const state = await decrypt(position);
    expect(state[0]).to.equal(SOL_SHIELD - SOL_UNSHIELD, "free_sol");
    expect(state[1]).to.equal(freeUsdc, "free_usdc");
    expect(state.slice(2)).to.deep.equal([0n, 0n, 0n]);
    expect(await storedNonce(position)).to.equal(
      derivedOutputNonce(position, sol.sequence),
    );

    // Native SOL: a separate owner-signed transaction closes the wSOL account.
    const lamportsBefore = await connection.getBalance(
      owner.publicKey,
      "confirmed",
    );
    const rent =
      await connection.getMinimumBalanceForRentExemption(TOKEN_ACCOUNT_SIZE);
    const unwrap = buildUnwrapWsolTransaction(owner.publicKey, wsolOut);
    await provider.sendAndConfirm(unwrap, [owner], {
      commitment: "confirmed",
    });
    expect(await connection.getAccountInfo(wsolOut, "confirmed")).to.equal(
      null,
      "wSOL account closed",
    );
    const lamportsAfter = await connection.getBalance(
      owner.publicKey,
      "confirmed",
    );
    // The unwrapped SOL plus the rent returns, minus the transaction fee.
    const gained = BigInt(lamportsAfter - lamportsBefore);
    expect(gained >= SOL_UNSHIELD + BigInt(rent) - 10_000n).to.equal(
      true,
      `owner regained ${gained} lamports`,
    );
  });

  type UnshieldArgs = { asset: Asset; destination: PublicKey; amount: bigint };

  async function unshieldTx(owner: Keypair, args: UnshieldArgs) {
    const position = positionOf(owner.publicKey);
    const m = await program.account.market.fetch(market, "confirmed");
    const counter = await program.account.operationCounter.fetch(
      counterOf(owner.publicKey),
      "confirmed",
    );
    const sequence = counter.nextSequence as BN;
    const offset = newOffset();
    const intent = intentOf(owner.publicKey, sequence);
    const instruction = await program.methods
      .unshield(offset, sequence, args.asset, new BN(args.amount.toString()))
      .accountsPartial({
        payer: owner.publicKey,
        ...arciumAccounts(UNSHIELD_CIRCUIT, offset),
        market,
        assetState: m.assetState,
        position,
        operationCounter: counterOf(owner.publicKey),
        operationIntent: intent,
        destination: args.destination,
      })
      .instruction();
    return { sequence, offset, intent, instruction };
  }

  async function send(owner: Keypair, instruction: TransactionInstruction) {
    return provider.sendAndConfirm(
      new Transaction().add(instruction),
      [owner],
      { commitment: "confirmed" },
    );
  }

  async function unshield(owner: Keypair, args: UnshieldArgs) {
    const { sequence, offset, intent, instruction } = await unshieldTx(
      owner,
      args,
    );
    const signature = await send(owner, instruction);
    const queuedEvents = await events(signature);
    await awaitComputationFinalization(
      provider,
      offset,
      program.programId,
      "confirmed",
    );
    return { sequence, intent, queuedEvents };
  }

  async function waitForIntentStatus(intent: PublicKey, status: string) {
    for (let attempt = 0; attempt < 90; attempt++) {
      const account = await program.account.operationIntent.fetch(
        intent,
        "confirmed",
      );
      if (status in account.status) return account;
      await sleep(500);
    }
    throw new Error(`Operation Intent did not reach ${status}`);
  }

  type ShieldArgs = {
    asset: Asset;
    mint: PublicKey;
    vault: PublicKey;
    source: PublicKey;
    amount: bigint;
  };

  async function shieldInstruction(owner: Keypair, args: ShieldArgs) {
    const position = positionOf(owner.publicKey);
    const m = await program.account.market.fetch(market, "confirmed");
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
        args.asset,
        new BN(args.amount.toString()),
      )
      .accountsPartial({
        payer: owner.publicKey,
        ...arciumAccounts(SHIELD_CIRCUIT, offset),
        market,
        assetState: m.assetState,
        position,
        operationCounter: counterOf(owner.publicKey),
        operationIntent: intentOf(owner.publicKey, sequence),
        pendingShield,
        mint: args.mint,
        vault: args.vault,
        sourceTokenAccount: args.source,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .instruction();
    return { pendingShield, offset, instruction };
  }

  async function shield(owner: Keypair, args: ShieldArgs) {
    const { pendingShield, offset, instruction } = await shieldInstruction(
      owner,
      args,
    );
    await send(owner, instruction);
    await awaitComputationFinalization(
      provider,
      offset,
      program.programId,
      "confirmed",
    );
    await waitForShieldStatus(pendingShield, "credited");
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

  /** The stored output nonce of a Position as a u128. */
  async function storedNonce(position: PublicKey): Promise<bigint> {
    const p = await program.account.position.fetch(position, "confirmed");
    return BigInt((p.encryptedState.nonce as BN).toString());
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

  async function waitForShieldStatus(pendingShield: PublicKey, status: string) {
    for (let attempt = 0; attempt < 60; attempt++) {
      const account = await program.account.pendingShield.fetch(
        pendingShield,
        "confirmed",
      );
      if (status in account.status) return account;
      await sleep(500);
    }
    throw new Error(`Pending Shield did not reach ${status}`);
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

  function syncNative(account: PublicKey): TransactionInstruction {
    // SyncNative: tag 17.
    return new TransactionInstruction({
      programId: TOKEN_PROGRAM_ID,
      keys: [{ pubkey: account, isSigner: false, isWritable: true }],
      data: Buffer.from([17]),
    });
  }

  async function isMintAuthority(
    mint: PublicKey,
    authority: PublicKey,
  ): Promise<boolean> {
    const info = await connection.getAccountInfo(mint, "confirmed");
    if (!info || info.data.readUInt32LE(0) !== 1) return false;
    return new PublicKey(info.data.subarray(4, 36)).equals(authority);
  }

  async function tokenBalance(account: PublicKey): Promise<bigint> {
    const info = await connection.getAccountInfo(account, "confirmed");
    expect(info, "token account").to.not.equal(null);
    return info!.data.readBigUInt64LE(64);
  }

  async function events(signature: string): Promise<ParsedEvent[]> {
    const tx = await connection.getTransaction(signature, {
      commitment: "confirmed",
      maxSupportedTransactionVersion: 0,
    });
    expect(tx?.meta?.logMessages, "transaction logs").to.not.equal(undefined);
    return [...parser.parseLogs(tx!.meta!.logMessages!)] as ParsedEvent[];
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
    const register =
      circuit === INIT_CIRCUIT
        ? program.methods.initPositionStateCompDef()
        : circuit === SHIELD_CIRCUIT
          ? program.methods.initShieldCreditCompDef()
          : program.methods.initUnshieldCompDef();
    await register.accounts(accounts).signers([wallet]).rpc(options);
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
