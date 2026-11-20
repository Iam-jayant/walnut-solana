// Walnut private operations (ADR-0004): devnet end-to-end demo of the
// ENCRYPTED-AMOUNT instructions `pool_op`, `position_private` and
// `liquidate_pool`.
//
// Run it AFTER the deployer ran `bash scripts/lend-devnet.sh` (program upgrade
// with the private instructions, comp defs `pool_op`, `position_private` and
// `liquidate_pool` finalized, Pool State active):
//
//   pnpm --filter @walnut/sdk demo:private
//
// Steps (each one compares decrypted or public values with the expected
// values and fails with a message that names the step):
//
//   0  setup                        private mode available, band rate source,
//                                   fresh utilization band
//   1  lender_private_supply        deployer: shield USDC, pool_op supply
//   2  borrower_private_collateral  demo owner: shield SOL, position_private add
//   3  borrower_private_borrow      pool_op borrow (band refresh + retry once)
//   4  privacy_check                no plaintext amount in any queue transaction
//   5  accrual                      wait, band update, borrow index rises
//   6  borrower_private_repay_full  pool_op repay of the full debt
//   7  borrower_withdraw_collateral position_private withdraw
//   8  lender_private_withdraw      pool_op withdraw supply
//   9  liquidation_refused_private  healthy Position: liquidate_pool ends `zero`
//   10 liquidation_applied_private  only when a Position is unhealthy
//
// The amounts of the private operations are never sent in the clear: the
// client computes the share conversion from the public LendingPool indexes
// (projected to a chain-time `accrue_to_ts`), encrypts `(amount, shares)` to
// the MXE and submits ciphertexts only. Balances are checked by decrypting the
// Position with the Viewing Key.
//
// State lives under ~/.walnut-demo/devnet (override WALNUT_DEMO_DIR): the demo
// owner (owner.json, shared with the other demos) and the third borrower
// (private-third.json). RPC and wallet come from ANCHOR_PROVIDER_URL and
// ANCHOR_WALLET like the other demos; the RPC URL and any api-key are never
// printed. Output: artifacts/devnet-private-e2e.json.

import * as anchor from "../src/anchor-shim.ts";
import { BN } from "../src/anchor-shim.ts";
import { getCompDefAccAddress } from "@arcium-hq/client";
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
import { MAX_COLLATERAL_LAMPORTS, MAX_LEND_PRICE } from "../src/lend-op.ts";
import {
  accruedDebt,
  accruedSupply,
  decodeLendingPool,
  LAUNCH_RATE_PARAMS,
  projectIndexesAuto,
  RATE_SOURCE_BAND,
  type LendingPool,
  type ProjectedIndexes,
  type RateParams,
} from "../src/lending-pool.ts";
import { normalizePrice, parsePriceUpdateV2 } from "../src/oracle.ts";
import {
  buildLiquidatePoolInstruction,
  buildPoolOpInstruction,
  buildPositionPrivateInstruction,
  buildSetLendingRateSourceInstruction,
  chooseAccrualTs,
  CollateralOp,
  encryptPrivateInput,
  fullRepayAmountPrivate,
  liquidationConversionHolds,
  liquidationInputs,
  PoolOp,
  poolOpConversionHolds,
  poolOpShares,
  type PrivateInput,
  type PrivateQueueParams,
} from "../src/private-op.ts";
import {
  compDefOffsetOf,
  deriveArciumQueueAccounts,
  deriveIntentPda,
  deriveLendingPoolPda,
  deriveOperationCounterPda,
} from "../src/pda.ts";
import {
  decryptPosition,
  deriveViewingKey,
  viewingKeyMessageBytes,
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

// Amounts are deliberately "odd" so a decimal or byte match in a transaction
// cannot be a coincidence.
const SUPPLY_USDC = 5_234_567n; // lender supplies 5.234567 test USDC
const COLLATERAL_LAMPORTS = 187_654_321n; // borrower collateral
const BORROW_MAX = 3_000_007n; // borrower borrows up to ~3 USDC
const BORROW_PERCENT = 50n; // of the LTV capacity at the current price
const MIN_BORROW = 1_000_000n;
const REPAY_MARGIN = 200_000n; // free USDC kept above the projected full repay
const EXTRA_LIQUIDITY_USDC = 2_000_017n; // deployer supply before the third borrow
const LIQUIDATOR_FUND = 3_000_000n; // minted and shielded for the liquidator
const THIRD_COLLATERAL = 50_000_003n; // third borrower, 0.05 SOL
const THIRD_BORROW = 500_009n;
const THIRD_MIN_BORROW = 200_000n;
const PROBE_REPAY = 300_011n; // liquidation attempt on a healthy Position
const ACCRUAL_WAIT_MS = 45_000;
const ACTOR_MIN_LAMPORTS = 250_000_000;
const ACTOR_TOPUP_TO = 400_000_000;
const COMPUTE_UNIT_LIMIT = 600_000;
const INTENT_TIMEOUT_MS = 300_000;
const MAX_POSITION_DEBT_CAP = 1_000_000_000_000n;
const PRIVATE_CIRCUITS = [
  "pool_op",
  "position_private",
  "liquidate_pool",
  "pool_band",
];
const BAND_STALE =
  /UtilizationBandStale|UtilizationBandNeverObserved|\b613[56]\b|0x17f[78]/i;

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
  admin: PublicKey;
  assetState: PublicKey;
  poolState: PublicKey;
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

/** A wallet that owns (or pays for) Positions. */
interface Actor {
  label: string;
  keypair: Keypair;
  position: PublicKey;
  counter: PublicKey;
  key?: ViewingKeyMaterial | undefined;
  epoch: bigint;
}

interface RiskView {
  ltvBps: bigint;
  liqThresholdBps: bigint;
  bonusBps: bigint;
  reserveFactorBps: bigint;
  positionDebtCap: bigint;
  rates: RateParams;
}

/** The accrual view a private operation is built against. */
interface AccrualView {
  pool: LendingPool;
  band: number;
  ts: bigint;
  idx: ProjectedIndexes;
}

interface PrivateTx {
  label: string;
  signature: string;
  /** The plaintext values encrypted into the transaction. */
  secrets: bigint[];
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

type AmountSpec =
  bigint | ((c: { prev: PositionBalances; idx: ProjectedIndexes }) => bigint);

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
function u64le(value: bigint): Buffer {
  const out = Buffer.alloc(8);
  out.writeBigUInt64LE(value, 0);
  return out;
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

/** The Position after a `pool_op` that the circuit applied. */
function afterPoolOp(
  prev: PositionBalances,
  op: PoolOp,
  amount: bigint,
  shares: bigint,
): PositionBalances {
  switch (op) {
    case PoolOp.Supply:
      return {
        ...prev,
        freeUsdc: prev.freeUsdc - amount,
        supplySharesUsdc: prev.supplySharesUsdc + shares,
      };
    case PoolOp.WithdrawSupply:
      return {
        ...prev,
        freeUsdc: prev.freeUsdc + amount,
        supplySharesUsdc: prev.supplySharesUsdc - shares,
      };
    case PoolOp.Borrow:
      return {
        ...prev,
        freeUsdc: prev.freeUsdc + amount,
        debtSharesUsdc: prev.debtSharesUsdc + shares,
      };
    case PoolOp.Repay:
      return {
        ...prev,
        freeUsdc: prev.freeUsdc - amount,
        debtSharesUsdc: prev.debtSharesUsdc - shares,
      };
  }
}

// -------------------------------------------------------------------- demo

class PrivateE2E {
  readonly ctx: BenchContext;
  readonly programId: PublicKey;
  readonly deployer: Actor;
  readonly borrower: Actor;
  readonly third: Actor;
  readonly market: PublicKey;
  readonly lendingPool: PublicKey;
  readonly steps: StepRecord[] = [];
  mk!: MarketAccount;
  risk!: RiskView;
  oracle!: PublicKey;
  checks: string[] = [];
  signatures: string[] = [];
  /** Every private queue transaction sent so far (for the privacy check). */
  privateTxs: PrivateTx[] = [];
  privacy: Record<string, unknown> = {};
  /** Supply shares the lender gained in step 1. */
  lenderShares = 0n;
  accrual: Record<string, string> = {};
  liquidation: Record<string, string> = {};
  private collateralBefore = 0n;
  private freeSolBefore = 0n;

  constructor(ctx: BenchContext) {
    this.ctx = ctx;
    this.programId = ctx.programId;
    this.market = PublicKey.findProgramAddressSync(
      [seedBytes("market")],
      this.programId,
    )[0];
    this.lendingPool = deriveLendingPoolPda(this.programId, this.market);
    this.deployer = this.actor("lender/liquidator (deployer)", ctx.payer);
    this.borrower = this.actor(
      "borrower (demo owner)",
      loadOrCreateKeypair(path.join(stateDir(), "owner.json")),
    );
    this.third = this.actor(
      "third borrower",
      loadOrCreateKeypair(path.join(stateDir(), "private-third.json")),
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

  /** Unix seconds of the chain (never the local clock). */
  private async chainTime(): Promise<bigint> {
    return retry("chain time", async () => {
      const slot = await this.ctx.connection.getSlot("confirmed");
      const t = await this.ctx.connection.getBlockTime(slot);
      if (t === null) throw new Error("failed to get block time");
      return BigInt(t);
    });
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

  private checkText(label: string, actual: string, expected: string): void {
    if (actual !== expected) {
      throw new Error(`${label}: changed (expected it to stay identical)`);
    }
    this.checks.push(`${label} unchanged`);
    log(`  ok  ${label} unchanged`);
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

  /** The stored ciphertext payload of a Position, as a comparable string. */
  private async payload(actor: Actor): Promise<string> {
    const position = await this.fetch<PositionAccount>(
      "position",
      actor.position,
    );
    if (position === null) throw new Error(`${actor.label}: Position missing`);
    return JSON.stringify(position.encryptedState);
  }

  /** The encrypted Pool State payload, as a comparable string. */
  private async poolPayload(): Promise<string> {
    const ps = await this.fetch<Json>("poolState", this.mk.poolState);
    if (ps === null) throw new Error("PoolState missing");
    return JSON.stringify(field(ps, "encrypted"));
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
   * `queued`. `build` receives the next sequence of the counter owned by
   * `counterOwner` (default: the payer; the band update uses the Pool's).
   */
  private async queue(
    payer: Keypair,
    label: string,
    build: (p: {
      sequence: BN;
      offset: BN;
      intent: PublicKey;
    }) => Promise<TransactionInstruction[]>,
    counterOwner: PublicKey = payer.publicKey,
  ): Promise<{ signature: string; status: string; ms: number }> {
    const started = Date.now();
    const counter = await this.fetch<CounterAccount>(
      "operationCounter",
      deriveOperationCounterPda(this.programId, counterOwner),
    );
    const sequence = counter?.nextSequence ?? new BN(0);
    const offset = newOffset();
    const intent = deriveIntentPda(
      this.programId,
      counterOwner,
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

  /** Mints and shields USDC when the Position's free USDC is below `need`. */
  private async ensureFreeUsdc(actor: Actor, need: bigint): Promise<void> {
    const s = await this.decrypt(actor);
    if (s.freeUsdc >= need) return;
    const missing = need - s.freeUsdc;
    await this.mintTestUsdc(actor, missing);
    await this.shield(actor, `${actor.label} shield USDC`, "usdc", missing);
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

  private async readAsset(): Promise<Json> {
    const asset = await this.fetch<Json>("assetState", this.mk.assetState);
    if (asset === null) throw new Error("AssetState missing");
    return asset;
  }

  /** The revealed utilization band index of the Asset State. */
  private async readBand(): Promise<number> {
    return Number(big(field(await this.readAsset(), "utilization_band")));
  }

  /**
   * LendingPool, band and the indexes projected to a fresh chain-time
   * `accrue_to_ts` (the same projection the program accrues to).
   */
  private async accrualView(): Promise<AccrualView> {
    const pool = await this.readPool();
    const band = await this.readBand();
    const now = await this.chainTime();
    const ts = chooseAccrualTs(pool.lastAccrualTs, now);
    const idx = projectIndexesAuto(pool, ts, {
      reserveFactorBps: this.risk.reserveFactorBps,
      rates: this.risk.rates,
      band,
    });
    return { pool, band, ts, idx };
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
    const price = normalized.value / 100n; // 1e8 USD scale -> USDC base units
    return price > MAX_LEND_PRICE ? MAX_LEND_PRICE : price;
  }

  private async borrowHeadroom(
    state: PositionBalances,
    percent: bigint,
  ): Promise<bigint> {
    const v = await this.accrualView();
    const price = await this.oraclePrice();
    const capacity =
      (state.collateralSol * price * this.risk.ltvBps) /
      (10_000n * 1_000_000_000n);
    const allowed = (capacity * percent) / 100n;
    const debt = accruedDebt(state.debtSharesUsdc, v.idx.borrowIndex1e9);
    const cap =
      this.risk.positionDebtCap > MAX_POSITION_DEBT_CAP
        ? MAX_POSITION_DEBT_CAP
        : this.risk.positionDebtCap;
    const room = allowed > debt ? allowed - debt : 0n;
    const capRoom = cap > debt ? cap - debt : 0n;
    return room < capRoom ? room : capRoom;
  }

  // ---- band

  /** Queues `update_pool_band` (the light private-Pool band) and waits. */
  private async updateBand(label: string): Promise<void> {
    const poolCounterOwner = this.mk.poolState;
    const r = await this.queue(
      this.deployer.keypair,
      label,
      async ({ sequence, offset, intent }) => [
        await this.method("updatePoolBand")(offset, sequence)
          .accountsPartial({
            payer: this.deployer.keypair.publicKey,
            ...this.queueAccounts("pool_band", offset),
            market: this.market,
            assetState: this.mk.assetState,
            poolState: this.mk.poolState,
            lendingPool: this.lendingPool,
            operationCounter: deriveOperationCounterPda(
              this.programId,
              poolCounterOwner,
            ),
            operationIntent: intent,
          })
          .instruction(),
      ],
      poolCounterOwner,
    );
    if (r.status !== "applied") {
      throw new Error(`${label}: Intent ended ${r.status}, expected applied`);
    }
  }

  // ---- private operations

  private params(
    signer: Keypair,
    sequence: BN,
    offset: BN,
    ts: bigint,
    input: PrivateInput,
  ): PrivateQueueParams {
    return {
      program: this.ctx.program,
      signer: signer.publicKey,
      market: this.market,
      assetState: this.mk.assetState,
      poolState: this.mk.poolState,
      sequence: BigInt(sequence.toString()),
      computationOffset: offset,
      clusterOffset: this.ctx.clusterOffset,
      priceUpdate: this.oracle,
      accrueToTs: ts,
      input,
    };
  }

  /**
   * Owner-signed `pool_op`: the client computes the shares, encrypts
   * `[amount, shares]`, and compares the decrypted Position with the expected
   * one. A refused operation (Intent `zero`, for example because the indexes
   * moved) or a stale band on a borrow is retried once with fresh indexes.
   */
  private async poolOp(
    actor: Actor,
    label: string,
    op: PoolOp,
    spec: AmountSpec,
  ): Promise<{ state: PositionBalances; amount: bigint; shares: bigint }> {
    for (let attempt = 1; ; attempt += 1) {
      const prev = await this.decrypt(actor);
      const v = await this.accrualView();
      const amount =
        typeof spec === "function" ? spec({ prev, idx: v.idx }) : spec;
      const shares = poolOpShares(
        op,
        amount,
        v.idx.supplyIndex1e9,
        v.idx.borrowIndex1e9,
      );
      if (
        !poolOpConversionHolds(
          op,
          amount,
          shares,
          v.idx.supplyIndex1e9,
          v.idx.borrowIndex1e9,
        )
      ) {
        throw new Error(`${label}: the SDK mirror rejects its own conversion`);
      }
      const input = encryptPrivateInput(this.ctx.mxePublicKey, [
        amount,
        shares,
      ]);
      let r: { signature: string; status: string };
      try {
        r = await this.queue(
          actor.keypair,
          label,
          async ({ sequence, offset }) => [
            await buildPoolOpInstruction({
              ...this.params(actor.keypair, sequence, offset, v.ts, input),
              op,
            }),
          ],
        );
      } catch (err) {
        const text = err instanceof Error ? err.message : String(err);
        if (attempt === 1 && op === PoolOp.Borrow && BAND_STALE.test(text)) {
          log(`  ${label}: utilization band is stale, refreshing and retrying`);
          await this.updateBand("band refresh");
          continue;
        }
        throw err;
      }
      this.privateTxs.push({
        label,
        signature: r.signature,
        secrets: [amount, shares],
      });
      if (r.status !== "applied") {
        if (attempt === 1 && r.status === "zero") {
          this.checkState(
            `${label}: refused, Position unchanged`,
            await this.decrypt(actor),
            prev,
          );
          log(`  ${label}: refused (indexes moved?), retrying once`);
          continue;
        }
        throw new Error(`${label}: Intent ended ${r.status}, expected applied`);
      }
      const state = await this.decrypt(actor);
      this.checkState(label, state, afterPoolOp(prev, op, amount, shares));
      return { state, amount, shares };
    }
  }

  /** Owner-signed `position_private` (collateral add or withdraw). */
  private async collateralOp(
    actor: Actor,
    label: string,
    op: CollateralOp,
    lamports: bigint,
  ): Promise<PositionBalances> {
    const prev = await this.decrypt(actor);
    const v = await this.accrualView();
    const input = encryptPrivateInput(this.ctx.mxePublicKey, [lamports, 0n]);
    const r = await this.queue(
      actor.keypair,
      label,
      async ({ sequence, offset }) => [
        await buildPositionPrivateInstruction({
          ...this.params(actor.keypair, sequence, offset, v.ts, input),
          op,
        }),
      ],
    );
    this.privateTxs.push({
      label,
      signature: r.signature,
      secrets: [lamports],
    });
    if (r.status !== "applied") {
      throw new Error(`${label}: Intent ended ${r.status}, expected applied`);
    }
    const direction = op === CollateralOp.Add ? 1n : -1n;
    const state = await this.decrypt(actor);
    this.checkState(label, state, {
      ...prev,
      freeSol: prev.freeSol - direction * lamports,
      collateralSol: prev.collateralSol + direction * lamports,
    });
    return state;
  }

  /**
   * `liquidate_pool` signed by the liquidator, paying from its own Position.
   * `zero`: target, liquidator and Pool payloads must stay identical.
   * `applied`: balances move by the circuit's `(amount, burnShares, seize)`.
   */
  private async liquidatePool(
    label: string,
    liquidator: Actor,
    target: Actor,
    amount: bigint,
    expected: "applied" | "zero",
  ): Promise<void> {
    const targetPrev = await this.decrypt(target);
    const liqPrev = await this.decrypt(liquidator);
    const targetRaw = await this.payload(target);
    const liqRaw = await this.payload(liquidator);
    const poolRaw = await this.poolPayload();
    const v = await this.accrualView();
    const price = await this.oraclePrice();
    const inputs = liquidationInputs(
      amount,
      v.idx.borrowIndex1e9,
      price,
      this.risk.bonusBps,
    );
    if (
      !liquidationConversionHolds(
        inputs,
        v.idx.borrowIndex1e9,
        price,
        this.risk.bonusBps,
      )
    ) {
      throw new Error(`${label}: the SDK mirror rejects its own conversion`);
    }
    const input = encryptPrivateInput(this.ctx.mxePublicKey, [
      inputs.amount,
      inputs.burnShares,
      inputs.seize,
    ]);
    const r = await this.queue(
      liquidator.keypair,
      label,
      async ({ sequence, offset }) => [
        await buildLiquidatePoolInstruction({
          ...this.params(liquidator.keypair, sequence, offset, v.ts, input),
          target: target.position,
        }),
      ],
    );
    this.privateTxs.push({
      label,
      signature: r.signature,
      secrets: [inputs.amount, inputs.burnShares, inputs.seize],
    });
    if (r.status !== expected) {
      throw new Error(
        `${label}: Intent ended ${r.status}, expected ${expected}`,
      );
    }
    const targetNow = await this.decrypt(target);
    const liqNow = await this.decrypt(liquidator);
    if (expected === "zero") {
      this.checkState(`${label}: target`, targetNow, targetPrev);
      this.checkState(`${label}: liquidator`, liqNow, liqPrev);
      this.checkText(
        `${label}: target payload`,
        await this.payload(target),
        targetRaw,
      );
      this.checkText(
        `${label}: liquidator payload`,
        await this.payload(liquidator),
        liqRaw,
      );
      this.checkText(
        `${label}: Pool payload`,
        await this.poolPayload(),
        poolRaw,
      );
      return;
    }
    this.checkState(`${label}: target`, targetNow, {
      ...targetPrev,
      collateralSol: targetPrev.collateralSol - inputs.seize,
      debtSharesUsdc: targetPrev.debtSharesUsdc - inputs.burnShares,
    });
    this.checkState(`${label}: liquidator`, liqNow, {
      ...liqPrev,
      freeUsdc: liqPrev.freeUsdc - inputs.amount,
      freeSol: liqPrev.freeSol + inputs.seize,
    });
    this.liquidation["repayUsdc"] = inputs.amount.toString();
    this.liquidation["seizedLamports"] = inputs.seize.toString();
    this.liquidation["debtSharesBurned"] = inputs.burnShares.toString();
  }

  // ---- privacy

  /**
   * Fetches every private queue transaction and asserts that neither the
   * instruction data, nor an event payload, nor a log line carries a
   * plaintext value of the operation.
   */
  private async privacyCheck(): Promise<number> {
    const details: Array<Record<string, unknown>> = [];
    for (const p of this.privateTxs) {
      const tx = await waitFor(
        `transaction ${p.label}`,
        async () =>
          (await this.ctx.connection.getTransaction(p.signature, {
            commitment: "confirmed",
            maxSupportedTransactionVersion: 0,
          })) ?? undefined,
        60_000,
      );
      const datas = tx.transaction.message.compiledInstructions.map((i) =>
        Buffer.from(i.data),
      );
      const logs = tx.meta?.logMessages ?? [];
      const secrets = p.secrets.filter((s) => s > 0n);
      for (const s of secrets) {
        const needle = u64le(s);
        datas.forEach((d, i) => {
          if (d.includes(needle)) {
            throw new Error(
              `${p.label}: instruction ${i} data contains the u64 bytes of a plaintext value`,
            );
          }
        });
        const decimal = new RegExp(`(?<!\\d)${s.toString()}(?!\\d)`);
        for (const line of logs) {
          if (line.startsWith("Program data: ")) {
            const bytes = Buffer.from(
              line.slice("Program data: ".length),
              "base64",
            );
            if (bytes.includes(needle)) {
              throw new Error(
                `${p.label}: an event payload contains the u64 bytes of a plaintext value`,
              );
            }
          } else if (!/consumed \d+ of \d+ compute units/.test(line)) {
            // Values under 100000 are too short to tell from log noise.
            if (s >= 100_000n && decimal.test(line)) {
              throw new Error(
                `${p.label}: a log line contains a plaintext value: ${line.slice(0, 80)}`,
              );
            }
          }
        }
      }
      this.checks.push(
        `${p.label}: no plaintext value in data, events or logs`,
      );
      log(
        `  ok  ${p.label}: ${datas.length} instruction(s), ${logs.length} log line(s), ${secrets.length} value(s) absent`,
      );
      details.push({
        label: p.label,
        signature: p.signature,
        valuesChecked: secrets.length,
        instructions: datas.length,
        logLines: logs.length,
      });
    }
    this.privacy = {
      passed: true,
      transactionsChecked: details.length,
      transactions: details,
    };
    return details.length;
  }

  // ---- steps

  async setup(): Promise<void> {
    await this.step("setup", async () => {
      const market = await this.fetch<MarketAccount>("market", this.market);
      if (market === null) throw new Error("Market missing: run demo:balance");
      this.mk = market;
      for (const name of ["poolOp", "positionPrivate", "liquidatePool"]) {
        this.method(name);
      }
      const hint = "run scripts/lend-devnet.sh first";
      const pool = await this.readPool();
      this.oracle = pool.oracle;
      const asset = await this.readAsset();
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
      // Private mode: comp defs and an active Pool State.
      for (const circuit of PRIVATE_CIRCUITS) {
        const address = getCompDefAccAddress(
          this.programId,
          compDefOffsetOf(circuit),
        );
        const info = await retry("comp def", () =>
          this.ctx.connection.getAccountInfo(address, "confirmed"),
        );
        if (info === null) {
          throw new Error(
            `private mode is not available: comp def ${circuit} is missing; ${hint}`,
          );
        }
      }
      const ps = await this.fetch<StatusAccount>(
        "poolState",
        this.mk.poolState,
      );
      if (ps === null || statusName(ps.status) !== "active") {
        throw new Error(
          `private mode is not available: PoolState is ${ps === null ? "missing" : statusName(ps.status)}; ${hint}`,
        );
      }
      this.checks.push(
        "comp defs pool_op, position_private, liquidate_pool, pool_band exist",
      );
      this.checks.push("PoolState is active");
      // Band rate source.
      if (pool.rateSource !== RATE_SOURCE_BAND) {
        if (!this.mk.admin.equals(this.deployer.keypair.publicKey)) {
          throw new Error(
            "LendingPool is not in band mode and the wallet is not the market admin",
          );
        }
        const ix = await buildSetLendingRateSourceInstruction({
          program: this.ctx.program,
          admin: this.deployer.keypair.publicKey,
          market: this.market,
          assetState: this.mk.assetState,
          rateSource: RATE_SOURCE_BAND,
        });
        const sig = await this.send([ix], [this.deployer.keypair]);
        log(`  set_lending_rate_source(1): ${explorer(sig)}`);
      }
      const after = await this.readPool();
      this.check("LendingPool rate source", BigInt(after.rateSource), 1n);
      await this.updateBand("band update (setup)");
      const band = await this.readBand();
      const price = await this.oraclePrice();
      log(`  oracle low-bound price: ${usdc(price)} USDC per SOL`);
      log(
        `  LTV ${this.risk.ltvBps} bps, liquidation threshold ${this.risk.liqThresholdBps} bps, bonus ${this.risk.bonusBps} bps, band ${band}`,
      );
      this.accrual["supplyIndexStart"] = after.supplyIndex1e9.toString();
      this.accrual["borrowIndexStart"] = after.borrowIndex1e9.toString();
      this.accrual["bandAtSetup"] = String(band);
      return {
        detail: `band mode, band ${band}, price ${usdc(price)} USDC/SOL`,
      };
    });
  }

  async lenderPrivateSupply(): Promise<void> {
    await this.step("lender_private_supply", async () => {
      await this.ensurePosition(this.deployer);
      await this.mintTestUsdc(this.deployer, SUPPLY_USDC);
      await this.shield(
        this.deployer,
        "lender shield USDC",
        "usdc",
        SUPPLY_USDC,
      );
      const r = await this.poolOp(
        this.deployer,
        "lender private supply",
        PoolOp.Supply,
        SUPPLY_USDC,
      );
      this.lenderShares = r.shares;
      this.checkAtLeast("supply shares minted", r.shares, 1n);
      return {
        detail: `supplied ${usdc(SUPPLY_USDC)} USDC for ${r.shares} shares (amount encrypted)`,
      };
    });
  }

  async borrowerPrivateCollateral(): Promise<void> {
    await this.step("borrower_private_collateral", async () => {
      await this.ensurePosition(this.borrower);
      const s = await this.decrypt(this.borrower);
      if (COLLATERAL_LAMPORTS > MAX_COLLATERAL_LAMPORTS) {
        throw new Error("collateral too large");
      }
      this.collateralBefore = s.collateralSol;
      this.freeSolBefore = s.freeSol;
      if (s.freeSol < COLLATERAL_LAMPORTS) {
        await this.shield(
          this.borrower,
          "borrower shield SOL",
          "sol",
          COLLATERAL_LAMPORTS - s.freeSol,
        );
      }
      const before = await this.decrypt(this.borrower);
      const after = await this.collateralOp(
        this.borrower,
        "private add collateral",
        CollateralOp.Add,
        COLLATERAL_LAMPORTS,
      );
      this.check(
        "collateral delta",
        after.collateralSol - before.collateralSol,
        COLLATERAL_LAMPORTS,
      );
      return { detail: `added ${sol(COLLATERAL_LAMPORTS)} SOL collateral` };
    });
  }

  async borrowerPrivateBorrow(): Promise<void> {
    await this.step("borrower_private_borrow", async () => {
      const s = await this.decrypt(this.borrower);
      let amount = await this.borrowHeadroom(s, BORROW_PERCENT);
      if (amount > BORROW_MAX) amount = BORROW_MAX;
      if (amount < MIN_BORROW) {
        throw new Error(
          `borrow headroom is only ${usdc(amount)} USDC (collateral or price too low)`,
        );
      }
      const r = await this.poolOp(
        this.borrower,
        "borrower private borrow",
        PoolOp.Borrow,
        amount,
      );
      this.checkAtLeast("debt shares minted", r.shares, 1n);
      this.accrual["borrowedUsdc"] = amount.toString();
      return { detail: `borrowed ${usdc(amount)} USDC (amount encrypted)` };
    });
  }

  async privacyStep(step = "privacy_check"): Promise<void> {
    await this.step(step, async () => {
      const n = await this.privacyCheck();
      return {
        detail: `${n} private queue transaction(s) carry no plaintext amount`,
      };
    });
  }

  async accrualStep(): Promise<void> {
    await this.step("accrual", async () => {
      const before = await this.readPool();
      log(`  stored borrow index ${before.borrowIndex1e9}`);
      log(`  waiting ${ACCRUAL_WAIT_MS / 1000} s for interest to accrue...`);
      await sleep(ACCRUAL_WAIT_MS);
      await this.updateBand("band update (accrual)");
      const v = await this.accrualView();
      this.checkAtLeast(
        "seconds since the last accrual",
        v.ts - v.pool.lastAccrualTs,
        30n,
      );
      this.checkAtLeast(
        "projected borrow index rise",
        v.idx.borrowIndex1e9 - v.pool.borrowIndex1e9,
        1n,
      );
      const s = await this.decrypt(this.borrower);
      const debtNow = accruedDebt(s.debtSharesUsdc, v.pool.borrowIndex1e9);
      const debtProjected = accruedDebt(s.debtSharesUsdc, v.idx.borrowIndex1e9);
      this.accrual["band"] = String(v.band);
      this.accrual["borrowIndexStored"] = v.pool.borrowIndex1e9.toString();
      this.accrual["borrowIndexProjected"] = v.idx.borrowIndex1e9.toString();
      this.accrual["supplyIndexStored"] = v.pool.supplyIndex1e9.toString();
      this.accrual["supplyIndexProjected"] = v.idx.supplyIndex1e9.toString();
      this.accrual["borrowerDebtStored"] = debtNow.toString();
      this.accrual["borrowerDebtProjected"] = debtProjected.toString();
      return {
        detail: `borrow index ${v.pool.borrowIndex1e9} -> ${v.idx.borrowIndex1e9} (band ${v.band}), borrower debt ${debtNow} -> ${debtProjected} base units`,
      };
    });
  }

  async borrowerPrivateRepayFull(): Promise<void> {
    await this.step("borrower_private_repay_full", async () => {
      const s = await this.decrypt(this.borrower);
      if (s.debtSharesUsdc === 0n) throw new Error("no debt to repay");
      const v = await this.accrualView();
      const estimate = accruedDebt(s.debtSharesUsdc, v.idx.borrowIndex1e9);
      await this.ensureFreeUsdc(this.borrower, estimate + REPAY_MARGIN);
      await this.poolOp(
        this.borrower,
        "borrower private repay (full)",
        PoolOp.Repay,
        ({ prev, idx }) => {
          const amount = fullRepayAmountPrivate(
            prev.debtSharesUsdc,
            idx.borrowIndex1e9,
          );
          if (prev.freeUsdc < amount) {
            throw new Error(
              `free USDC ${prev.freeUsdc} does not cover the full repay ${amount}`,
            );
          }
          const shares = poolOpShares(
            PoolOp.Repay,
            amount,
            idx.supplyIndex1e9,
            idx.borrowIndex1e9,
          );
          if (shares !== prev.debtSharesUsdc) {
            throw new Error(
              `full repay burns ${shares} shares, debt is ${prev.debtSharesUsdc}`,
            );
          }
          return amount;
        },
      );
      const after = await this.decrypt(this.borrower);
      this.check(
        "borrower debt shares after full repay",
        after.debtSharesUsdc,
        0n,
      );
      return { detail: "debt cleared with the encrypted full repay amount" };
    });
  }

  async borrowerWithdrawCollateral(): Promise<void> {
    await this.step("borrower_withdraw_collateral", async () => {
      const after = await this.collateralOp(
        this.borrower,
        "private withdraw collateral",
        CollateralOp.Withdraw,
        COLLATERAL_LAMPORTS,
      );
      this.check(
        "collateral back to start",
        after.collateralSol,
        this.collateralBefore,
      );
      this.check(
        "free SOL back (withdrawn collateral)",
        after.freeSol,
        this.freeSolBefore + COLLATERAL_LAMPORTS,
      );
      return { detail: `withdrew ${sol(COLLATERAL_LAMPORTS)} SOL collateral` };
    });
  }

  async lenderPrivateWithdraw(): Promise<void> {
    await this.step("lender_private_withdraw", async () => {
      if (this.lenderShares === 0n) {
        throw new Error("no supply shares recorded by lender_private_supply");
      }
      const shares = this.lenderShares;
      const before = await this.decrypt(this.deployer);
      const r = await this.poolOp(
        this.deployer,
        "lender private withdraw supply",
        PoolOp.WithdrawSupply,
        ({ idx }) => accruedSupply(shares, idx.supplyIndex1e9),
      );
      this.check("USDC received", r.state.freeUsdc - before.freeUsdc, r.amount);
      this.checkAtLeast("supply shares burned", r.shares, 1n);
      // Rounding favours the pool by at most a couple of base units.
      this.checkAtLeast(
        "USDC received vs supplied",
        r.amount,
        SUPPLY_USDC - 2n,
      );
      this.accrual["lenderReceivedUsdc"] = r.amount.toString();
      this.accrual["lenderSuppliedUsdc"] = SUPPLY_USDC.toString();
      const gain = r.amount - SUPPLY_USDC;
      return {
        detail: `received ${r.amount} base units for ${SUPPLY_USDC} supplied (${gain >= 0n ? "+" : ""}${gain})`,
      };
    });
  }

  async liquidationRefusedPrivate(): Promise<void> {
    await this.step("liquidation_refused_private", async () => {
      // Pool cash for the third borrower (the lender withdrew in step 8).
      await this.ensureFreeUsdc(
        this.deployer,
        LIQUIDATOR_FUND + EXTRA_LIQUIDITY_USDC,
      );
      await this.poolOp(
        this.deployer,
        "deployer private supply (liquidity)",
        PoolOp.Supply,
        EXTRA_LIQUIDITY_USDC,
      );
      await this.ensurePosition(this.third);
      let s = await this.decrypt(this.third);
      if (s.collateralSol < THIRD_COLLATERAL) {
        const need = THIRD_COLLATERAL - s.collateralSol;
        if (s.freeSol < need) {
          await this.shield(
            this.third,
            "third shield SOL",
            "sol",
            need - s.freeSol,
          );
        }
        await this.collateralOp(
          this.third,
          "third private add collateral",
          CollateralOp.Add,
          need,
        );
        s = await this.decrypt(this.third);
      }
      if (s.debtSharesUsdc === 0n) {
        let amount = await this.borrowHeadroom(s, BORROW_PERCENT);
        if (amount > THIRD_BORROW) amount = THIRD_BORROW;
        if (amount < THIRD_MIN_BORROW) {
          throw new Error(
            `third borrower headroom is only ${usdc(amount)} USDC`,
          );
        }
        await this.poolOp(
          this.third,
          "third private borrow",
          PoolOp.Borrow,
          amount,
        );
      }
      const t = await this.decrypt(this.third);
      this.checkAtLeast("third borrower debt shares", t.debtSharesUsdc, 1n);
      await this.ensureFreeUsdc(this.deployer, PROBE_REPAY);
      await this.liquidatePool(
        "liquidate healthy Position (private)",
        this.deployer,
        this.third,
        PROBE_REPAY,
        "zero",
      );
      return {
        detail: `refused: Intent zero, target, liquidator and Pool payloads unchanged`,
      };
    });
  }

  async liquidationAppliedPrivate(): Promise<void> {
    await this.step("liquidation_applied_private", async () => {
      const price = await this.oraclePrice();
      const v = await this.accrualView();
      let victim: Actor | undefined;
      for (const candidate of [this.third, this.borrower]) {
        const s = await this.decrypt(candidate);
        if (s.debtSharesUsdc === 0n) continue;
        const debt = accruedDebt(s.debtSharesUsdc, v.idx.borrowIndex1e9);
        // unhealthy: debt * 1e4 > collateral value * liquidation threshold
        if (
          debt * 10_000n * 1_000_000_000n >
          s.collateralSol * price * this.risk.liqThresholdBps
        ) {
          victim = candidate;
          break;
        }
      }
      if (victim === undefined) {
        const reason = `skipped: no Position is unhealthy at the current price (${usdc(price)} USDC/SOL)`;
        this.liquidation["applied"] = reason;
        return { detail: reason, skipped: true };
      }
      const s = await this.decrypt(victim);
      const debt = accruedDebt(s.debtSharesUsdc, v.idx.borrowIndex1e9);
      // Close factor: repay at most half the debt, with a margin.
      const repay = debt / 2n > 2n ? debt / 2n - 2n : 0n;
      if (repay === 0n) {
        return { detail: "skipped: debt too small to split", skipped: true };
      }
      await this.ensureFreeUsdc(this.deployer, repay);
      await this.liquidatePool(
        "liquidate unhealthy Position (private)",
        this.deployer,
        victim,
        repay,
        "applied",
      );
      this.liquidation["applied"] = "applied";
      return {
        detail: `repaid ${usdc(repay)} USDC on ${victim.label}, collateral seized with the bonus`,
      };
    });
  }

  // ---- output

  writeArtifact(passed: boolean): string {
    const dir = path.join(this.ctx.repoRoot, "artifacts");
    fs.mkdirSync(dir, { recursive: true });
    const file = path.join(dir, "devnet-private-e2e.json");
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
            third: this.third.position.toBase58(),
          },
          accrual: this.accrual,
          liquidation: this.liquidation,
          privacy: this.privacy,
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
    log("\n=== Walnut devnet private-operations end-to-end run ===");
    log(`program  ${this.programId.toBase58()}`);
    log(`pool     ${this.lendingPool.toBase58()}`);
    log("");
    log(`${pad("step", 32)}${pad("result", 9)}${pad("ms", 9)}detail`);
    for (const s of this.steps) {
      log(
        `${pad(s.name, 32)}${pad(s.status, 9)}${pad(String(s.ms), 9)}${s.detail}`,
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
  const demo = new PrivateE2E(ctx);
  log(
    `Walnut devnet private-operations demo: program ${ctx.programId.toBase58()}, cluster offset ${ctx.clusterOffset}`,
  );
  log(`deployer ${ctx.payer.publicKey.toBase58()}`);
  await demo.setup();
  await demo.lenderPrivateSupply();
  await demo.borrowerPrivateCollateral();
  await demo.borrowerPrivateBorrow();
  await demo.privacyStep();
  await demo.accrualStep();
  await demo.borrowerPrivateRepayFull();
  await demo.borrowerWithdrawCollateral();
  await demo.lenderPrivateWithdraw();
  await demo.liquidationRefusedPrivate();
  await demo.liquidationAppliedPrivate();
  // Re-check every private queue transaction of the run, not only steps 1-3.
  await demo.privacyStep("privacy_check_all");
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
