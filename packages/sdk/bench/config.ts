// Configuration, CLI/env resolution, and runtime context for the Task 2.6
// benchmark harness.
//
// The harness runs against a configurable cluster: public devnet by default,
// or any RPC (including localnet) via env / flags. It reads the RPC URL and
// wallet the same way `arcium test --cluster devnet` does -- from the Anchor
// provider environment (ANCHOR_PROVIDER_URL, ANCHOR_WALLET) -- and the Arcium
// cluster offset from the environment or Arcium.toml. No private endpoint or
// key is ever hard-coded; the only built-in default is the FREE public devnet
// RPC and the standard Solana CLI keypair path.

import * as anchor from "../src/anchor-shim.ts";
import {
  Connection,
  Keypair,
  PublicKey,
  type Commitment,
} from "@solana/web3.js";
import { getArciumEnv, getMXEPublicKey } from "@arcium-hq/client";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import type { BenchKind, BenchOp } from "./types.ts";
import { loadWalnutIdl, makeProgram, type BenchMethods } from "./program.ts";

/** Free, keyless public devnet RPC. Used only when no RPC is configured. */
export const DEFAULT_DEVNET_RPC = "https://api.devnet.solana.com";
const COMMITMENT: Commitment = "confirmed";

/** Fully resolved run options. */
export interface BenchOptions {
  command: BenchKind;
  op: BenchOp;
  runs: number;
  concurrency: number;
  durationSec: number;
  maxAttempts: number;
  finalizeTimeoutMs: number;
  reclaim: boolean;
  initCompDefs: boolean;
  outDir: string;
  rpcUrl: string;
  walletPath: string;
  clusterOffset: number;
}

/** Everything a benchmark needs to run. */
export interface BenchContext {
  options: BenchOptions;
  repoRoot: string;
  provider: anchor.AnchorProvider;
  connection: Connection;
  payer: Keypair;
  program: anchor.Program;
  programId: PublicKey;
  methods: BenchMethods;
  clusterOffset: number;
  mxePublicKey: Uint8Array;
}

/** The directory this file lives in, used to locate the repository root. */
function moduleDir(): string {
  return path.dirname(fileURLToPath(import.meta.url));
}

/** packages/sdk/bench -> repository root. */
export function repoRoot(): string {
  return path.resolve(moduleDir(), "..", "..", "..");
}

interface RawFlags {
  positional: string[];
  values: Map<string, string>;
  bools: Set<string>;
}

/** Split argv into positional arguments, `--k v` values, and `--flag` bools. */
function parseArgs(argv: string[]): RawFlags {
  const positional: string[] = [];
  const values = new Map<string, string>();
  const bools = new Set<string>();
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i] as string;
    if (!arg.startsWith("--")) {
      positional.push(arg);
      continue;
    }
    const key = arg.slice(2);
    const eq = key.indexOf("=");
    if (eq >= 0) {
      values.set(key.slice(0, eq), key.slice(eq + 1));
      continue;
    }
    const next = argv[i + 1];
    if (next !== undefined && !next.startsWith("--")) {
      values.set(key, next);
      i += 1;
    } else {
      bools.add(key);
    }
  }
  return { positional, values, bools };
}

function intFlag(
  flags: RawFlags,
  name: string,
  envName: string,
  fallback: number,
): number {
  const raw = flags.values.get(name) ?? process.env[envName];
  if (raw === undefined) return fallback;
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed) || parsed < 0) {
    throw new Error(`Invalid value for --${name}: ${raw}`);
  }
  return parsed;
}

function stringFlag(
  flags: RawFlags,
  name: string,
  envName: string | undefined,
  fallback: string,
): string {
  const env = envName ? process.env[envName] : undefined;
  return flags.values.get(name) ?? env ?? fallback;
}

function boolFlag(
  flags: RawFlags,
  name: string,
  negated: string,
  fallback: boolean,
): boolean {
  if (flags.bools.has(name)) return true;
  if (flags.bools.has(negated)) return false;
  return fallback;
}

/** Best-effort extraction of `offset = N` from a `[clusters.<network>]` block. */
function clusterOffsetFromToml(
  tomlPath: string,
  network: string,
): number | null {
  if (!fs.existsSync(tomlPath)) return null;
  const text = fs.readFileSync(tomlPath, "utf8");
  const section = new RegExp(
    `\\[clusters\\.${network}\\][^\\[]*?offset\\s*=\\s*(\\d+)`,
    "s",
  );
  const match = section.exec(text);
  if (!match) return null;
  return Number.parseInt(match[1] as string, 10);
}

/**
 * Resolve the Arcium cluster offset with this precedence:
 *   1. --cluster-offset flag
 *   2. WALNUT_BENCH_CLUSTER_OFFSET / ARCIUM_CLUSTER_OFFSET env
 *   3. getArciumEnv() (set by `arcium test`)
 *   4. Arcium.toml [clusters.<network>] offset
 *   5. 0
 */
function resolveClusterOffset(
  flags: RawFlags,
  rpcUrl: string,
  root: string,
): number {
  const flag = flags.values.get("cluster-offset");
  if (flag !== undefined) return Number.parseInt(flag, 10);

  const env =
    process.env["WALNUT_BENCH_CLUSTER_OFFSET"] ??
    process.env["ARCIUM_CLUSTER_OFFSET"];
  if (env !== undefined) return Number.parseInt(env, 10);

  try {
    return getArciumEnv().arciumClusterOffset;
  } catch {
    // getArciumEnv throws outside an `arcium test` context; fall through.
  }

  const network = rpcUrl.includes("devnet")
    ? "devnet"
    : rpcUrl.includes("mainnet")
      ? "mainnet"
      : "localnet";
  const fromToml = clusterOffsetFromToml(
    path.join(root, "Arcium.toml"),
    network,
  );
  return fromToml ?? 0;
}

function loadKeypair(walletPath: string): Keypair {
  const resolved = walletPath.startsWith("~")
    ? path.join(os.homedir(), walletPath.slice(1))
    : walletPath;
  if (!fs.existsSync(resolved)) {
    throw new Error(
      `Wallet keypair not found at ${resolved}. Set ANCHOR_WALLET or ` +
        `--wallet to a funded devnet keypair (see bench/README.md).`,
    );
  }
  const secret = JSON.parse(fs.readFileSync(resolved, "utf8")) as number[];
  return Keypair.fromSecretKey(new Uint8Array(secret));
}

async function getMxePublicKeyWithRetry(
  provider: anchor.AnchorProvider,
  programId: PublicKey,
  maxRetries = 20,
  retryDelayMs = 500,
): Promise<Uint8Array> {
  for (let attempt = 1; attempt <= maxRetries; attempt += 1) {
    try {
      const key = await getMXEPublicKey(provider, programId);
      if (key) return key;
    } catch {
      // transient RPC errors are retried below.
    }
    if (attempt < maxRetries) {
      await new Promise((resolve) => setTimeout(resolve, retryDelayMs));
    }
  }
  throw new Error(
    `Failed to fetch the MXE public key after ${maxRetries} attempts. ` +
      `Confirm the program is deployed and its MXE is initialized on this cluster.`,
  );
}

/** Parse argv into fully resolved options (no network access). */
export function resolveOptions(argv: string[]): BenchOptions {
  const flags = parseArgs(argv);
  const command = (flags.positional[0] ?? "latency") as BenchKind;
  if (command !== "latency" && command !== "contention") {
    throw new Error(
      `Unknown command "${command}". Use "latency" or "contention".`,
    );
  }

  const root = repoRoot();
  const rpcUrl = stringFlag(
    flags,
    "rpc",
    "ANCHOR_PROVIDER_URL",
    DEFAULT_DEVNET_RPC,
  );
  const walletPath = stringFlag(
    flags,
    "wallet",
    "ANCHOR_WALLET",
    path.join(os.homedir(), ".config", "solana", "id.json"),
  );

  const op = (flags.values.get("op") ??
    (command === "contention"
      ? "apply_position_delta"
      : "add_together")) as BenchOp;
  if (op !== "add_together" && op !== "apply_position_delta") {
    throw new Error(`Unknown --op "${op}".`);
  }

  return {
    command,
    op,
    runs: intFlag(flags, "runs", "WALNUT_BENCH_RUNS", 50),
    concurrency: intFlag(flags, "concurrency", "WALNUT_BENCH_CONCURRENCY", 10),
    durationSec: intFlag(flags, "duration", "WALNUT_BENCH_DURATION", 120),
    maxAttempts: intFlag(flags, "max-attempts", "WALNUT_BENCH_MAX_ATTEMPTS", 3),
    finalizeTimeoutMs: intFlag(
      flags,
      "timeout",
      "WALNUT_BENCH_TIMEOUT_MS",
      120000,
    ),
    reclaim: boolFlag(flags, "reclaim", "no-reclaim", false),
    initCompDefs: boolFlag(flags, "init-compdefs", "no-init-compdefs", true),
    outDir: stringFlag(
      flags,
      "out",
      "WALNUT_BENCH_OUT",
      path.join(root, "packages", "sdk", "bench", "results"),
    ),
    rpcUrl,
    walletPath,
    clusterOffset: resolveClusterOffset(flags, rpcUrl, root),
  };
}

/** Build the full runtime context (opens the RPC connection). */
export async function createContext(
  options: BenchOptions,
): Promise<BenchContext> {
  const root = repoRoot();
  const connection = new Connection(options.rpcUrl, COMMITMENT);
  const payer = loadKeypair(options.walletPath);
  const wallet = new anchor.Wallet(payer);
  const provider = new anchor.AnchorProvider(connection, wallet, {
    commitment: COMMITMENT,
    preflightCommitment: COMMITMENT,
  });
  anchor.setProvider(provider);

  const idl = loadWalnutIdl(root);
  const { program, methods } = makeProgram(idl, provider);
  const programId = program.programId;
  const mxePublicKey = await getMxePublicKeyWithRetry(provider, programId);

  return {
    options,
    repoRoot: root,
    provider,
    connection,
    payer,
    program,
    programId,
    methods,
    clusterOffset: options.clusterOffset,
    mxePublicKey,
  };
}
