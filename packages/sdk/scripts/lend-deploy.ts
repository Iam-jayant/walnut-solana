// Helper for scripts/lend-devnet.sh (walnut-lend task 6.1). Run from the repo
// root by the shell script, with ANCHOR_PROVIDER_URL, ANCHOR_WALLET and
// WALNUT_BENCH_CLUSTER_OFFSET set. Never prints the RPC URL or any key.
//
//   node packages/sdk/scripts/lend-deploy.ts init-pool
//       Calls init_lending_pool once (idempotent: prints "exists" when the
//       LendingPool PDA is already on chain).
//   node packages/sdk/scripts/lend-deploy.ts comp-defs status
//       Prints "<circuit> absent|pending|finalized" per new circuit.
//   node packages/sdk/scripts/lend-deploy.ts comp-defs run
//       Registers and uploads every circuit that is not finalized
//       (ensureCompDef resumes interrupted uploads).
//
// Names and accounts match target/idl/walnut.json: `init_lending_pool(
// max_price_age_seconds, oracle, feed_id)` with accounts admin, market,
// lending_pool, usdc_vault, system_program.

import {
  getArciumProgram,
  getCircuitState,
  getCompDefAccAddress,
} from "@arcium-hq/client";
import { PublicKey } from "@solana/web3.js";
import { createContext, resolveOptions } from "../bench/config.ts";
import type { BenchContext } from "../bench/config.ts";
import { ensureCompDef, type InitCompDefBuilder } from "../bench/program.ts";
import { PYTH_FEED_IDS, LAUNCH_ORACLE_PARAMS } from "../src/oracle.ts";
import { compDefOffsetOf } from "../src/pda.ts";

/** Circuits registered by this deploy, with the IDL initializer of each. */
const NEW_CIRCUITS: ReadonlyArray<readonly [circuit: string, init: string]> = [
  ["lend_op", "initLendOpCompDef"],
  ["disclose_position", "initDisclosePositionCompDef"],
  ["rotate_position_key", "initRotatePositionKeyCompDef"],
];

/** Pyth push-oracle program that owns the shard-0 sponsored feed PDAs. */
const PUSH_ORACLE_PROGRAM_ID = new PublicKey(
  "pythWSnswVUd12oZpeFP8e9CVaEqJg25g1Vtc2biRsT",
);

function redact(text: string): string {
  return text
    .replace(/api-key=[^&\s"')]+/gi, "api-key=REDACTED")
    .replace(/https?:\/\/[^\s"')]+/gi, "<url>");
}

function hexToBytes(hex: string): Uint8Array {
  return Uint8Array.from(Buffer.from(hex, "hex"));
}

type Json = Record<string, unknown>;
interface IdlAccountItem {
  name: string;
  address?: string;
  pda?: unknown;
}
interface IdlArgItem {
  name: string;
  type: unknown;
}
interface IdlInstructionItem {
  name: string;
  accounts: IdlAccountItem[];
  args: IdlArgItem[];
}
interface MethodBuilder {
  accountsPartial(accounts: Record<string, PublicKey>): MethodBuilder;
  rpc(opts?: Json): Promise<string>;
}

// ---------------------------------------------------------------------------
// init_lending_pool
// ---------------------------------------------------------------------------

/**
 * Instruction call for `init_lending_pool(max_price_age_seconds, oracle,
 * feed_id)`. Accounts: admin (deployer), market and lending_pool (PDAs) and
 * usdc_vault (from the Market account); system_program resolves from the IDL.
 */
async function initLendingPool(ctx: BenchContext): Promise<void> {
  const seed = (s: string): Uint8Array => new TextEncoder().encode(s);
  const market = PublicKey.findProgramAddressSync(
    [seed("market")],
    ctx.programId,
  )[0];
  const lendingPool = PublicKey.findProgramAddressSync(
    [seed("lending_pool"), market.toBytes()],
    ctx.programId,
  )[0];

  const existing = await ctx.connection.getAccountInfo(
    lendingPool,
    "confirmed",
  );
  // LEND_UPDATE_ORACLE=1 re-sends init_lending_pool to an existing pool; the
  // program then updates only the oracle account, feed id and max price age.
  const update = process.env["LEND_UPDATE_ORACLE"] === "1";
  if (existing !== null && !update) {
    console.log("init_lending_pool: exists");
    return;
  }

  const idl = ctx.program.idl as unknown as {
    instructions: IdlInstructionItem[];
  };
  const ix = idl.instructions.find((i) => i.name === "initLendingPool");
  if (ix === undefined) {
    throw new Error(
      "IDL has no instruction initLendingPool (see target/idl/walnut.json)",
    );
  }

  const marketAcc = (await (
    ctx.program.account as unknown as Record<
      string,
      { fetch(a: PublicKey): Promise<Json> }
    >
  )["market"]?.fetch(market)) as Record<string, PublicKey> | undefined;
  if (marketAcc === undefined) throw new Error("Market account not found");

  const oracle = new PublicKey(
    process.env["LEND_ORACLE"] ??
      PublicKey.findProgramAddressSync(
        [
          Buffer.from([0, 0]),
          Buffer.from(hexToBytes(PYTH_FEED_IDS["SOL/USD"])),
        ],
        PUSH_ORACLE_PROGRAM_ID,
      )[0].toBase58(),
  );
  const feedId = Array.from(hexToBytes(PYTH_FEED_IDS["SOL/USD"]));
  const maxAge = Number(
    process.env["LEND_MAX_PRICE_AGE"] ?? LAUNCH_ORACLE_PARAMS.maxAgeSeconds,
  );

  const known: Record<string, PublicKey | undefined> = {
    admin: ctx.payer.publicKey,
    market,
    lendingPool,
    usdcVault: marketAcc["usdcVault"],
  };
  const accounts: Record<string, PublicKey> = {};
  const missing: string[] = [];
  for (const acc of ix.accounts) {
    const value = known[acc.name];
    if (value !== undefined) accounts[acc.name] = value;
    else if (acc.address === undefined && acc.pda === undefined) {
      missing.push(acc.name);
    }
  }
  if (missing.length > 0) {
    throw new Error(
      `init_lending_pool needs accounts not mapped in lend-deploy.ts: ${missing.join(", ")}`,
    );
  }

  const args: unknown[] = [maxAge, oracle, feedId];
  const methods = ctx.program.methods as unknown as Record<
    string,
    ((...a: unknown[]) => MethodBuilder) | undefined
  >;
  const build = methods["initLendingPool"];
  if (build === undefined) throw new Error("IDL has no method initLendingPool");
  const sig = await build
    .call(ctx.program.methods, ...args)
    .accountsPartial(accounts)
    .rpc({ commitment: "confirmed", preflightCommitment: "confirmed" });
  console.log(
    `init_lending_pool: ${existing === null ? "created" : `updated, max price age ${maxAge} s`} (${sig})`,
  );

  const after = await ctx.connection.getAccountInfo(lendingPool, "confirmed");
  if (after === null) throw new Error("LendingPool account missing after init");
}

// ---------------------------------------------------------------------------
// comp defs
// ---------------------------------------------------------------------------

type CompDefState = "absent" | "pending" | "finalized";

async function compDefState(
  ctx: BenchContext,
  circuit: string,
): Promise<CompDefState> {
  const address = getCompDefAccAddress(ctx.programId, compDefOffsetOf(circuit));
  const arcium = getArciumProgram(ctx.provider);
  const acc = (await (
    arcium.account as unknown as Record<
      string,
      {
        fetchNullable(a: PublicKey): Promise<{ circuitSource: unknown } | null>;
      }
    >
  )["computationDefinitionAccount"]?.fetchNullable(address)) as
    { circuitSource: Parameters<typeof getCircuitState>[0] } | null | undefined;
  if (acc === null || acc === undefined) return "absent";
  return getCircuitState(acc.circuitSource) === "OnchainFinalized"
    ? "finalized"
    : "pending";
}

async function compDefs(ctx: BenchContext, mode: string): Promise<void> {
  for (const [circuit, initName] of NEW_CIRCUITS) {
    const state = await compDefState(ctx, circuit);
    if (mode === "status") {
      console.log(`${circuit} ${state}`);
      continue;
    }
    if (state === "finalized") {
      console.log(`comp def ${circuit}: finalized, skipping`);
      continue;
    }
    const methods = ctx.program.methods as unknown as Record<
      string,
      (() => InitCompDefBuilder) | undefined
    >;
    const init = methods[initName];
    if (init === undefined) {
      throw new Error(
        `IDL has no instruction ${initName} (see target/idl/walnut.json)`,
      );
    }
    const result = await ensureCompDef(
      ctx.provider,
      ctx.programId,
      ctx.repoRoot,
      circuit,
      init.call(ctx.program.methods),
      ctx.payer,
    );
    console.log(`comp def ${circuit}: ${result}`);
  }
}

async function main(): Promise<void> {
  const [command, mode] = process.argv.slice(2);
  const ctx = await createContext(resolveOptions([]));
  if (command === "init-pool") await initLendingPool(ctx);
  else if (command === "comp-defs" && (mode === "status" || mode === "run")) {
    await compDefs(ctx, mode);
  } else {
    throw new Error("usage: lend-deploy.ts init-pool | comp-defs status|run");
  }
}

main()
  .then(() => process.exit(0))
  .catch((err: unknown) => {
    const message = err instanceof Error ? err.message : String(err);
    console.error(redact(`lend-deploy failed: ${message}`));
    process.exit(1);
  });
