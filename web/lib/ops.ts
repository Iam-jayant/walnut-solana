// Write side: instruction building, queued-operation tracking and the
// transaction sender. Ported from packages/sdk/demo/loan-demo.ts; signing goes
// through the wallet instead of keypairs.
import "./polyfill.ts";
import { BN, type Program } from "@anchor-lang/core";
import { Buffer } from "buffer";
import {
  buildAbandonLendOpInstruction,
  buildAbandonPrivateOpInstruction,
  buildCheckHealthInstruction,
  buildDiscloseInstruction,
  buildLendOpInstruction,
  buildLiquidateInstruction,
  buildLiquidatePoolInstruction,
  buildPoolOpInstruction,
  buildPositionPrivateInstruction,
  buildRotatePositionKeyInstruction,
  buildUnwrapWsolInstruction,
  buildUpdatePoolBandInstruction,
  chooseAccrualTs,
  deriveArciumQueueAccounts,
  deriveIntentPda,
  deriveOperationCounterPda,
  derivePoolCounterPda,
  derivePoolStatePda,
  encryptPrivateInput,
  fullRepayAmountPrivate,
  INDEX_SCALE,
  liquidationConversionHolds,
  liquidationInputs,
  poolOpConversionHolds,
  PoolOp,
  poolOpShares,
  projectIndexesAuto,
  rotateViewingKey,
  type CollateralOp,
  type LendOp,
  type PositionBalances,
  type ViewingKeyMaterial,
} from "@walnut/sdk";
import {
  ComputeBudgetProgram,
  SystemProgram,
  Transaction,
  TransactionInstruction,
  PublicKey,
  type Connection,
} from "@solana/web3.js";
import { CLUSTER_OFFSET, PROGRAM_ID, QUEUE_COMPUTE_UNITS } from "./config.ts";
import walnutIdl from "./idl/walnut.json";
import {
  ATA_PROGRAM_ID,
  ataAddress,
  chainNowSeconds,
  fetchAccount,
  fetchPoolLowPrice,
  loadLendingView,
  method,
  pendingShieldAddress,
  positionAddress,
  randomU64,
  randomU128,
  sleep,
  TOKEN_PROGRAM_ID,
  variantName,
  withRetry,
  type IntentAccount,
  type LendingView,
  type OperationCounterAccount,
  type PendingShieldAccount,
  type PositionAccount,
  type Protocol,
} from "./chain.ts";

export type Asset = "sol" | "usdc";

/** Where a single operation is in its life, shown live in the UI. */
export type Stage =
  "signing" | "submitted" | "computing" | "applied" | "rejected" | "failed";

export interface Progress {
  stage: Stage;
  signature?: string;
  label?: string;
  detail?: string;
}

export type Report = (progress: Progress) => void;

export interface Signer {
  publicKey: PublicKey;
  signTransaction(tx: Transaction): Promise<Transaction>;
}

export interface OpsContext {
  connection: Connection;
  program: Program;
  protocol: Protocol;
  signer: Signer;
}

const INTENT_TIMEOUT_MS = 300_000;

// ------------------------------------------------------------------ errors

interface IdlError {
  code: number;
  name: string;
  msg: string;
}

const PROGRAM_ERRORS = new Map<number, IdlError>(
  (walnutIdl as unknown as { errors: IdlError[] }).errors.map((e) => [
    e.code,
    e,
  ]),
);

const RETRY = " Please try again shortly.";

/** Plain-language messages for the pause, cap, liquidity and oracle errors. */
const ERROR_TEXT: Record<number, string> = {
  6002: "Another operation is still processing on this account. Wait for it to finish, then try again.",
  6052:
    "This action is paused right now. Repaying, withdrawing and removing collateral stay available." +
    RETRY,
  6054: "Repayment and adding collateral cannot be paused.",
  6071: "The price account is not a valid Pyth price account." + RETRY,
  6072:
    "The price account reports a different feed than the market uses." + RETRY,
  6073:
    "The SOL price is out of date. Wait for the next oracle update." + RETRY,
  6074: "The SOL price is not usable right now." + RETRY,
  6075: "The SOL price is too uncertain right now." + RETRY,
  6078:
    "The SOL price moved too sharply compared with its recent average." + RETRY,
  6081: "The oracle is in anomaly mode, so this action is blocked." + RETRY,
  6159: "This lending operation cannot be run from here.",
  6160: "Unknown lending operation.",
  6161: "Enter an amount above zero.",
  6162: "That amount is too small to register. Try a larger amount.",
  6163: "The market's supply cap is reached. Try a smaller amount or try again later.",
  6164: "The market's borrow cap would be exceeded. Try a smaller amount or try again later.",
  6165: "This amount is above the per-asset cap.",
  6166: "The pool does not have enough free USDC right now. Try a smaller amount or try again later.",
  6167: "The price account does not match the market's oracle. Refresh the page and try again.",
  6168: "The price account could not be verified. Refresh the page and try again.",
  6169: "The lending market configuration is invalid." + RETRY,
  6170: "The pool totals would go below zero. Refresh and try again.",
  6171: "The pool totals would overflow. Try a smaller amount.",
  6172: "The liquidation accounts do not match the market.",
  6173: "The liquidation amount is zero or above what can be repaid. Try a smaller amount.",
  6174: "Choose at least one field to disclose.",
  6175: "That disclosure is not pending for this account.",
  6176: "The disclosure was not sealed to the requested auditor key.",
  6135:
    "The utilization band is refreshing. Please try again in a few seconds.",
  6136: "The utilization band has not been observed yet, so borrowing is not available yet.",
  6177: "That private operation is not valid here.",
  6178: "The accrual time is out of the accepted window. Refresh and try again.",
  6179: "The private operation does not match its intent. Refresh and try again.",
  6180: "You cannot liquidate your own account.",
  6181: "The encrypted input key is invalid. Please try again.",
};

/** The program error code inside an RPC or simulation error text, if any. */
function programErrorCode(text: string): number | null {
  const custom = /"Custom":\s*(\d+)/.exec(text);
  if (custom?.[1] !== undefined) return Number(custom[1]);
  const number = /Error Number:\s*(\d+)/.exec(text);
  if (number?.[1] !== undefined) return Number(number[1]);
  const hex = /custom program error:\s*0x([0-9a-f]+)/i.exec(text);
  if (hex?.[1] !== undefined) return parseInt(hex[1], 16);
  return null;
}

export function friendlyError(err: unknown): string {
  const raw = err instanceof Error ? err.message : String(err);
  const text = raw.replace(/api-key=[^&\s"')]+/gi, "api-key=REDACTED");
  if (
    /reject|denied|cancel|declined/i.test(text) &&
    /user|request|wallet|transaction/i.test(text)
  ) {
    return "The request was cancelled in your wallet.";
  }
  const code = programErrorCode(text);
  if (code !== null && code >= 6000) {
    const known = ERROR_TEXT[code];
    if (known !== undefined) return known;
    const idl = PROGRAM_ERRORS.get(code);
    if (idl !== undefined) return `${idl.msg}.`;
  }
  if (/insufficient (funds|lamports)|0x1\b/i.test(text)) {
    return "Not enough SOL in the wallet to pay the network fee and rent.";
  }
  if (/blockhash|expired/i.test(text)) {
    return "The transaction expired before it was confirmed. Please try again.";
  }
  return text.split("\n")[0]?.slice(0, 220) ?? "Unknown error";
}

// ------------------------------------------------------------- send / wait

/** Signs with the wallet, sends through our RPC, and waits for confirmation. */
export async function sendTransaction(
  ctx: OpsContext,
  instructions: TransactionInstruction[],
  options: { computeUnits?: number; onSubmitted?: (sig: string) => void } = {},
): Promise<string> {
  const { connection, signer } = ctx;
  const { blockhash, lastValidBlockHeight } = await withRetry(() =>
    connection.getLatestBlockhash("confirmed"),
  );
  const tx = new Transaction({
    feePayer: signer.publicKey,
    blockhash,
    lastValidBlockHeight,
  });
  if (options.computeUnits !== undefined) {
    tx.add(
      ComputeBudgetProgram.setComputeUnitLimit({ units: options.computeUnits }),
    );
  }
  tx.add(...instructions);
  const signed = await signer.signTransaction(tx);
  const signature = await withRetry(() =>
    connection.sendRawTransaction(signed.serialize(), {
      preflightCommitment: "confirmed",
      maxRetries: 5,
    }),
  );
  options.onSubmitted?.(signature);
  await confirmSignature(connection, signature, lastValidBlockHeight);
  return signature;
}

async function confirmSignature(
  connection: Connection,
  signature: string,
  lastValidBlockHeight: number,
): Promise<void> {
  for (;;) {
    const { value } = await withRetry(() =>
      connection.getSignatureStatuses([signature]),
    );
    const status = value[0];
    if (status?.err) {
      throw new Error(`Transaction failed: ${JSON.stringify(status.err)}`);
    }
    if (
      status?.confirmationStatus === "confirmed" ||
      status?.confirmationStatus === "finalized"
    ) {
      return;
    }
    const height = await withRetry(() =>
      connection.getBlockHeight("confirmed"),
    );
    if (height > lastValidBlockHeight && !status) {
      throw new Error("The transaction expired before it was confirmed.");
    }
    await sleep(1500);
  }
}

async function waitFor<T>(
  fn: () => Promise<T | undefined>,
  timeoutMs: number,
  intervalMs = 2000,
): Promise<T | undefined> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const value = await fn();
    if (value !== undefined) return value;
    if (Date.now() > deadline) return undefined;
    await sleep(intervalMs);
  }
}

// ---------------------------------------------------------- token accounts

function createAtaInstruction(
  payer: PublicKey,
  owner: PublicKey,
  mint: PublicKey,
): TransactionInstruction {
  return new TransactionInstruction({
    programId: ATA_PROGRAM_ID,
    keys: [
      { pubkey: payer, isSigner: true, isWritable: true },
      { pubkey: ataAddress(owner, mint), isSigner: false, isWritable: true },
      { pubkey: owner, isSigner: false, isWritable: false },
      { pubkey: mint, isSigner: false, isWritable: false },
      {
        pubkey: SystemProgram.programId,
        isSigner: false,
        isWritable: false,
      },
      { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
    ],
    data: Buffer.from([1]), // CreateIdempotent
  });
}

/** Creates the owner's token account for `mint` if it does not exist. */
async function ensureAta(
  ctx: OpsContext,
  mint: PublicKey,
  report: Report,
  label: string,
): Promise<PublicKey> {
  const address = ataAddress(ctx.signer.publicKey, mint);
  const existing = await withRetry(() =>
    ctx.connection.getAccountInfo(address, "confirmed"),
  );
  if (existing !== null) return address;
  report({ stage: "signing", label });
  const signature = await sendTransaction(
    ctx,
    [createAtaInstruction(ctx.signer.publicKey, ctx.signer.publicKey, mint)],
    {
      onSubmitted: (sig) =>
        report({ stage: "submitted", signature: sig, label }),
    },
  );
  report({ stage: "submitted", signature, label });
  return address;
}

// ------------------------------------------------------- queued operations

interface QueueParams {
  sequence: BN;
  offset: BN;
  shieldSequence: BN;
  intent: PublicKey;
}

export type OpOutcome = "applied" | "rejected" | "failed";

export interface QueuedResult {
  outcome: OpOutcome;
  signature?: string;
  /** The Operation Intent this run created. */
  intent: PublicKey;
  /** Terminal intent status (`applied`, `zero`, ...), `queued` on timeout. */
  status: string;
}

interface RunOptions {
  shield?: boolean;
  /** Shown when the MPC circuit refuses the operation. */
  rejectedDetail?: string;
  /** A `zero` result is a valid answer (health checks), not a refusal. */
  zeroIsResult?: boolean;
}

function queueAccounts(offset: BN, circuit: string) {
  return deriveArciumQueueAccounts({
    programId: PROGRAM_ID,
    clusterOffset: CLUSTER_OFFSET,
    computationOffset: offset,
    circuit,
  });
}

const ENDED_DETAIL: Record<string, string> = {
  stale:
    "Prices or rates changed while the MPC network was computing. Nothing was changed. Please try again.",
  expired:
    "The MPC result arrived too late and was discarded. Nothing was changed. Please try again.",
  failed: "The MPC computation failed. Nothing was changed. Please try again.",
  abandoned: "The operation was released. Nothing was changed.",
};

/**
 * Queues one MPC operation and follows it to a terminal state:
 * Signing -> Submitted -> Computing (MPC) -> Applied / Rejected / Failed.
 */
async function runQueued(
  ctx: OpsContext,
  label: string,
  build: (params: QueueParams) => Promise<TransactionInstruction[]>,
  report: Report,
  options: RunOptions = {},
): Promise<QueuedResult> {
  const owner = ctx.signer.publicKey;
  const counterAddress = deriveOperationCounterPda(PROGRAM_ID, owner);
  const counter = await fetchAccount<OperationCounterAccount>(
    ctx.program,
    "operationCounter",
    counterAddress,
  );
  const position = await fetchAccount<PositionAccount>(
    ctx.program,
    "position",
    positionAddress(owner),
  );
  const sequence = counter?.nextSequence ?? new BN(0);
  const params: QueueParams = {
    sequence,
    offset: randomU64(),
    shieldSequence: position?.nextShieldSeq ?? new BN(0),
    intent: deriveIntentPda(PROGRAM_ID, owner, BigInt(sequence.toString())),
  };
  const instructions = await build(params);
  report({ stage: "signing", label });
  const signature = await sendTransaction(ctx, instructions, {
    computeUnits: QUEUE_COMPUTE_UNITS,
    onSubmitted: (sig) => report({ stage: "submitted", signature: sig, label }),
  });
  report({ stage: "computing", signature, label });

  const terminal = await waitFor(async () => {
    const intent = await fetchAccount<IntentAccount>(
      ctx.program,
      "operationIntent",
      params.intent,
    );
    if (intent === null) return undefined;
    const name = variantName(intent.status);
    return name === "queued" ? undefined : name;
  }, INTENT_TIMEOUT_MS);
  const done = (outcome: OpOutcome, status: string): QueuedResult => ({
    outcome,
    signature,
    intent: params.intent,
    status,
  });

  if (terminal === undefined) {
    report({
      stage: "failed",
      signature,
      label,
      detail:
        "Still waiting for the MPC result. The operation stays queued and your account is locked until it completes or can be abandoned below.",
    });
    return done("failed", "queued");
  }
  if (terminal === "zero" && options.zeroIsResult !== true) {
    report({
      stage: "rejected",
      signature,
      label,
      detail:
        options.rejectedDetail ??
        "Refused by the MPC circuit. Your balances are unchanged.",
    });
    return done("rejected", terminal);
  }
  if (terminal !== "applied" && terminal !== "zero") {
    report({
      stage: "failed",
      signature,
      label,
      detail:
        ENDED_DETAIL[terminal] ??
        `The operation ended as ${terminal}. Your balances are unchanged.`,
    });
    return done("failed", terminal);
  }
  if (options.shield === true) {
    const pending = pendingShieldAddress(
      positionAddress(owner),
      BigInt(params.shieldSequence.toString()),
    );
    await waitFor(async () => {
      const p = await fetchAccount<PendingShieldAccount>(
        ctx.program,
        "pendingShield",
        pending,
      );
      return p !== null && variantName(p.status) === "credited" ? p : undefined;
    }, 90_000);
  }
  report({ stage: "applied", signature, label });
  return done("applied", terminal);
}

// ------------------------------------------------------------- operations

/** Creates the owner's Position and waits until it is Active. */
export async function initializePosition(
  ctx: OpsContext,
  viewingPublicKey: Uint8Array,
  report: Report,
): Promise<OpOutcome> {
  const owner = ctx.signer.publicKey;
  const position = positionAddress(owner);
  const offset = randomU64();
  const label = "Create private account";
  const instruction = await method(ctx.program, "initializePosition")(
    offset,
    new BN(0),
    Array.from(viewingPublicKey),
    randomU128(),
  )
    .accountsPartial({
      payer: owner,
      ...queueAccounts(offset, "init_position_state"),
      market: ctx.protocol.marketAddress,
      position,
      operationCounter: deriveOperationCounterPda(PROGRAM_ID, owner),
      operationIntent: deriveIntentPda(PROGRAM_ID, owner, 0),
    })
    .instruction();
  report({ stage: "signing", label });
  const signature = await sendTransaction(ctx, [instruction], {
    computeUnits: QUEUE_COMPUTE_UNITS,
    onSubmitted: (sig) => report({ stage: "submitted", signature: sig, label }),
  });
  report({ stage: "computing", signature, label });
  const status = await waitFor(async () => {
    const p = await fetchAccount<PositionAccount>(
      ctx.program,
      "position",
      position,
    );
    if (p === null) return undefined;
    const name = variantName(p.status);
    return name === "active" || name === "initFailed" ? name : undefined;
  }, INTENT_TIMEOUT_MS);
  if (status === "active") {
    report({ stage: "applied", signature, label });
    return "applied";
  }
  report({
    stage: "failed",
    signature,
    label,
    detail: "The account could not be created yet. Please try again.",
  });
  return "failed";
}

/** Deposit: shield wallet tokens into the private balance. */
export async function deposit(
  ctx: OpsContext,
  asset: Asset,
  amount: bigint,
  report: Report,
): Promise<OpOutcome> {
  const { market } = ctx.protocol;
  const owner = ctx.signer.publicKey;
  const mint = asset === "sol" ? market.wsolMint : market.usdcMint;
  const vault = asset === "sol" ? market.wsolVault : market.usdcVault;
  const label = asset === "sol" ? "Deposit SOL" : "Deposit USDC";
  const source = await ensureAta(
    ctx,
    mint,
    report,
    asset === "sol" ? "Prepare SOL account" : "Prepare USDC account",
  );
  const result = await runQueued(
    ctx,
    label,
    async ({ sequence, offset, shieldSequence, intent }) => {
      const ixs: TransactionInstruction[] = [];
      if (asset === "sol") {
        ixs.push(
          SystemProgram.transfer({
            fromPubkey: owner,
            toPubkey: source,
            lamports: amount,
          }),
          new TransactionInstruction({
            programId: TOKEN_PROGRAM_ID,
            keys: [{ pubkey: source, isSigner: false, isWritable: true }],
            data: Buffer.from([17]), // SyncNative
          }),
        );
      }
      ixs.push(
        await method(ctx.program, "shield")(
          offset,
          sequence,
          shieldSequence,
          asset === "sol" ? { sol: {} } : { usdc: {} },
          new BN(amount.toString()),
        )
          .accountsPartial({
            payer: owner,
            ...queueAccounts(offset, "shield_credit"),
            market: ctx.protocol.marketAddress,
            assetState: market.assetState,
            position: positionAddress(owner),
            operationCounter: deriveOperationCounterPda(PROGRAM_ID, owner),
            operationIntent: intent,
            pendingShield: pendingShieldAddress(
              positionAddress(owner),
              BigInt(shieldSequence.toString()),
            ),
            mint,
            vault,
            sourceTokenAccount: source,
            tokenProgram: TOKEN_PROGRAM_ID,
          })
          .instruction(),
      );
      return ixs;
    },
    report,
    { shield: true },
  );
  return result.outcome;
}

/** Withdraw: unshield private tokens to the wallet (SOL is then unwrapped). */
export async function withdraw(
  ctx: OpsContext,
  asset: Asset,
  amount: bigint,
  report: Report,
): Promise<OpOutcome> {
  const { market } = ctx.protocol;
  const owner = ctx.signer.publicKey;
  const mint = asset === "sol" ? market.wsolMint : market.usdcMint;
  const label = asset === "sol" ? "Withdraw SOL" : "Withdraw USDC";
  const destination = await ensureAta(
    ctx,
    mint,
    report,
    asset === "sol" ? "Prepare SOL account" : "Prepare USDC account",
  );
  const result = await runQueued(
    ctx,
    label,
    async ({ sequence, offset, intent }) => [
      await method(ctx.program, "unshield")(
        offset,
        sequence,
        asset === "sol" ? { sol: {} } : { usdc: {} },
        new BN(amount.toString()),
      )
        .accountsPartial({
          payer: owner,
          ...queueAccounts(offset, "unshield"),
          market: ctx.protocol.marketAddress,
          assetState: market.assetState,
          position: positionAddress(owner),
          operationCounter: deriveOperationCounterPda(PROGRAM_ID, owner),
          operationIntent: intent,
          destination,
        })
        .instruction(),
    ],
    asset === "sol"
      ? (p) =>
          report(
            p.stage === "applied"
              ? { ...p, stage: "computing", detail: "Unwrapping to SOL" }
              : p,
          )
      : report,
  );
  if (result.outcome === "applied" && asset === "sol") {
    await unwrapWsol(ctx, report, "Unwrap to SOL");
  }
  return result.outcome;
}

/** Closes the owner's wSOL account so its lamports return as native SOL. */
export async function unwrapWsol(
  ctx: OpsContext,
  report: Report,
  label = "Unwrap to SOL",
): Promise<void> {
  const owner = ctx.signer.publicKey;
  const wsol = ataAddress(owner, ctx.protocol.market.wsolMint);
  report({ stage: "signing", label });
  const signature = await sendTransaction(
    ctx,
    [buildUnwrapWsolInstruction(owner, wsol)],
    {
      onSubmitted: (sig) =>
        report({ stage: "submitted", signature: sig, label }),
    },
  );
  report({ stage: "applied", signature, label });
}

/**
 * Lending operations of the Position owner (`lend_op` ops 0 to 5): add or
 * remove collateral, borrow, repay, supply, withdraw supply. The program turns
 * the USDC amount into shares from the on-chain index.
 */
export async function lendOp(
  ctx: OpsContext,
  lending: LendingView,
  op: LendOp,
  label: string,
  amount: bigint,
  report: Report,
): Promise<OpOutcome> {
  const owner = ctx.signer.publicKey;
  const { market, marketAddress } = ctx.protocol;
  const result = await runQueued(
    ctx,
    label,
    async ({ sequence, offset }) => [
      await buildLendOpInstruction({
        program: ctx.program,
        signer: owner,
        market: marketAddress,
        position: positionAddress(owner),
        sequence: BigInt(sequence.toString()),
        computationOffset: offset,
        clusterOffset: CLUSTER_OFFSET,
        assetState: market.assetState,
        priceUpdate: lending.pool.oracle,
        usdcVault: market.usdcVault,
        op,
        amount,
      }),
    ],
    report,
  );
  return result.outcome;
}

// ------------------------------------------------- private operations (P.7)

const NOT_APPLIED =
  "Not applied, nothing moved. The MPC circuit refused this amount. Check your balances and try again.";

/** Indexes and accrual time both sides of a private operation agree on. */
interface PrivateAccrual {
  view: LendingView;
  accrueToTs: bigint;
  supplyIndex: bigint;
  borrowIndex: bigint;
}

/**
 * Re-reads the LendingPool and the cluster clock, picks `accrue_to_ts` from
 * the cluster time (never the browser clock) and projects the indexes to it.
 */
async function prepareAccrual(
  ctx: OpsContext,
  fallback: LendingView,
): Promise<PrivateAccrual> {
  const view = (await loadLendingView(ctx, ctx.protocol)) ?? fallback;
  const now = await chainNowSeconds(ctx.connection);
  const accrueToTs = chooseAccrualTs(view.pool.lastAccrualTs, now);
  const projected = projectIndexesAuto(view.pool, accrueToTs, {
    reserveFactorBps: ctx.protocol.reserveFactorBps,
    rates: ctx.protocol.rates,
    band: view.band,
  });
  return {
    view,
    accrueToTs,
    supplyIndex: projected.supplyIndex1e9,
    borrowIndex: projected.borrowIndex1e9,
  };
}

/**
 * Permissionless utilization band refresh: updates the rate curve for confidential borrows.
 * Tries the serverless Next.js API keeper first, then falls back to signing with the connected wallet.
 */
export async function refreshPoolBand(
  ctx: OpsContext,
  report?: Report,
): Promise<void> {
  report?.({ stage: "computing", detail: "Checking rate curve status..." });
  try {
    const res = await fetch("/api/band/refresh?force=1", { method: "POST" });
    if (res.ok) {
      const data = await res.json();
      if (data.success && data.refreshed) {
        report?.({
          stage: "applied",
          detail: "Rate curve refreshed by network keeper.",
        });
        return;
      }
    }
  } catch {
    // API route unavailable, fallback to direct wallet signing
  }

  report?.({
    stage: "signing",
    detail: "Queueing utilization band update with Arcium MPC...",
  });
  const { market, marketAddress } = ctx.protocol;
  const poolState = derivePoolStatePda(ctx.program.programId, marketAddress);
  const poolCounter = derivePoolCounterPda(ctx.program.programId, poolState);
  const counter = await fetchAccount<OperationCounterAccount>(
    ctx.program,
    "operationCounter",
    poolCounter,
  );
  const sequence = BigInt(counter?.nextSequence?.toString() ?? 0);
  const offset = BigInt(Math.floor(Math.random() * 2 ** 48));

  const instruction = await buildUpdatePoolBandInstruction({
    program: ctx.program,
    payer: ctx.signer.publicKey,
    market: marketAddress,
    assetState: market.assetState,
    poolState,
    sequence,
    computationOffset: offset,
    clusterOffset: CLUSTER_OFFSET,
  });

  await sendTransaction(ctx, [instruction], {
    computeUnits: QUEUE_COMPUTE_UNITS,
    onSubmitted: (sig) =>
      report?.({
        stage: "submitted",
        signature: sig,
        detail: "Band update queued, awaiting Arcium callback...",
      }),
  });

  report?.({
    stage: "computing",
    detail: "Awaiting Arcium MPC callback...",
  });

  const intent = deriveIntentPda(ctx.program.programId, poolState, sequence);
  for (let i = 0; i < 20; i++) {
    await sleep(1500);
    const intentAcc = await fetchAccount<IntentAccount>(
      ctx.program,
      "operationIntent",
      intent,
    );
    const status = intentAcc ? variantName(intentAcc.status) : "missing";
    if (status === "applied") {
      report?.({
        stage: "applied",
        detail: "Rate curve refreshed successfully!",
      });
      return;
    }
  }
}

/**
 * Supply, withdraw supply, borrow or repay with an encrypted amount
 * (`pool_op`). The amount and its share count are encrypted to the MXE; only
 * the `applied` bit of the result is public.
 */
export async function privatePoolOp(
  ctx: OpsContext,
  lending: LendingView,
  op: PoolOp,
  label: string,
  amount: bigint,
  report: Report,
  /**
   * The owned share count for repay and withdraw supply. The indexes at the
   * accrual time can sit a little below the displayed ones, so a "max" amount
   * is trimmed to burn exactly this many shares instead of more.
   */
  shareCap?: bigint,
): Promise<OpOutcome> {
  const owner = ctx.signer.publicKey;
  const { market, marketAddress } = ctx.protocol;

  // Auto-refresh rate curve if borrowing and band is approaching expiry (>6000 slots old)
  if (op === PoolOp.Borrow) {
    try {
      const currentSlot = await ctx.connection.getSlot("confirmed");
      const bandAge = Math.max(0, currentSlot - lending.bandObservationSlot);
      if (bandAge >= 6000) {
        report?.({
          stage: "computing",
          detail: `Utilization band is ${bandAge} slots old. Refreshing rate curve before borrowing...`,
        });
        await refreshPoolBand(ctx, report);
        const refreshed = await loadLendingView(ctx, ctx.protocol);
        if (refreshed) lending = refreshed;
      }
    } catch (e) {
      console.warn("Auto-refresh band check encountered:", e);
    }
  }

  const result = await runQueued(
    ctx,
    label,
    async ({ sequence, offset }) => {
      const a = await prepareAccrual(ctx, lending);
      let value = amount;
      let shares = poolOpShares(op, value, a.supplyIndex, a.borrowIndex);
      if (shareCap !== undefined && shares > shareCap) {
        if (op === PoolOp.Repay) {
          const trimmed = fullRepayAmountPrivate(shareCap, a.borrowIndex);
          if (trimmed <= value) value = trimmed;
        } else if (op === PoolOp.WithdrawSupply) {
          value = (shareCap * a.supplyIndex) / INDEX_SCALE;
        }
        shares = poolOpShares(op, value, a.supplyIndex, a.borrowIndex);
      }
      if (
        shares <= 0n ||
        !poolOpConversionHolds(op, value, shares, a.supplyIndex, a.borrowIndex)
      ) {
        throw new Error("That amount is too small to register.");
      }
      const input = encryptPrivateInput(ctx.protocol.mxePublicKey, [
        value,
        shares,
      ]);
      return [
        await buildPoolOpInstruction({
          program: ctx.program,
          signer: owner,
          market: marketAddress,
          assetState: market.assetState,
          sequence: BigInt(sequence.toString()),
          computationOffset: offset,
          clusterOffset: CLUSTER_OFFSET,
          priceUpdate: a.view.pool.oracle,
          accrueToTs: a.accrueToTs,
          input,
          op,
        }),
      ];
    },
    report,
    { rejectedDetail: NOT_APPLIED },
  );
  return result.outcome;
}

/** Adds or removes SOL collateral with an encrypted amount (`position_private`). */
export async function privateCollateral(
  ctx: OpsContext,
  lending: LendingView,
  op: CollateralOp,
  label: string,
  lamports: bigint,
  report: Report,
): Promise<OpOutcome> {
  const owner = ctx.signer.publicKey;
  const { market, marketAddress } = ctx.protocol;
  const result = await runQueued(
    ctx,
    label,
    async ({ sequence, offset }) => {
      const a = await prepareAccrual(ctx, lending);
      const input = encryptPrivateInput(ctx.protocol.mxePublicKey, [
        lamports,
        0n,
      ]);
      return [
        await buildPositionPrivateInstruction({
          program: ctx.program,
          signer: owner,
          market: marketAddress,
          assetState: market.assetState,
          sequence: BigInt(sequence.toString()),
          computationOffset: offset,
          clusterOffset: CLUSTER_OFFSET,
          priceUpdate: a.view.pool.oracle,
          accrueToTs: a.accrueToTs,
          input,
          op,
        }),
      ];
    },
    report,
    { rejectedDetail: NOT_APPLIED },
  );
  return result.outcome;
}

/** The SOL a liquidation of `usdcAmount` seizes, at the current indexes and price. */
export async function estimateSeize(
  ctx: Pick<OpsContext, "connection" | "protocol">,
  lending: LendingView,
  usdcAmount: bigint,
): Promise<bigint> {
  const now = await chainNowSeconds(ctx.connection);
  const accrueToTs = chooseAccrualTs(lending.pool.lastAccrualTs, now);
  const projected = projectIndexesAuto(lending.pool, accrueToTs, {
    reserveFactorBps: ctx.protocol.reserveFactorBps,
    rates: ctx.protocol.rates,
    band: lending.band,
  });
  const price = await fetchPoolLowPrice(ctx.connection, lending.pool.oracle);
  return liquidationInputs(
    usdcAmount,
    projected.borrowIndex1e9,
    price,
    ctx.protocol.liquidationBonusBps,
  ).seize;
}

/**
 * The borrower Position for an owner address, or the address itself when it
 * already is an active Position.
 */
async function resolveTargetPosition(
  ctx: OpsContext,
  ownerOrPosition: PublicKey,
): Promise<PublicKey> {
  const byOwner = positionAddress(ownerOrPosition);
  const owned = await fetchAccount<PositionAccount>(
    ctx.program,
    "position",
    byOwner,
  );
  if (owned !== null && variantName(owned.status) === "active") return byOwner;
  const direct = await fetchAccount<PositionAccount>(
    ctx.program,
    "position",
    ownerOrPosition,
  );
  if (direct !== null && variantName(direct.status) === "active") {
    return ownerOrPosition;
  }
  throw new Error("No active private account was found for that address.");
}

/**
 * Liquidates a borrower with an encrypted amount (`liquidate_pool`). The
 * signer repays from its own free USDC and receives the seized SOL into its
 * own Position; nothing is transferred publicly.
 */
export async function privateLiquidate(
  ctx: OpsContext,
  lending: LendingView,
  targetOwnerOrPosition: PublicKey,
  usdcAmount: bigint,
  report: Report,
): Promise<LiquidationResult> {
  const owner = ctx.signer.publicKey;
  const { market, marketAddress } = ctx.protocol;
  const target = await resolveTargetPosition(ctx, targetOwnerOrPosition);
  if (target.equals(positionAddress(owner))) {
    throw new Error("You cannot liquidate your own account.");
  }
  const result = await runQueued(
    ctx,
    "Liquidate",
    async ({ sequence, offset }) => {
      const a = await prepareAccrual(ctx, lending);
      const price = await fetchPoolLowPrice(ctx.connection, a.view.pool.oracle);
      const bonus = ctx.protocol.liquidationBonusBps;
      const inputs = liquidationInputs(usdcAmount, a.borrowIndex, price, bonus);
      if (!liquidationConversionHolds(inputs, a.borrowIndex, price, bonus)) {
        throw new Error("That amount is too small to register.");
      }
      const input = encryptPrivateInput(ctx.protocol.mxePublicKey, [
        inputs.amount,
        inputs.burnShares,
        inputs.seize,
      ]);
      return [
        await buildLiquidatePoolInstruction({
          program: ctx.program,
          signer: owner,
          market: marketAddress,
          assetState: market.assetState,
          sequence: BigInt(sequence.toString()),
          computationOffset: offset,
          clusterOffset: CLUSTER_OFFSET,
          priceUpdate: a.view.pool.oracle,
          accrueToTs: a.accrueToTs,
          input,
          target,
        }),
      ];
    },
    report,
    {
      rejectedDetail:
        "Not applied, nothing moved. The position is healthy, or the amount is above what can be repaid at once, or the price moved.",
    },
  );
  // A queued private liquidation holds the signer's own Position Lock; it is
  // released from the stuck-operation notice, not from a pending record.
  return { outcome: result.outcome, pending: null };
}

/** Why a borrower address cannot be checked or liquidated, or null. */
async function borrowerPosition(
  ctx: OpsContext,
  borrower: PublicKey,
): Promise<PublicKey> {
  const address = positionAddress(borrower);
  const position = await fetchAccount<PositionAccount>(
    ctx.program,
    "position",
    address,
  );
  if (position === null || variantName(position.status) !== "active") {
    throw new Error("No active private account was found for that address.");
  }
  return address;
}

export interface HealthResult {
  outcome: OpOutcome;
  /** True when the position is below its liquidation threshold. */
  unhealthy: boolean | null;
}

/** Permissionless health check of any borrower; the result comes from the MPC. */
export async function checkHealth(
  ctx: OpsContext,
  lending: LendingView,
  borrower: PublicKey,
  report: Report,
): Promise<HealthResult> {
  const owner = ctx.signer.publicKey;
  const target = await borrowerPosition(ctx, borrower);
  const result = await runQueued(
    ctx,
    "Check health",
    async ({ sequence, offset }) => [
      await buildCheckHealthInstruction({
        program: ctx.program,
        signer: owner,
        market: ctx.protocol.marketAddress,
        position: target,
        sequence: BigInt(sequence.toString()),
        computationOffset: offset,
        clusterOffset: CLUSTER_OFFSET,
        assetState: ctx.protocol.market.assetState,
        priceUpdate: lending.pool.oracle,
      }),
    ],
    report,
    { zeroIsResult: true },
  );
  if (result.outcome !== "applied")
    return { outcome: "failed", unhealthy: null };
  const unhealthy = result.status === "applied";
  report({
    stage: "applied",
    ...(result.signature !== undefined ? { signature: result.signature } : {}),
    label: "Check health",
    detail: unhealthy
      ? "This position is below its liquidation threshold."
      : "This position is healthy.",
  });
  return { outcome: "applied", unhealthy };
}

export interface LiquidationResult {
  outcome: OpOutcome;
  /** Set when the result is still pending, so the refund can be released later. */
  pending: PendingLiquidation | null;
}

export interface PendingLiquidation {
  intent: PublicKey;
  position: PublicKey;
}

/**
 * Repays part of a borrower's debt in USDC and receives their SOL collateral
 * with the liquidation bonus. Creates the liquidator's USDC and wSOL token
 * accounts when missing.
 */
export async function liquidate(
  ctx: OpsContext,
  lending: LendingView,
  borrower: PublicKey,
  repayAmount: bigint,
  report: Report,
): Promise<LiquidationResult> {
  const owner = ctx.signer.publicKey;
  const { market, marketAddress } = ctx.protocol;
  const target = await borrowerPosition(ctx, borrower);
  const liquidatorUsdc = await ensureAta(
    ctx,
    market.usdcMint,
    report,
    "Prepare USDC account",
  );
  const liquidatorWsol = await ensureAta(
    ctx,
    market.wsolMint,
    report,
    "Prepare SOL account",
  );
  const result = await runQueued(
    ctx,
    "Liquidate",
    async ({ sequence, offset }) => [
      await buildLiquidateInstruction({
        program: ctx.program,
        signer: owner,
        market: marketAddress,
        position: target,
        sequence: BigInt(sequence.toString()),
        computationOffset: offset,
        clusterOffset: CLUSTER_OFFSET,
        assetState: market.assetState,
        priceUpdate: lending.pool.oracle,
        usdcVault: market.usdcVault,
        wsolVault: market.wsolVault,
        liquidatorUsdc,
        liquidatorWsol,
        tokenProgram: TOKEN_PROGRAM_ID,
        repayAmount,
      }),
    ],
    report,
    {
      rejectedDetail:
        "The liquidation was refused: the position is healthy or the amount is above what can be repaid at once. Your USDC was returned.",
    },
  );
  return {
    outcome: result.outcome,
    pending:
      result.status === "queued"
        ? { intent: result.intent, position: target }
        : null,
  };
}

/** Releases a timed-out liquidation and returns the repaid USDC. */
export async function abandonLiquidation(
  ctx: OpsContext,
  pending: PendingLiquidation,
  report: Report,
): Promise<void> {
  const owner = ctx.signer.publicKey;
  const { market, marketAddress } = ctx.protocol;
  const label = "Release liquidation";
  const instruction = await buildAbandonLendOpInstruction({
    program: ctx.program,
    signer: owner,
    market: marketAddress,
    position: pending.position,
    operationIntent: pending.intent,
    liquidationAccounts: [
      market.usdcVault,
      market.wsolVault,
      market.vaultAuthority,
      TOKEN_PROGRAM_ID,
      ataAddress(owner, market.usdcMint),
      ataAddress(owner, market.wsolMint),
    ],
  });
  report({ stage: "signing", label });
  const signature = await sendTransaction(ctx, [instruction], {
    onSubmitted: (sig) => report({ stage: "submitted", signature: sig, label }),
  });
  report({ stage: "applied", signature, label });
}

export interface RotationRequest {
  /** The new Viewing Key (derived for epoch + 1). */
  newKey: ViewingKeyMaterial;
  oldEpoch: bigint;
  oldPublicKey: Uint8Array;
  /** Balances decrypted with the current key, to compare after rotation. */
  expectedBalances: PositionBalances;
  /** Called only after the rotated Position decrypts to the same balances. */
  commitKey(newKey: ViewingKeyMaterial): void;
}

/** Rotates the Viewing Key; the old key stays in use unless every check passes. */
export async function rotateKey(
  ctx: OpsContext,
  request: RotationRequest,
  report: Report,
): Promise<OpOutcome> {
  const owner = ctx.signer.publicKey;
  const position = positionAddress(owner);
  const label = "Rotate Viewing Key";
  await rotateViewingKey({
    newKey: request.newKey,
    expectedBalances: request.expectedBalances,
    mxePublicKey: ctx.protocol.mxePublicKey,
    submit: async (newPublicKey) => {
      const result = await runQueued(
        ctx,
        label,
        async ({ sequence, offset }) => [
          await buildRotatePositionKeyInstruction({
            program: ctx.program,
            owner,
            position,
            sequence: BigInt(sequence.toString()),
            oldEpoch: request.oldEpoch,
            oldPublicKey: request.oldPublicKey,
            newPublicKey,
            computationOffset: offset,
            clusterOffset: CLUSTER_OFFSET,
          }),
        ],
        report,
      );
      if (result.outcome !== "applied") {
        throw new Error("The rotation was not applied.");
      }
    },
    fetchState: async () => {
      const current = await fetchAccount<PositionAccount>(
        ctx.program,
        "position",
        position,
      );
      if (current === null) throw new Error("Position not found");
      return {
        nonce: BigInt(current.encryptedState.nonce.toString()),
        ciphertexts: current.encryptedState.ciphertexts,
      };
    },
    commitKey: request.commitKey,
  });
  return "applied";
}

/** Seals the selected fields of the Position to an auditor's x25519 key. */
export async function disclose(
  ctx: OpsContext,
  auditorPublicKey: Uint8Array,
  mask: number,
  report: Report,
): Promise<OpOutcome> {
  const owner = ctx.signer.publicKey;
  const result = await runQueued(
    ctx,
    "Disclose to auditor",
    async ({ sequence, offset }) => [
      await buildDiscloseInstruction({
        program: ctx.program,
        owner,
        position: positionAddress(owner),
        sequence: BigInt(sequence.toString()),
        auditorPubkey: auditorPublicKey,
        mask,
        computationOffset: offset,
        clusterOffset: CLUSTER_OFFSET,
      }),
    ],
    report,
  );
  return result.outcome;
}

// ------------------------------------------------------------- stuck locks

export interface StuckOperation {
  intent: PublicKey;
  kind: string;
  expirySlot: number;
  /** Whether the operation can be abandoned right now. */
  expired: boolean;
  /** Estimated seconds until the Lock expires (0 when expired). */
  secondsLeft: number;
  /** The abandon instruction for this kind exists. */
  supported: boolean;
  /** Primary Position of a private operation (the liquidation target). */
  position: PublicKey | null;
  /** The liquidator's Position of a private liquidation. */
  secondPosition: PublicKey | null;
}

const ABANDON_BY_KIND: Record<string, string> = {
  positionOp: "abandonPositionOp",
  lendOp: "abandonLendOp",
  shield: "abandonShield",
  unshield: "abandonUnshield",
  positionInit: "abandonPositionInit",
  viewingKeyRotation: "abandonRotation",
  disclose: "abandonDisclosure",
  poolOp: "abandonPrivateOp",
  positionPrivate: "abandonPrivateOp",
  liquidatePool: "abandonPrivateOp",
};

const KIND_LABEL: Record<string, string> = {
  positionOp: "collateral, borrow or repay",
  lendOp: "lending operation",
  shield: "deposit",
  unshield: "withdrawal",
  positionInit: "account setup",
  viewingKeyRotation: "Viewing Key rotation",
  disclose: "disclosure",
  poolOp: "private supply, withdrawal, borrow or repay",
  positionPrivate: "private collateral change",
  liquidatePool: "private liquidation",
};

/** Intent kinds of the encrypted-amount operations (ADR-0004). */
function isPrivateKind(kind: string): boolean {
  return (
    kind === "poolOp" || kind === "positionPrivate" || kind === "liquidatePool"
  );
}

export function kindLabel(kind: string): string {
  return KIND_LABEL[kind] ?? kind;
}

/** Reports a previous operation that still holds the Position Lock. */
export async function findStuckOperation(
  ctx: Pick<OpsContext, "connection" | "program">,
  position: PositionAccount,
): Promise<StuckOperation | null> {
  const lock = position.coordination.lock;
  if (!lock.active) return null;
  const intent = await fetchAccount<IntentAccount>(
    ctx.program,
    "operationIntent",
    lock.operationIntent,
  );
  if (intent === null || variantName(intent.status) !== "queued") return null;
  const kind = variantName(intent.kind);
  const slot = await withRetry(() => ctx.connection.getSlot("confirmed"));
  const expirySlot = Number(lock.expirySlot.toString());
  // A liquidation queued by someone else needs that liquidator's accounts to
  // release, so only the owner's own lending operations can be released here.
  const releasable =
    (kind !== "lendOp" && kind !== "liquidatePool") ||
    intent.payer.equals(position.owner);
  return {
    intent: lock.operationIntent,
    kind,
    expirySlot,
    expired: slot > expirySlot,
    secondsLeft: slot > expirySlot ? 0 : Math.ceil((expirySlot - slot) * 0.4),
    supported: ABANDON_BY_KIND[kind] !== undefined && releasable,
    position: intent.position ?? null,
    secondPosition: intent.secondPosition ?? null,
  };
}

/** Abandons the stuck operation after its Lock expired. */
export async function abandonStuck(
  ctx: OpsContext,
  stuck: StuckOperation,
  position: PositionAccount,
  report: Report,
): Promise<void> {
  const name = ABANDON_BY_KIND[stuck.kind];
  if (name === undefined) throw new Error("This operation cannot be abandoned");
  const owner = ctx.signer.publicKey;
  let instruction: TransactionInstruction;
  if (isPrivateKind(stuck.kind)) {
    const own = positionAddress(owner);
    instruction = await buildAbandonPrivateOpInstruction({
      program: ctx.program,
      signer: owner,
      market: ctx.protocol.marketAddress,
      position: stuck.kind === "liquidatePool" ? (stuck.position ?? own) : own,
      operationIntent: stuck.intent,
      ...(stuck.kind !== "positionPrivate"
        ? {
            poolState: derivePoolStatePda(
              PROGRAM_ID,
              ctx.protocol.marketAddress,
            ),
          }
        : {}),
      ...(stuck.kind === "liquidatePool"
        ? { secondPosition: stuck.secondPosition ?? own }
        : {}),
    });
  } else if (stuck.kind === "lendOp") {
    instruction = await buildAbandonLendOpInstruction({
      program: ctx.program,
      signer: owner,
      market: ctx.protocol.marketAddress,
      position: positionAddress(owner),
      operationIntent: stuck.intent,
    });
  } else {
    const accounts: Record<string, PublicKey> =
      stuck.kind === "disclose"
        ? { signer: owner, operationIntent: stuck.intent }
        : {
            signer: owner,
            position: positionAddress(owner),
            operationIntent: stuck.intent,
          };
    if (stuck.kind === "shield") {
      const next = BigInt(position.nextShieldSeq.toString());
      if (next === 0n) throw new Error("No pending deposit found");
      accounts["pendingShield"] = pendingShieldAddress(
        positionAddress(owner),
        next - 1n,
      );
    }
    instruction = await method(ctx.program, name)()
      .accountsPartial(accounts)
      .instruction();
  }
  const label = "Release stuck operation";
  report({ stage: "signing", label });
  const signature = await sendTransaction(ctx, [instruction], {
    onSubmitted: (sig) => report({ stage: "submitted", signature: sig, label }),
  });
  report({ stage: "applied", signature, label });
}
