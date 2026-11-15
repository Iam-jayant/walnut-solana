// Latency, fee, compute-unit, failure-rate, and throughput benchmark.
//
// Runs at least `runs` operations SEQUENTIALLY (default 50) so each queue ->
// callback-finalization latency is measured without self-contention, and so
// payer balance deltas give a clean per-operation fee breakdown. Emits p50/p95
// (and p99) latency, callback CU actually consumed, the fee breakdown, the
// failure rate, and accepted operations per minute.

import {
  buildAddTogetherRunner,
  buildApplyDeltaRunner,
  derivePositionPda,
  ensurePosition,
  runOp,
  type OpRunner,
} from "./measure.ts";
import { ensureCompDefsForOp } from "./setup.ts";
import { buildArtifact, printArtifact, writeArtifact } from "./report.ts";
import type { BenchContext } from "./config.ts";
import type { OpSample } from "./types.ts";

export async function runLatencyBenchmark(ctx: BenchContext): Promise<void> {
  await ensureCompDefsForOp(ctx);

  let runner: OpRunner;
  if (ctx.options.op === "apply_position_delta") {
    const positionPda = derivePositionPda(ctx.programId, ctx.payer.publicKey);
    await ensurePosition(ctx, positionPda, ctx.payer);
    runner = buildApplyDeltaRunner(ctx, positionPda);
  } else {
    runner = buildAddTogetherRunner(ctx);
  }

  const runs = Math.max(1, ctx.options.runs);
  console.log(
    `Running ${runs} sequential ${ctx.options.op} operations for latency...`,
  );

  const samples: OpSample[] = [];
  const startMs = performance.now();
  for (let i = 0; i < runs; i += 1) {
    // Sequential => measureBalance = true for a clean fee breakdown.
    const sample = await runOp(ctx, runner, i, true);
    samples.push(sample);
    const tag = sample.ok
      ? `ok ${Math.round(sample.latencyMs ?? 0)}ms (attempts ${sample.attempts})`
      : `ABANDONED after ${sample.attempts} attempts`;
    console.log(`  [${i + 1}/${runs}] ${tag}`);
  }
  const wallClockSec = (performance.now() - startMs) / 1000;

  const artifact = buildArtifact(ctx, "latency", samples, wallClockSec);
  printArtifact(artifact);
  const file = writeArtifact(artifact, ctx.options.outDir);
  console.log(`JSON artifact: ${file}`);
}
