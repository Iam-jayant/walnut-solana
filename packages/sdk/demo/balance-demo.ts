// Task 5.7: devnet balance demo.
//
// Runs the whole Position balance path against REAL Arcium devnet MPC:
//   shield SOL and USDC -> decrypt locally -> unshield USDC (+ an oversized
//   request that resolves Zero) -> unshield wSOL, then unwrap it to native SOL
//   in a SECOND, separate transaction.
//
// It is idempotent and resumable: progress is persisted under a state
// directory OUTSIDE the repository (default ~/.walnut-demo/devnet, override
// with WALNUT_DEMO_DIR) and every step also re-checks on-chain state, so a
// rerun skips finished steps and continues. The state directory holds the demo
// owner keypair; it is never written inside the repository.
//
// Usage (RPC and wallet come from ANCHOR_PROVIDER_URL / ANCHOR_WALLET exactly
// like the benchmarks; the RPC URL is never printed):
//   pnpm --filter @walnut/sdk demo:balance
//   pnpm --filter @walnut/sdk demo:balance -- --cluster-offset 456
//
// See docs/DEVNET_DEMO.md. This is a demonstration tool, not production code.

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
import { buildUnwrapWsolTransaction } from "../src/unwrap.ts";
import {
  decryptPosition,
  deriveViewingKey,
  viewingKeyMessageBytes,
} from "../src/viewing-key.ts";

// ---------------------------------------------------------------- constants

const NETWORK = "devnet";
const TOKEN_PROGRAM_ID = new PublicKey(
  "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
);
const ATA_PROGRAM_ID = new PublicKey(
  "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
);
const NATIVE_MINT = new PublicKey(
  "So11111111111111111111111111111111111111112",
);
const BPF_LOADER_UPGRADEABLE_ID = new PublicKey(
  "BPFLoaderUpgradeab1e11111111111111111111111",
);
const MINT_SIZE = 82;
const TEST_USDC_NAME = "WALNUT-DEVNET-TEST-USDC (6 decimals, deployer-minted)";

/** Circuits this demo needs, with the comp-def initializer of each. */
const COMP_DEFS: ReadonlyArray<[string, string]> = [
  ["init_pool_state", "initPoolStateCompDef"],
  ["init_position_state", "initPositionStateCompDef"],
  ["shield_credit", "initShieldCreditCompDef"],
  ["unshield", "initUnshieldCompDef"],
];

const USDC_MINTED = 5_000_000n;
const USDC_SHIELD = 1_234_567n;
const USDC_UNSHIELD = 1_000_000n;
const SOL_SHIELD = 54_321_000n;
const SOL_UNSHIELD = 20_000_000n;
const OWNER_FUND_LAMPORTS = 500_000_000;
const OWNER_LOW_LAMPORTS = 120_000_000;
const OWNER_TOPUP_TO_LAMPORTS = 300_000_000;
const COMPUTE_UNIT_LIMIT = 600_000;
const INTENT_TIMEOUT_MS = 240_000;

// ------------------------------------------------------------------ logging

/** Strips api keys (and any literal RPC URL) from everything printed. */
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

/** Retries `fn` on transient RPC errors with exponential backoff. */
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
      const wait = Math.min(1000 * 2 ** i, 15_000);
      log(`  (retry ${i + 1}/${attempts - 1} ${label}: ${text.slice(0, 80)})`);
      await sleep(wait);
    }
  }
  throw last;
}

/** Polls `fn` until it returns a value, or throws after `timeoutMs`. */
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
  poolState: PublicKey;
  wsolMint: PublicKey;
  usdcMint: PublicKey;
  wsolVault: PublicKey;
  usdcVault: PublicKey;
  positionCount: BN;
  positionCountCap: BN;
}
interface PositionAccount {
  status: Json;
  nextShieldSeq: BN;
  viewingKeyEpoch: BN;
  encryptedState: { nonce: BN; ciphertexts: number[][] };
}
interface PoolAccount {
  status: Json;
}
interface IntentAccount {
  status: Json;
}
interface CounterAccount {
  nextSequence: BN;
}
interface PendingShieldAccount {
  status: Json;
}

/** One finished step, as printed in the summary and the JSON artifact. */
interface StepResult {
  name: string;
  signatures: string[];
  queueMs?: number | undefined;
  callbackMs?: number | undefined;
  feeLamports: number;
  /** Compute units the queue transaction consumed (0 if unknown). */
  computeUnits?: number | undefined;
  detail: string;
}

/** Queued-but-unfinished operation, kept so a rerun can resume it. */
interface PendingOp {
  sequence: string;
  offset: string;
  shieldSequence?: string;
  balanceBefore?: string;
  lamportsBefore?: string;
}

interface DemoState {
  programId: string;
  usdcMint?: string;
  ownerPath?: string;
  deployerStartLamports?: string;
  ownerFundedLamports: string;
  ownerReturnedLamports: string;
  steps: Record<string, StepResult>;
  pending: Record<string, PendingOp>;
}

// -------------------------------------------------------------- state files

function stateDir(): string {
  return (
    process.env["WALNUT_DEMO_DIR"] ??
    path.join(os.homedir(), ".walnut-demo", NETWORK)
  );
}

function loadState(programId: PublicKey): DemoState {
  const file = path.join(stateDir(), "state.json");
  if (fs.existsSync(file)) {
    const parsed = JSON.parse(fs.readFileSync(file, "utf8")) as DemoState;
    if (parsed.programId === programId.toBase58()) return parsed;
  }
  return {
    programId: programId.toBase58(),
    ownerFundedLamports: "0",
    ownerReturnedLamports: "0",
    steps: {},
    pending: {},
  };
}

function saveState(state: DemoState): void {
  fs.mkdirSync(stateDir(), { recursive: true });
  fs.writeFileSync(
    path.join(stateDir(), "state.json"),
    JSON.stringify(state, null, 2),
  );
}

/** Loads (or creates, mode 0600) a keypair file in the state directory. */
function loadOrCreateRoleKey(fileName: string): Keypair {
  fs.mkdirSync(stateDir(), { recursive: true });
  const file = path.join(stateDir(), fileName);
  if (fs.existsSync(file)) {
    const secret = JSON.parse(fs.readFileSync(file, "utf8")) as number[];
    return Keypair.fromSecretKey(new Uint8Array(secret));
  }
  const key = Keypair.generate();
  fs.writeFileSync(file, JSON.stringify(Array.from(key.secretKey)), {
    mode: 0o600,
  });
  return key;
}

function loadOrCreateOwner(state: DemoState): Keypair {
  fs.mkdirSync(stateDir(), { recursive: true });
  const file = state.ownerPath ?? path.join(stateDir(), "owner.json");
  if (fs.existsSync(file)) {
    const secret = JSON.parse(fs.readFileSync(file, "utf8")) as number[];
    return Keypair.fromSecretKey(new Uint8Array(secret));
  }
  const owner = Keypair.generate();
  fs.writeFileSync(file, JSON.stringify(Array.from(owner.secretKey)), {
    mode: 0o600,
  });
  state.ownerPath = file;
  saveState(state);
  return owner;
}

// ------------------------------------------------------------------ helpers

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
  log(`  ok  ${label} = ${actual}`);
}

/** Ed25519 signature of `message` by `keypair` (deterministic, no new deps). */
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

// -------------------------------------------------------------------- demo

class Demo {
  readonly ctx: BenchContext;
  readonly state: DemoState;
  readonly programId: PublicKey;
  readonly deployer: Keypair;
  readonly owner: Keypair;
  readonly market: PublicKey;
  readonly poolState: PublicKey;
  readonly vaultAuthority: PublicKey;
  readonly recoveryPosition: PublicKey;
  readonly position: PublicKey;
  readonly counter: PublicKey;
  readonly results: StepResult[] = [];
  viewingSecret: Uint8Array | undefined;
  viewingPublic: Uint8Array | undefined;

  constructor(ctx: BenchContext) {
    this.ctx = ctx;
    this.programId = ctx.programId;
    this.deployer = ctx.payer;
    this.state = loadState(ctx.programId);
    this.owner = loadOrCreateOwner(this.state);
    const pda = (...seeds: Uint8Array[]): PublicKey =>
      PublicKey.findProgramAddressSync(seeds, this.programId)[0];
    const seed = (s: string): Uint8Array => new TextEncoder().encode(s);
    this.market = pda(seed("market"));
    this.poolState = pda(seed("pool_state"), this.market.toBytes());
    this.vaultAuthority = pda(seed("vault_authority"), this.market.toBytes());
    this.recoveryPosition = pda(
      seed("recovery_position"),
      this.market.toBytes(),
    );
    this.position = pda(
      seed("position"),
      this.market.toBytes(),
      this.owner.publicKey.toBytes(),
    );
    this.counter = deriveOperationCounterPda(
      this.programId,
      this.owner.publicKey,
    );
  }

  // ---- generic RPC plumbing

  private method(name: string): MethodFn {
    const methods = this.ctx.program.methods as unknown as Record<
      string,
      MethodFn | undefined
    >;
    const fn = methods[name];
    if (fn === undefined) {
      throw new Error(`IDL has no instruction ${name}; rebuild the program`);
    }
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

  /** Sends one transaction signed by `signers[0]` (the fee payer). */
  private async send(
    instructions: TransactionInstruction[],
    signers: Keypair[],
  ): Promise<{ signature: string; ms: number; fee: number; cu: number }> {
    const started = Date.now();
    const signature = await retry("send", async () => {
      // Queue instructions need more than the default 200,000 compute units.
      const tx = new Transaction().add(
        ComputeBudgetProgram.setComputeUnitLimit({
          units: COMPUTE_UNIT_LIMIT,
        }),
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
    const ms = Date.now() - started;
    const { fee, cu } = await this.feeOf(signature);
    return { signature, ms, fee, cu };
  }

  private async feeOf(signature: string): Promise<{ fee: number; cu: number }> {
    try {
      const tx = await waitFor(
        "transaction fee",
        async () =>
          (await this.ctx.connection.getTransaction(signature, {
            commitment: "confirmed",
            maxSupportedTransactionVersion: 0,
          })) ?? undefined,
        20_000,
      );
      return {
        fee: tx.meta?.fee ?? 0,
        cu: tx.meta?.computeUnitsConsumed ?? 0,
      };
    } catch {
      return { fee: 0, cu: 0 };
    }
  }

  private record(result: StepResult): void {
    this.state.steps[result.name] = result;
    delete this.state.pending[result.name];
    saveState(this.state);
    this.results.push(result);
    log(
      `  step ${result.name}: done (${result.detail})` +
        (result.signatures[0] ? `\n    ${explorer(result.signatures[0])}` : ""),
    );
  }

  private skipped(name: string): boolean {
    const done = this.state.steps[name];
    if (done === undefined) return false;
    log(`  step ${name}: already done (skipped)`);
    this.results.push(done);
    return true;
  }

  private queueAccounts(
    circuit: string,
    offset: BN,
  ): Record<string, PublicKey> {
    return {
      ...deriveArciumQueueAccounts({
        programId: this.programId,
        clusterOffset: this.ctx.clusterOffset,
        computationOffset: offset,
        circuit,
      }),
    };
  }

  private async ensureOwnerBalance(min = OWNER_LOW_LAMPORTS): Promise<number> {
    const have = await this.balanceOf(this.owner.publicKey);
    if (have >= min) return 0;
    const top = OWNER_TOPUP_TO_LAMPORTS - have;
    const { fee } = await this.send(
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
    saveState(this.state);
    log(`  funded demo owner with ${sol(top)} SOL`);
    return fee;
  }

  // ---- (a) comp defs

  async compDefs(): Promise<void> {
    log("[a] computation definitions + circuit uploads");
    for (const [circuit, initName] of COMP_DEFS) {
      const builder = this.method(initName)() as unknown as InitCompDefBuilder;
      const started = Date.now();
      const state = await retry(
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
      log(`  comp def ${circuit}: ${state} (${Date.now() - started} ms)`);
    }
  }

  // ---- (b) market + pool

  async marketAndPool(): Promise<MarketAccount> {
    log("[b] market and pool");
    let market = await this.fetch<MarketAccount>("market", this.market);
    if (market === null) {
      const usdcMint = await this.ensureUsdcMint();
      const programData = PublicKey.findProgramAddressSync(
        [this.programId.toBytes()],
        BPF_LOADER_UPGRADEABLE_ID,
      )[0];
      const pda = (...seeds: Uint8Array[]): PublicKey =>
        PublicKey.findProgramAddressSync(seeds, this.programId)[0];
      const seed = (s: string): Uint8Array => new TextEncoder().encode(s);
      const vaultFor = (mint: PublicKey): PublicKey =>
        pda(seed("vault"), this.market.toBytes(), mint.toBytes());
      // The roles must be distinct keys; their keypairs stay in the state dir.
      const admin = this.deployer.publicKey;
      const risk = loadOrCreateRoleKey("risk.json").publicKey;
      const treasury = loadOrCreateRoleKey("treasury.json").publicKey;
      const guardian = loadOrCreateRoleKey("guardian.json").publicKey;
      const instruction = await this.method("initializeMarket")({
        riskAuthority: risk,
        treasuryAuthority: treasury,
        guardianAuthority: guardian,
        caps: {
          supplyCapUsdc: new BN(1_000_000_000_000),
          totalDebtCapUsdc: new BN(500_000_000_000),
          positionDebtCapUsdc: new BN(10_000_000_000),
        },
        positionCountCap: new BN(1_000),
        debtPositionCountCap: new BN(250),
      })
        .accountsPartial({
          authority: admin,
          walnutProgram: this.programId,
          programData,
          wsolMint: NATIVE_MINT,
          usdcMint,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          market: this.market,
          assetState: pda(seed("asset_state"), this.market.toBytes()),
          poolState: this.poolState,
          vaultAuthority: this.vaultAuthority,
          wsolVault: vaultFor(NATIVE_MINT),
          usdcVault: vaultFor(usdcMint),
          recoveryPosition: this.recoveryPosition,
        })
        .instruction();
      const sent = await this.send([instruction], [this.deployer]);
      log(`  initialize_market: ${explorer(sent.signature)}`);
      market = await this.fetch<MarketAccount>("market", this.market);
    } else {
      log(`  market exists: ${this.market.toBase58()}`);
    }
    if (market === null) throw new Error("market missing after initialization");

    let pool = await this.fetch<PoolAccount>("poolState", this.poolState);
    if (pool !== null && statusName(pool.status) === "initializing") {
      const offset = newOffset();
      const instruction = await this.method("queuePoolStateInit")(offset)
        .accountsPartial({
          payer: this.deployer.publicKey,
          ...this.queueAccounts("init_pool_state", offset),
          market: this.market,
          poolState: this.poolState,
        })
        .instruction();
      const sent = await this.send([instruction], [this.deployer]);
      log(`  queue_pool_state_init: ${explorer(sent.signature)}`);
      const started = Date.now();
      await waitFor(
        "Pool Active",
        async () => {
          const p = await this.fetch<PoolAccount>("poolState", this.poolState);
          return p !== null && statusName(p.status) === "active"
            ? p
            : undefined;
        },
        INTENT_TIMEOUT_MS,
      );
      log(`  pool Active after ${Date.now() - started} ms`);
    }
    pool = await this.fetch<PoolAccount>("poolState", this.poolState);
    if (pool === null || statusName(pool.status) !== "active") {
      throw new Error("Pool is not Active");
    }
    log(`  pool: active; usdc mint ${market.usdcMint.toBase58()}`);
    return market;
  }

  private async ensureUsdcMint(): Promise<PublicKey> {
    if (this.state.usdcMint !== undefined) {
      const existing = new PublicKey(this.state.usdcMint);
      if (
        await retry("mint exists", () =>
          this.ctx.connection.getAccountInfo(existing, "confirmed"),
        )
      ) {
        return existing;
      }
    }
    const mint = Keypair.generate();
    const lamports = await retry("rent", () =>
      this.ctx.connection.getMinimumBalanceForRentExemption(MINT_SIZE),
    );
    // InitializeMint2: tag 20 | decimals | mint authority | no freeze authority.
    const data = Buffer.alloc(35);
    data.writeUInt8(20, 0);
    data.writeUInt8(6, 1);
    this.deployer.publicKey.toBuffer().copy(data, 2);
    data.writeUInt8(0, 34);
    const sent = await this.send(
      [
        SystemProgram.createAccount({
          fromPubkey: this.deployer.publicKey,
          newAccountPubkey: mint.publicKey,
          lamports,
          space: MINT_SIZE,
          programId: TOKEN_PROGRAM_ID,
        }),
        new TransactionInstruction({
          programId: TOKEN_PROGRAM_ID,
          keys: [{ pubkey: mint.publicKey, isSigner: false, isWritable: true }],
          data,
        }),
      ],
      [this.deployer, mint],
    );
    this.state.usdcMint = mint.publicKey.toBase58();
    saveState(this.state);
    log(`  created ${TEST_USDC_NAME}: ${this.state.usdcMint}`);
    log(`    ${explorer(sent.signature)}`);
    return mint.publicKey;
  }

  // ---- (c) owner + position

  async ownerAndPosition(market: MarketAccount): Promise<void> {
    log("[c] demo owner and Position");
    log(`  demo owner: ${this.owner.publicKey.toBase58()}`);
    // Fund only before the Position exists; later steps top up on demand, so
    // a rerun after the owner returned its funds does not strand new SOL.
    if (
      this.state.steps["initialize_position"] === undefined &&
      (await this.balanceOf(this.owner.publicKey)) < OWNER_LOW_LAMPORTS
    ) {
      const have = await this.balanceOf(this.owner.publicKey);
      const lamports = Math.max(OWNER_FUND_LAMPORTS - have, 0);
      if (lamports > 0) {
        await this.send(
          [
            SystemProgram.transfer({
              fromPubkey: this.deployer.publicKey,
              toPubkey: this.owner.publicKey,
              lamports,
            }),
          ],
          [this.deployer],
        );
        this.state.ownerFundedLamports = (
          BigInt(this.state.ownerFundedLamports) + BigInt(lamports)
        ).toString();
        saveState(this.state);
        log(`  funded demo owner with ${sol(lamports)} SOL`);
      }
    }

    // Wallet-signature-style Viewing Key for epoch 0.
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
    this.viewingPublic = material.publicKey;

    if (this.skipped("initialize_position")) return;
    let position = await this.fetch<PositionAccount>("position", this.position);
    let signature: string | undefined;
    let fee = 0;
    let cu = 0;
    let queueMs: number | undefined;
    const started = Date.now();
    if (position === null) {
      if (market.positionCount.gte(market.positionCountCap)) {
        throw new Error("Position count cap reached on this Market");
      }
      const pending =
        this.state.pending["initialize_position"] ??
        (this.state.pending["initialize_position"] = {
          sequence: "0",
          offset: newOffset().toString(),
        });
      saveState(this.state);
      const offset = new BN(pending.offset);
      const sequence = new BN(0);
      const nonce = new BN(randomBytes(16));
      const instruction = await this.method("initializePosition")(
        offset,
        sequence,
        Array.from(material.publicKey),
        nonce,
      )
        .accountsPartial({
          payer: this.owner.publicKey,
          ...this.queueAccounts("init_position_state", offset),
          market: this.market,
          position: this.position,
          operationCounter: this.counter,
          operationIntent: deriveIntentPda(
            this.programId,
            this.owner.publicKey,
            0,
          ),
        })
        .instruction();
      const sent = await this.send([instruction], [this.owner]);
      signature = sent.signature;
      fee = sent.fee;
      cu = sent.cu;
      queueMs = sent.ms;
      log(`  initialize_position: ${explorer(signature)}`);
    }
    const waitStart = Date.now();
    position = await waitFor(
      "Position Active",
      async () => {
        const p = await this.fetch<PositionAccount>("position", this.position);
        if (p === null) return undefined;
        const name = statusName(p.status);
        if (name === "failedInit" || name === "failed") {
          throw new Error(`Position initialization ended ${name}`);
        }
        return name === "active" ? p : undefined;
      },
      INTENT_TIMEOUT_MS,
    );
    const callbackMs = Date.now() - waitStart;
    expectEq(
      "viewing key epoch",
      BigInt(position.viewingKeyEpoch.toString()),
      0n,
    );
    const balances = this.viewBalances(position);
    expectEq("free_sol (new Position)", balances.freeSol, 0n);
    expectEq("free_usdc (new Position)", balances.freeUsdc, 0n);
    this.record({
      name: "initialize_position",
      signatures: signature ? [signature] : [],
      queueMs,
      callbackMs,
      feeLamports: fee,
      computeUnits: cu,
      detail: `Active in ${Date.now() - started} ms`,
    });
  }

  private viewBalances(position: PositionAccount) {
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

  private async decrypt() {
    const position = await this.fetch<PositionAccount>(
      "position",
      this.position,
    );
    if (position === null) throw new Error("Position missing");
    return this.viewBalances(position);
  }

  // ---- token accounts

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

  private createAtaInstruction(mint: PublicKey): TransactionInstruction {
    // CreateIdempotent: tag 1.
    return new TransactionInstruction({
      programId: ATA_PROGRAM_ID,
      keys: [
        { pubkey: this.owner.publicKey, isSigner: true, isWritable: true },
        { pubkey: this.ata(mint), isSigner: false, isWritable: true },
        { pubkey: this.owner.publicKey, isSigner: false, isWritable: false },
        { pubkey: mint, isSigner: false, isWritable: false },
        { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
        { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
      ],
      data: Buffer.from([1]),
    });
  }

  /** Creates the owner token account of `mint` in its own transaction. */
  private async ensureAta(mint: PublicKey): Promise<void> {
    const address = this.ata(mint);
    const existing = await retry("ata", () =>
      this.ctx.connection.getAccountInfo(address, "confirmed"),
    );
    if (existing !== null) return;
    await this.ensureOwnerBalance();
    await this.send([this.createAtaInstruction(mint)], [this.owner]);
  }

  // ---- queue/await machinery

  /**
   * Queues one Position operation (or resumes it), then waits for the Intent
   * to reach a terminal status. Returns the terminal status and timings.
   */
  private async runOperation(
    name: string,
    build: (p: {
      sequence: BN;
      offset: BN;
      shieldSequence: BN;
      intent: PublicKey;
    }) => Promise<TransactionInstruction[]>,
    options: { shield?: boolean; before?: () => Promise<Partial<PendingOp>> },
  ): Promise<{
    status: string;
    signature?: string | undefined;
    queueMs?: number | undefined;
    callbackMs: number;
    fee: number;
    cu: number;
    pending: PendingOp;
    pendingShield?: PublicKey | undefined;
  }> {
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
        ...(options.before ? await options.before() : {}),
      };
      this.state.pending[name] = pending;
      saveState(this.state);
    }
    const sequence = new BN(pending.sequence);
    const offset = new BN(pending.offset);
    const shieldSequence = new BN(pending.shieldSequence ?? "0");
    const intent = deriveIntentPda(
      this.programId,
      this.owner.publicKey,
      BigInt(pending.sequence),
    );
    const pendingShield = options.shield
      ? PublicKey.findProgramAddressSync(
          [
            new TextEncoder().encode("pending_shield"),
            this.position.toBytes(),
            shieldSequence.toArrayLike(Buffer, "le", 8),
          ],
          this.programId,
        )[0]
      : undefined;

    let signature: string | undefined;
    let fee = 0;
    let cu = 0;
    let queueMs: number | undefined;
    const existing = await this.fetch<IntentAccount>("operationIntent", intent);
    if (existing === null) {
      await this.ensureOwnerBalance();
      const instructions = await build({
        sequence,
        offset,
        shieldSequence,
        intent,
      });
      const sent = await this.send(instructions, [this.owner]);
      signature = sent.signature;
      fee = sent.fee;
      cu = sent.cu;
      queueMs = sent.ms;
      log(`  ${name}: queued ${explorer(signature)}`);
    } else {
      log(`  ${name}: Intent already exists, resuming wait`);
    }
    const waitStart = Date.now();
    const terminal = await waitFor(
      `${name} Intent terminal`,
      async () => {
        const i = await this.fetch<IntentAccount>("operationIntent", intent);
        return i !== null && statusName(i.status) !== "queued"
          ? statusName(i.status)
          : undefined;
      },
      INTENT_TIMEOUT_MS,
    );
    const callbackMs = Date.now() - waitStart;
    log(`  ${name}: Intent ${terminal} ${callbackMs} ms after confirmation`);
    return {
      status: terminal,
      signature,
      queueMs,
      callbackMs,
      fee,
      cu,
      pending,
      pendingShield,
    };
  }

  private async shield(
    name: string,
    asset: "sol" | "usdc",
    market: MarketAccount,
    amount: bigint,
    expectFree: { freeSol: bigint; freeUsdc: bigint },
  ): Promise<void> {
    if (this.skipped(name)) return;
    const mint = asset === "sol" ? market.wsolMint : market.usdcMint;
    const vault = asset === "sol" ? market.wsolVault : market.usdcVault;
    const source = this.ata(mint);
    await this.ensureAta(mint);
    const vaultBefore = await this.tokenBalance(vault);

    const op = await this.runOperation(
      name,
      async ({ sequence, offset, shieldSequence, intent }) => {
        const instructions: TransactionInstruction[] = [];
        if (asset === "sol") {
          // Wrap and shield in the SAME transaction.
          instructions.push(
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
        instructions.push(
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
              assetState: market.assetState,
              position: this.position,
              operationCounter: this.counter,
              operationIntent: intent,
              pendingShield: PublicKey.findProgramAddressSync(
                [
                  new TextEncoder().encode("pending_shield"),
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
        return instructions;
      },
      { shield: true },
    );
    if (op.status !== "applied") {
      throw new Error(`${name}: Intent ended ${op.status}, expected applied`);
    }
    if (op.pendingShield !== undefined) {
      const shielded = await waitFor(
        "Pending Shield credited",
        async () => {
          const ps = await this.fetch<PendingShieldAccount>(
            "pendingShield",
            op.pendingShield as PublicKey,
          );
          return ps !== null && statusName(ps.status) === "credited"
            ? ps
            : undefined;
        },
        60_000,
      );
      log(`  Pending Shield: ${statusName(shielded.status)}`);
    }
    // The vault grew by exactly the shielded amount (checked unless resumed).
    const vaultAfter = await this.tokenBalance(vault);
    if (op.signature !== undefined) {
      expectEq(`${asset} vault delta`, vaultAfter - vaultBefore, amount);
    }
    const balances = await this.decrypt();
    expectEq("decrypted free_sol", balances.freeSol, expectFree.freeSol);
    expectEq("decrypted free_usdc", balances.freeUsdc, expectFree.freeUsdc);
    this.record({
      name,
      signatures: op.signature ? [op.signature] : [],
      queueMs: op.queueMs,
      callbackMs: op.callbackMs,
      feeLamports: op.fee,
      computeUnits: op.cu,
      detail: `${asset === "sol" ? sol(amount) + " SOL (wrapped in same tx)" : Number(amount) / 1e6 + " test USDC"}`,
    });
  }

  async shields(market: MarketAccount): Promise<void> {
    log("[d] shield USDC and SOL, decrypt locally");
    if (!this.skipped("mint_test_usdc")) {
      const ata = this.ata(market.usdcMint);
      // MintTo: tag 7 | amount u64 LE; the deployer is the mint authority.
      const data = Buffer.alloc(9);
      data.writeUInt8(7, 0);
      data.writeBigUInt64LE(USDC_MINTED, 1);
      await this.ensureOwnerBalance();
      await this.ensureAta(market.usdcMint);
      const minted = await this.send(
        [
          new TransactionInstruction({
            programId: TOKEN_PROGRAM_ID,
            keys: [
              { pubkey: market.usdcMint, isSigner: false, isWritable: true },
              { pubkey: ata, isSigner: false, isWritable: true },
              {
                pubkey: this.deployer.publicKey,
                isSigner: true,
                isWritable: false,
              },
            ],
            data,
          }),
        ],
        [this.deployer],
      );
      expectEq(
        "owner test USDC balance",
        await this.tokenBalance(ata),
        USDC_MINTED,
      );
      this.record({
        name: "mint_test_usdc",
        signatures: [minted.signature],
        feeLamports: minted.fee,
        detail: `minted ${Number(USDC_MINTED) / 1e6} test USDC to the owner`,
      });
    }
    await this.shield("shield_usdc", "usdc", market, USDC_SHIELD, {
      freeSol: 0n,
      freeUsdc: USDC_SHIELD,
    });
    await this.shield("shield_sol", "sol", market, SOL_SHIELD, {
      freeSol: SOL_SHIELD,
      freeUsdc: USDC_SHIELD,
    });
  }

  // ---- (e) unshield

  private async unshield(
    name: string,
    asset: "sol" | "usdc",
    market: MarketAccount,
    amount: bigint,
    expectStatus: "applied" | "zero",
    expectFree: { freeSol: bigint; freeUsdc: bigint },
    destinationBalanceDelta: bigint,
  ): Promise<void> {
    if (this.skipped(name)) return;
    const mint = asset === "sol" ? market.wsolMint : market.usdcMint;
    const vault = asset === "sol" ? market.wsolVault : market.usdcVault;
    const destination = this.ata(mint);
    const op = await this.runOperation(
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
            assetState: market.assetState,
            position: this.position,
            operationCounter: this.counter,
            operationIntent: intent,
            destination,
          })
          .instruction(),
      ],
      {
        before: async () => ({
          balanceBefore: (await this.tokenBalance(destination)).toString(),
          lamportsBefore: (await this.tokenBalance(vault)).toString(),
        }),
      },
    );
    if (op.status !== expectStatus) {
      throw new Error(
        `${name}: Intent ended ${op.status}, expected ${expectStatus}`,
      );
    }
    const before = BigInt(op.pending.balanceBefore ?? "0");
    const vaultBefore = BigInt(op.pending.lamportsBefore ?? "0");
    expectEq(
      `${asset} destination token balance change`,
      (await this.tokenBalance(destination)) - before,
      destinationBalanceDelta,
    );
    expectEq(
      `${asset} vault balance change`,
      (await this.tokenBalance(vault)) - vaultBefore,
      -destinationBalanceDelta,
    );
    const balances = await this.decrypt();
    expectEq("decrypted free_sol", balances.freeSol, expectFree.freeSol);
    expectEq("decrypted free_usdc", balances.freeUsdc, expectFree.freeUsdc);
    this.record({
      name,
      signatures: op.signature ? [op.signature] : [],
      queueMs: op.queueMs,
      callbackMs: op.callbackMs,
      feeLamports: op.fee,
      computeUnits: op.cu,
      detail:
        expectStatus === "zero"
          ? `oversized ${Number(amount) / 1e6} USDC resolved Zero, no transfer`
          : `${asset === "sol" ? sol(amount) + " wSOL" : Number(amount) / 1e6 + " test USDC"} approved`,
    });
  }

  async unshields(market: MarketAccount): Promise<void> {
    log("[e] unshield USDC, oversized request, wSOL, then unwrap");
    const usdcAfter = USDC_SHIELD - USDC_UNSHIELD;
    await this.unshield(
      "unshield_usdc",
      "usdc",
      market,
      USDC_UNSHIELD,
      "applied",
      { freeSol: SOL_SHIELD, freeUsdc: usdcAfter },
      USDC_UNSHIELD,
    );
    await this.unshield(
      "unshield_usdc_oversized",
      "usdc",
      market,
      usdcAfter + 1n,
      "zero",
      { freeSol: SOL_SHIELD, freeUsdc: usdcAfter },
      0n,
    );
    await this.unshield(
      "unshield_wsol",
      "sol",
      market,
      SOL_UNSHIELD,
      "applied",
      { freeSol: SOL_SHIELD - SOL_UNSHIELD, freeUsdc: usdcAfter },
      SOL_UNSHIELD,
    );

    // Native SOL: a SEPARATE owner-signed CloseAccount transaction.
    if (this.skipped("unwrap_wsol")) return;
    const wsolAccount = this.ata(market.wsolMint);
    const info = await retry("wsol account", () =>
      this.ctx.connection.getAccountInfo(wsolAccount, "confirmed"),
    );
    if (info === null) {
      log("  wSOL account already closed; unwrap assumed done");
      return;
    }
    const wrapped = tokenAmountOf(info.data);
    const rent = info.lamports - Number(wrapped);
    const lamportsBefore = await this.balanceOf(this.owner.publicKey);
    const tx = buildUnwrapWsolTransaction(this.owner.publicKey, wsolAccount);
    const sent = await this.send(tx.instructions, [this.owner]);
    const gone = await retry("wsol closed", () =>
      this.ctx.connection.getAccountInfo(wsolAccount, "confirmed"),
    );
    if (gone !== null) throw new Error("wSOL account was not closed");
    const gained = BigInt(
      (await this.balanceOf(this.owner.publicKey)) - lamportsBefore,
    );
    expectEq(
      "native SOL regained (wrapped + rent - fee)",
      gained,
      wrapped + BigInt(rent) - BigInt(sent.fee),
    );
    this.record({
      name: "unwrap_wsol",
      signatures: [sent.signature],
      queueMs: sent.ms,
      feeLamports: sent.fee,
      detail: `${sol(wrapped)} SOL + ${sol(rent)} rent returned as native SOL`,
    });
  }

  // ---- (f) wrap up

  async returnLeftovers(): Promise<void> {
    if (this.skipped("return_owner_funds")) return;
    const have = await this.balanceOf(this.owner.publicKey);
    const fee = 5_000;
    if (have <= fee) return;
    const sent = await this.send(
      [
        SystemProgram.transfer({
          fromPubkey: this.owner.publicKey,
          toPubkey: this.deployer.publicKey,
          lamports: have - fee,
        }),
      ],
      [this.owner],
    );
    this.state.ownerReturnedLamports = (
      BigInt(this.state.ownerReturnedLamports) + BigInt(have - fee)
    ).toString();
    this.record({
      name: "return_owner_funds",
      signatures: [sent.signature],
      feeLamports: sent.fee,
      detail: `${sol(have - fee)} SOL returned to the deployer`,
    });
  }

  summary(
    market: MarketAccount,
    deployerStart: bigint,
    deployerEnd: bigint,
  ): void {
    const rows = Object.values(this.state.steps);
    const pad = (s: string, n: number): string => s.padEnd(n).slice(0, n);
    log("\n=== Walnut devnet balance demo ===");
    log(`program        ${this.programId.toBase58()}`);
    log(`market         ${this.market.toBase58()}`);
    log(`test USDC mint ${market.usdcMint.toBase58()}  (${TEST_USDC_NAME})`);
    log(`demo owner     ${this.owner.publicKey.toBase58()}`);
    log(`position       ${this.position.toBase58()}`);
    log("");
    log(
      `${pad("step", 26)}${pad("queue ms", 10)}${pad("callback ms", 13)}${pad("fee SOL", 11)}${pad("CU", 9)}detail`,
    );
    for (const r of rows) {
      log(
        `${pad(r.name, 26)}${pad(r.queueMs?.toString() ?? "-", 10)}${pad(r.callbackMs?.toString() ?? "-", 13)}${pad(sol(r.feeLamports), 11)}${pad(r.computeUnits ? String(r.computeUnits) : "-", 9)}${r.detail}`,
      );
      for (const sig of r.signatures) log(`    ${explorer(sig)}`);
    }
    const cost = deployerStart - deployerEnd;
    log("");
    log(
      `deployer SOL: start ${sol(deployerStart)}  end ${sol(deployerEnd)}  net cost ${sol(cost)} (includes comp-def circuit uploads and market creation on the first run)`,
    );
    log(
      `owner funded ${sol(BigInt(this.state.ownerFundedLamports))} SOL, returned ${sol(BigInt(this.state.ownerReturnedLamports))} SOL`,
    );
  }

  writeArtifact(
    market: MarketAccount,
    deployerStart: bigint,
    deployerEnd: bigint,
  ): string {
    const dir = path.join(this.ctx.repoRoot, "artifacts");
    fs.mkdirSync(dir, { recursive: true });
    const file = path.join(dir, "devnet-balance-demo.json");
    const artifact = {
      network: NETWORK,
      generatedAt: new Date().toISOString(),
      program: this.programId.toBase58(),
      clusterOffset: this.ctx.clusterOffset,
      market: this.market.toBase58(),
      testUsdcMint: market.usdcMint.toBase58(),
      testUsdcMintName: TEST_USDC_NAME,
      owner: this.owner.publicKey.toBase58(),
      position: this.position.toBase58(),
      amounts: {
        usdcShielded: USDC_SHIELD.toString(),
        usdcUnshielded: USDC_UNSHIELD.toString(),
        solShieldedLamports: SOL_SHIELD.toString(),
        wsolUnshieldedLamports: SOL_UNSHIELD.toString(),
      },
      steps: Object.values(this.state.steps).map((s) => ({
        ...s,
        explorer: s.signatures.map(explorer),
      })),
      deployerStartLamports: deployerStart.toString(),
      deployerEndLamports: deployerEnd.toString(),
      deployerNetCostLamports: (deployerStart - deployerEnd).toString(),
      ownerFundedLamports: this.state.ownerFundedLamports,
      ownerReturnedLamports: this.state.ownerReturnedLamports,
    };
    fs.writeFileSync(file, JSON.stringify(artifact, null, 2));
    return file;
  }
}

async function main(): Promise<void> {
  const options = resolveOptions(process.argv.slice(2));
  secretStrings = [options.rpcUrl];
  const ctx = await createContext(options);
  anchor.setProvider(ctx.provider);
  const demo = new Demo(ctx);

  log(
    `Walnut devnet balance demo: program ${ctx.programId.toBase58()}, cluster offset ${ctx.clusterOffset}`,
  );
  log(`deployer ${ctx.payer.publicKey.toBase58()}`);
  if (demo.state.deployerStartLamports === undefined) {
    demo.state.deployerStartLamports = String(
      await ctx.connection.getBalance(ctx.payer.publicKey, "confirmed"),
    );
    saveState(demo.state);
  }

  await demo.compDefs();
  const market = await demo.marketAndPool();
  await demo.ownerAndPosition(market);
  await demo.shields(market);
  await demo.unshields(market);
  await demo.returnLeftovers();

  const deployerEnd = BigInt(
    await ctx.connection.getBalance(ctx.payer.publicKey, "confirmed"),
  );
  const deployerStart = BigInt(demo.state.deployerStartLamports);
  demo.summary(market, deployerStart, deployerEnd);
  const file = demo.writeArtifact(market, deployerStart, deployerEnd);
  log(`\nartifact: ${path.relative(ctx.repoRoot, file)}`);
}

main()
  .then(() => {
    // Open websocket subscriptions keep Node alive; exit explicitly.
    process.exit(0);
  })
  .catch((err: unknown) => {
    const message =
      err instanceof Error ? (err.stack ?? err.message) : String(err);
    console.error(redact(`Demo failed: ${message}`));
    process.exit(1);
  });
