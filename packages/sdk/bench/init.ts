// Standalone one-time comp-def provisioning for the configured cluster.
//
// `arcium deploy` creates and initializes the MXE account, but the computation
// definitions the benchmarks exercise (add_together, init_position, and
// apply_position_delta) still have to be registered and their circuits uploaded
// once per deployment. On a shared devnet deployment it is cleaner to do that
// here, once, and then run the benchmarks with --no-init-compdefs rather than
// letting every benchmark process re-check the definitions.
//
// This reuses the same resolution (config.ts) and skip-if-exists registration
// (program.ts `ensureCompDef`) as the inline setup.ts path, so it provisions
// exactly the definitions the benchmarked ops need and is fully idempotent:
// already-provisioned definitions are reported as "exists" and left untouched.
//
// Usage (devnet is the default RPC + Arcium.toml [clusters.devnet] offset 456):
//   pnpm --filter @walnut/sdk bench:init-devnet
//   pnpm --filter @walnut/sdk bench:init-devnet -- --rpc http://127.0.0.1:8899 --cluster-offset 0
//
// Building the context fetches the MXE x25519 public key (with retries), so a
// successful run also confirms the MXE keygen has finalized on the cluster.

import { createContext, resolveOptions } from "./config.ts";
import { ensureCompDef } from "./program.ts";

/** Every comp def the latency + contention benchmarks can exercise. */
function compDefs(
  ctx: Awaited<ReturnType<typeof createContext>>,
): Array<[string, Parameters<typeof ensureCompDef>[4]]> {
  return [
    ["add_together", ctx.methods.initAddTogetherCompDef()],
    ["init_position", ctx.methods.initPositionCompDef()],
    ["apply_position_delta", ctx.methods.applyPositionDeltaCompDef()],
  ];
}

async function main(): Promise<void> {
  const options = resolveOptions(process.argv.slice(2));
  const ctx = await createContext(options);

  console.log(
    `Provisioning comp defs on ${options.rpcUrl} (offset ${ctx.clusterOffset}) ` +
      `program ${ctx.programId.toBase58()}`,
  );
  console.log("MXE public key fetched: keygen has finalized on this cluster.");

  for (const [name, builder] of compDefs(ctx)) {
    const state = await ensureCompDef(
      ctx.provider,
      ctx.programId,
      ctx.repoRoot,
      name,
      builder,
      ctx.payer,
    );
    console.log(`  comp def ${name}: ${state}`);
  }

  console.log(
    "Done. Run the benchmarks with --no-init-compdefs to skip re-checking.",
  );
}

main().catch((err: unknown) => {
  const message =
    err instanceof Error ? (err.stack ?? err.message) : String(err);
  console.error(`Comp-def init failed: ${message}`);
  process.exit(1);
});
