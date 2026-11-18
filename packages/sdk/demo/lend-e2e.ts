// walnut-lend task 6.2: devnet end-to-end lending demo.
//
// Run it AFTER the deployer ran `bash scripts/lend-devnet.sh` (one program
// upgrade, `init_lending_pool`, and the `lend_op`, `disclose_position` and
// `rotate_position_key` circuits registered):
//
//   pnpm --filter @walnut/sdk demo:lend
//
// Steps (each one compares decrypted or public values with the expected
// values and fails with a message that names the step):
//
//   1  setup                      pool, oracle and SDK decode cross-check
//   2  lender_supply              deployer: shield USDC, lend_op supply
//   3  borrower_collateral        demo owner: shield SOL, add collateral
//   4  borrower_borrow            lend_op borrow
//   5  accrual_and_health_check   wait, check_health accrues the pool, indexes rise
//   6  borrower_repay_full        repay with amount = full_repay_amount
//   7  lender_withdraw_supply     withdraw supply, receives >= supplied
//   8  target_position            third keypair Position borrows near the limit
//   9  health_check_target        check_health on the third Position
//   10 liquidation_refused        healthy Position: refused, USDC refunded
//   11 liquidation_applied        only when the health check reports unhealthy
//   12 key_rotation               new Viewing Key decrypts, old one fails
//   13 disclosure                 field-masked disclosure, auditor decrypts
//
// Instructions are built directly with `program.methods.<name>` and the exact
// account names of the final IDL (target/idl/walnut.json). The SDK is only
// used for pure maths (applyLendOp, share helpers, pool decode and index
// projection), PDA derivation, Viewing Key and disclosure decryption.
//
// State lives under ~/.walnut-demo/devnet (override WALNUT_DEMO_DIR): the demo
// owner (owner.json, shared with demo:balance and demo:loan) and the third
// keypair (lend-target.json). RPC and wallet come from ANCHOR_PROVIDER_URL and
// ANCHOR_WALLET like the other demos; the RPC URL and any api-key are never
// printed. Output: artifacts/devnet-lend-e2e.json.

import * as anchor from "@anchor-lang/core";
import { BN } from "@anchor-lang/core";
import { x25519 } from "@arcium-hq/client";
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
import {
  decodeDisclosure,
  decryptDisclosure,
  DisclosureField,
  disclosureMask,
} from "../src/disclosure.ts";
import {
  applyLendOp,
  fullRepayAmount,
  LendOp,
  MAX_COLLATERAL_LAMPORTS,
  type LendCtx,
} from "../src/lend-op.ts";
import {
  accruedDebt,
  accruedSupply,
  decodeLendingPool,
  LAUNCH_RATE_PARAMS,
  projectIndexes,
  type LendingPool,
  type RateParams,
} from "../src/lending-pool.ts";
import { normalizePrice, parsePriceUpdateV2 } from "../src/oracle.ts";
import {
  deriveArciumQueueAccounts,
  deriveDisclosurePda,
  deriveIntentPda,
  deriveLendingPoolPda,
  deriveOperationCounterPda,
} from "../src/pda.ts";
import {
  decryptPosition,
  deriveViewingKey,
  rotationArgs,
  viewingKeyMessageBytes,
  ViewingKeyDecryptError,
  type PositionBalances,
  type ViewingKeyMaterial,
} from "../src/viewing-key.ts";

const NETWORK = "devnet";
const TOKEN_PROGRAM_ID = new PublicKey(
  "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
);
const ATA_PROGRAM_ID = new PublicKey(
  "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
);

const SUPPLY_USDC = 5_000_000n; // lender supplies 5 test USDC
const COLLATERAL_LAMPORTS = 200_000_000n; // borrower collateral, 0.2 SOL
const BORROW_MAX = 10_000_000n; // borrower borrows up to 10 USDC
const BORROW_PERCENT = 80n; // of the LTV capacity at the current price
const TOPUP_USDC = 1_000_000n; // covers accrued interest on the full repay
const TARGET_COLLATERAL = 50_000_000n; // third Position, 0.05 SOL
const TARGET_PERCENT = 90n; // "near the limit"
const MIN_BORROW = 1_000_000n; // below this a borrow is not worth a step
const PROBE_REPAY = 500_000n; // 0.5 USDC liquidation attempt on a healthy Position
const ACCRUAL_WAIT_MS = 45_000;
const ACTOR_MIN_LAMPORTS = 250_000_000;
const ACTOR_TOPUP_TO = 400_000_000;
const COMPUTE_UNIT_LIMIT = 600_000;
const INTENT_TIMEOUT_MS = 300_000;
const MAX_POSITION_DEBT_CAP = 1_000_000_000_000n;

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
  remainingAccounts(
    accounts: Array<{
      pubkey: PublicKey;
      isSigner: boolean;
      isWritable: boolean;
    }>,
  ): MethodBuilder;
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
  viewingPubkey: number[];
  viewingKeyEpoch: BN;
  nextShieldSeq: BN;
  encryptedState: { nonce: BN; ciphertexts: number[][] };
}
interface StatusAccount {
  status: Json;
}
interface CounterAccount {
  nextSequence: BN;
}
interface DisclosureAccount {
  mask: number;
  nonce: BN;
  ciphertexts: number[][];
}

/** A wallet that owns (or pays for) Positions. */
interface Actor {
  label: string;
  keypair: Keypair;
  position: PublicKey;
  counter: PublicKey;
  key?: ViewingKeyMaterial | undefined;
  epoch: bigint;
}

interface QueuedEvent {
  amount: bigint;
  shares: bigint;
  borrowIndex1e9: bigint;
  supplyIndex1e9: bigint;
  price: bigint;
}

interface RiskView {
  ltvBps: bigint;
  liqThresholdBps: bigint;
  bonusBps: bigint;
  reserveFactorBps: bigint;
  positionDebtCap: bigint;
  rates: RateParams;
}

interface StepRecord {
  name: string;
  status: "ok" | "skipped" | "failed";
  detail: string;
  ms: number;
  checks: string[];
  signatures: string[];
}

interface StepOutcome {
  detail: string;
  skipped?: boolean;
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
function usdc(base: bigint): string {
  return (Number(base) / 1e6).toFixed(6);
}
function explorer(signature: string): string {
  return `https://explorer.solana.com/tx/${signature}?cluster=devnet`;
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
const big = (v: unknown): bigint =>
  BigInt((v as { toString(): string }).toString());

/** Reads `name` from an Anchor-decoded object, ignoring case and underscores. */
function field(obj: unknown, name: string): unknown {
  const wanted = name.toLowerCase().replace(/_/g, "");
  for (const [k, v] of Object.entries(obj as Record<string, unknown>)) {
    if (k.toLowerCase().replace(/_/g, "") === wanted) return v;
  }
  throw new Error(`decoded account has no field ${name}`);
}

function loadOrCreateKeypair(file: string): Keypair {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  if (fs.existsSync(file)) {
    return Keypair.fromSecretKey(
      new Uint8Array(JSON.parse(fs.readFileSync(file, "utf8")) as number[]),
    );
  }
  const key = Keypair.generate();
  fs.writeFileSync(file, JSON.stringify(Array.from(key.secretKey)), {
    mode: 0o600,
  });
  return key;
}

const BALANCE_KEYS = [
  "freeSol",
  "freeUsdc",
  "collateralSol",
  "supplySharesUsdc",
  "debtSharesUsdc",
] as const;

// -------------------------------------------------------------------- demo

class LendE2E {
  readonly ctx: BenchContext;
  readonly programId: PublicKey;
  readonly deployer: Actor;
  readonly borrower: Actor;
  readonly target: Actor;
  readonly market: PublicKey;
  readonly lendingPool: PublicKey;
  readonly steps: StepRecord[] = [];
  mk!: MarketAccount;
  risk!: RiskView;
  oracle!: PublicKey;
  checks: string[] = [];
  signatures: string[] = [];
  /** Supply shares the lender gained in this run. */
  lenderShares = 0n;
  lenderSupplyIndex = 0n;
  accrual: Record<string, string> = {};
  liquidation: Record<string, string> = {};

  constructor(ctx: BenchContext) {
    this.ctx = ctx;
    this.programId = ctx.programId;
    this.market = PublicKey.findProgramAddressSync(
      [seedBytes("market")],
      this.programId,
    )[0];
    this.lendingPool = deriveLendingPoolPda(this.programId, this.market);
    this.deployer = this.actor("lender (deployer)", ctx.payer);
    this.borrower = this.actor(
      "borrower (demo owner)",
      loadOrCreateKeypair(path.join(stateDir(), "owner.json")),
    );
    this.target = this.actor(
      "third position",
      loadOrCreateKeypair(path.join(stateDir(), "lend-target.json")),
    );
  }

  private actor(label: string, keypair: Keypair): Actor {
    return {
      label,
      keypair,
      position: PublicKey.findProgramAddressSync(
        [
          seedBytes("position"),
          this.market.toBytes(),
          keypair.publicKey.toBytes(),
        ],
        this.programId,
      )[0],
      counter: deriveOperationCounterPda(this.programId, keypair.publicKey),
      epoch: 0n,
    };
  }

  // ---- plumbing

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
  ): Promise<string> {
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
    this.signatures.push(signature);
    return signature;
  }

  private queueAccounts(circuit: string, offset: BN) {
    return deriveArciumQueueAccounts({
      programId: this.programId,
      clusterOffset: this.ctx.clusterOffset,
      computationOffset: offset,
      circuit,
    });
  }

  private ata(owner: PublicKey, mint: PublicKey): PublicKey {
    return PublicKey.findProgramAddressSync(
      [owner.toBytes(), TOKEN_PROGRAM_ID.toBytes(), mint.toBytes()],
      ATA_PROGRAM_ID,
    )[0];
  }

  private async ensureAta(actor: Actor, mint: PublicKey): Promise<PublicKey> {
    const address = this.ata(actor.keypair.publicKey, mint);
    const existing = await retry("ata", () =>
      this.ctx.connection.getAccountInfo(address, "confirmed"),
    );
    if (existing !== null) return address;
    const ix = new TransactionInstruction({
      programId: ATA_PROGRAM_ID,
      keys: [
        { pubkey: actor.keypair.publicKey, isSigner: true, isWritable: true },
        { pubkey: address, isSigner: false, isWritable: true },
        {
          pubkey: actor.keypair.publicKey,
          isSigner: false,
          isWritable: false,
        },
        { pubkey: mint, isSigner: false, isWritable: false },
        { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
        { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
      ],
      data: Buffer.from([1]), // CreateIdempotent
    });
    await this.send([ix], [actor.keypair]);
    return address;
  }

  /** The deployer is the test USDC mint authority: mints to `owner`'s ATA. */
  private async mintTestUsdc(owner: Actor, amount: bigint): Promise<void> {
    const dest = await this.ensureAta(owner, this.mk.usdcMint);
    const data = Buffer.alloc(9);
    data.writeUInt8(7, 0); // MintTo
    data.writeBigUInt64LE(amount, 1);
    const ix = new TransactionInstruction({
      programId: TOKEN_PROGRAM_ID,
      keys: [
        { pubkey: this.mk.usdcMint, isSigner: false, isWritable: true },
        { pubkey: dest, isSigner: false, isWritable: true },
        {
          pubkey: this.deployer.keypair.publicKey,
          isSigner: true,
          isWritable: false,
        },
      ],
      data,
    });
    await this.send([ix], [this.deployer.keypair]);
  }

  private async ensureFunds(actor: Actor): Promise<void> {
    if (actor === this.deployer) return;
    const have = await this.balanceOf(actor.keypair.publicKey);
    if (have >= ACTOR_MIN_LAMPORTS) return;
    await this.send(
      [
        SystemProgram.transfer({
          fromPubkey: this.deployer.keypair.publicKey,
          toPubkey: actor.keypair.publicKey,
          lamports: ACTOR_TOPUP_TO - have,
        }),
      ],
      [this.deployer.keypair],
    );
    log(`  funded ${actor.label} with ${sol(ACTOR_TOPUP_TO - have)} SOL`);
  }

  // ---- assertions

  private check(label: string, actual: bigint, expected: bigint): void {
    if (actual !== expected) {
      throw new Error(`${label}: expected ${expected}, got ${actual}`);
    }
    this.checks.push(`${label} = ${actual}`);
    log(`  ok  ${label} = ${actual}`);
  }

  private checkAtLeast(label: string, actual: bigint, minimum: bigint): void {
    if (actual < minimum) {
      throw new Error(`${label}: expected at least ${minimum}, got ${actual}`);
    }
    this.checks.push(`${label} = ${actual} (>= ${minimum})`);
    log(`  ok  ${label} = ${actual} (>= ${minimum})`);
  }

  private checkState(
    label: string,
    got: PositionBalances,
    expected: PositionBalances,
  ): void {
    for (const k of BALANCE_KEYS) {
      this.check(`${label}: ${k}`, got[k], expected[k]);
    }
  }

  private async step(
    name: string,
    fn: () => Promise<StepOutcome>,
  ): Promise<void> {
    log(`\n[${name}]`);
    this.checks = [];
    this.signatures = [];
    const started = Date.now();
    try {
      const out = await fn();
      this.steps.push({
        name,
        status: out.skipped === true ? "skipped" : "ok",
        detail: out.detail,
        ms: Date.now() - started,
        checks: this.checks,
        signatures: this.signatures,
      });
      log(`  -> ${out.skipped === true ? "skipped" : "ok"}: ${out.detail}`);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      this.steps.push({
        name,
        status: "failed",
        detail: message.split("\n")[0] ?? message,
        ms: Date.now() - started,
        checks: this.checks,
        signatures: this.signatures,
      });
      this.writeArtifact(false);
      throw new Error(`step "${name}" failed: ${message}`, { cause: err });
    }
  }

  // ---- Viewing Keys and Positions

  private async keyFor(
    actor: Actor,
    epoch: bigint,
  ): Promise<ViewingKeyMaterial> {
    const message = viewingKeyMessageBytes({
      programId: this.programId,
      network: NETWORK,
      positionPda: actor.position,
      epoch,
    });
    return deriveViewingKey(signMessage(actor.keypair, message), {
      wipeSignature: true,
    });
  }

  /** Derives the key of the on-chain epoch and checks it against the Position. */
  private async syncKey(actor: Actor): Promise<void> {
    const position = await this.fetch<PositionAccount>(
      "position",
      actor.position,
    );
    if (position === null) throw new Error(`${actor.label}: Position missing`);
    const epoch = big(position.viewingKeyEpoch);
    const key = await this.keyFor(actor, epoch);
    const same =
      key.publicKey.length === position.viewingPubkey.length &&
      key.publicKey.every((b, i) => b === position.viewingPubkey[i]);
    if (!same) {
      throw new Error(
        `${actor.label}: the wallet-derived Viewing Key for epoch ${epoch} does not match the Position`,
      );
    }
    actor.key = key;
    actor.epoch = epoch;
  }

  private decryptWith(
    position: PositionAccount,
    secret: Uint8Array,
  ): PositionBalances {
    return decryptPosition({
      encryptedState: {
        nonce: big(position.encryptedState.nonce),
        ciphertexts: position.encryptedState.ciphertexts,
      },
      viewingSecret: secret,
      mxePublicKey: this.ctx.mxePublicKey,
    });
  }

  private async decrypt(actor: Actor): Promise<PositionBalances> {
    const position = await this.fetch<PositionAccount>(
      "position",
      actor.position,
    );
    if (position === null) throw new Error(`${actor.label}: Position missing`);
    if (actor.key === undefined) throw new Error(`${actor.label}: no key`);
    return this.decryptWith(position, actor.key.secret);
  }

  private async ensurePosition(actor: Actor): Promise<void> {
    await this.ensureFunds(actor);
    const position = await this.fetch<PositionAccount>(
      "position",
      actor.position,
    );
    if (position === null) {
      const key = await this.keyFor(actor, 0n);
      const counter = await this.fetch<CounterAccount>(
        "operationCounter",
        actor.counter,
      );
      const sequence = counter?.nextSequence ?? new BN(0);
      const offset = newOffset();
      const ix = await this.method("initializePosition")(
        offset,
        sequence,
        Array.from(key.publicKey),
        new BN(randomBytes(16)),
      )
        .accountsPartial({
          payer: actor.keypair.publicKey,
          ...this.queueAccounts("init_position_state", offset),
          market: this.market,
          position: actor.position,
          operationCounter: actor.counter,
          operationIntent: deriveIntentPda(
            this.programId,
            actor.keypair.publicKey,
            BigInt(sequence.toString()),
          ),
        })
        .instruction();
      const sig = await this.send([ix], [actor.keypair]);
      log(`  initialize_position (${actor.label}): ${explorer(sig)}`);
    }
    await waitFor(
      `${actor.label} Position Active`,
      async () => {
        const p = await this.fetch<PositionAccount>("position", actor.position);
        if (p === null) return undefined;
        const name = statusName(p.status);
        if (name === "failedInit" || name === "failed") {
          throw new Error(`Position initialization ended ${name}`);
        }
        return name === "active" ? p : undefined;
      },
      INTENT_TIMEOUT_MS,
    );
    await this.syncKey(actor);
  }

  // ---- queue machinery

  /**
   * Queues one operation signed by `payer`, then waits for its Intent to leave
   * `queued`. `build` receives the next sequence of the payer's counter.
   */
  private async queue(
    payer: Keypair,
    label: string,
    build: (p: {
      sequence: BN;
      offset: BN;
      intent: PublicKey;
    }) => Promise<TransactionInstruction[]>,
  ): Promise<{ signature: string; status: string; ms: number }> {
    const started = Date.now();
    const counter = await this.fetch<CounterAccount>(
      "operationCounter",
      deriveOperationCounterPda(this.programId, payer.publicKey),
    );
    const sequence = counter?.nextSequence ?? new BN(0);
    const offset = newOffset();
    const intent = deriveIntentPda(
      this.programId,
      payer.publicKey,
      BigInt(sequence.toString()),
    );
    const instructions = await build({ sequence, offset, intent });
    const signature = await this.send(instructions, [payer]);
    log(`  ${label}: queued ${explorer(signature)}`);
    const status = await waitFor(
      `${label} Intent terminal`,
      async () => {
        const i = await this.fetch<StatusAccount>("operationIntent", intent);
        return i !== null && statusName(i.status) !== "queued"
          ? statusName(i.status)
          : undefined;
      },
      INTENT_TIMEOUT_MS,
    );
    return { signature, status, ms: Date.now() - started };
  }

  /** Reads the `LendOpQueued` event of a queue transaction. */
  private async queuedEvent(signature: string): Promise<QueuedEvent> {
    const tx = await waitFor(
      "queue transaction logs",
      async () => {
        const t = await this.ctx.connection.getTransaction(signature, {
          commitment: "confirmed",
          maxSupportedTransactionVersion: 0,
        });
        return t?.meta?.logMessages ? t : undefined;
      },
      60_000,
    );
    const parser = new anchor.EventParser(
      this.programId,
      this.ctx.program.coder,
    );
    for (const ev of parser.parseLogs(tx.meta?.logMessages ?? [])) {
      if (ev.name.toLowerCase() === "lendopqueued") {
        return {
          amount: big(field(ev.data, "amount")),
          shares: big(field(ev.data, "shares")),
          borrowIndex1e9: big(field(ev.data, "borrow_index_1e9")),
          supplyIndex1e9: big(field(ev.data, "supply_index_1e9")),
          price: big(field(ev.data, "price")),
        };
      }
    }
    throw new Error(`no LendOpQueued event in ${signature}`);
  }

  private lendCtx(ev: QueuedEvent): LendCtx {
    return {
      borrowIndex1e9: ev.borrowIndex1e9,
      price: ev.price,
      ltvBps: this.risk.ltvBps,
      liqThresholdBps: this.risk.liqThresholdBps,
      positionDebtCap:
        this.risk.positionDebtCap > MAX_POSITION_DEBT_CAP
          ? MAX_POSITION_DEBT_CAP
          : this.risk.positionDebtCap,
    };
  }

  /**
   * Owner-signed `lend_op` (ops 0 to 5). Compares the decrypted Position with
   * the SDK mirror `applyLendOp` applied to the previous state, using the
   * shares, index and price the program reported in `LendOpQueued`.
   */
  private async lendOp(
    actor: Actor,
    label: string,
    op: LendOp,
    amount: bigint,
    expected: "applied" | "zero" = "applied",
  ): Promise<{
    applied: boolean;
    state: PositionBalances;
    event: QueuedEvent;
  }> {
    const prev = await this.decrypt(actor);
    const r = await this.queue(
      actor.keypair,
      label,
      async ({ sequence, offset, intent }) => [
        await this.method("lendOp")(
          offset,
          sequence,
          op,
          new BN(amount.toString()),
        )
          .accountsPartial({
            payer: actor.keypair.publicKey,
            ...this.queueAccounts("lend_op", offset),
            market: this.market,
            assetState: this.mk.assetState,
            lendingPool: this.lendingPool,
            usdcVault: this.mk.usdcVault,
            position: actor.position,
            operationCounter: actor.counter,
            operationIntent: intent,
            priceUpdate: this.oracle,
          })
          .instruction(),
      ],
    );
    if (r.status !== expected) {
      throw new Error(
        `${label}: Intent ended ${r.status}, expected ${expected}`,
      );
    }
    const ev = await this.queuedEvent(r.signature);
    const mirror = applyLendOp(prev, op, amount, ev.shares, this.lendCtx(ev));
    if (mirror.applied !== (r.status === "applied")) {
      throw new Error(
        `${label}: SDK mirror applied=${mirror.applied} but the circuit ended ${r.status}`,
      );
    }
    const got = await this.decrypt(actor);
    this.checkState(label, got, mirror.state);
    return { applied: mirror.applied, state: got, event: ev };
  }

  private async healthCheck(
    label: string,
    payer: Actor,
    target: Actor,
  ): Promise<{ unhealthy: boolean; signature: string }> {
    const prev = await this.decrypt(target);
    const r = await this.queue(
      payer.keypair,
      label,
      async ({ sequence, offset, intent }) => [
        await this.method("checkHealth")(offset, sequence)
          .accountsPartial({
            payer: payer.keypair.publicKey,
            ...this.queueAccounts("lend_op", offset),
            market: this.market,
            assetState: this.mk.assetState,
            lendingPool: this.lendingPool,
            position: target.position,
            operationCounter: payer.counter,
            operationIntent: intent,
            priceUpdate: this.oracle,
          })
          .instruction(),
      ],
    );
    if (r.status !== "applied" && r.status !== "zero") {
      throw new Error(`${label}: Intent ended ${r.status}`);
    }
    // A health check never changes the Position.
    this.checkState(
      `${label}: Position unchanged`,
      await this.decrypt(target),
      prev,
    );
    return { unhealthy: r.status === "applied", signature: r.signature };
  }

  private async shield(
    actor: Actor,
    label: string,
    asset: "sol" | "usdc",
    amount: bigint,
  ): Promise<void> {
    const prev = await this.decrypt(actor);
    const mint = asset === "sol" ? this.mk.wsolMint : this.mk.usdcMint;
    const vault = asset === "sol" ? this.mk.wsolVault : this.mk.usdcVault;
    const source = await this.ensureAta(actor, mint);
    const vaultBefore = await this.tokenBalance(vault);
    const position = await this.fetch<PositionAccount>(
      "position",
      actor.position,
    );
    if (position === null) throw new Error(`${label}: Position missing`);
    const shieldSequence = position.nextShieldSeq;
    const pendingShield = PublicKey.findProgramAddressSync(
      [
        seedBytes("pending_shield"),
        actor.position.toBytes(),
        shieldSequence.toArrayLike(Buffer, "le", 8),
      ],
      this.programId,
    )[0];
    const r = await this.queue(
      actor.keypair,
      label,
      async ({ sequence, offset, intent }) => {
        const ixs: TransactionInstruction[] = [];
        if (asset === "sol") {
          ixs.push(
            SystemProgram.transfer({
              fromPubkey: actor.keypair.publicKey,
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
              payer: actor.keypair.publicKey,
              ...this.queueAccounts("shield_credit", offset),
              market: this.market,
              assetState: this.mk.assetState,
              position: actor.position,
              operationCounter: actor.counter,
              operationIntent: intent,
              pendingShield,
              mint,
              vault,
              sourceTokenAccount: source,
              tokenProgram: TOKEN_PROGRAM_ID,
            })
            .instruction(),
        );
        return ixs;
      },
    );
    if (r.status !== "applied") {
      throw new Error(`${label}: Intent ended ${r.status}, expected applied`);
    }
    await waitFor(
      "Pending Shield credited",
      async () => {
        const p = await this.fetch<StatusAccount>(
          "pendingShield",
          pendingShield,
        );
        return p !== null && statusName(p.status) === "credited"
          ? p
          : undefined;
      },
      90_000,
    );
    this.check(
      `${label}: vault delta`,
      (await this.tokenBalance(vault)) - vaultBefore,
      amount,
    );
    const expected: PositionBalances =
      asset === "sol"
        ? { ...prev, freeSol: prev.freeSol + amount }
        : { ...prev, freeUsdc: prev.freeUsdc + amount };
    this.checkState(label, await this.decrypt(actor), expected);
  }

  // ---- public values

  private async readPool(): Promise<LendingPool> {
    const info = await retry("getAccountInfo", () =>
      this.ctx.connection.getAccountInfo(this.lendingPool, "confirmed"),
    );
    if (info === null) {
      throw new Error("LendingPool missing: run scripts/lend-devnet.sh first");
    }
    return decodeLendingPool(info.data);
  }

  /** Low-bound SOL price in USDC base units per SOL from the pool's oracle. */
  private async oraclePrice(): Promise<bigint> {
    const info = await retry("oracle", () =>
      this.ctx.connection.getAccountInfo(this.oracle, "confirmed"),
    );
    if (info === null) throw new Error("oracle account not found");
    const update = parsePriceUpdateV2(info.data);
    const normalized = normalizePrice(
      update.price - update.conf,
      update.exponent,
      false,
    );
    if (!normalized.ok) throw new Error(`oracle price: ${normalized.error}`);
    return normalized.value / 100n; // 1e8 USD scale -> USDC base units (1e6)
  }

  private async borrowHeadroom(
    state: PositionBalances,
    percent: bigint,
  ): Promise<bigint> {
    const pool = await this.readPool();
    const now = BigInt(Math.floor(Date.now() / 1000));
    const index = projectIndexes(
      pool,
      now > pool.lastAccrualTs ? now : pool.lastAccrualTs,
      {
        reserveFactorBps: this.risk.reserveFactorBps,
        rates: this.risk.rates,
      },
    ).borrowIndex1e9;
    const price = await this.oraclePrice();
    const capacity =
      (state.collateralSol * price * this.risk.ltvBps) /
      (10_000n * 1_000_000_000n);
    const allowed = (capacity * percent) / 100n;
    const debt = accruedDebt(state.debtSharesUsdc, index);
    let headroom = allowed > debt ? allowed - debt : 0n;
    const vault = await this.tokenBalance(this.mk.usdcVault);
    const free = vault > pool.reservesUsdc ? vault - pool.reservesUsdc : 0n;
    if (headroom > free) headroom = free > 1_000_000n ? free - 1_000_000n : 0n;
    return headroom;
  }

  // ---- liquidation

  private async liquidate(
    label: string,
    target: Actor,
    repay: bigint,
    expected: "applied" | "zero",
  ): Promise<void> {
    const liquidator = this.deployer;
    const usdcAta = await this.ensureAta(liquidator, this.mk.usdcMint);
    const wsolAta = await this.ensureAta(liquidator, this.mk.wsolMint);
    if ((await this.tokenBalance(usdcAta)) < repay) {
      await this.mintTestUsdc(liquidator, repay);
    }
    const prev = await this.decrypt(target);
    const usdcBefore = await this.tokenBalance(usdcAta);
    const wsolBefore = await this.tokenBalance(wsolAta);
    const vaultBefore = await this.tokenBalance(this.mk.usdcVault);
    const r = await this.queue(
      liquidator.keypair,
      label,
      async ({ sequence, offset, intent }) => [
        await this.method("liquidate")(
          offset,
          sequence,
          new BN(repay.toString()),
        )
          .accountsPartial({
            payer: liquidator.keypair.publicKey,
            ...this.queueAccounts("lend_op", offset),
            market: this.market,
            assetState: this.mk.assetState,
            lendingPool: this.lendingPool,
            usdcVault: this.mk.usdcVault,
            position: target.position,
            operationCounter: liquidator.counter,
            operationIntent: intent,
            priceUpdate: this.oracle,
          })
          // [liquidator_usdc, liquidator_wsol, token_program, wsol_vault]
          .remainingAccounts([
            { pubkey: usdcAta, isSigner: false, isWritable: true },
            { pubkey: wsolAta, isSigner: false, isWritable: true },
            { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
            { pubkey: this.mk.wsolVault, isSigner: false, isWritable: false },
          ])
          .instruction(),
      ],
    );
    if (r.status !== expected) {
      throw new Error(
        `${label}: Intent ended ${r.status}, expected ${expected}`,
      );
    }
    const after = await this.decrypt(target);
    const usdcAfter = await this.tokenBalance(usdcAta);
    const wsolAfter = await this.tokenBalance(wsolAta);
    if (expected === "zero") {
      this.checkState(`${label}: Position unchanged`, after, prev);
      this.check(`${label}: liquidator USDC refunded`, usdcAfter, usdcBefore);
      this.check(`${label}: liquidator wSOL unchanged`, wsolAfter, wsolBefore);
      this.check(
        `${label}: vault USDC unchanged`,
        await this.tokenBalance(this.mk.usdcVault),
        vaultBefore,
      );
      return;
    }
    const ev = await this.queuedEvent(r.signature);
    const mirror = applyLendOp(
      prev,
      LendOp.Liquidate,
      ev.amount,
      ev.shares,
      this.lendCtx(ev),
    );
    if (!mirror.applied) {
      throw new Error(
        `${label}: SDK mirror refuses the liquidation the circuit applied`,
      );
    }
    this.checkState(label, after, mirror.state);
    this.check(`${label}: wSOL seized`, wsolAfter - wsolBefore, ev.amount);
    this.check(`${label}: USDC paid`, usdcBefore - usdcAfter, repay);
    this.liquidation["repayUsdc"] = repay.toString();
    this.liquidation["seizedLamports"] = ev.amount.toString();
    this.liquidation["debtSharesBurned"] = ev.shares.toString();
  }

  // ---- steps

  async setup(): Promise<void> {
    await this.step("setup", async () => {
      const market = await this.fetch<MarketAccount>("market", this.market);
      if (market === null) throw new Error("Market missing: run demo:balance");
      this.mk = market;
      const pool = await this.readPool();
      this.oracle = pool.oracle;
      // The SDK decoder must agree with the IDL decoder.
      const decoded = await this.fetch<Json>("lendingPool", this.lendingPool);
      if (decoded === null) throw new Error("LendingPool missing");
      this.check(
        "SDK decode: supply index",
        pool.supplyIndex1e9,
        big(field(decoded, "supply_index_1e9")),
      );
      this.check(
        "SDK decode: borrow index",
        pool.borrowIndex1e9,
        big(field(decoded, "borrow_index_1e9")),
      );
      const asset = await this.fetch<Json>("assetState", market.assetState);
      if (asset === null) throw new Error("AssetState missing");
      const risk = field(asset, "risk");
      const rates = field(asset, "rates");
      const caps = field(asset, "caps");
      this.risk = {
        ltvBps: big(field(risk, "ltv_bps")),
        liqThresholdBps: big(field(risk, "liquidation_threshold_bps")),
        bonusBps: big(field(risk, "liquidation_bonus_bps")),
        reserveFactorBps: big(field(risk, "reserve_factor_bps")),
        positionDebtCap: big(field(caps, "position_debt_cap_usdc")),
        rates: {
          baseRateBps: big(field(rates, "base_rate_bps")),
          slope1Bps: big(field(rates, "slope1_bps")),
          slope2Bps: big(field(rates, "slope2_bps")),
          optimalUtilizationBps: big(field(rates, "optimal_utilization_bps")),
        },
      };
      if (
        this.risk.rates.optimalUtilizationBps <= 0n ||
        this.risk.rates.slope1Bps < 0n
      ) {
        this.risk.rates = LAUNCH_RATE_PARAMS;
      }
      const price = await this.oraclePrice();
      log(`  oracle low-bound price: ${usdc(price)} USDC per SOL`);
      log(
        `  LTV ${this.risk.ltvBps} bps, liquidation threshold ${this.risk.liqThresholdBps} bps, bonus ${this.risk.bonusBps} bps`,
      );
      this.accrual["supplyIndexStart"] = pool.supplyIndex1e9.toString();
      this.accrual["borrowIndexStart"] = pool.borrowIndex1e9.toString();
      return {
        detail: `pool ${this.lendingPool.toBase58()}, price ${usdc(price)} USDC/SOL`,
      };
    });
  }

  async lenderSupply(): Promise<void> {
    await this.step("lender_supply", async () => {
      await this.ensurePosition(this.deployer);
      await this.mintTestUsdc(this.deployer, SUPPLY_USDC);
      const before = await this.decrypt(this.deployer);
      await this.shield(
        this.deployer,
        "lender shield USDC",
        "usdc",
        SUPPLY_USDC,
      );
      const r = await this.lendOp(
        this.deployer,
        "lender supply",
        LendOp.Supply,
        SUPPLY_USDC,
      );
      this.lenderShares = r.state.supplySharesUsdc - before.supplySharesUsdc;
      this.lenderSupplyIndex = r.event.supplyIndex1e9;
      this.checkAtLeast("lender supply shares minted", this.lenderShares, 1n);
      return {
        detail: `supplied ${usdc(SUPPLY_USDC)} USDC for ${this.lenderShares} shares at supply index ${this.lenderSupplyIndex}`,
      };
    });
  }

  async borrowerCollateral(): Promise<void> {
    await this.step("borrower_collateral", async () => {
      await this.ensurePosition(this.borrower);
      const s = await this.decrypt(this.borrower);
      if (s.collateralSol >= COLLATERAL_LAMPORTS) {
        return { detail: "collateral already in place", skipped: true };
      }
      const need = COLLATERAL_LAMPORTS - s.collateralSol;
      if (need > MAX_COLLATERAL_LAMPORTS)
        throw new Error("collateral too large");
      if (s.freeSol < need) {
        await this.shield(
          this.borrower,
          "borrower shield SOL",
          "sol",
          need - s.freeSol,
        );
      }
      await this.lendOp(
        this.borrower,
        "add collateral",
        LendOp.AddCollateral,
        need,
      );
      return { detail: `collateral ${sol(COLLATERAL_LAMPORTS)} SOL` };
    });
  }

  async borrowerBorrow(): Promise<void> {
    await this.step("borrower_borrow", async () => {
      const s = await this.decrypt(this.borrower);
      let amount = await this.borrowHeadroom(s, BORROW_PERCENT);
      if (amount > BORROW_MAX) amount = BORROW_MAX;
      if (amount < MIN_BORROW) {
        throw new Error(
          `borrow headroom is only ${usdc(amount)} USDC (collateral, price or vault liquidity too low)`,
        );
      }
      const r = await this.lendOp(
        this.borrower,
        "borrow",
        LendOp.Borrow,
        amount,
      );
      this.checkAtLeast(
        "debt shares minted",
        r.state.debtSharesUsdc - s.debtSharesUsdc,
        1n,
      );
      this.accrual["borrowedUsdc"] = amount.toString();
      return { detail: `borrowed ${usdc(amount)} USDC` };
    });
  }

  async accrualAndHealth(): Promise<void> {
    await this.step("accrual_and_health_check", async () => {
      const before = await this.readPool();
      log(
        `  indexes now: supply ${before.supplyIndex1e9}, borrow ${before.borrowIndex1e9}`,
      );
      log(`  waiting ${ACCRUAL_WAIT_MS / 1000} s for interest to accrue...`);
      await sleep(ACCRUAL_WAIT_MS);
      const projected = projectIndexes(
        before,
        BigInt(Math.floor(Date.now() / 1000)),
        {
          reserveFactorBps: this.risk.reserveFactorBps,
          rates: this.risk.rates,
        },
      );
      // check_health accrues the pool (and never changes the Position).
      const h = await this.healthCheck(
        "health check (borrower)",
        this.deployer,
        this.borrower,
      );
      if (h.unhealthy) {
        throw new Error(
          "borrower Position reported unhealthy right after borrowing 80% of the LTV",
        );
      }
      this.checks.push("borrower Position healthy (unhealthy = false)");
      const after = await this.readPool();
      this.checkAtLeast(
        "accrual timestamp advanced",
        after.lastAccrualTs - before.lastAccrualTs,
        1n,
      );
      this.checkAtLeast(
        "Borrow_Index rose",
        after.borrowIndex1e9 - before.borrowIndex1e9,
        1n,
      );
      if (projected.supplyIndex1e9 > before.supplyIndex1e9) {
        this.checkAtLeast(
          "Supply_Index rose",
          after.supplyIndex1e9 - before.supplyIndex1e9,
          1n,
        );
      } else {
        this.checkAtLeast(
          "Supply_Index did not fall",
          after.supplyIndex1e9 - before.supplyIndex1e9,
          0n,
        );
      }
      this.accrual["borrowIndexBefore"] = before.borrowIndex1e9.toString();
      this.accrual["borrowIndexAfter"] = after.borrowIndex1e9.toString();
      this.accrual["supplyIndexBefore"] = before.supplyIndex1e9.toString();
      this.accrual["supplyIndexAfter"] = after.supplyIndex1e9.toString();
      this.accrual["supplyIndexRoundsToZeroAtThisUtilization"] = String(
        projected.supplyIndex1e9 <= before.supplyIndex1e9,
      );
      return {
        detail: `borrow index ${before.borrowIndex1e9} -> ${after.borrowIndex1e9}, supply index ${before.supplyIndex1e9} -> ${after.supplyIndex1e9}`,
      };
    });
  }

  async borrowerRepay(): Promise<void> {
    await this.step("borrower_repay_full", async () => {
      await this.mintTestUsdc(this.borrower, TOPUP_USDC);
      await this.shield(
        this.borrower,
        "borrower shield USDC (interest)",
        "usdc",
        TOPUP_USDC,
      );
      let repaid = false;
      for (let attempt = 1; attempt <= 3 && !repaid; attempt += 1) {
        const s = await this.decrypt(this.borrower);
        const pool = await this.readPool();
        // The repay queue accrues the pool again; predict that index.
        const now = BigInt(Math.floor(Date.now() / 1000) + 15);
        const index = projectIndexes(
          pool,
          now > pool.lastAccrualTs ? now : pool.lastAccrualTs,
          {
            reserveFactorBps: this.risk.reserveFactorBps,
            rates: this.risk.rates,
          },
        ).borrowIndex1e9;
        const amount = fullRepayAmount(s.debtSharesUsdc, index);
        this.checkAtLeast(
          "free USDC covers the full repay",
          s.freeUsdc,
          amount,
        );
        const label = `repay full debt (attempt ${attempt})`;
        const r = await this.tryLendOp(
          this.borrower,
          label,
          LendOp.Repay,
          amount,
        );
        if (!r.applied) {
          this.checkState(`: refused, unchanged`, r.state, s);
          log(`  ${label}: refused (index moved), retrying`);
        }
        repaid = r.applied;
      }
      if (!repaid) {
        throw new Error(
          "the full repay was refused three times (index prediction off)",
        );
      }
      const after = await this.decrypt(this.borrower);
      this.check(
        "borrower debt shares after full repay",
        after.debtSharesUsdc,
        0n,
      );
      return { detail: "debt cleared with amount = full_repay_amount" };
    });
  }

  /** `lendOp` that reports a refusal (Intent zero) instead of throwing. */
  private async tryLendOp(
    actor: Actor,
    label: string,
    op: LendOp,
    amount: bigint,
  ): Promise<{ applied: boolean; state: PositionBalances }> {
    try {
      const r = await this.lendOp(actor, label, op, amount, "applied");
      return { applied: true, state: r.state };
    } catch (err) {
      const text = err instanceof Error ? err.message : String(err);
      if (!/Intent ended zero/.test(text)) throw err;
      return { applied: false, state: await this.decrypt(actor) };
    }
  }
  async lenderWithdraw(): Promise<void> {
    await this.step("lender_withdraw_supply", async () => {
      const pool = await this.readPool();
      const shares = this.lenderShares;
      if (shares === 0n)
        throw new Error("no supply shares recorded by lender_supply");
      this.checkAtLeast(
        "Supply_Index at withdrawal vs supply",
        pool.supplyIndex1e9 - this.lenderSupplyIndex,
        0n,
      );
      // At the stored index; the queue accrues further, which only lowers the
      // burn, so the share balance always covers it.
      const amount = accruedSupply(shares, pool.supplyIndex1e9);
      const before = await this.decrypt(this.deployer);
      const r = await this.lendOp(
        this.deployer,
        "withdraw supply",
        LendOp.WithdrawSupply,
        amount,
      );
      const received = r.state.freeUsdc - before.freeUsdc;
      this.check("USDC received", received, amount);
      // Rounding favours the pool by at most one base unit per leg.
      this.checkAtLeast(
        "USDC received vs supplied",
        received,
        SUPPLY_USDC - 1n,
      );
      const gain = received - SUPPLY_USDC;
      this.accrual["lenderReceivedUsdc"] = received.toString();
      this.accrual["lenderSuppliedUsdc"] = SUPPLY_USDC.toString();
      return {
        detail:
          gain > 0n
            ? `received ${received} base units for ${SUPPLY_USDC} supplied (+${gain} interest)`
            : `received ${received} base units for ${SUPPLY_USDC} supplied (interest rounds to ${gain} in this short window)`,
      };
    });
  }

  async targetPosition(): Promise<void> {
    await this.step("target_position", async () => {
      await this.ensurePosition(this.target);
      let s = await this.decrypt(this.target);
      if (s.collateralSol < TARGET_COLLATERAL) {
        const need = TARGET_COLLATERAL - s.collateralSol;
        if (s.freeSol < need) {
          await this.shield(
            this.target,
            "target shield SOL",
            "sol",
            need - s.freeSol,
          );
        }
        await this.lendOp(
          this.target,
          "target add collateral",
          LendOp.AddCollateral,
          need,
        );
        s = await this.decrypt(this.target);
      }
      const amount = await this.borrowHeadroom(s, TARGET_PERCENT);
      if (amount >= MIN_BORROW) {
        await this.lendOp(
          this.target,
          "target borrow near the limit",
          LendOp.Borrow,
          amount,
        );
        return {
          detail: `borrowed ${usdc(amount)} USDC against ${sol(TARGET_COLLATERAL)} SOL`,
        };
      }
      return {
        detail:
          "Position already borrowed near the limit (reused from an earlier run)",
      };
    });
  }

  private targetUnhealthy = false;

  async healthTarget(): Promise<void> {
    await this.step("health_check_target", async () => {
      const h = await this.healthCheck(
        "health check (third position)",
        this.deployer,
        this.target,
      );
      this.targetUnhealthy = h.unhealthy;
      this.liquidation["targetUnhealthy"] = String(h.unhealthy);
      return {
        detail: h.unhealthy
          ? "unhealthy at the current price"
          : "healthy at the current price (unhealthy = false)",
      };
    });
  }

  async liquidationRefused(): Promise<void> {
    await this.step("liquidation_refused", async () => {
      // A healthy Position is needed. The third Position is the subject unless
      // it is unhealthy right now; then the (repaid, debt-free) borrower is.
      const subject = this.targetUnhealthy ? this.borrower : this.target;
      await this.liquidate(
        "liquidate healthy Position",
        subject,
        PROBE_REPAY,
        "zero",
      );
      return {
        detail: `refused on ${subject.label}; ${usdc(PROBE_REPAY)} USDC refunded`,
      };
    });
  }

  async liquidationApplied(): Promise<void> {
    await this.step("liquidation_applied", async () => {
      if (!this.targetUnhealthy) {
        this.liquidation["applied"] =
          "skipped: no unhealthy position at current price";
        return {
          detail: "skipped: no unhealthy position at current price",
          skipped: true,
        };
      }
      const s = await this.decrypt(this.target);
      const pool = await this.readPool();
      const debt = accruedDebt(s.debtSharesUsdc, pool.borrowIndex1e9);
      // Close factor: burn at most half the debt shares; keep a margin.
      const repay = debt / 2n > 2n ? debt / 2n - 2n : 0n;
      if (repay === 0n) {
        return { detail: "skipped: debt too small to split", skipped: true };
      }
      await this.liquidate(
        "liquidate unhealthy Position",
        this.target,
        repay,
        "applied",
      );
      this.liquidation["applied"] = "applied";
      return {
        detail: `repaid ${usdc(repay)} USDC, collateral seized with the bonus`,
      };
    });
  }

  async keyRotation(): Promise<void> {
    await this.step("key_rotation", async () => {
      const actor = this.target;
      const before = await this.fetch<PositionAccount>(
        "position",
        actor.position,
      );
      if (before === null || actor.key === undefined)
        throw new Error("no Position");
      const oldKey = actor.key;
      const oldSecret = new Uint8Array(oldKey.secret);
      const oldEpoch = actor.epoch;
      const balances = this.decryptWith(before, oldSecret);
      const newKey = await this.keyFor(actor, oldEpoch + 1n);
      const args = rotationArgs({
        oldEpoch,
        oldPublicKey: oldKey.publicKey,
        newPublicKey: newKey.publicKey,
      });
      const r = await this.queue(
        actor.keypair,
        "rotate Viewing Key",
        async ({ sequence, offset, intent }) => [
          await this.method("rotatePositionKey")(
            offset,
            sequence,
            new BN(args.expectedOldEpoch.toString()),
            new BN(args.expectedNewEpoch.toString()),
            Array.from(args.expectedOldPubkey),
            Array.from(args.newViewingPubkey),
          )
            .accountsPartial({
              payer: actor.keypair.publicKey,
              ...this.queueAccounts("rotate_position_key", offset),
              position: actor.position,
              operationCounter: actor.counter,
              operationIntent: intent,
            })
            .instruction(),
        ],
      );
      if (r.status !== "applied") {
        throw new Error(`rotation Intent ended ${r.status}, expected applied`);
      }
      const after = await this.fetch<PositionAccount>(
        "position",
        actor.position,
      );
      if (after === null) throw new Error("Position missing after rotation");
      this.check(
        "viewing key epoch",
        big(after.viewingKeyEpoch),
        oldEpoch + 1n,
      );
      this.check(
        "stored Viewing public key is the new key",
        BigInt(
          Buffer.from(after.viewingPubkey).equals(Buffer.from(newKey.publicKey))
            ? 1
            : 0,
        ),
        1n,
      );
      this.checkState(
        "new key decrypts the same balances",
        this.decryptWith(after, newKey.secret),
        balances,
      );
      let oldFailed = false;
      try {
        this.decryptWith(after, oldSecret);
      } catch (err) {
        oldFailed = err instanceof ViewingKeyDecryptError;
        if (!oldFailed) throw err;
      }
      this.check(
        "old key fails to decrypt the rotated Position",
        BigInt(oldFailed ? 1 : 0),
        1n,
      );
      actor.key = newKey;
      actor.epoch = oldEpoch + 1n;
      return {
        detail: `epoch ${oldEpoch} -> ${oldEpoch + 1n}, new key decrypts, old key fails`,
      };
    });
  }

  async disclosure(): Promise<void> {
    await this.step("disclosure", async () => {
      const actor = this.target;
      const state = await this.decrypt(actor);
      const auditorSecret = x25519.utils.randomSecretKey();
      const auditorPublic = x25519.getPublicKey(auditorSecret);
      const mask = disclosureMask(
        DisclosureField.FreeUsdc,
        DisclosureField.CollateralSol,
        DisclosureField.DebtShares,
      );
      const disclosure = deriveDisclosurePda(
        this.programId,
        actor.position,
        new PublicKey(auditorPublic),
      );
      const r = await this.queue(
        actor.keypair,
        "disclose to auditor",
        async ({ offset, intent }) => [
          await this.method("disclosePosition")(
            offset,
            Array.from(auditorPublic),
            new BN(mask),
          )
            .accountsPartial({
              payer: actor.keypair.publicKey,
              ...this.queueAccounts("disclose_position", offset),
              position: actor.position,
              operationCounter: actor.counter,
              operationIntent: intent,
              disclosure,
            })
            .instruction(),
        ],
      );
      if (r.status !== "applied") {
        throw new Error(
          `disclosure Intent ended ${r.status}, expected applied`,
        );
      }
      const account = await this.fetch<DisclosureAccount>(
        "disclosure",
        disclosure,
      );
      if (account === null)
        throw new Error("Disclosure account was not written");
      this.check("Disclosure mask", BigInt(account.mask), BigInt(mask));
      // The SDK decoder must read the same bytes.
      const raw = await retry("disclosure bytes", () =>
        this.ctx.connection.getAccountInfo(disclosure, "confirmed"),
      );
      if (raw === null) throw new Error("Disclosure account vanished");
      this.check(
        "SDK decode: Disclosure mask",
        BigInt(decodeDisclosure(raw.data).mask),
        BigInt(mask),
      );
      const fields = decryptDisclosure(
        {
          mask: account.mask,
          nonce: big(account.nonce),
          ciphertexts: account.ciphertexts.map((c) => Uint8Array.from(c)),
        },
        auditorSecret,
        this.ctx.mxePublicKey,
      );
      this.check(
        "auditor sees free USDC",
        fields.freeUsdc ?? -1n,
        state.freeUsdc,
      );
      this.check(
        "auditor sees collateral",
        fields.collateralSol ?? -1n,
        state.collateralSol,
      );
      this.check(
        "auditor sees debt shares",
        fields.debtSharesUsdc ?? -1n,
        state.debtSharesUsdc,
      );
      const hidden =
        fields.freeSol === undefined && fields.supplySharesUsdc === undefined;
      this.check("unselected fields stay hidden", BigInt(hidden ? 1 : 0), 1n);
      // A different key must not read the Disclosure.
      let wrongFailed = false;
      try {
        decryptDisclosure(
          {
            mask: account.mask,
            nonce: big(account.nonce),
            ciphertexts: account.ciphertexts.map((c) => Uint8Array.from(c)),
          },
          x25519.utils.randomSecretKey(),
          this.ctx.mxePublicKey,
        );
      } catch (err) {
        wrongFailed = err instanceof ViewingKeyDecryptError;
        if (!wrongFailed) throw err;
      }
      this.check(
        "another key cannot read the Disclosure",
        BigInt(wrongFailed ? 1 : 0),
        1n,
      );
      return {
        detail: `fields free USDC, collateral and debt disclosed to ${Buffer.from(auditorPublic).toString("hex").slice(0, 12)}...`,
      };
    });
  }

  // ---- output

  writeArtifact(passed: boolean): string {
    const dir = path.join(this.ctx.repoRoot, "artifacts");
    fs.mkdirSync(dir, { recursive: true });
    const file = path.join(dir, "devnet-lend-e2e.json");
    fs.writeFileSync(
      file,
      JSON.stringify(
        {
          network: NETWORK,
          generatedAt: new Date().toISOString(),
          passed,
          program: this.programId.toBase58(),
          market: this.market.toBase58(),
          lendingPool: this.lendingPool.toBase58(),
          positions: {
            lender: this.deployer.position.toBase58(),
            borrower: this.borrower.position.toBase58(),
            third: this.target.position.toBase58(),
          },
          accrual: this.accrual,
          liquidation: this.liquidation,
          steps: this.steps.map((s) => ({
            ...s,
            explorer: s.signatures.map(explorer),
          })),
        },
        null,
        2,
      ),
    );
    return file;
  }

  summary(): string {
    const pad = (s: string, n: number): string => s.padEnd(n).slice(0, n);
    log("\n=== Walnut devnet lending end-to-end run ===");
    log(`program  ${this.programId.toBase58()}`);
    log(`pool     ${this.lendingPool.toBase58()}`);
    log("");
    log(`${pad("step", 28)}${pad("result", 9)}${pad("ms", 9)}detail`);
    for (const s of this.steps) {
      log(
        `${pad(s.name, 28)}${pad(s.status, 9)}${pad(String(s.ms), 9)}${s.detail}`,
      );
      for (const sig of s.signatures) log(`    ${explorer(sig)}`);
    }
    return this.writeArtifact(this.steps.every((s) => s.status !== "failed"));
  }
}

async function main(): Promise<void> {
  const options = resolveOptions(process.argv.slice(2));
  secretStrings = [options.rpcUrl];
  const ctx = await createContext(options);
  anchor.setProvider(ctx.provider);
  const demo = new LendE2E(ctx);
  log(
    `Walnut devnet lending demo: program ${ctx.programId.toBase58()}, cluster offset ${ctx.clusterOffset}`,
  );
  log(`deployer ${ctx.payer.publicKey.toBase58()}`);
  await demo.setup();
  await demo.lenderSupply();
  await demo.borrowerCollateral();
  await demo.borrowerBorrow();
  await demo.accrualAndHealth();
  await demo.borrowerRepay();
  await demo.lenderWithdraw();
  await demo.targetPosition();
  await demo.healthTarget();
  await demo.liquidationRefused();
  await demo.liquidationApplied();
  await demo.keyRotation();
  await demo.disclosure();
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
