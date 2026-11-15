// One-time per-cluster setup: make sure the computation definitions the chosen
// operation needs are registered and their circuits uploaded. Mirrors the
// skip-if-exists pattern in the Task 2.2/2.4 tests. Controlled by
// --init-compdefs (default on); pass --no-init-compdefs on a cluster where the
// definitions are already provisioned (e.g. a shared devnet deployment).

import { ensureCompDef } from "./program.ts";
import type { BenchContext } from "./config.ts";

export async function ensureCompDefsForOp(ctx: BenchContext): Promise<void> {
  if (!ctx.options.initCompDefs) {
    console.log("Skipping comp-def setup (--no-init-compdefs).");
    return;
  }

  const register = async (
    name: string,
    initBuilder: Parameters<typeof ensureCompDef>[4],
  ): Promise<void> => {
    const state = await ensureCompDef(
      ctx.provider,
      ctx.programId,
      ctx.repoRoot,
      name,
      initBuilder,
      ctx.payer,
    );
    console.log(`  comp def ${name}: ${state}`);
  };

  console.log("Ensuring computation definitions...");
  if (ctx.options.op === "apply_position_delta") {
    await register("init_position", ctx.methods.initPositionCompDef());
    await register(
      "apply_position_delta",
      ctx.methods.applyPositionDeltaCompDef(),
    );
  } else {
    await register("add_together", ctx.methods.initAddTogetherCompDef());
  }
}
