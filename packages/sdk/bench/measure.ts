// Core measurement: build an operation runner, drive one operation through its
// full queue -> callback-finalization lifecycle with the Task 2.1 retry /
// abandonment pattern, and read the realized Solana fee and compute units from
// transaction metadata.
//
// Metric sources (Arcium 0.16.0):
//   - queue tx fee + queue CU: getTransaction(queueSig).meta.{fee,
//     computeUnitsConsumed} -- the user-paid Solana cost of enqueuing.
//   - callback CU actually consumed + callback tx fee:
//     getTransaction(finalizeSig).meta.{computeUnitsConsumed, fee}, where
//     finalizeSig is returned by awaitComputationFinalization. The callback fee
//     is node-paid; it is recorded for completeness, not billed to the user.
//   - Arcium computation fee: measured as the payer balance delta across the
//     lifecycle minus the queue tx fee (sequential runs only; balance deltas are
//     meaningless when operations overlap). This bundles the computation fee and
//     the computation-account rent; --reclaim separates the rent back out by
//     calling claimComputationRent and measuring the refund.

import * as anchor from "../src/anchor-shim.js";
import type { BN } from "../src/anchor-shim.js";
import { PublicKey, type Keypair } from "@solana/web3.js";
import { randomBytes } from "node:crypto";
import {
  awaitComputationFinalization,
  claimComputationRent,
  getClusterAccAddress,
  getCompDefAccAddress,
  getCompDefAccOffset,
  getComputationAccAddress,
  getExecutingPoolAccAddress,
  getMempoolAccAddress,
  getMXEAccAddress,
  deserializeLE,
  RescueCipher,
  x25519,
} from "@arcium-hq/client";
import { derivePositionPda } from "../src/pda.ts";
import type { BenchContext } from "./config.ts";
import type { BenchOp, OpSample, TxCost } from "./types.ts";

// SpikePosition PDA derivation lives in the SDK (src/pda.ts); re-exported for
// the other bench modules.
export { derivePositionPda };

/** Builds the queue transaction for one operation instance. */
export interface OpRunner {
  op: BenchOp;
  /** Enqueue a computation for this offset; resolves to the queue tx signature. */
  queue(offset: BN): Promise<string>;
}

function newOffset(): BN {
  return new anchor.BN(randomBytes(8), "hex");
}

function nonceBn(nonce: Buffer): BN {
  return new anchor.BN(deserializeLE(nonce).toString());
}

/** Fresh x25519 viewing key + Rescue cipher for one encrypted input. */
function freshCipher(mxePublicKey: Uint8Array): {
  cipher: RescueCipher;
  publicKey: Uint8Array;
} {
  const privateKey = x25519.utils.randomSecretKey();
  const publicKey = x25519.getPublicKey(privateKey);
  const sharedSecret = x25519.getSharedSecret(privateKey, mxePublicKey);
  return { cipher: new RescueCipher(sharedSecret), publicKey };
}

/** Standard Arcium queue accounts for a circuit, mirroring the test files. */
function queueAccounts(
  ctx: BenchContext,
  offset: BN,
  circuit: string,
): Record<string, PublicKey> {
  return {
    payer: ctx.payer.publicKey,
    computationAccount: getComputationAccAddress(ctx.clusterOffset, offset),
    clusterAccount: getClusterAccAddress(ctx.clusterOffset),
    mxeAccount: getMXEAccAddress(ctx.programId),
    mempoolAccount: getMempoolAccAddress(ctx.clusterOffset),
    executingPool: getExecutingPoolAccAddress(ctx.clusterOffset),
    compDefAccount: getCompDefAccAddress(
      ctx.programId,
      Buffer.from(getCompDefAccOffset(circuit)).readUInt32LE(),
    ),
  };
}

/** Runner for `add_together`: two fresh encrypted u8 inputs per call. */
export function buildAddTogetherRunner(ctx: BenchContext): OpRunner {
  return {
    op: "add_together",
    async queue(offset: BN): Promise<string> {
      const { cipher, publicKey } = freshCipher(ctx.mxePublicKey);
      const nonce = randomBytes(16);
      const a = BigInt(randomBytes(1)[0] as number);
      const b = BigInt(randomBytes(1)[0] as number);
      const ct = cipher.encrypt([a, b], nonce); // number[][], 2 blocks
      return ctx.methods
        .addTogether(
          offset,
          ct[0] as number[],
          ct[1] as number[],
          Array.from(publicKey),
          nonceBn(nonce),
        )
        .accountsPartial(queueAccounts(ctx, offset, "add_together"))
        .signers([ctx.payer])
        .rpc({ skipPreflight: true, commitment: "confirmed" });
    },
  };
}

/**
 * Runner for `apply_position_delta` against a specific Position PDA. Every
 * writer that targets the SAME PDA contends on one account, which is the
 * Pool-like serialization the contention benchmark measures.
 */
export function buildApplyDeltaRunner(
  ctx: BenchContext,
  positionPda: PublicKey,
): OpRunner {
  return {
    op: "apply_position_delta",
    async queue(offset: BN): Promise<string> {
      const { cipher, publicKey } = freshCipher(ctx.mxePublicKey);
      const nonce = randomBytes(16);
      // A small non-zero delta on free_sol; other fields unchanged.
      const delta: bigint[] = [1n, 0n, 0n, 0n, 0n];
      const ct = cipher.encrypt(delta, nonce); // number[][], 5 blocks
      return ctx.methods
        .applyPositionDelta(offset, ct, Array.from(publicKey), nonceBn(nonce))
        .accountsPartial({
          ...queueAccounts(ctx, offset, "apply_position_delta"),
          position: positionPda,
        })
        .signers([ctx.payer])
        .rpc({ skipPreflight: true, commitment: "confirmed" });
    },
  };
}

/** Initialize the shared Position once if it does not already exist. */
export async function ensurePosition(
  ctx: BenchContext,
  positionPda: PublicKey,
  payer: Keypair,
): Promise<void> {
  const info = await ctx.connection.getAccountInfo(positionPda);
  if (info) return;

  const offset = newOffset();
  const { cipher, publicKey } = freshCipher(ctx.mxePublicKey);
  const nonce = randomBytes(16);
  const zero: bigint[] = [0n, 0n, 0n, 0n, 0n];
  const ct = cipher.encrypt(zero, nonce);
  await ctx.methods
    .initPosition(offset, ct, Array.from(publicKey), nonceBn(nonce))
    .accountsPartial({
      ...queueAccounts(ctx, offset, "init_position"),
      position: positionPda,
    })
    .signers([payer])
    .rpc({ skipPreflight: true, commitment: "confirmed" });
  await awaitComputationFinalization(
    ctx.provider,
    offset,
    ctx.programId,
    "confirmed",
    ctx.options.finalizeTimeoutMs,
  );
}

/** Read fee + compute units from a confirmed transaction, with light retries. */
export async function readTxCost(
  ctx: BenchContext,
  signature: string,
): Promise<TxCost | null> {
  for (let attempt = 0; attempt < 5; attempt += 1) {
    try {
      const tx = await ctx.connection.getTransaction(signature, {
        commitment: "confirmed",
        maxSupportedTransactionVersion: 0,
      });
      if (tx?.meta) {
        return {
          feeLamports: tx.meta.fee,
          computeUnitsConsumed: tx.meta.computeUnitsConsumed ?? 0,
        };
      }
    } catch {
      // transient; retried below.
    }
    await new Promise((resolve) => setTimeout(resolve, 400));
  }
  return null;
}

/**
 * Drive one operation through its lifecycle with retry and abandonment.
 *
 * Each attempt uses a fresh computation offset (mirroring the design's "retry
 * installs a new offset"). On queue or finalization failure the attempt is
 * retried up to `maxAttempts`; if every attempt fails the operation is marked
 * `abandoned` (the client-side analogue of the Task 2.1 timeout abandonment,
 * since the test-only program carries no financial Lock enforcement). A single
 * failure never aborts the surrounding run.
 *
 * `measureBalance` must be false under concurrency: overlapping operations make
 * a per-operation balance delta meaningless.
 */
export async function runOp(
  ctx: BenchContext,
  runner: OpRunner,
  index: number,
  measureBalance: boolean,
): Promise<OpSample> {
  const { maxAttempts, finalizeTimeoutMs, reclaim } = ctx.options;
  const startedAtMs = Date.now();
  let firstError: string | null = null;
  let attempts = 0;

  const balanceBefore = measureBalance ? await safeBalance(ctx) : null;

  for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
    attempts = attempt;
    const offset = newOffset();
    const t0 = performance.now();
    try {
      const queueSignature = await runner.queue(offset);
      const finalizeSignature = await awaitComputationFinalization(
        ctx.provider,
        offset,
        ctx.programId,
        "confirmed",
        finalizeTimeoutMs,
      );
      const latencyMs = performance.now() - t0;

      const queueCost = await readTxCost(ctx, queueSignature);
      const callbackCost = await readTxCost(ctx, finalizeSignature);

      let totalUserLamports: number | null = null;
      let arciumFeePlusRentLamports: number | null = null;
      let reclaimedRentLamports: number | null = null;

      if (measureBalance && balanceBefore !== null) {
        const afterFinal = await safeBalance(ctx);
        if (afterFinal !== null) {
          totalUserLamports = balanceBefore - afterFinal;
          if (queueCost) {
            arciumFeePlusRentLamports =
              totalUserLamports - queueCost.feeLamports;
          }
        }
        if (reclaim) {
          reclaimedRentLamports = await tryReclaim(ctx, offset, afterFinal);
        }
      }

      return {
        index,
        ok: true,
        abandoned: false,
        attempts,
        retries: attempts - 1,
        latencyMs,
        startedAtMs,
        queueSignature,
        finalizeSignature,
        queueCu: queueCost?.computeUnitsConsumed ?? null,
        callbackCu: callbackCost?.computeUnitsConsumed ?? null,
        queueTxFeeLamports: queueCost?.feeLamports ?? null,
        callbackTxFeeLamports: callbackCost?.feeLamports ?? null,
        totalUserLamports,
        arciumFeePlusRentLamports,
        reclaimedRentLamports,
        error: firstError,
      };
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      if (firstError === null) firstError = message;
      // Brief backoff before the next attempt; keeps RPC pressure down.
      await new Promise((resolve) => setTimeout(resolve, 500 * attempt));
    }
  }

  return {
    index,
    ok: false,
    abandoned: true,
    attempts,
    retries: Math.max(0, attempts - 1),
    latencyMs: null,
    startedAtMs,
    queueSignature: null,
    finalizeSignature: null,
    queueCu: null,
    callbackCu: null,
    queueTxFeeLamports: null,
    callbackTxFeeLamports: null,
    totalUserLamports: null,
    arciumFeePlusRentLamports: null,
    reclaimedRentLamports: null,
    error: firstError,
  };
}

async function safeBalance(ctx: BenchContext): Promise<number | null> {
  try {
    return await ctx.connection.getBalance(ctx.payer.publicKey, "confirmed");
  } catch {
    return null;
  }
}

/** Reclaim finalized computation rent and report the net lamports returned. */
async function tryReclaim(
  ctx: BenchContext,
  offset: BN,
  balanceAfterFinal: number | null,
): Promise<number | null> {
  if (balanceAfterFinal === null) return null;
  try {
    await claimComputationRent(ctx.provider, ctx.clusterOffset, offset);
    const afterReclaim = await safeBalance(ctx);
    if (afterReclaim === null) return null;
    // Net of the reclaim transaction fee the payer just spent.
    return afterReclaim - balanceAfterFinal;
  } catch {
    return null;
  }
}
