// Aggregation and output for the Task 2.6 benchmark harness.
//
// Produces BOTH a human-readable console table and a machine-readable JSON
// artifact. The JSON is the authoritative input for ADR-0003 (Task 2.7): it
// carries the run metadata, latency percentiles, callback/queue compute units,
// the fee breakdown, failure rate, throughput, the contention outcome counts,
// and every raw sample so any statistic can be re-derived later.

import { LAMPORTS_PER_SOL } from "@solana/web3.js";
import * as fs from "node:fs";
import * as path from "node:path";
import { summarize, round, type Summary } from "./stats.ts";
import type {
  BenchArtifact,
  BenchKind,
  FeeBreakdown,
  OpSample,
  OutcomeCounts,
} from "./types.ts";
import type { BenchContext } from "./config.ts";

function nonNull(values: (number | null)[]): number[] {
  return values.filter((v): v is number => v !== null);
}

function okSamples(samples: OpSample[]): OpSample[] {
  return samples.filter((s) => s.ok);
}

function buildFeeBreakdown(samples: OpSample[]): FeeBreakdown {
  const ok = okSamples(samples);
  const queueTxFee = summarize(nonNull(ok.map((s) => s.queueTxFeeLamports)));
  const callbackTxFee = summarize(
    nonNull(ok.map((s) => s.callbackTxFeeLamports)),
  );
  const arciumFeePlusRent = summarize(
    nonNull(ok.map((s) => s.arciumFeePlusRentLamports)),
  );
  const totalUserCost = summarize(nonNull(ok.map((s) => s.totalUserLamports)));
  const toSol = (lamports: number): number =>
    round(lamports / LAMPORTS_PER_SOL, 9);
  return {
    samples: ok.length,
    queueTxFee,
    callbackTxFee,
    arciumFeePlusRent,
    totalUserCost,
    meanQueueTxFeeSol: toSol(queueTxFee.mean),
    meanArciumFeePlusRentSol: toSol(arciumFeePlusRent.mean),
    meanTotalUserCostSol: toSol(totalUserCost.mean),
  };
}

function buildOutcomes(samples: OpSample[]): OutcomeCounts {
  const total = samples.length;
  const ok = samples.filter((s) => s.ok).length;
  const abandoned = samples.filter((s) => s.abandoned).length;
  const retried = samples.filter((s) => s.ok && s.retries > 0).length;
  const totalRetries = samples.reduce((acc, s) => acc + s.retries, 0);
  return {
    total,
    ok,
    abandoned,
    retried,
    totalRetries,
    failureRate: total === 0 ? 0 : round(abandoned / total, 4),
  };
}

/** Assemble the full artifact from the collected samples. */
export function buildArtifact(
  ctx: BenchContext,
  kind: BenchKind,
  samples: OpSample[],
  wallClockSec: number,
): BenchArtifact {
  const ok = okSamples(samples);
  const latencyMs = summarize(nonNull(ok.map((s) => s.latencyMs)));
  const callbackCu = summarize(nonNull(ok.map((s) => s.callbackCu)));
  const queueCu = summarize(nonNull(ok.map((s) => s.queueCu)));
  const throughputPerMin =
    wallClockSec > 0 ? round(ok.length / (wallClockSec / 60), 2) : 0;

  return {
    meta: {
      kind,
      op: ctx.options.op,
      timestamp: new Date().toISOString(),
      rpcUrl: ctx.options.rpcUrl,
      clusterOffset: ctx.clusterOffset,
      programId: ctx.programId.toBase58(),
      walletPubkey: ctx.payer.publicKey.toBase58(),
      params: {
        runs: ctx.options.runs,
        concurrency: ctx.options.concurrency,
        durationSec: ctx.options.durationSec,
        maxAttempts: ctx.options.maxAttempts,
        finalizeTimeoutMs: ctx.options.finalizeTimeoutMs,
        reclaim: ctx.options.reclaim,
      },
    },
    wallClockSec: round(wallClockSec, 2),
    throughputPerMin,
    latencyMs,
    callbackCu,
    queueCu,
    fees: buildFeeBreakdown(samples),
    outcomes: buildOutcomes(samples),
    samples,
  };
}

function pad(value: string, width: number): string {
  return value.length >= width
    ? value
    : value + " ".repeat(width - value.length);
}

function fmt(value: number): string {
  if (!Number.isFinite(value)) return "n/a";
  return round(value, 2).toLocaleString("en-US");
}

function summaryRow(label: string, s: Summary): string {
  return [
    pad(label, 16),
    pad(fmt(s.p50), 12),
    pad(fmt(s.p95), 12),
    pad(fmt(s.p99), 12),
    pad(fmt(s.mean), 12),
    pad(fmt(s.min), 12),
    pad(fmt(s.max), 12),
  ].join("  ");
}

/** Print the human-readable table to the console. */
export function printArtifact(a: BenchArtifact): void {
  const { meta, outcomes } = a;
  console.log("");
  console.log(
    `Walnut benchmark: ${meta.kind} / ${meta.op} ` +
      `(cluster offset ${meta.clusterOffset})`,
  );
  console.log(`RPC: ${meta.rpcUrl}`);
  console.log(`Program: ${meta.programId}`);
  console.log(
    `Params: runs=${meta.params.runs} concurrency=${meta.params.concurrency} ` +
      `duration=${meta.params.durationSec}s maxAttempts=${meta.params.maxAttempts} ` +
      `reclaim=${meta.params.reclaim}`,
  );
  console.log("");

  const header = [
    pad("metric", 16),
    pad("p50", 12),
    pad("p95", 12),
    pad("p99", 12),
    pad("mean", 12),
    pad("min", 12),
    pad("max", 12),
  ].join("  ");
  console.log(header);
  console.log("-".repeat(header.length));
  console.log(summaryRow("latency (ms)", a.latencyMs));
  console.log(summaryRow("callback CU", a.callbackCu));
  console.log(summaryRow("queue CU", a.queueCu));
  console.log(summaryRow("queue fee (lam)", a.fees.queueTxFee));
  console.log(summaryRow("arc fee+rent", a.fees.arciumFeePlusRent));
  console.log(summaryRow("total cost", a.fees.totalUserCost));
  console.log("");

  console.log(
    `Throughput: ${a.throughputPerMin} accepted ops/min ` +
      `over ${a.wallClockSec}s wall clock`,
  );
  console.log(
    `Outcomes: total=${outcomes.total} ok=${outcomes.ok} ` +
      `abandoned=${outcomes.abandoned} retried=${outcomes.retried} ` +
      `totalRetries=${outcomes.totalRetries} ` +
      `failureRate=${(outcomes.failureRate * 100).toFixed(2)}%`,
  );
  console.log(
    `Mean fees (SOL): queue=${a.fees.meanQueueTxFeeSol} ` +
      `arciumFee+rent=${a.fees.meanArciumFeePlusRentSol} ` +
      `total=${a.fees.meanTotalUserCostSol}`,
  );
  console.log("");
}

/** Write the JSON artifact and return its absolute path. */
export function writeArtifact(a: BenchArtifact, outDir: string): string {
  fs.mkdirSync(outDir, { recursive: true });
  const stamp = a.meta.timestamp.replace(/[:.]/g, "-");
  const file = path.join(outDir, `${a.meta.kind}-${a.meta.op}-${stamp}.json`);
  fs.writeFileSync(file, `${JSON.stringify(a, null, 2)}\n`, "utf8");
  return file;
}
