// Write side: instruction building, queued-operation tracking and the
// transaction sender. Ported from packages/sdk/demo/loan-demo.ts; signing goes
// through the wallet instead of keypairs.
import "./polyfill.ts";
import { BN, type Program } from "@anchor-lang/core";
import { Buffer } from "buffer";
import {
  buildUnwrapWsolInstruction,
  deriveArciumQueueAccounts,
  deriveIntentPda,
  deriveOperationCounterPda,
  PositionOp,
} from "@walnut/sdk";
import {
  ComputeBudgetProgram,
  SystemProgram,
  Transaction,
  TransactionInstruction,
  type Connection,
  type PublicKey,
} from "@solana/web3.js";
import { CLUSTER_OFFSET, PROGRAM_ID, QUEUE_COMPUTE_UNITS } from "./config.ts";
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

export function friendlyError(err: unknown): string {
  const raw = err instanceof Error ? err.message : String(err);
  const text = raw.replace(/api-key=[^&\s"')]+/gi, "api-key=REDACTED");
  if (
    /reject|denied|cancel|declined/i.test(text) &&
    /user|request|wallet|transaction/i.test(text)
  ) {
    return "The request was cancelled in your wallet.";
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

function queueAccounts(offset: BN, circuit: string) {
  return deriveArciumQueueAccounts({
    programId: PROGRAM_ID,
    clusterOffset: CLUSTER_OFFSET,
    computationOffset: offset,
    circuit,
  });
}

/**
 * Queues one MPC operation and follows it to a terminal state:
 * Signing -> Submitted -> Computing (MPC) -> Applied / Rejected / Failed.
 */
async function runQueued(
  ctx: OpsContext,
  label: string,
  build: (params: QueueParams) => Promise<TransactionInstruction[]>,
  report: Report,
  options: { shield?: boolean } = {},
): Promise<{ outcome: OpOutcome; signature?: string }> {
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

  if (terminal === undefined) {
    report({
      stage: "failed",
      signature,
      label,
      detail:
        "Still waiting for the MPC result. The operation stays queued and your account is locked until it completes or can be abandoned below.",
    });
    return { outcome: "failed", signature };
  }
  if (terminal === "zero") {
    report({
      stage: "rejected",
      signature,
      label,
      detail: "Refused by the MPC circuit. Your balances are unchanged.",
    });
    return { outcome: "rejected", signature };
  }
  if (terminal !== "applied") {
    report({
      stage: "failed",
      signature,
      label,
      detail: `The operation ended as ${terminal}. Your balances are unchanged.`,
    });
    return { outcome: "failed", signature };
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
  return { outcome: "applied", signature };
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

/** add / remove collateral, borrow, repay. */
export async function positionOp(
  ctx: OpsContext,
  op: PositionOp,
  label: string,
  amount: bigint,
  price: bigint,
  report: Report,
): Promise<OpOutcome> {
  const owner = ctx.signer.publicKey;
  const result = await runQueued(
    ctx,
    label,
    async ({ sequence, offset, intent }) => [
      await method(ctx.program, "positionOp")(
        offset,
        sequence,
        op,
        new BN(amount.toString()),
        new BN(price.toString()),
      )
        .accountsPartial({
          payer: owner,
          ...queueAccounts(offset, "position_op"),
          market: ctx.protocol.marketAddress,
          assetState: ctx.protocol.market.assetState,
          position: positionAddress(owner),
          operationCounter: deriveOperationCounterPda(PROGRAM_ID, owner),
          operationIntent: intent,
        })
        .instruction(),
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
  shield: "abandonShield",
  unshield: "abandonUnshield",
  positionInit: "abandonPositionInit",
};

const KIND_LABEL: Record<string, string> = {
  positionOp: "collateral, borrow or repay",
  shield: "deposit",
  unshield: "withdrawal",
  positionInit: "account setup",
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
  return {
    intent: lock.operationIntent,
    kind,
    expirySlot,
    expired: slot > expirySlot,
    secondsLeft: slot > expirySlot ? 0 : Math.ceil((expirySlot - slot) * 0.4),
    supported: ABANDON_BY_KIND[kind] !== undefined,
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
  const accounts: Record<string, PublicKey> = {
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
  const label = "Release stuck operation";
  const instruction = await method(ctx.program, name)()
    .accountsPartial(accounts)
    .instruction();
  report({ stage: "signing", label });
  const signature = await sendTransaction(ctx, [instruction], {
    onSubmitted: (sig) => report({ stage: "submitted", signature: sig, label }),
  });
  report({ stage: "applied", signature, label });
}
