// Typed facade over the Walnut Anchor program for the benchmark harness.
//
// The harness must type-check in the CI "JS checks" job, which does NOT run
// `arcium build`, so `target/types/walnut.ts` is not present there. To stay
// CI-safe the harness never imports the generated program type: it loads the
// IDL JSON at runtime and drives a generic `Program` through the small typed
// facade below. The facade only declares the handful of instructions and
// accounts the benchmarks actually use, matching the on-chain signatures in
// programs/walnut/src/lib.rs (add_together, init_position,
// apply_position_delta, and their comp-def initializers).

import * as anchor from "../src/anchor-shim.ts";
import type { BN } from "../src/anchor-shim.ts";
import type { ConfirmOptions, Keypair, PublicKey } from "@solana/web3.js";
import {
  getArciumAccountBaseSeed,
  getArciumProgram,
  getArciumProgramId,
  getCircuitState,
  getCompDefAccOffset,
  getLookupTableAddress,
  getMXEAccAddress,
  uploadCircuit,
} from "@arcium-hq/client";
import * as fs from "node:fs";
import * as path from "node:path";
import { PublicKey as Web3PublicKey } from "@solana/web3.js";

/** A queue-instruction builder (success/callback computations). */
export interface QueueBuilder {
  accountsPartial(accounts: Record<string, PublicKey>): QueueBuilder;
  signers(signers: Keypair[]): QueueBuilder;
  rpc(opts?: ConfirmOptions): Promise<string>;
}

/** A computation-definition initializer builder. */
export interface InitCompDefBuilder {
  accounts(accounts: Record<string, PublicKey>): InitCompDefBuilder;
  signers(signers: Keypair[]): InitCompDefBuilder;
  rpc(opts?: ConfirmOptions): Promise<string>;
}

/** The exact instruction surface the benchmarks call. */
export interface BenchMethods {
  addTogether(
    offset: BN,
    ciphertext0: number[],
    ciphertext1: number[],
    pubKey: number[],
    nonce: BN,
  ): QueueBuilder;
  initPosition(
    offset: BN,
    ciphertexts: number[][],
    pubKey: number[],
    nonce: BN,
  ): QueueBuilder;
  applyPositionDelta(
    offset: BN,
    deltaCiphertexts: number[][],
    deltaPubKey: number[],
    deltaNonce: BN,
  ): QueueBuilder;
  initAddTogetherCompDef(): InitCompDefBuilder;
  initPositionCompDef(): InitCompDefBuilder;
  applyPositionDeltaCompDef(): InitCompDefBuilder;
}

/** Minimal shape of the Task 2.2 SpikePosition account the harness reads. */
export interface SpikePositionAccount {
  revision: BN;
}

/** Load the camelCase IDL from target/idl/walnut.json at runtime. */
export function loadWalnutIdl(repoRoot: string): anchor.Idl {
  const idlPath = path.join(repoRoot, "target", "idl", "walnut.json");
  if (!fs.existsSync(idlPath)) {
    throw new Error(
      `Walnut IDL not found at ${idlPath}. Build the program first ` +
        `(\`arcium build\`) or deploy it to the target cluster.`,
    );
  }
  return JSON.parse(fs.readFileSync(idlPath, "utf8")) as anchor.Idl;
}

/** Construct the generic program and its typed method facade. */
export function makeProgram(
  idl: anchor.Idl,
  provider: anchor.AnchorProvider,
): { program: anchor.Program; methods: BenchMethods } {
  const program = new anchor.Program(idl, provider);
  const methods = program.methods as unknown as BenchMethods;
  return { program, methods };
}

/** Fetch the SpikePosition account, or null when it does not exist yet. */
export async function fetchPosition(
  program: anchor.Program,
  positionPda: PublicKey,
): Promise<SpikePositionAccount | null> {
  const accounts = program.account as unknown as {
    spikePosition: {
      fetchNullable(pk: PublicKey): Promise<SpikePositionAccount | null>;
    };
  };
  return accounts.spikePosition.fetchNullable(positionPda);
}

/**
 * Register a computation definition and upload its circuit if it is not
 * already present, mirroring the skip-if-exists pattern used by the Task 2.2
 * and 2.4 tests. The circuit bytes are read from build/<name>.arcis.
 */
async function isOffchain(
  provider: anchor.AnchorProvider,
  compDef: PublicKey,
): Promise<boolean> {
  const accounts = getArciumProgram(provider).account as unknown as Record<
    string,
    {
      fetch(
        a: PublicKey,
      ): Promise<{ circuitSource: Parameters<typeof getCircuitState>[0] }>;
    }
  >;
  const account =
    await accounts["computationDefinitionAccount"]!.fetch(compDef);
  return getCircuitState(account.circuitSource) === "Offchain";
}

export async function ensureCompDef(
  provider: anchor.AnchorProvider,
  programId: PublicKey,
  repoRoot: string,
  circuitName: string,
  init: InitCompDefBuilder,
  payer: Keypair,
): Promise<"created" | "exists"> {
  const baseSeed = getArciumAccountBaseSeed("ComputationDefinitionAccount");
  const offset = getCompDefAccOffset(circuitName);
  const compDefPda = Web3PublicKey.findProgramAddressSync(
    [baseSeed, programId.toBuffer(), offset],
    getArciumProgramId(),
  )[0];

  const circuitPath = path.join(repoRoot, "build", `${circuitName}.arcis`);
  const existing = await provider.connection.getAccountInfo(compDefPda);
  if (existing) {
    // An off-chain comp def has nothing to upload.
    if (await isOffchain(provider, compDefPda)) return "exists";
    // The comp def may exist while its circuit upload was interrupted
    // (the queue then fails with ComputationDefinitionNotCompleted).
    // uploadCircuit skips raw circuit accounts that are already complete,
    // so re-running it resumes the upload and is a no-op when finished.
    await uploadCircuit(
      provider,
      circuitName,
      programId,
      fs.readFileSync(circuitPath),
      true,
    );
    return "exists";
  }

  const arciumProgram = getArciumProgram(provider);
  const mxeAccount = getMXEAccAddress(programId);
  const mxeAcc = (await arciumProgram.account["mxeAccount"].fetch(
    mxeAccount,
  )) as { lutOffsetSlot: BN };
  const lutAddress = getLookupTableAddress(programId, mxeAcc.lutOffsetSlot);

  await init
    .accounts({
      compDefAccount: compDefPda,
      payer: payer.publicKey,
      mxeAccount,
      addressLookupTable: lutAddress,
    })
    .signers([payer])
    .rpc({ preflightCommitment: "confirmed", commitment: "confirmed" });

  if (await isOffchain(provider, compDefPda)) return "created";

  const rawCircuit = fs.readFileSync(circuitPath);
  await uploadCircuit(provider, circuitName, programId, rawCircuit, true);
  return "created";
}
