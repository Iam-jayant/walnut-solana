/**
 * Task 5.3: shield SOL and USDC (Requirements 5.4-5.6, 5.9).
 *
 * Proves on localnet, against the Market created by `market-init.ts`, that:
 *   1. `shield` moves exactly the public amount from the owner's token
 *      account into the Market vault, opens a `Deposited` Pending Shield,
 *      and queues `shield_credit`;
 *   2. the callback credits the encrypted Position: the owner decrypts it
 *      locally and `free_usdc` / `free_sol` grew by exactly the shielded
 *      amounts, and the Pending Shield is `Credited`;
 *   3. SOL is wrapped in the same transaction (System transfer into the
 *      owner's wSOL token account, then `SyncNative`, then `shield`);
''s Market vault is rejected.
 *
 * Each run creates a fresh owner and Position (the Viewing secret of the
 * `position-init.ts` Position is not persisted, so its state could not be
 * decrypted here). The USDC part needs the provider wallet to be the USDC mint
 * authority, which holds when `market-init.ts` created the mint; otherwise it
 * is skipped with a log line. Refund after a failed attempt needs a failed
 * credit or Lock expiry (1,800 slots); it is covered by the Rust tests in
 * `programs/walnut/src/state/shield.rs`.
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

const TOKEN_PROGRAM_ID = new PublicKey(
  "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
);
const TOKEN_ACCOUNT_SIZE = 165;
const INIT_CIRCUIT = "init_position_state";
const SHIELD_CIRCUIT = "shield_credit";
const USDC_AMOUNT = 1_234_567n;
const SOL_AMOUNT = 54_321_000n;
const OUTPUT_NONCE_DOMAIN = "walnut:position-output-nonce:v1";

type ParsedEvent = { name: string; data: Record<string, unknown> };

describe("Walnut Task 5.3: shield SOL and USDC", () => {
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
  });

  it("credits shielded USDC and SOL to the encrypted Position", async function () {
    const m = await program.account.market.fetch(market, "confirmed");
    if (m.positionCount.gte(m.positionCountCap)) {
      console.log("      Position count cap reached; shield test skipped");
      this.skip();
    }
    const owner = await fundedOwner(400_000_000);
    const viewingSecret = x25519.utils.randomSecretKey();
    const position = positionOf(owner.publicKey);
    await activePosition(owner, viewingSecret);
    const decrypt = await decryptor(viewingSecret);

    // USDC: only when this wallet owns the USDC mint authority.
    let expectedUsdc = 0n;
    if (await isMintAuthority(m.usdcMint, wallet.publicKey)) {
      const usdcAccount = await tokenAccount(owner, m.usdcMint, 0);
      await mintTo(m.usdcMint, usdcAccount, 5_000_000n);
      const vaultBefore = await tokenBalance(m.usdcVault);
      const { pendingShield, signature } = await shield(owner, {
        asset: { usdc: {} },
        mint: m.usdcMint,
        vault: m.usdcVault,
        source: usdcAccount,
        amount: USDC_AMOUNT,
      });
      const queued = await events(signature);
      expect(queued.map((e) => e.name)).to.deep.equal(["shieldQueued"]);
      expect(new BN(queued[0].data.amount as BN).toString()).to.equal(
        USDC_AMOUNT.toString(),
      );
      expect(await tokenBalance(m.usdcVault)).to.equal(
        vaultBefore + USDC_AMOUNT,
      );
      expect(await tokenBalance(usdcAccount)).to.equal(
        5_000_000n - USDC_AMOUNT,
      );
      await waitForShieldStatus(pendingShield, "credited");
      expectedUsdc = USDC_AMOUNT;
      const state = await decrypt(position);
      expect(state[0]).to.equal(0n, "free_sol");
      expect(state[1]).to.equal(expectedUsdc, "free_usdc");
    } else {
      console.log(
        "      USDC mint authority is not this wallet; USDC shield skipped",
      );
    }

    // SOL: wrap and shield in one transaction.
    const wsolAccount = await tokenAccount(owner, m.wsolMint, 0);
    const wsolVaultBefore = await tokenBalance(m.wsolVault);
    const { pendingShield, offset, instruction, sequence } =
      await shieldInstruction(owner, {
        asset: { sol: {} },
        mint: m.wsolMint,
        vault: m.wsolVault,
        source: wsolAccount,
        amount: SOL_AMOUNT,
      });
    const tx = new Transaction().add(
      SystemProgram.transfer({
        fromPubkey: owner.publicKey,
        toPubkey: wsolAccount,
        lamports: Number(SOL_AMOUNT),
      }),
      syncNative(wsolAccount),
      instruction,
    );
    await provider.sendAndConfirm(tx, [owner], { commitment: "confirmed" });
    expect(await tokenBalance(m.wsolVault)).to.equal(
      wsolVaultBefore + SOL_AMOUNT,
    );
    expect(await tokenBalance(wsolAccount)).to.equal(0n);
    await awaitComputationFinalization(
      provider,
      offset,
      program.programId,
      "confirmed",
    );
    const credited = await waitForShieldStatus(pendingShield, "credited");
    expect(credited.attemptSequence.toNumber()).to.equal(1);
    const state = await decrypt(position);
    expect(state[0]).to.equal(SOL_AMOUNT, "free_sol");
    expect(state[1]).to.equal(expectedUsdc, "free_usdc");
    expect(state.slice(2)).to.deep.equal([0n, 0n, 0n]);
    const stored = await program.account.position.fetch(position, "confirmed");
    expect(BigInt((stored.encryptedState.nonce as BN).toString())).to.equal(
      derivedOutputNonce(position, sequence),
      "stored output nonce is derived from the Intent sequence",
    );

    // A credited shield cannot be refunded.
    await expectRejected(
      program.methods
        .refundShield()
        .accountsPartial({
          owner: owner.publicKey,
          market,
          vaultAuthority: m.vaultAuthority,
          pendingShield,
          activeIntent: credited.activeIntent,
          mint: m.wsolMint,
          vault: m.wsolVault,
          destination: wsolAccount,
        })
        .signers([owner])
        .rpc({ commitment: "confirmed" }),
      "PendingShieldNotDeposited",
    );
    // A vault that is not the asset's Market vault is rejected.
    const bad = await shieldInstruction(owner, {
      asset: { sol: {} },
      mint: m.wsolMint,
      vault: m.usdcVault,
      source: wsolAccount,
      amount: 1n,
    });
    await expectRejected(
      provider.sendAndConfirm(new Transaction().add(bad.instruction), [owner], {
        commitment: "confirmed",
      }),
      "ShieldVaultMismatch",
      "0x17dc",
    );
    const after = await program.account.position.fetch(position, "confirmed");
    expect(after.coordination.lock.active).to.equal(false);
  });

  type ShieldArgs = {
    asset: { sol: Record<string, never> } | { usdc: Record<string, never> };
    mint: PublicKey;
    vault: PublicKey;
    source: PublicKey;
    amount: bigint;
  };

  async function shieldInstruction(owner: Keypair, args: ShieldArgs) {
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
        args.asset,
        new BN(args.amount.toString()),
      )
      .accountsPartial({
        payer: owner.publicKey,
        ...arciumAccounts(SHIELD_CIRCUIT, offset),
        market,
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
    return { pendingShield, offset, instruction, sequence };
  }

  async function shield(owner: Keypair, args: ShieldArgs) {
    const { pendingShield, offset, instruction } = await shieldInstruction(
      owner,
      args,
    );
    const signature = await provider.sendAndConfirm(
      new Transaction().add(instruction),
      [owner],
      { commitment: "confirmed" },
    );
    await awaitComputationFinalization(
      provider,
      offset,
      program.programId,
      "confirmed",
    );
    return { pendingShield, signature };
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
    if (circuit === INIT_CIRCUIT) {
      await program.methods
        .initPositionStateCompDef()
        .accounts(accounts)
        .signers([wallet])
        .rpc(options);
    } else {
      await program.methods
        .initShieldCreditCompDef()
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
