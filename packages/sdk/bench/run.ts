// CLI entry point for the Task 2.6 benchmark harness.
//
// Usage (run from the repository root or this package):
//   node packages/sdk/bench/run.ts latency    [flags]
//   node packages/sdk/bench/run.ts contention [flags]
//
// Or via the wired npm scripts:
//   pnpm --filter @walnut/sdk bench:latency    -- [flags]
//   pnpm --filter @walnut/sdk bench:contention -- [flags]
//
// This is a MEASUREMENT TOOL, not production code. It sends real transactions
// to the configured cluster and spends real (dev)net SOL. See bench/README.md
// for the required environment, how to fund a devnet keypair, the exact
// commands, and the JSON artifact schema consumed by ADR-0003.

import { createContext, resolveOptions } from "./config.ts";
import { runLatencyBenchmark } from "./latency.ts";
import { runContentionBenchmark } from "./contention.ts";

async function main(): Promise<void> {
  const options = resolveOptions(process.argv.slice(2));
  const ctx = await createContext(options);

  console.log(
    `Cluster: ${options.rpcUrl} (offset ${ctx.clusterOffset}) ` +
      `wallet ${ctx.payer.publicKey.toBase58()}`,
  );

  if (options.command === "latency") {
    await runLatencyBenchmark(ctx);
  } else {
    await runContentionBenchmark(ctx);
  }
}

main()
  .then(() => {
    // Open websocket subscriptions keep Node alive after the run; exit explicitly.
    process.exit(0);
  })
  .catch((err: unknown) => {
    const message =
      err instanceof Error ? (err.stack ?? err.message) : String(err);
    console.error(`Benchmark failed: ${message}`);
    process.exit(1);
  });
