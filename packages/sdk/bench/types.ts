// Shared types and the machine-readable JSON artifact schema for the Task 2.6
// benchmark harness. The artifact written by report.ts is the authoritative
// input for ADR-0003 (Task 2.7): run count, percentiles, raw samples, fee
// breakdown, compute units, failure rate, throughput, and contention outcomes.

import type { Summary } from "./stats.ts";

/** Which benchmark produced an artifact. */
export type BenchKind = "latency" | "contention";

/** The computation exercised by a run. */
export type BenchOp = "add_together" | "apply_position_delta";

/** Cost + compute reading pulled from a confirmed transaction's metadata. */
export interface TxCost {
  feeLamports: number;
  computeUnitsConsumed: number;
}

/** One operation's full lifecycle result (queue -> callback finalization). */
export interface OpSample {
  /** Zero-based index within the run. */
  index: number;
  /** True when the callback finalized within the per-attempt timeout. */
  ok: boolean;
  /** True when every attempt failed and the operation was abandoned. */
  abandoned: boolean;
  /** Total attempts made (1 = succeeded first try). */
  attempts: number;
  /** attempts - 1. */
  retries: number;
  /** Wall-clock queue->finalization latency of the successful attempt, ms. */
  latencyMs: number | null;
  /** Epoch-ms the operation started (first attempt), for throughput windows. */
  startedAtMs: number;
  queueSignature: string | null;
  finalizeSignature: string | null;
  /** Compute units consumed by the queue transaction (user-paid). */
  queueCu: number | null;
  /** Compute units actually consumed by the callback transaction. */
  callbackCu: number | null;
  /** Solana fee of the queue transaction, lamports (user-paid). */
  queueTxFeeLamports: number | null;
  /** Solana fee of the callback transaction, lamports (node-paid; informational). */
  callbackTxFeeLamports: number | null;
  /** Payer balance delta across queue->finalization (sequential runs only). */
  totalUserLamports: number | null;
  /** totalUserLamports - queueTxFeeLamports: Arcium computation fee + comp rent. */
  arciumFeePlusRentLamports: number | null;
  /** Net lamports returned by claimComputationRent, if --reclaim was set. */
  reclaimedRentLamports: number | null;
  /** First error message observed across attempts, if any. */
  error: string | null;
}

/** Aggregated fee breakdown across the successful samples of a run. */
export interface FeeBreakdown {
  samples: number;
  queueTxFee: Summary;
  callbackTxFee: Summary;
  arciumFeePlusRent: Summary;
  totalUserCost: Summary;
  /** Means expressed in SOL for human reading. */
  meanQueueTxFeeSol: number;
  meanArciumFeePlusRentSol: number;
  meanTotalUserCostSol: number;
}

/** Counts of how operations resolved, for failure-rate and contention stats. */
export interface OutcomeCounts {
  total: number;
  ok: number;
  abandoned: number;
  /** Operations that needed at least one retry but eventually succeeded. */
  retried: number;
  /** Sum of retries across all operations. */
  totalRetries: number;
  /** abandoned / total. */
  failureRate: number;
}

/** Context describing where and how the run executed. */
export interface BenchMeta {
  kind: BenchKind;
  op: BenchOp;
  timestamp: string;
  rpcUrl: string;
  clusterOffset: number;
  programId: string;
  walletPubkey: string;
  /** Parameters as resolved from CLI flags / env / defaults. */
  params: {
    runs: number;
    concurrency: number;
    durationSec: number;
    maxAttempts: number;
    finalizeTimeoutMs: number;
    reclaim: boolean;
  };
}

/** The full artifact written to disk and summarized on the console. */
export interface BenchArtifact {
  meta: BenchMeta;
  /** Total wall-clock seconds the run took. */
  wallClockSec: number;
  /** Accepted (ok) operations per minute over the wall-clock window. */
  throughputPerMin: number;
  latencyMs: Summary;
  callbackCu: Summary;
  queueCu: Summary;
  fees: FeeBreakdown;
  outcomes: OutcomeCounts;
  /** Every raw sample, so ADR-0003 can re-derive any statistic. */
  samples: OpSample[];
}
