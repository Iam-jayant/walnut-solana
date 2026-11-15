// Contention benchmark: concurrent and sustained writers on one shared account.
//
// Two phases, both targeting a SINGLE shared Position PDA so every writer
// contends on one account -- the Pool-like serialization the design describes
// (every Pool-financial mutation locks the Pool). Each operation carries the
// retry / abandonment lifecycle from measure.ts, so the harness records how
// many operations succeeded, how many needed retries, and how many were
// abandoned under load, plus latency under contention and accepted throughput.
//
//   Phase 1 (burst):      `concurrency` writers (default 10) fired at once.
//   Phase 2 (sustained):  operations kept in flight (up to `concurrency`) for
//                         `durationSec`, modeling sustained Pool contention.
//
// Balance-delta fee measurement is disabled here because overlapping operations
// make a per-operation payer balance delta meaningless; use the latency
// benchmark for the fee breakdown.

import {
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

async function burst(
  ctx: BenchContext,
  runner: OpRunner,
  concurrency: number,
  startIndex: number,
): Promise<OpSample[]> {
  const tasks: Promise<OpSample>[] = [];
  for (let i = 0; i < concurrency; i += 1) {
    tasks.push(runOp(ctx, runner, startIndex + i, false));
  }
  return Promise.all(tasks);
}

async function sustained(
  ctx: BenchContext,
  runner: OpRunner,
  concurrency: number,
  durationSec: number,
  startIndex: number,
): Promise<OpSample[]> {
  const deadline = performance.now() + durationSec * 1000;
  const samples: OpSample[] = [];
  let next = startIndex;

  // A fixed pool of workers, each pulling the next operation until the window
  // closes. This keeps up to `concurrency` operations in flight at all times.
  async function worker(): Promise<void> {
    while (performance.now() < deadline) {
      const index = next;
      next += 1;
      const sample = await runOp(ctx, runner, index, false);
      samples.push(sample);
    }
  }

  const workers: Promise<void>[] = [];
  for (let i = 0; i < concurrency; i += 1) workers.push(worker());
  await Promise.all(workers);
  return samples;
}

export async function runContentionBenchmark(ctx: BenchContext): Promise<void> {
  await ensureCompDefsForOp(ctx);

  const positionPda = derivePositionPda(ctx.programId, ctx.payer.publicKey);
  await ensurePosition(ctx, positionPda, ctx.payer);
  const runner = buildApplyDeltaRunner(ctx, positionPda);

  const concurrency = Math.max(1, ctx.options.concurrency);
  const durationSec = Math.max(0, ctx.options.durationSec);

  console.log(
    `Contention phase 1: ${concurrency} concurrent writers on one account...`,
  );
  const startMs = performance.now();
  const burstSamples = await burst(ctx, runner, concurrency, 0);

  let sustainedSamples: OpSample[] = [];
  if (durationSec > 0) {
    console.log(
      `Contention phase 2: sustained load (${concurrency} in flight) ` +
        `for ${durationSec}s...`,
    );
    sustainedSamples = await sustained(
      ctx,
      runner,
      concurrency,
      durationSec,
      burstSamples.length,
    );
  }
  const wallClockSec = (performance.now() - startMs) / 1000;

  const samples = [...burstSamples, ...sustainedSamples];
  const artifact = buildArtifact(ctx, "contention", samples, wallClockSec);
  printArtifact(artifact);
  const file = writeArtifact(artifact, ctx.options.outDir);
  console.log(`JSON artifact: ${file}`);
}
