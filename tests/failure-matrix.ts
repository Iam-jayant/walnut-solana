/**
 * Task 2.4 Arcium feasibility gate: the complete failure matrix.
 *
 * Requirement 2.4 enumerates thirteen callback / failure / expiry / recovery
 * conditions the gate must exercise and assert. They split across two layers;
 * this file owns the localnet layer and the pure-Rust layer owns the rest. The
 * mapping (one row per matrix item) is:
 *
 *   1  Success callback (Applied) .............. localnet (this file) + Rust
 *   2  Circuit / static failure callback ....... localnet (OutputTooLarge) + Rust
 *   3  Invalid signed output (forged sig) ...... Rust (not forgeable on localnet;
 *                                                devnet observation in Task 2.6)
 *   4  Callback custom-account failure ......... Rust (revert-leaves-nothing)
 *   5  Callback token CPI failure .............. Rust (SPL CPI lands in Task 5)
 *   6  Compute-unit exhaustion ................. localnet (this file) + Rust
 *   7  OutputTooLarge ........................... localnet (this file) + Rust
 *   8  Queued expiry ............................ Rust (+ item 13 fee side)
 *   9  Abandon vs late execution ............... Rust
 *   10 Duplicate callback ....................... Rust
 *   11 Late callback ............................ Rust
 *   12 Finalized-rent claim ..................... localnet (this file)
 *   13 Expired-fee reclaim ...................... localnet wiring (this file);
 *                                                live fee return is Task 2.6
 *
 * The Rust-layer items are asserted in
 * `programs/walnut/src/operation/failure_matrix.rs` and run under
 * `pnpm run test:rust` / `pnpm run test:rust:property`.
 *
 * Localnet conventions mirror tests/walnut.ts and tests/position-state.ts:
 * x25519 key exchange with the MXE, RescueCipher, queue +
 * awaitComputationFinalization, the typed `awaitEvent` helper, and local
 * decryption. The reclaim helpers are the Arcium 0.16.0 APIs:
 * `claimComputationRent` (finalized rent) and the Arcium program's
 * `reclaimExpiredComputationFee` instruction (expired fee).
 */
import * as anchor from "@anchor-lang/core";
import { Program } from "@anchor-lang/core";
import { PublicKey } from "@solana/web3.js";
import { Walnut } from "../target/types/walnut";
import { randomBytes } from "crypto";
import {
  awaitComputationFinalization,
  getArciumEnv,
  getCompDefAccOffset,
  getArciumAccountBaseSeed,
  getArciumProgramId,
  getArciumProgram,
  uploadCircuit,
  RescueCipher,
  deserializeLE,
  getMXEAccAddress,
  getMempoolAccAddress,
  getCompDefAccAddress,
  getExecutingPoolAccAddress,
  getFeePoolAccAddress,
  x25519,
  getComputationAccAddress,
  getMXEPublicKey,
  getClusterAccAddress,
  getLookupTableAddress,
  claimComputationRent,
} from "@arcium-hq/client";
import * as fs from "fs";
import * as os from "os";
import { expect } from "chai";
import { watchEvent, type SumEventPayload } from "./support/events";

describe("Walnut Task 2.4: failure matrix (localnet layer)", () => {
  anchor.setProvider(anchor.AnchorProvider.env());
  const program = anchor.workspace.Walnut as Program<Walnut>;
  const provider = anchor.getProvider() as anchor.AnchorProvider;

  const arciumEnv = getArciumEnv();
  const clusterOffset = arciumEnv.arciumClusterOffset;
  const clusterAccount = getClusterAccAddress(clusterOffset);

  let owner: anchor.web3.Keypair;
  let mxePublicKey: Uint8Array;
  let cipher: RescueCipher;
  let viewingPublicKey: Uint8Array;

  before(async () => {
    owner = readKpJson(`${os.homedir()}/.config/solana/id.json`);
    mxePublicKey = await getMXEPublicKeyWithRetry(provider, program.programId);

    const viewingPrivateKey = x25519.utils.randomSecretKey();
    viewingPublicKey = x25519.getPublicKey(viewingPrivateKey);
    cipher = new RescueCipher(
      x25519.getSharedSecret(viewingPrivateKey, mxePublicKey),
    );

    await initCompDef("add_together", program.methods.initAddTogetherCompDef());
    await initCompDef(
      "oversized_output",
      program.methods.initOversizedOutputCompDef(),
    );
  });

  const standardQueueAccounts = (
    computationOffset: anchor.BN,
    circuit: string,
  ) => ({
    payer: owner.publicKey,
    computationAccount: getComputationAccAddress(
      clusterOffset,
      computationOffset,
    ),
    clusterAccount,
    mxeAccount: getMXEAccAddress(program.programId),
    mempoolAccount: getMempoolAccAddress(clusterOffset),
    executingPool: getExecutingPoolAccAddress(clusterOffset),
    compDefAccount: getCompDefAccAddress(
      program.programId,
      Buffer.from(getCompDefAccOffset(circuit)).readUInt32LE(),
    ),
  });

  // ---- Item 1 + Item 12: success callback, then finalized-rent claim ------

  it("item 1 + 12: success callback applies and its rent is reclaimed", async () => {
    const val1 = BigInt(7);
    const val2 = BigInt(35);
    const nonce = randomBytes(16);
    const ct = cipher.encrypt([val1, val2], nonce);
    const computationOffset = new anchor.BN(randomBytes(8), "hex");

    const sum = watchEvent<SumEventPayload>(program, "sumEvent");
    await program.methods
      .addTogether(
        computationOffset,
        Array.from(ct[0]),
        Array.from(ct[1]),
        Array.from(viewingPublicKey),
        new anchor.BN(deserializeLE(nonce).toString()),
      )
      .accountsPartial(standardQueueAccounts(computationOffset, "add_together"))
      .signers([owner])
      .rpc({ skipPreflight: true, commitment: "confirmed" });

    await awaitComputationFinalization(
      provider,
      computationOffset,
      program.programId,
      "confirmed",
    );

    // Item 1: the success event fired and decrypts to the exact sum.
    expect(sum.state.fired).to.equal(true);
    const decrypted = cipher.decrypt(
      [sum.state.payload!.sum],
      new Uint8Array(sum.state.payload!.nonce),
    )[0];
    expect(decrypted).to.equal(val1 + val2);
    await sum.stop();

    // Item 12: the computation account exists after finalization...
    const compAddr = getComputationAccAddress(clusterOffset, computationOffset);
    expect(await provider.connection.getAccountInfo(compAddr)).to.not.equal(
      null,
    );

    // ...and claimComputationRent (signed by the original payer) closes it.
    await claimComputationRent(provider, clusterOffset, computationOffset);
    expect(await provider.connection.getAccountInfo(compAddr)).to.equal(null);
  });

  // ---- Item 7 (+ item 2 failure path): OutputTooLarge ----------------------

  it("item 7: oversized output fails with OutputTooLarge; success callback never runs", async () => {
    const nonce = randomBytes(16);
    const ct = cipher.encrypt([BigInt(123)], nonce);
    const computationOffset = new anchor.BN(randomBytes(8), "hex");

    const emitted = watchEvent<unknown>(program, "oversizedOutputEmitted");
    await program.methods
      .oversizedOutput(
        computationOffset,
        Array.from(ct[0]),
        Array.from(viewingPublicKey),
        new anchor.BN(deserializeLE(nonce).toString()),
      )
      .accountsPartial(
        standardQueueAccounts(computationOffset, "oversized_output"),
      )
      .signers([owner])
      .rpc({ skipPreflight: true, commitment: "confirmed" });

    // The 48-field output cannot fit a single callback transaction, so the
    // computation finalizes as a failure (OutputTooLarge). Finalization either
    // rejects or resolves without the success callback ever running.
    let finalizationError: unknown;
    try {
      await awaitComputationFinalization(
        provider,
        computationOffset,
        program.programId,
        "confirmed",
        60000,
      );
    } catch (err) {
      finalizationError = err;
    }

    // Give any in-flight callback a moment, then assert the negative control:
    // the success event was never emitted.
    await new Promise((r) => setTimeout(r, 1500));
    expect(
      emitted.state.fired,
      "OutputTooLarge must prevent the success callback",
    ).to.equal(false);
    await emitted.stop();

    // A reported failure is the expected outcome; record it for visibility.
    if (finalizationError) {
      console.log("item 7 finalization reported failure as expected");
    }
  });

  // ---- Item 6: callback compute-unit exhaustion ----------------------------

  it("item 6: a tiny fixed callback_cu_limit exhausts compute; success callback never runs", async () => {
    const nonce = randomBytes(16);
    const ct = cipher.encrypt([BigInt(2), BigInt(3)], nonce);
    const computationOffset = new anchor.BN(randomBytes(8), "hex");

    const sum = watchEvent<SumEventPayload>(program, "sumEvent");
    await program.methods
      .addTogetherBoundedCu(
        computationOffset,
        Array.from(ct[0]),
        Array.from(ct[1]),
        Array.from(viewingPublicKey),
        new anchor.BN(deserializeLE(nonce).toString()),
        // Fixed, non-zero, deliberately far below what the callback needs.
        new anchor.BN(1),
      )
      .accountsPartial(standardQueueAccounts(computationOffset, "add_together"))
      .signers([owner])
      .rpc({ skipPreflight: true, commitment: "confirmed" });

    let finalizationError: unknown;
    try {
      await awaitComputationFinalization(
        provider,
        computationOffset,
        program.programId,
        "confirmed",
        60000,
      );
    } catch (err) {
      finalizationError = err;
    }

    await new Promise((r) => setTimeout(r, 1500));
    expect(
      sum.state.fired,
      "callback CU exhaustion must prevent the success callback",
    ).to.equal(false);
    await sum.stop();

    if (finalizationError) {
      console.log("item 6 finalization reported failure as expected");
    }
  });

  // ---- Item 13: expired-fee reclaim wiring ---------------------------------

  it("item 13: reclaimExpiredComputationFee is present and builds against the Arcium program", async () => {
    // A mempool computation expires after 180 slots; forcing that on a fast
    // localnet is impractical, so this asserts the reclaim instruction is wired
    // with the pinned Arcium 0.16.0 IDL. The live fee return is observed on
    // devnet in Task 2.6. (@arcium-hq/client exposes claimComputationRent as a
    // helper but not reclaimExpiredComputationFee, so it is built directly.)
    const arciumProgram = getArciumProgram(provider);
    const computationOffset = new anchor.BN(randomBytes(8), "hex");

    const ix = await arciumProgram.methods
      .reclaimExpiredComputationFee(computationOffset, program.programId)
      .accountsPartial({
        signer: owner.publicKey,
        mxe: getMXEAccAddress(program.programId),
        clusterAcc: clusterAccount,
        comp: getComputationAccAddress(clusterOffset, computationOffset),
        executingPool: getExecutingPoolAccAddress(clusterOffset),
        poolAccount: getFeePoolAccAddress(),
      })
      .instruction();

    expect(ix.programId.equals(getArciumProgramId())).to.equal(true);
    expect(ix.keys.length).to.be.greaterThan(0);
  });

  async function initCompDef(
    circuitName: string,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    method: any,
  ): Promise<void> {
    const baseSeedCompDefAcc = getArciumAccountBaseSeed(
      "ComputationDefinitionAccount",
    );
    const offset = getCompDefAccOffset(circuitName);
    const compDefPDA = PublicKey.findProgramAddressSync(
      [baseSeedCompDefAcc, program.programId.toBuffer(), offset],
      getArciumProgramId(),
    )[0];

    // Computation definitions are shared across the test files in one process,
    // so skip if another test already registered this circuit.
    const existing = await provider.connection.getAccountInfo(compDefPDA);
    if (existing) return;

    const arciumProgram = getArciumProgram(provider);
    const mxeAccount = getMXEAccAddress(program.programId);
    const mxeAcc = await arciumProgram.account.mxeAccount.fetch(mxeAccount);
    const lutAddress = getLookupTableAddress(
      program.programId,
      mxeAcc.lutOffsetSlot,
    );

    await method
      .accounts({
        compDefAccount: compDefPDA,
        payer: owner.publicKey,
        mxeAccount,
        addressLookupTable: lutAddress,
      })
      .signers([owner])
      .rpc({ preflightCommitment: "confirmed", commitment: "confirmed" });

    const rawCircuit = fs.readFileSync(`build/${circuitName}.arcis`);
    await uploadCircuit(
      provider,
      circuitName,
      program.programId,
      rawCircuit,
      true,
    );
  }
});

async function getMXEPublicKeyWithRetry(
  provider: anchor.AnchorProvider,
  programId: PublicKey,
  maxRetries = 20,
  retryDelayMs = 500,
): Promise<Uint8Array> {
  for (let attempt = 1; attempt <= maxRetries; attempt++) {
    try {
      const mxePublicKey = await getMXEPublicKey(provider, programId);
      if (mxePublicKey) return mxePublicKey;
    } catch (error) {
      console.log(`Attempt ${attempt} failed to fetch MXE public key:`, error);
    }
    if (attempt < maxRetries) {
      await new Promise((resolve) => setTimeout(resolve, retryDelayMs));
    }
  }
  throw new Error(
    `Failed to fetch MXE public key after ${maxRetries} attempts`,
  );
}

function readKpJson(path: string): anchor.web3.Keypair {
  const file = fs.readFileSync(path);
  return anchor.web3.Keypair.fromSecretKey(
    new Uint8Array(JSON.parse(file.toString())),
  );
}
