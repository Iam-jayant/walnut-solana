// Unit tests for Network Profile validation (Task 3.4).

import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { getArciumProgramId } from "@arcium-hq/client";
import { PublicKey } from "@solana/web3.js";
import {
  loadNetworkProfile,
  MAINNET_USDC_MINT,
  NetworkProfileError,
  validateNetworkProfile,
  WSOL_MINT,
} from "../src/index.ts";

const DEVNET_PATH = fileURLToPath(
  new URL("../../../config/networks/devnet.json", import.meta.url),
);
const WALNUT = "5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea";
// Arbitrary valid addresses derived at runtime, so no high-entropy literal sits in the repo.
const KEY_A = new PublicKey(new Uint8Array(32).fill(1)).toBase58();
const KEY_B = new PublicKey(new Uint8Array(32).fill(2)).toBase58();
const FEED_SOL = "0x" + "ab".repeat(32);
const FEED_USDC = "cd".repeat(32);

function devnet(): Record<string, unknown> {
  return {
    network: "devnet",
    solanaCluster: "devnet",
    rpc: { default: "https://api.devnet.solana.com" },
    arcium: { clusterOffset: 456 },
    program: { walnut: WALNUT },
  };
}

function mainnet(): Record<string, unknown> {
  return {
    network: "mainnet",
    solanaCluster: "mainnet-beta",
    rpc: { default: "https://api.mainnet-beta.solana.com" },
    arcium: { clusterOffset: 2026 },
    program: { walnut: WALNUT },
    mints: { wsol: WSOL_MINT, usdc: MAINNET_USDC_MINT },
    vaults: { wsol: KEY_A, usdc: KEY_B },
    pyth: { solUsdFeedId: FEED_SOL, usdcUsdFeedId: FEED_USDC },
    commitment: "finalized",
    mainnetConfirmation: { required: true },
  };
}

function codes(input: unknown): string[] {
  const result = validateNetworkProfile(input);
  return result.ok ? [] : result.issues.map((i) => `${i.path}:${i.code}`);
}

test("config/networks/devnet.json is a valid profile", async () => {
  const profile = await loadNetworkProfile(DEVNET_PATH);
  assert.equal(profile.network, "devnet");
  assert.equal(profile.arcium.clusterOffset, 456);
  assert.equal(profile.program.walnut, WALNUT);
});

test("a complete mainnet profile is valid", () => {
  const result = validateNetworkProfile(mainnet());
  assert.equal(result.ok, true);
});

test("a devnet profile may carry the optional fields", () => {
  const result = validateNetworkProfile({
    ...devnet(),
    arcium: {
      clusterOffset: 456,
      programId: getArciumProgramId().toBase58(),
    },
    mints: { wsol: WSOL_MINT, usdc: KEY_A },
    pyth: { solUsdFeedId: FEED_SOL, usdcUsdFeedId: FEED_USDC },
    commitment: "confirmed",
  });
  assert.equal(result.ok, true);
});

test("rejects mainnet with the devnet offset and devnet with the mainnet offset", () => {
  const wrongMainnet = { ...mainnet(), arcium: { clusterOffset: 456 } };
  assert.deepEqual(codes(wrongMainnet), [
    "arcium.clusterOffset:CLUSTER_OFFSET_MISMATCH",
  ]);
  const wrongDevnet = { ...devnet(), arcium: { clusterOffset: 2026 } };
  assert.deepEqual(codes(wrongDevnet), [
    "arcium.clusterOffset:CLUSTER_OFFSET_MISMATCH",
  ]);
});

test("localnet accepts any u32 offset and http", () => {
  const result = validateNetworkProfile({
    network: "localnet",
    solanaCluster: "localnet",
    rpc: { default: "http://127.0.0.1:8899" },
    arcium: { clusterOffset: 1 },
    program: { walnut: WALNUT },
  });
  assert.equal(result.ok, true);
});

test("reports every problem at once", () => {
  const found = codes({
    network: "testnet",
    solanaCluster: "devnet",
    rpc: { default: "ftp://example.com" },
    arcium: { clusterOffset: -1 },
    program: { walnut: "not-a-key" },
    mints: { wsol: KEY_A, usdc: KEY_A },
    pyth: { solUsdFeedId: "abc", usdcUsdFeedId: FEED_USDC },
    commitment: "instant",
    surprise: 1,
  });
  assert.deepEqual(found.sort(), [
    "arcium.clusterOffset:INVALID_TYPE",
    "commitment:INVALID_COMMITMENT",
    "mints:DUPLICATE_VALUE",
    "network:INVALID_NETWORK",
    "program.walnut:INVALID_PUBKEY",
    "pyth.solUsdFeedId:INVALID_FEED_ID",
    "rpc.default:INVALID_URL",
    "surprise:UNKNOWN_FIELD",
  ]);
});

test("mainnet requires mints, vaults, feeds, commitment, and confirmation", () => {
  const profile = {
    network: "mainnet",
    solanaCluster: "mainnet-beta",
    rpc: { default: "https://api.mainnet-beta.solana.com" },
    arcium: { clusterOffset: 2026 },
    program: { walnut: WALNUT },
  };
  assert.deepEqual(codes(profile).sort(), [
    "commitment:MISSING_FIELD",
    "mainnetConfirmation:MISSING_FIELD",
    "mints:MISSING_FIELD",
    "pyth:MISSING_FIELD",
    "vaults:MISSING_FIELD",
  ]);
});

test("mainnet policy: no processed commitment, confirmation must be required, canonical mints", () => {
  const profile = {
    ...mainnet(),
    commitment: "processed",
    mainnetConfirmation: { required: false },
    mints: { wsol: KEY_A, usdc: KEY_B },
  };
  assert.deepEqual(codes(profile).sort(), [
    "commitment:MAINNET_POLICY",
    "mainnetConfirmation.required:MAINNET_POLICY",
    "mints.usdc:MINT_MISMATCH",
    "mints.wsol:MINT_MISMATCH",
  ]);
});

test("rejects http outside localnet and an RPC host of the wrong network", () => {
  assert.deepEqual(
    codes({ ...devnet(), rpc: { default: "http://api.devnet.solana.com" } }),
    ["rpc.default:INSECURE_URL"],
  );
  assert.deepEqual(
    codes({
      ...devnet(),
      rpc: { default: "https://api.mainnet-beta.solana.com" },
    }),
    ["rpc.default:RPC_NETWORK_MISMATCH"],
  );
  assert.deepEqual(
    codes({ ...mainnet(), rpc: { default: "https://api.devnet.solana.com" } }),
    ["rpc.default:RPC_NETWORK_MISMATCH"],
  );
});

test("an API key in the RPC URL is a warning that fails by default and tells users to use env", () => {
  const profile = {
    ...devnet(),
    rpc: { default: "https://rpc.example.com/?api-key=SECRET123" },
  };
  const strict = validateNetworkProfile(profile);
  assert.equal(strict.ok, false);
  if (!strict.ok) {
    assert.equal(strict.issues.length, 1);
    assert.equal(strict.issues[0]?.code, "SECRET_IN_URL");
    assert.equal(strict.issues[0]?.severity, "warning");
    assert.match(strict.issues[0]?.message ?? "", /environment variable/);
    assert.doesNotMatch(strict.issues[0]?.message ?? "", /SECRET123/);
  }
  const lenient = validateNetworkProfile(profile, { failOnWarnings: false });
  assert.equal(lenient.ok, true);
  if (lenient.ok) assert.equal(lenient.warnings.length, 1);
  assert.deepEqual(
    codes({
      ...devnet(),
      rpc: { default: "https://rpc.example.com/?apikey=x" },
    }),
    ["rpc.default:SECRET_IN_URL"],
  );
  assert.deepEqual(
    codes({ ...devnet(), rpc: { default: "https://user:pw@rpc.example.com" } }),
    ["rpc.default:SECRET_IN_URL"],
  );
});

test("rejects secret-looking fields and base58 secret-key-sized values", () => {
  const found = codes({
    ...devnet(),
    privateKey: "x",
    program: { walnut: WALNUT, note: "1".repeat(88) },
  });
  assert.deepEqual(found.sort(), [
    "privateKey:SECRET_FIELD",
    "program.note:SECRET_VALUE",
  ]);
});

test("rejects a program id that is not an exact base58 public key", () => {
  assert.deepEqual(codes({ ...devnet(), program: { walnut: WALNUT + "0" } }), [
    "program.walnut:INVALID_PUBKEY",
  ]);
});

test("rejects a different Arcium program id and non-objects", () => {
  assert.deepEqual(
    codes({ ...devnet(), arcium: { clusterOffset: 456, programId: KEY_A } }),
    ["arcium.programId:ARCIUM_PROGRAM_MISMATCH"],
  );
  assert.deepEqual(codes(null), [":NOT_AN_OBJECT"]);
  assert.deepEqual(codes([]), [":NOT_AN_OBJECT"]);
});

test("loadNetworkProfile throws a typed error listing the issues", async () => {
  const dir = await import("node:fs/promises");
  const os = await import("node:os");
  const path = await import("node:path");
  const tmp = await dir.mkdtemp(path.join(os.tmpdir(), "walnut-profile-"));
  const file = path.join(tmp, "bad.json");
  try {
    await dir.writeFile(file, JSON.stringify({ network: "devnet" }));
    await assert.rejects(loadNetworkProfile(file), (error: unknown) => {
      assert.ok(error instanceof NetworkProfileError);
      assert.ok(error.issues.length >= 4);
      return true;
    });
    await dir.writeFile(file, "{ nope");
    await assert.rejects(loadNetworkProfile(file), NetworkProfileError);
  } finally {
    await dir.rm(tmp, { recursive: true, force: true });
  }
});
