// Write side: instruction building, queued-operation tracking and the
// transaction sender. Ported from packages/sdk/demo/loan-demo.ts; signing goes
// through the wallet instead of keypairs.
import "./polyfill.ts";
import { BN, type Program } from "@anchor-lang/core";
import { Buffer } from "buffer";
import {
  buildAbandonLendOpInstruction,
  buildCheckHealthInstruction,
  buildDiscloseInstruction,
  buildLendOpInstruction,
  buildLiquidateInstruction,
  buildRotatePositionKeyInstruction,
  buildUnwrapWsolInstruction,
  deriveArciumQueueAccounts,
  deriveIntentPda,
  deriveOperationCounterPda,
  rotateViewingKey,
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
  fetchAccount,
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
}

const ABANDON_BY_KIND: Record<string, string> = {
  positionOp: "abandonPositionOp",
  lendOp: "abandonLendOp",
  shield: "abandonShield",
  unshield: "abandonUnshield",
  positionInit: "abandonPositionInit",
  viewingKeyRotation: "abandonRotation",
  disclose: "abandonDisclosure",
};

const KIND_LABEL: Record<string, string> = {
  positionOp: "collateral, borrow or repay",
  lendOp: "lending operation",
  shield: "deposit",
  unshield: "withdrawal",
  positionInit: "account setup",
  viewingKeyRotation: "Viewing Key rotation",
  disclose: "disclosure",
};

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
  const releasable = kind !== "lendOp" || intent.payer.equals(position.owner);
  return {
    intent: lock.operationIntent,
    kind,
    expirySlot,
    expired: slot > expirySlot,
    secondsLeft: slot > expirySlot ? 0 : Math.ceil((expirySlot - slot) * 0.4),
    supported: ABANDON_BY_KIND[kind] !== undefined && releasable,
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
  if (stuck.kind === "lendOp") {
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
