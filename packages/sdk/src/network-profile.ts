// Network Profile schema and validation (Requirements 1.x, 14.2).
//
// A Network Profile is a public JSON file in config/networks/. It holds only
// public configuration, so the validator also refuses anything that looks like
// a secret. The validator is hand-rolled (no new dependencies) and reports
// every problem it finds, not just the first.
//
// Fields: the ones config/networks/devnet.json already carries (network,
// solanaCluster, rpc, arcium.clusterOffset, program.walnut, plus free-text
// description and note fields) and the ones Requirement 14.2 adds (Arcium
// program id, mints, vaults, Pyth feeds, commitment, mainnet confirmation
// policy). The added fields are optional for localnet and devnet until the
// market is initialized (Task 4) and required for mainnet.
//
// Cluster offsets: devnet 456, mainnet 2026 (docs/DEVNET_DEPLOY.md). A profile
// with the other network's offset is rejected, because a wrong offset points
// every cluster PDA at the wrong cluster. Localnet accepts any u32 offset
// since the local cluster offset is chosen by the test harness.

import { getArciumProgramId } from "@arcium-hq/client";
import { PublicKey } from "@solana/web3.js";

export type NetworkName = "localnet" | "devnet" | "mainnet";
export type SolanaClusterName = "localnet" | "devnet" | "mainnet-beta";
export type CommitmentName = "processed" | "confirmed" | "finalized";

/** The Arcium cluster offset each public network must use. */
export const EXPECTED_CLUSTER_OFFSET = {
  devnet: 456,
  mainnet: 2026,
} as const;

/** Canonical wrapped-SOL mint (the same on every Solana cluster). */
export const WSOL_MINT = "So11111111111111111111111111111111111111112";
/** Canonical Circle USDC mint on mainnet. */
export const MAINNET_USDC_MINT = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

const SOLANA_CLUSTER_FOR: Record<NetworkName, SolanaClusterName> = {
  localnet: "localnet",
  devnet: "devnet",
  mainnet: "mainnet-beta",
};

/** A validated Network Profile. */
export interface NetworkProfile {
  readonly network: NetworkName;
  readonly solanaCluster: SolanaClusterName;
  readonly description?: string;
  readonly rpc: { readonly default: string; readonly note?: string };
  readonly arcium: {
    readonly clusterOffset: number;
    /** When present it must equal the program id pinned by @arcium-hq/client. */
    readonly programId?: string;
    readonly note?: string;
  };
  readonly program: { readonly walnut: string; readonly note?: string };
  readonly mints?: { readonly wsol: string; readonly usdc: string };
  readonly vaults?: { readonly wsol: string; readonly usdc: string };
  /** 32-byte Pyth price feed ids as hex, with or without a 0x prefix. */
  readonly pyth?: {
    readonly solUsdFeedId: string;
    readonly usdcUsdFeedId: string;
  };
  readonly commitment?: CommitmentName;
  /** Required on mainnet: users must confirm before a mainnet action. */
  readonly mainnetConfirmation?: { readonly required: boolean };
}

export type NetworkProfileIssueCode =
  | "NOT_AN_OBJECT"
  | "MISSING_FIELD"
  | "INVALID_TYPE"
  | "UNKNOWN_FIELD"
  | "INVALID_NETWORK"
  | "INVALID_CLUSTER"
  | "CLUSTER_OFFSET_MISMATCH"
  | "INVALID_URL"
  | "INSECURE_URL"
  | "RPC_NETWORK_MISMATCH"
  | "INVALID_PUBKEY"
  | "INVALID_FEED_ID"
  | "INVALID_COMMITMENT"
  | "MAINNET_POLICY"
  | "ARCIUM_PROGRAM_MISMATCH"
  | "MINT_MISMATCH"
  | "DUPLICATE_VALUE"
  | "SECRET_IN_URL"
  | "SECRET_FIELD"
  | "SECRET_VALUE";

/** One validation problem, with the JSON path it was found at. */
export interface NetworkProfileIssue {
  /** Dotted JSON path, e.g. `rpc.default`. The empty string is the root. */
  readonly path: string;
  readonly code: NetworkProfileIssueCode;
  readonly severity: "error" | "warning";
  readonly message: string;
}

export type NetworkProfileValidation =
  | {
      readonly ok: true;
      readonly profile: NetworkProfile;
      /** Warning-level issues, when `failOnWarnings` is false. */
      readonly warnings: readonly NetworkProfileIssue[];
    }
  | {
      readonly ok: false;
      /** Every error and warning found, in document order. */
      readonly issues: readonly NetworkProfileIssue[];
    };

export interface ValidateNetworkProfileOptions {
  /**
   * Treat warning-level issues (a likely secret inside an RPC URL) as
   * failures. Defaults to true, so a secret cannot slip into a public profile.
   */
  readonly failOnWarnings?: boolean;
}

/** Raised by {@link loadNetworkProfile} when a profile is invalid. */
export class NetworkProfileError extends Error {
  readonly issues: readonly NetworkProfileIssue[];
  constructor(source: string, issues: readonly NetworkProfileIssue[]) {
    super(
      `invalid Network Profile ${source}:\n` +
        issues
          .map(
            (i) =>
              `  - [${i.severity}] ${i.path || "(root)"}: ${i.message} (${i.code})`,
          )
          .join("\n"),
    );
    this.name = "NetworkProfileError";
    this.issues = issues;
  }
}

type Json = Record<string, unknown>;

const NETWORKS: readonly NetworkName[] = ["localnet", "devnet", "mainnet"];
const COMMITMENTS: readonly CommitmentName[] = [
  "processed",
  "confirmed",
  "finalized",
];
const BASE58_PUBKEY = /^[1-9A-HJ-NP-Za-km-z]{32,44}$/;
const BASE58_SECRET_SIZED = /^[1-9A-HJ-NP-Za-km-z]{80,90}$/;
const FEED_ID = /^(0x)?[0-9a-fA-F]{64}$/;
const SECRET_FIELD_NAME =
  /(private|secret|seed|mnemonic|keypair|password|api[-_]?key)/i;
const SECRET_QUERY_PARAM =
  /^(api[-_]?key|key|token|access[-_]?token|auth|secret|password)$/i;

function isRecord(value: unknown): value is Json {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parsePubkey(value: string): PublicKey | null {
  if (!BASE58_PUBKEY.test(value)) return null;
  try {
    const key = new PublicKey(value);
    return key.toBase58() === value ? key : null;
  } catch {
    return null;
  }
}

class Collector {
  readonly issues: NetworkProfileIssue[] = [];

  add(
    path: string,
    code: NetworkProfileIssueCode,
    message: string,
    severity: "error" | "warning" = "error",
  ): void {
    this.issues.push({ path, code, severity, message });
  }

  /** Reads an optional or required object section. */
  section(
    parent: Json,
    key: string,
    path: string,
    required: boolean,
  ): Json | undefined {
    const value = parent[key];
    if (value === undefined) {
      if (required) this.add(path, "MISSING_FIELD", `${path} is required`);
      return undefined;
    }
    if (!isRecord(value)) {
      this.add(path, "INVALID_TYPE", `${path} must be an object`);
      return undefined;
    }
    return value;
  }

  string(
    parent: Json,
    key: string,
    path: string,
    required: boolean,
  ): string | undefined {
    const value = parent[key];
    if (value === undefined) {
      if (required) this.add(path, "MISSING_FIELD", `${path} is required`);
      return undefined;
    }
    if (typeof value !== "string" || value.length === 0) {
      this.add(path, "INVALID_TYPE", `${path} must be a non-empty string`);
      return undefined;
    }
    return value;
  }

  pubkey(
    parent: Json,
    key: string,
    path: string,
    required: boolean,
  ): string | undefined {
    const value = this.string(parent, key, path, required);
    if (value !== undefined && parsePubkey(value) === null) {
      this.add(
        path,
        "INVALID_PUBKEY",
        `${path} is not a valid base58 public key`,
      );
      return undefined;
    }
    return value;
  }

  /** Flags keys outside `allowed`; secret-looking names get a sharper code. */
  noUnknown(obj: Json, path: string, allowed: readonly string[]): void {
    for (const key of Object.keys(obj)) {
      if (allowed.includes(key)) continue;
      const at = path ? `${path}.${key}` : key;
      if (SECRET_FIELD_NAME.test(key)) {
        this.add(
          at,
          "SECRET_FIELD",
          `${at} looks like a secret. Network Profiles are public; keep secrets in environment variables`,
        );
      } else {
        this.add(at, "UNKNOWN_FIELD", `unknown field ${at}`);
      }
    }
  }
}

function checkRpcUrl(
  c: Collector,
  value: string,
  network: NetworkName | undefined,
): void {
  const path = "rpc.default";
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    c.add(path, "INVALID_URL", `${path} is not a valid URL`);
    return;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") {
    c.add(path, "INVALID_URL", `${path} must use http or https`);
    return;
  }
  if (
    url.protocol === "http:" &&
    network !== undefined &&
    network !== "localnet"
  ) {
    c.add(path, "INSECURE_URL", `${path} must use https on ${network}`);
  }
  if (network !== undefined) {
    const host = url.hostname.toLowerCase();
    const has = (word: string): boolean =>
      new RegExp(`(^|[.-])${word}([.-]|$)`).test(host);
    if (network === "mainnet" && (has("devnet") || has("testnet"))) {
      c.add(
        path,
        "RPC_NETWORK_MISMATCH",
        `${path} points at ${host}, not mainnet`,
      );
    }
    if (network === "devnet" && (has("mainnet") || has("mainnet-beta"))) {
      c.add(
        path,
        "RPC_NETWORK_MISMATCH",
        `${path} points at ${host}, not devnet`,
      );
    }
  }
  const envHint =
    "Keep keys out of the profile and supply the endpoint through an environment variable (ANCHOR_PROVIDER_URL)";
  if (url.username !== "" || url.password !== "") {
    c.add(
      path,
      "SECRET_IN_URL",
      `${path} embeds credentials. ${envHint}`,
      "warning",
    );
  }
  for (const name of url.searchParams.keys()) {
    if (SECRET_QUERY_PARAM.test(name)) {
      c.add(
        path,
        "SECRET_IN_URL",
        `${path} has a "${name}" query parameter that looks like an API key. ${envHint}`,
        "warning",
      );
    }
  }
  for (const segment of url.pathname.split("/")) {
    if (segment.length >= 32 && /^[A-Za-z0-9_-]+$/.test(segment)) {
      c.add(
        path,
        "SECRET_IN_URL",
        `${path} has a long path segment that looks like an API token. ${envHint}`,
        "warning",
      );
      break;
    }
  }
}

function walkStrings(
  value: unknown,
  path: string,
  visit: (path: string, text: string) => void,
): void {
  if (typeof value === "string") visit(path, value);
  else if (Array.isArray(value)) {
    value.forEach((v, i) => walkStrings(v, `${path}[${i}]`, visit));
  } else if (isRecord(value)) {
    for (const [k, v] of Object.entries(value)) {
      walkStrings(v, path ? `${path}.${k}` : k, visit);
    }
  }
}

function checkPair(
  c: Collector,
  root: Json,
  key: "mints" | "vaults",
  required: boolean,
): { wsol: string; usdc: string } | undefined {
  const section = c.section(root, key, key, required);
  if (!section) return undefined;
  c.noUnknown(section, key, ["wsol", "usdc"]);
  const wsol = c.pubkey(section, "wsol", `${key}.wsol`, true);
  const usdc = c.pubkey(section, "usdc", `${key}.usdc`, true);
  if (wsol !== undefined && wsol === usdc) {
    c.add(key, "DUPLICATE_VALUE", `${key}.wsol and ${key}.usdc must differ`);
  }
  return wsol !== undefined && usdc !== undefined ? { wsol, usdc } : undefined;
}

/**
 * Validates untrusted JSON as a {@link NetworkProfile}.
 *
 * Returns every problem found. On success the returned profile contains only
 * the known fields.
 */
export function validateNetworkProfile(
  input: unknown,
  options: ValidateNetworkProfileOptions = {},
): NetworkProfileValidation {
  const failOnWarnings = options.failOnWarnings ?? true;
  const c = new Collector();
  if (!isRecord(input)) {
    c.add("", "NOT_AN_OBJECT", "a Network Profile must be a JSON object");
    return { ok: false, issues: c.issues };
  }

  c.noUnknown(input, "", [
    "network",
    "solanaCluster",
    "description",
    "rpc",
    "arcium",
    "program",
    "mints",
    "vaults",
    "pyth",
    "commitment",
    "mainnetConfirmation",
  ]);

  // Cluster name and its Solana cluster.
  let network: NetworkName | undefined;
  const rawNetwork = c.string(input, "network", "network", true);
  if (rawNetwork !== undefined) {
    if ((NETWORKS as readonly string[]).includes(rawNetwork)) {
      network = rawNetwork as NetworkName;
    } else {
      c.add(
        "network",
        "INVALID_NETWORK",
        `network must be one of ${NETWORKS.join(", ")}`,
      );
    }
  }
  const rawCluster = c.string(input, "solanaCluster", "solanaCluster", true);
  if (
    rawCluster !== undefined &&
    network !== undefined &&
    rawCluster !== SOLANA_CLUSTER_FOR[network]
  ) {
    c.add(
      "solanaCluster",
      "INVALID_CLUSTER",
      `solanaCluster must be "${SOLANA_CLUSTER_FOR[network]}" for network "${network}"`,
    );
  }
  const description = c.string(input, "description", "description", false);

  // RPC.
  const rpc = c.section(input, "rpc", "rpc", true);
  let rpcDefault: string | undefined;
  let rpcNote: string | undefined;
  if (rpc) {
    c.noUnknown(rpc, "rpc", ["default", "note"]);
    rpcDefault = c.string(rpc, "default", "rpc.default", true);
    rpcNote = c.string(rpc, "note", "rpc.note", false);
    if (rpcDefault !== undefined) checkRpcUrl(c, rpcDefault, network);
  }

  // Arcium.
  const arcium = c.section(input, "arcium", "arcium", true);
  let clusterOffset: number | undefined;
  let arciumProgramId: string | undefined;
  let arciumNote: string | undefined;
  if (arcium) {
    c.noUnknown(arcium, "arcium", ["clusterOffset", "programId", "note"]);
    const raw = arcium["clusterOffset"];
    if (raw === undefined) {
      c.add(
        "arcium.clusterOffset",
        "MISSING_FIELD",
        "arcium.clusterOffset is required",
      );
    } else if (
      typeof raw !== "number" ||
      !Number.isInteger(raw) ||
      raw < 0 ||
      raw > 0xffff_ffff
    ) {
      c.add(
        "arcium.clusterOffset",
        "INVALID_TYPE",
        "arcium.clusterOffset must be a u32 integer",
      );
    } else {
      clusterOffset = raw;
      if (network === "devnet" || network === "mainnet") {
        const expected = EXPECTED_CLUSTER_OFFSET[network];
        if (raw !== expected) {
          const other = network === "devnet" ? "mainnet" : "devnet";
          const hint =
            raw === EXPECTED_CLUSTER_OFFSET[other]
              ? ` (${raw} is the ${other} offset)`
              : "";
          c.add(
            "arcium.clusterOffset",
            "CLUSTER_OFFSET_MISMATCH",
            `${network} must use cluster offset ${expected}, got ${raw}${hint}`,
          );
        }
      }
    }
    arciumProgramId = c.pubkey(arcium, "programId", "arcium.programId", false);
    if (
      arciumProgramId !== undefined &&
      arciumProgramId !== getArciumProgramId().toBase58()
    ) {
      c.add(
        "arcium.programId",
        "ARCIUM_PROGRAM_MISMATCH",
        `arcium.programId must equal the Arcium program pinned by @arcium-hq/client (${getArciumProgramId().toBase58()})`,
      );
    }
    arciumNote = c.string(arcium, "note", "arcium.note", false);
  }

  // Walnut program.
  const program = c.section(input, "program", "program", true);
  let walnut: string | undefined;
  let programNote: string | undefined;
  if (program) {
    c.noUnknown(program, "program", ["walnut", "note"]);
    walnut = c.pubkey(program, "walnut", "program.walnut", true);
    programNote = c.string(program, "note", "program.note", false);
  }

  const mainnet = network === "mainnet";

  // Mints and vaults.
  const mints = checkPair(c, input, "mints", mainnet);
  if (mints && network !== undefined && network !== "localnet") {
    if (mints.wsol !== WSOL_MINT) {
      c.add("mints.wsol", "MINT_MISMATCH", `mints.wsol must be ${WSOL_MINT}`);
    }
    if (mainnet && mints.usdc !== MAINNET_USDC_MINT) {
      c.add(
        "mints.usdc",
        "MINT_MISMATCH",
        `mints.usdc must be ${MAINNET_USDC_MINT} on mainnet`,
      );
    }
  }
  const vaults = checkPair(c, input, "vaults", mainnet);

  // Pyth feeds.
  const pythSection = c.section(input, "pyth", "pyth", mainnet);
  let pyth: NetworkProfile["pyth"];
  if (pythSection) {
    c.noUnknown(pythSection, "pyth", ["solUsdFeedId", "usdcUsdFeedId"]);
    const ids: Record<string, string | undefined> = {};
    for (const key of ["solUsdFeedId", "usdcUsdFeedId"] as const) {
      const value = c.string(pythSection, key, `pyth.${key}`, true);
      if (value !== undefined && !FEED_ID.test(value)) {
        c.add(
          `pyth.${key}`,
          "INVALID_FEED_ID",
          `pyth.${key} must be 32 bytes of hex`,
        );
      } else {
        ids[key] = value;
      }
    }
    const sol = ids["solUsdFeedId"];
    const usdc = ids["usdcUsdFeedId"];
    if (sol !== undefined && usdc !== undefined) {
      if (
        sol.replace(/^0x/, "").toLowerCase() ===
        usdc.replace(/^0x/, "").toLowerCase()
      ) {
        c.add(
          "pyth",
          "DUPLICATE_VALUE",
          "pyth.solUsdFeedId and pyth.usdcUsdFeedId must differ",
        );
      } else {
        pyth = { solUsdFeedId: sol, usdcUsdFeedId: usdc };
      }
    }
  }

  // Commitment and mainnet confirmation policy.
  let commitment: CommitmentName | undefined;
  const rawCommitment = c.string(input, "commitment", "commitment", mainnet);
  if (rawCommitment !== undefined) {
    if ((COMMITMENTS as readonly string[]).includes(rawCommitment)) {
      commitment = rawCommitment as CommitmentName;
      if (mainnet && commitment === "processed") {
        c.add(
          "commitment",
          "MAINNET_POLICY",
          "mainnet must use confirmed or finalized commitment",
        );
      }
    } else {
      c.add(
        "commitment",
        "INVALID_COMMITMENT",
        `commitment must be one of ${COMMITMENTS.join(", ")}`,
      );
    }
  }
  const confirmation = c.section(
    input,
    "mainnetConfirmation",
    "mainnetConfirmation",
    mainnet,
  );
  let mainnetConfirmation: NetworkProfile["mainnetConfirmation"];
  if (confirmation) {
    c.noUnknown(confirmation, "mainnetConfirmation", ["required"]);
    const required = confirmation["required"];
    if (typeof required !== "boolean") {
      c.add(
        "mainnetConfirmation.required",
        "INVALID_TYPE",
        "mainnetConfirmation.required must be a boolean",
      );
    } else {
      mainnetConfirmation = { required };
      if (mainnet && !required) {
        c.add(
          "mainnetConfirmation.required",
          "MAINNET_POLICY",
          "mainnet profiles must set mainnetConfirmation.required to true",
        );
      }
    }
  }

  // Secret-looking values anywhere in the document (e.g. a pasted keypair).
  walkStrings(input, "", (path, text) => {
    if (BASE58_SECRET_SIZED.test(text)) {
      c.add(
        path,
        "SECRET_VALUE",
        `${path} looks like a base58 secret key. Network Profiles are public`,
      );
    }
  });

  const errors = c.issues.filter((i) => i.severity === "error");
  const warnings = c.issues.filter((i) => i.severity === "warning");
  if (
    errors.length > 0 ||
    (failOnWarnings && warnings.length > 0) ||
    network === undefined ||
    rawCluster === undefined ||
    rpcDefault === undefined ||
    clusterOffset === undefined ||
    walnut === undefined
  ) {
    return { ok: false, issues: c.issues };
  }

  const profile: NetworkProfile = {
    network,
    solanaCluster: SOLANA_CLUSTER_FOR[network],
    ...(description !== undefined ? { description } : {}),
    rpc: {
      default: rpcDefault,
      ...(rpcNote !== undefined ? { note: rpcNote } : {}),
    },
    arcium: {
      clusterOffset,
      ...(arciumProgramId !== undefined ? { programId: arciumProgramId } : {}),
      ...(arciumNote !== undefined ? { note: arciumNote } : {}),
    },
    program: {
      walnut,
      ...(programNote !== undefined ? { note: programNote } : {}),
    },
    ...(mints ? { mints } : {}),
    ...(vaults ? { vaults } : {}),
    ...(pyth ? { pyth } : {}),
    ...(commitment ? { commitment } : {}),
    ...(mainnetConfirmation ? { mainnetConfirmation } : {}),
  };
  return { ok: true, profile, warnings };
}

/**
 * Reads and validates a Network Profile from disk (Node only).
 *
 * `node:fs` is imported lazily so the rest of the SDK stays browser-safe.
 * Throws {@link NetworkProfileError} listing every problem.
 */
export async function loadNetworkProfile(
  path: string,
  options: ValidateNetworkProfileOptions = {},
): Promise<NetworkProfile> {
  const { readFile } = await import("node:fs/promises");
  const text = await readFile(path, "utf8");
  let json: unknown;
  try {
    json = JSON.parse(text);
  } catch (error) {
    throw new NetworkProfileError(path, [
      {
        path: "",
        code: "NOT_AN_OBJECT",
        severity: "error",
        message: `not valid JSON: ${error instanceof Error ? error.message : String(error)}`,
      },
    ]);
  }
  const result = validateNetworkProfile(json, options);
  if (!result.ok) throw new NetworkProfileError(path, result.issues);
  return result.profile;
}
