// walnut-diffn task 2: devnet private loan demo.
//
// add collateral -> borrow -> refused over-limit borrow -> unshield the loan ->
// shield it back -> repay -> withdraw collateral -> unshield wSOL -> unwrap.
// After EVERY step the Position is decrypted locally and compared with the
// expected five values (computed with applyPositionOp and the shield/unshield
// effects); any mismatch aborts.
//
// Resumable: progress lives in loan-state.json next to the balance demo state
// (default ~/.walnut-demo/devnet, override WALNUT_DEMO_DIR). RPC and wallet
// come from ANCHOR_PROVIDER_URL / ANCHOR_WALLET like the balance demo.
//
//   pnpm --filter @walnut/sdk demo:loan
//
// Lender liquidity is a stand-in: the deployer (test USDC mint authority)
// mints test USDC straight into the market USDC vault.

import * as anchor from "@anchor-lang/core";
import { BN } from "@anchor-lang/core";
import {
  ComputeBudgetProgram,
  Keypair,
  LAMPORTS_PER_SOL,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";
import { createPrivateKey, randomBytes, sign } from "node:crypto";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import {
  createContext,
  resolveOptions,
  type BenchContext,
} from "../bench/config.ts";
import { ensureCompDef, type InitCompDefBuilder } from "../bench/program.ts";
import {
  deriveArciumQueueAccounts,
  deriveIntentPda,
  deriveOperationCounterPda,
} from "../src/pda.ts";
import { applyPositionOp, PositionOp } from "../src/position-op.ts";
import { buildUnwrapWsolTransaction } from "../src/unwrap.ts";
import {
  decryptPosition,
  deriveViewingKey,
  viewingKeyMessageBytes,
  type PositionBalances,
} from "../src/viewing-key.ts";

const NETWORK = "devnet";
const TOKEN_PROGRAM_ID = new PublicKey(
  "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
);
const ATA_PROGRAM_ID = new PublicKey(
  "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
);
const COMP_DEFS: ReadonlyArray<[string, string]> = [
  ["init_position_state", "initPositionStateCompDef"],
  ["shield_credit", "initShieldCreditCompDef"],
  ["unshield", "initUnshieldCompDef"],
  ["position_op", "initPositionOpCompDef"],
];

const PRICE = 150_000_000n; // $150 per SOL, USDC base units
const LTV_BPS = 5_000n;
const SOL_AMOUNT = 200_000_000n;
const BORROW = 10_000_000n;
const OVER_BORROW = 100_000_000n;
const VAULT_MIN = 20_000_000n;
const OWNER_MIN_LAMPORTS = 250_000_000;
const OWNER_TOPUP_TO = 400_000_000;
const COMPUTE_UNIT_LIMIT = 600_000;
const INTENT_TIMEOUT_MS = 240_000;

// ------------------------------------------------------------------ logging

let secretStrings: string[] = [];
function redact(text: string): string {
  let out = text.replace(/api-key=[^&\s"')]+/gi, "api-key=REDACTED");
  for (const secret of secretStrings) {
    if (secret.length > 0) out = out.split(secret).join("<rpc>");
  }
  return out;
}
function log(message: string): void {
  console.log(redact(message));
}
function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

const TRANSIENT =
  /429|too many|fetch failed|econn|etimedout|enotfound|timed out|socket|503|502|504|blockhash not found|failed to get|network|rate limit|und_err|block height exceeded|expired/i;

async function retry<T>(
  label: string,
  fn: () => Promise<T>,
  attempts = 6,
): Promise<T> {
  let last: unknown;
  for (let i = 0; i < attempts; i += 1) {
    try {
      return await fn();
    } catch (err) {
      last = err;
      const text = err instanceof Error ? err.message : String(err);
      if (!TRANSIENT.test(text) || i === attempts - 1) break;
      log(`  (retry ${i + 1}/${attempts - 1} ${label}: ${text.slice(0, 80)})`);
      await sleep(Math.min(1000 * 2 ** i, 15_000));
    }
  }
  throw last;
}

async function waitFor<T>(
  label: string,
  fn: () => Promise<T | undefined>,
  timeoutMs: number,
  intervalMs = 1500,
): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    try {
      const value = await fn();
      if (value !== undefined) return value;
    } catch (err) {
      const text = err instanceof Error ? err.message : String(err);
      if (!TRANSIENT.test(text)) throw err;
    }
    if (Date.now() > deadline) {
      throw new Error(`timed out after ${timeoutMs} ms waiting for ${label}`);
    }
    await sleep(intervalMs);
  }
}

// -------------------------------------------------------------------- types

type Json = Record<string, unknown>;
interface MethodBuilder {
  accountsPartial(accounts: Record<string, PublicKey>): MethodBuilder;
  instruction(): Promise<TransactionInstruction>;
}
type MethodFn = (...args: unknown[]) => MethodBuilder;
interface AccountClient {
  fetchNullable(address: PublicKey, commitment?: string): Promise<unknown>;
}
interface MarketAccount {
  assetState: PublicKey;
  wsolMint: PublicKey;
  usdcMint: PublicKey;
  wsolVault: PublicKey;
  usdcVault: PublicKey;
}
interface PositionAccount {
  status: Json;
  nextShieldSeq: BN;
  encryptedState: { nonce: BN; ciphertexts: number[][] };
}
interface StatusAccount {
  status: Json;
}
interface CounterAccount {
  nextSequence: BN;
}

type Bal = PositionBalances;

interface StepResult {
  name: string;
  signature?: string | undefined;
  ms: number;
  detail: string;
  /** freeSol, freeUsdc, collateral, debt as decimal strings. */
  decrypted: [string, string, string, string];
}

interface PendingOp {
  sequence: string;
  offset: string;
  shieldSequence: string;
}

interface LoanState {
  programId: string;
  start?: Record<keyof Bal, string>;
  ownerFundedLamports: string;
  ownerReturned: boolean;
  steps: Record<string, StepResult>;
  pending: Record<string, PendingOp>;
}

// ------------------------------------------------------------------ helpers

function stateDir(): string {
  return (
    process.env["WALNUT_DEMO_DIR"] ??
    path.join(os.homedir(), ".walnut-demo", NETWORK)
  );
}
function statusName(status: Json): string {
  return Object.keys(status)[0] ?? "unknown";
}
function sol(lamports: bigint | number): string {
  return (Number(lamports) / LAMPORTS_PER_SOL).toFixed(6);
}
function explorer(signature: string): string {
  return `https://explorer.solana.com/tx/${signature}?cluster=devnet`;
}
function expectEq(label: string, actual: bigint, expected: bigint): void {
  if (actual !== expected) {
    throw new Error(`${label}: expected ${expected}, got ${actual}`);
  }
}
function signMessage(keypair: Keypair, message: Uint8Array): Uint8Array {
  const prefix = Buffer.from("302e020100300506032b657004220420", "hex");
  const der = Buffer.concat([
    prefix,
    Buffer.from(keypair.secretKey.slice(0, 32)),
  ]);
  const key = createPrivateKey({ key: der, format: "der", type: "pkcs8" });
  return new Uint8Array(sign(null, Buffer.from(message), key));
}
function newOffset(): BN {
  return new BN(randomBytes(8));
}
function tokenAmountOf(data: Buffer): bigint {
  return data.readBigUInt64LE(64);
}
const seedBytes = (s: string): Uint8Array => new TextEncoder().encode(s);

// -------------------------------------------------------------------- demo

class LoanDemo {
  readonly ctx: BenchContext;
  readonly state: LoanState;
  readonly programId: PublicKey;
  readonly deployer: Keypair;
  readonly owner: Keypair;
  readonly market: PublicKey;
  readonly position: PublicKey;
  readonly counter: PublicKey;
  viewingSecret: Uint8Array | undefined;
  mk!: MarketAccount;

  constructor(ctx: BenchContext) {
    this.ctx = ctx;
    this.programId = ctx.programId;
    this.deployer = ctx.payer;
    const file = path.join(stateDir(), "loan-state.json");
    const loaded = fs.existsSync(file)
      ? (JSON.parse(fs.readFileSync(file, "utf8")) as LoanState)
      : undefined;
    this.state =
      loaded && loaded.programId === ctx.programId.toBase58()
        ? loaded
        : {
            programId: ctx.programId.toBase58(),
            ownerFundedLamports: "0",
            ownerReturned: false,
            steps: {},
            pending: {},
          };
    // The owner keypair created by the balance demo.
    const ownerFile = path.join(stateDir(), "owner.json");
    if (!fs.existsSync(ownerFile)) {
      throw new Error(`${ownerFile} not found: run demo:balance first`);
    }
    this.owner = Keypair.fromSecretKey(
      new Uint8Array(JSON.parse(fs.readFileSync(ownerFile, "utf8"))),
    );
    const pda = (...seeds: Uint8Array[]): PublicKey =>
      PublicKey.findProgramAddressSync(seeds, this.programId)[0];
    this.market = pda(seedBytes("market"));
    this.position = pda(
      seedBytes("position"),
      this.market.toBytes(),
      this.owner.publicKey.toBytes(),
    );
    this.counter = deriveOperationCounterPda(
      this.programId,
      this.owner.publicKey,
    );
  }

  private save(): void {
    fs.mkdirSync(stateDir(), { recursive: true });
    fs.writeFileSync(
      path.join(stateDir(), "loan-state.json"),
      JSON.stringify(this.state, null, 2),
    );
  }

  private method(name: string): MethodFn {
    const methods = this.ctx.program.methods as unknown as Record<
      string,
      MethodFn | undefined
    >;
    const fn = methods[name];
    if (fn === undefined) throw new Error(`IDL has no instruction ${name}`);
    return (...args) => fn.call(this.ctx.program.methods, ...args);
  }

  private async fetch<T>(name: string, address: PublicKey): Promise<T | null> {
    const client = (
      this.ctx.program.account as unknown as Record<
        string,
        AccountClient | undefined
      >
    )[name];
    if (client === undefined) throw new Error(`IDL has no account ${name}`);
    return retry(`fetch ${name}`, async () => {
      return (await client.fetchNullable(address, "confirmed")) as T | null;
    });
  }

  private balanceOf(address: PublicKey): Promise<number> {
    return retry("getBalance", () =>
      this.ctx.connection.getBalance(address, "confirmed"),
    );
  }

  private async tokenBalance(address: PublicKey): Promise<bigint> {
    const info = await retry("getAccountInfo", () =>
      this.ctx.connection.getAccountInfo(address, "confirmed"),
    );
    if (info === null) throw new Error(`token account ${address} not found`);
    return tokenAmountOf(info.data);
  }

  private async send(
    instructions: TransactionInstruction[],
    signers: Keypair[],
  ): Promise<{ signature: string; ms: number }> {
    const started = Date.now();
    const signature = await retry("send", async () => {
      const tx = new Transaction().add(
        ComputeBudgetProgram.setComputeUnitLimit({ units: COMPUTE_UNIT_LIMIT }),
        ...instructions,
      );
      tx.feePayer = (signers[0] as Keypair).publicKey;
      const { blockhash, lastValidBlockHeight } =
        await this.ctx.connection.getLatestBlockhash("confirmed");
      tx.recentBlockhash = blockhash;
      tx.sign(...signers);
      let sig: string;
      try {
        sig = await this.ctx.connection.sendRawTransaction(tx.serialize(), {
          preflightCommitment: "confirmed",
          maxRetries: 5,
        });
      } catch (err) {
        const logs = (err as { logs?: string[] }).logs;
        if (logs) {
          throw new Error(
            `${(err as Error).message}\n${logs.slice(-12).join("\n")}`,
            { cause: err },
          );
        }
        throw err;
      }
      const res = await this.ctx.connection.confirmTransaction(
        { signature: sig, blockhash, lastValidBlockHeight },
        "confirmed",
      );
      if (res.value.err) {
        throw new Error(
          `transaction ${sig} failed: ${JSON.stringify(res.value.err)}`,
        );
      }
      return sig;
    });
    return { signature, ms: Date.now() - started };
  }

  private queueAccounts(circuit: string, offset: BN) {
    return {
      ...deriveArciumQueueAccounts({
        programId: this.programId,
        clusterOffset: this.ctx.clusterOffset,
        computationOffset: offset,
        circuit,
      }),
    };
  }

  private ata(mint: PublicKey): PublicKey {
    return PublicKey.findProgramAddressSync(
      [
        this.owner.publicKey.toBytes(),
        TOKEN_PROGRAM_ID.toBytes(),
        mint.toBytes(),
      ],
      ATA_PROGRAM_ID,
    )[0];
  }

  private async ensureAta(mint: PublicKey): Promise<void> {
    const address = this.ata(mint);
    const existing = await retry("ata", () =>
      this.ctx.connection.getAccountInfo(address, "confirmed"),
    );
    if (existing !== null) return;
    const ix = new TransactionInstruction({
      programId: ATA_PROGRAM_ID,
      keys: [
        { pubkey: this.owner.publicKey, isSigner: true, isWritable: true },
        { pubkey: address, isSigner: false, isWritable: true },
        { pubkey: this.owner.publicKey, isSigner: false, isWritable: false },
        { pubkey: mint, isSigner: false, isWritable: false },
        { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
        { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
      ],
      data: Buffer.from([1]), // CreateIdempotent
    });
    await this.send([ix], [this.owner]);
  }

  private mintTo(
    mint: PublicKey,
    dest: PublicKey,
    amount: bigint,
  ): TransactionInstruction {
    const data = Buffer.alloc(9);
    data.writeUInt8(7, 0); // MintTo
    data.writeBigUInt64LE(amount, 1);
    return new TransactionInstruction({
      programId: TOKEN_PROGRAM_ID,
      keys: [
        { pubkey: mint, isSigner: false, isWritable: true },
        { pubkey: dest, isSigner: false, isWritable: true },
        { pubkey: this.deployer.publicKey, isSigner: true, isWritable: false },
      ],
      data,
    });
  }

  private async decrypt(): Promise<Bal> {
    const position = await this.fetch<PositionAccount>(
      "position",
      this.position,
    );
    if (position === null) throw new Error("Position missing");
    if (this.viewingSecret === undefined) throw new Error("no viewing key");
    return decryptPosition({
      encryptedState: {
        nonce: BigInt(position.encryptedState.nonce.toString()),
        ciphertexts: position.encryptedState.ciphertexts,
      },
      viewingSecret: this.viewingSecret,
      mxePublicKey: this.ctx.mxePublicKey,
    });
  }

  // ---- setup

  async compDefs(): Promise<void> {
    log("[a] computation definitions");
    for (const [circuit, initName] of COMP_DEFS) {
      const builder = this.method(initName)() as unknown as InitCompDefBuilder;
      const started = Date.now();
      const result = await retry(
        `comp def ${circuit}`,
        () =>
          ensureCompDef(
            this.ctx.provider,
            this.programId,
            this.ctx.repoRoot,
            circuit,
            builder,
            this.deployer,
          ),
        4,
      );
      log(`  comp def ${circuit}: ${result} (${Date.now() - started} ms)`);
    }
  }

  async setup(): Promise<void> {
    const market = await this.fetch<MarketAccount>("market", this.market);
    if (market === null) throw new Error("Market missing: run demo:balance");
    this.mk = market;

    log("[b] lender liquidity (stand-in: deployer mints test USDC to vault)");
    const vault = await this.tokenBalance(market.usdcVault);
    if (vault < VAULT_MIN) {
      const amount = VAULT_MIN - vault;
      const sent = await this.send(
        [this.mintTo(market.usdcMint, market.usdcVault, amount)],
        [this.deployer],
      );
      log(`  minted ${Number(amount) / 1e6} test USDC into the vault`);
      log(`    ${explorer(sent.signature)}`);
    } else {
      log(`  vault already holds ${Number(vault) / 1e6} test USDC`);
    }

    log("[c] borrower");
    const have = await this.balanceOf(this.owner.publicKey);
    if (have < OWNER_MIN_LAMPORTS && this.state.start === undefined) {
      const top = OWNER_TOPUP_TO - have;
      await this.send(
        [
          SystemProgram.transfer({
            fromPubkey: this.deployer.publicKey,
            toPubkey: this.owner.publicKey,
            lamports: top,
          }),
        ],
        [this.deployer],
      );
      this.state.ownerFundedLamports = (
        BigInt(this.state.ownerFundedLamports) + BigInt(top)
      ).toString();
      this.save();
      log(`  funded owner with ${sol(top)} SOL`);
    } else if (have < OWNER_MIN_LAMPORTS) {
      const top = OWNER_TOPUP_TO - have;
      await this.send(
        [
          SystemProgram.transfer({
            fromPubkey: this.deployer.publicKey,
            toPubkey: this.owner.publicKey,
            lamports: top,
          }),
        ],
        [this.deployer],
      );
      this.state.ownerFundedLamports = (
        BigInt(this.state.ownerFundedLamports) + BigInt(top)
      ).toString();
      this.state.ownerReturned = false;
      this.save();
      log(`  funded owner with ${sol(top)} SOL`);
    }

    const message = viewingKeyMessageBytes({
      programId: this.programId,
      network: NETWORK,
      positionPda: this.position,
      epoch: 0n,
    });
    const material = await deriveViewingKey(signMessage(this.owner, message), {
      wipeSignature: true,
    });
    this.viewingSecret = material.secret;

    const position = await this.fetch<PositionAccount>(
      "position",
      this.position,
    );
    if (position === null || statusName(position.status) !== "active") {
      throw new Error("demo owner Position is not Active: run demo:balance");
    }
    if (this.state.start === undefined) {
      const s = await this.decrypt();
      this.state.start = {
        freeSol: s.freeSol.toString(),
        freeUsdc: s.freeUsdc.toString(),
        collateralSol: s.collateralSol.toString(),
        supplySharesUsdc: s.supplySharesUsdc.toString(),
        debtSharesUsdc: s.debtSharesUsdc.toString(),
      };
      this.save();
    }
  }

  startBal(): Bal {
    const s = this.state.start;
    if (s === undefined) throw new Error("no start state");
    return {
      freeSol: BigInt(s.freeSol),
      freeUsdc: BigInt(s.freeUsdc),
      collateralSol: BigInt(s.collateralSol),
      supplySharesUsdc: BigInt(s.supplySharesUsdc),
      debtSharesUsdc: BigInt(s.debtSharesUsdc),
    };
  }

  // ---- generic queued operation

  private async runOperation(
    name: string,
    build: (p: {
      sequence: BN;
      offset: BN;
      shieldSequence: BN;
      intent: PublicKey;
    }) => Promise<TransactionInstruction[]>,
    expectStatus: "applied" | "zero",
    isShield = false,
  ): Promise<{ signature?: string | undefined; ms: number }> {
    const started = Date.now();
    let pending = this.state.pending[name];
    if (pending === undefined) {
      const counter = await this.fetch<CounterAccount>(
        "operationCounter",
        this.counter,
      );
      const position = await this.fetch<PositionAccount>(
        "position",
        this.position,
      );
      pending = {
        sequence: (counter?.nextSequence ?? new BN(0)).toString(),
        offset: newOffset().toString(),
        shieldSequence: position?.nextShieldSeq.toString() ?? "0",
      };
      this.state.pending[name] = pending;
      this.save();
    }
    const sequence = new BN(pending.sequence);
    const offset = new BN(pending.offset);
    const shieldSequence = new BN(pending.shieldSequence);
    const intent = deriveIntentPda(
      this.programId,
      this.owner.publicKey,
      BigInt(pending.sequence),
    );
    let signature: string | undefined;
    const existing = await this.fetch<StatusAccount>("operationIntent", intent);
    if (existing === null) {
      const instructions = await build({
        sequence,
        offset,
        shieldSequence,
        intent,
      });
      const sent = await this.send(instructions, [this.owner]);
      signature = sent.signature;
      log(`  ${name}: queued ${explorer(signature)}`);
    } else {
      log(`  ${name}: Intent exists, resuming wait`);
    }
    const terminal = await waitFor(
      `${name} Intent terminal`,
      async () => {
        const i = await this.fetch<StatusAccount>("operationIntent", intent);
        return i !== null && statusName(i.status) !== "queued"
          ? statusName(i.status)
          : undefined;
      },
      INTENT_TIMEOUT_MS,
    );
    if (terminal !== expectStatus) {
      throw new Error(
        `${name}: Intent ended ${terminal}, expected ${expectStatus}`,
      );
    }
    if (isShield) {
      const ps = PublicKey.findProgramAddressSync(
        [
          seedBytes("pending_shield"),
          this.position.toBytes(),
          shieldSequence.toArrayLike(Buffer, "le", 8),
        ],
        this.programId,
      )[0];
      await waitFor(
        "Pending Shield credited",
        async () => {
          const p = await this.fetch<StatusAccount>("pendingShield", ps);
          return p !== null && statusName(p.status) === "credited"
            ? p
            : undefined;
        },
        60_000,
      );
    }
    return { signature, ms: Date.now() - started };
  }

  /** Decrypts, asserts the five values against `exp`, records the step. */
  private async finish(
    name: string,
    exp: Bal,
    r: { signature?: string | undefined; ms: number },
    detail: string,
  ): Promise<void> {
    const got = await this.decrypt();
    expectEq(`${name}: free_sol`, got.freeSol, exp.freeSol);
    expectEq(`${name}: free_usdc`, got.freeUsdc, exp.freeUsdc);
    expectEq(`${name}: collateral_sol`, got.collateralSol, exp.collateralSol);
    expectEq(
      `${name}: supply_shares`,
      got.supplySharesUsdc,
      exp.supplySharesUsdc,
    );
    expectEq(`${name}: debt`, got.debtSharesUsdc, exp.debtSharesUsdc);
    this.state.steps[name] = {
      name,
      signature: r.signature,
      ms: r.ms,
      detail,
      decrypted: [
        got.freeSol.toString(),
        got.freeUsdc.toString(),
        got.collateralSol.toString(),
        got.debtSharesUsdc.toString(),
      ],
    };
    delete this.state.pending[name];
    this.save();
    log(`  step ${name}: ok, state matches expected (${detail})`);
  }

  private done(name: string): boolean {
    if (this.state.steps[name] === undefined) return false;
    log(`  step ${name}: already done (skipped)`);
    return true;
  }

  private async positionOp(
    name: string,
    op: PositionOp,
    amount: bigint,
    cur: Bal,
  ): Promise<Bal> {
    const { state: exp, applied } = applyPositionOp(
      cur,
      op,
      amount,
      PRICE,
      LTV_BPS,
    );
    if (this.done(name)) return exp;
    const r = await this.runOperation(
      name,
      async ({ sequence, offset, intent }) => [
        await this.method("positionOp")(
          offset,
          sequence,
          op,
          new BN(amount.toString()),
          new BN(PRICE.toString()),
        )
          .accountsPartial({
            payer: this.owner.publicKey,
            ...this.queueAccounts("position_op", offset),
            market: this.market,
            assetState: this.mk.assetState,
            position: this.position,
            operationCounter: this.counter,
            operationIntent: intent,
          })
          .instruction(),
      ],
      applied ? "applied" : "zero",
    );
    await this.finish(
      name,
      exp,
      r,
      applied ? "applied" : "refused, state unchanged",
    );
    return exp;
  }

  private async shield(
    name: string,
    asset: "sol" | "usdc",
    amount: bigint,
    cur: Bal,
  ): Promise<Bal> {
    const exp: Bal =
      asset === "sol"
        ? { ...cur, freeSol: cur.freeSol + amount }
        : { ...cur, freeUsdc: cur.freeUsdc + amount };
    if (this.done(name)) return exp;
    const mint = asset === "sol" ? this.mk.wsolMint : this.mk.usdcMint;
    const vault = asset === "sol" ? this.mk.wsolVault : this.mk.usdcVault;
    const source = this.ata(mint);
    await this.ensureAta(mint);
    const vaultBefore = await this.tokenBalance(vault);
    const r = await this.runOperation(
      name,
      async ({ sequence, offset, shieldSequence, intent }) => {
        const ixs: TransactionInstruction[] = [];
        if (asset === "sol") {
          ixs.push(
            SystemProgram.transfer({
              fromPubkey: this.owner.publicKey,
              toPubkey: source,
              lamports: Number(amount),
            }),
            new TransactionInstruction({
              programId: TOKEN_PROGRAM_ID,
              keys: [{ pubkey: source, isSigner: false, isWritable: true }],
              data: Buffer.from([17]), // SyncNative
            }),
          );
        }
        ixs.push(
          await this.method("shield")(
            offset,
            sequence,
            shieldSequence,
            asset === "sol" ? { sol: {} } : { usdc: {} },
            new BN(amount.toString()),
          )
            .accountsPartial({
              payer: this.owner.publicKey,
              ...this.queueAccounts("shield_credit", offset),
              market: this.market,
              assetState: this.mk.assetState,
              position: this.position,
              operationCounter: this.counter,
              operationIntent: intent,
              pendingShield: PublicKey.findProgramAddressSync(
                [
                  seedBytes("pending_shield"),
                  this.position.toBytes(),
                  shieldSequence.toArrayLike(Buffer, "le", 8),
                ],
                this.programId,
              )[0],
              mint,
              vault,
              sourceTokenAccount: source,
              tokenProgram: TOKEN_PROGRAM_ID,
            })
            .instruction(),
        );
        return ixs;
      },
      "applied",
      true,
    );
    if (r.signature !== undefined) {
      expectEq(
        `${name}: vault delta`,
        (await this.tokenBalance(vault)) - vaultBefore,
        amount,
      );
    }
    await this.finish(name, exp, r, `${asset} shield`);
    return exp;
  }

  private async unshield(
    name: string,
    asset: "sol" | "usdc",
    amount: bigint,
    cur: Bal,
  ): Promise<Bal> {
    const exp: Bal =
      asset === "sol"
        ? { ...cur, freeSol: cur.freeSol - amount }
        : { ...cur, freeUsdc: cur.freeUsdc - amount };
    if (this.done(name)) return exp;
    const mint = asset === "sol" ? this.mk.wsolMint : this.mk.usdcMint;
    const vault = asset === "sol" ? this.mk.wsolVault : this.mk.usdcVault;
    const destination = this.ata(mint);
    await this.ensureAta(mint);
    const destBefore = await this.tokenBalance(destination);
    const vaultBefore = await this.tokenBalance(vault);
    const r = await this.runOperation(
      name,
      async ({ sequence, offset, intent }) => [
        await this.method("unshield")(
          offset,
          sequence,
          asset === "sol" ? { sol: {} } : { usdc: {} },
          new BN(amount.toString()),
        )
          .accountsPartial({
            payer: this.owner.publicKey,
            ...this.queueAccounts("unshield", offset),
            market: this.market,
            assetState: this.mk.assetState,
            position: this.position,
            operationCounter: this.counter,
            operationIntent: intent,
            destination,
          })
          .instruction(),
      ],
      "applied",
    );
    if (r.signature !== undefined) {
      expectEq(
        `${name}: destination delta`,
        (await this.tokenBalance(destination)) - destBefore,
        amount,
      );
      expectEq(
        `${name}: vault delta`,
        (await this.tokenBalance(vault)) - vaultBefore,
        -amount,
      );
    }
    await this.finish(name, exp, r, `${asset} unshield`);
    return exp;
  }

  async loan(): Promise<void> {
    log("[d] private loan flow");
    let cur = this.startBal();
    cur = await this.shield("shield_sol", "sol", SOL_AMOUNT, cur);
    cur = await this.positionOp(
      "add_collateral",
      PositionOp.AddCollateral,
      SOL_AMOUNT,
      cur,
    );
    cur = await this.positionOp("borrow", PositionOp.Borrow, BORROW, cur);
    cur = await this.positionOp(
      "borrow_over_limit",
      PositionOp.Borrow,
      OVER_BORROW,
      cur,
    );
    cur = await this.unshield("unshield_usdc", "usdc", BORROW, cur);
    cur = await this.shield("shield_usdc", "usdc", BORROW, cur);
    cur = await this.positionOp("repay", PositionOp.Repay, BORROW, cur);
    cur = await this.positionOp(
      "withdraw_collateral",
      PositionOp.WithdrawCollateral,
      SOL_AMOUNT,
      cur,
    );
    cur = await this.unshield("unshield_wsol", "sol", SOL_AMOUNT, cur);

    // Native SOL: a SEPARATE owner-signed CloseAccount transaction.
    if (!this.done("unwrap_wsol")) {
      const wsolAccount = this.ata(this.mk.wsolMint);
      const info = await retry("wsol account", () =>
        this.ctx.connection.getAccountInfo(wsolAccount, "confirmed"),
      );
      let signature: string | undefined;
      let ms = 0;
      if (info !== null) {
        const tx = buildUnwrapWsolTransaction(
          this.owner.publicKey,
          wsolAccount,
        );
        const sent = await this.send(tx.instructions, [this.owner]);
        signature = sent.signature;
        ms = sent.ms;
        const gone = await retry("wsol closed", () =>
          this.ctx.connection.getAccountInfo(wsolAccount, "confirmed"),
        );
        if (gone !== null) throw new Error("wSOL account was not closed");
      }
      await this.finish("unwrap_wsol", cur, { signature, ms }, "wSOL -> SOL");
    }
  }

  async returnLeftovers(): Promise<string | undefined> {
    if (BigInt(this.state.ownerFundedLamports) === 0n) return undefined;
    if (this.state.ownerReturned) return undefined;
    const have = await this.balanceOf(this.owner.publicKey);
    if (have <= 5_000) return undefined;
    const sent = await this.send(
      [
        SystemProgram.transfer({
          fromPubkey: this.owner.publicKey,
          toPubkey: this.deployer.publicKey,
          lamports: have - 5_000,
        }),
      ],
      [this.owner],
    );
    this.state.ownerReturned = true;
    this.save();
    log(`  returned ${sol(have - 5_000)} SOL to the deployer`);
    return sent.signature;
  }

  summary(): string {
    const rows = Object.values(this.state.steps);
    const pad = (s: string, n: number): string => s.padEnd(n).slice(0, n);
    log("\n=== Walnut devnet private loan demo ===");
    log(`market   ${this.market.toBase58()}`);
    log(`position ${this.position.toBase58()}`);
    log(`price $${Number(PRICE) / 1e6}/SOL, LTV ${Number(LTV_BPS) / 100}%`);
    log(
      "liquidity stand-in: test USDC minted by the deployer into the vault (not a real lender deposit)",
    );
    log("");
    log(
      `${pad("step", 22)}${pad("latency ms", 12)}${pad("free SOL", 10)}${pad("free USDC", 11)}${pad("collat SOL", 11)}${pad("debt USDC", 10)}detail`,
    );
    for (const r of rows) {
      const [fs0, fu, col, debt] = r.decrypted;
      log(
        `${pad(r.name, 22)}${pad(String(r.ms), 12)}${pad(sol(BigInt(fs0)), 10)}${pad((Number(fu) / 1e6).toFixed(2), 11)}${pad(sol(BigInt(col)), 11)}${pad((Number(debt) / 1e6).toFixed(2), 10)}${r.detail}`,
      );
      if (r.signature) log(`    ${explorer(r.signature)}`);
    }
    const dir = path.join(this.ctx.repoRoot, "artifacts");
    fs.mkdirSync(dir, { recursive: true });
    const file = path.join(dir, "devnet-loan-demo.json");
    fs.writeFileSync(
      file,
      JSON.stringify(
        {
          network: NETWORK,
          generatedAt: new Date().toISOString(),
          program: this.programId.toBase58(),
          market: this.market.toBase58(),
          position: this.position.toBase58(),
          price: PRICE.toString(),
          ltvBps: LTV_BPS.toString(),
          liquidity:
            "test USDC minted by the deployer into the market vault (stand-in for lender deposits)",
          steps: rows.map((r) => ({
            ...r,
            explorer: r.signature ? explorer(r.signature) : undefined,
            columns: ["freeSol", "freeUsdc", "collateralSol", "debtUsdc"],
          })),
        },
        null,
        2,
      ),
    );
    return file;
  }
}

async function main(): Promise<void> {
  const options = resolveOptions(process.argv.slice(2));
  secretStrings = [options.rpcUrl];
  const ctx = await createContext(options);
  anchor.setProvider(ctx.provider);
  const demo = new LoanDemo(ctx);
  log(
    `Walnut devnet loan demo: program ${ctx.programId.toBase58()}, cluster offset ${ctx.clusterOffset}`,
  );
  log(`deployer ${ctx.payer.publicKey.toBase58()}`);
  await demo.compDefs();
  await demo.setup();
  await demo.loan();
  await demo.returnLeftovers();
  const file = demo.summary();
  log(`\nartifact: ${path.relative(ctx.repoRoot, file)}`);
}

main()
  .then(() => process.exit(0))
  .catch((err: unknown) => {
    const message =
      err instanceof Error ? (err.stack ?? err.message) : String(err);
    console.error(redact(`Demo failed: ${message}`));
    process.exit(1);
  });
