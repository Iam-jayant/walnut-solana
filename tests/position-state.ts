/**
 * Task 2.2 Arcium feasibility gate: persistent encrypted Position state.
 *
 * Proves (Requirement 2.2) that the complete proposed Position payload can be:
 *   1. stored    - `init_position` encrypts the five-field `PositionSecret`
 *                  payload and the callback writes the ciphertext into a
 *                  persistent account;
 *   2. reread    - each later `apply_position_delta` consumes the ciphertext
 *                  stored by the previous computation (persistence ACROSS
 *                  computations, not within one call);
 *   3. updated   - fields are updated in a randomized order over several
 *                  computations and the stored state stays consistent;
 *   4. decrypted - the client decrypts the stored ciphertext locally after
 *                  every step and asserts the exact expected value.
 *
 * It also records every (viewing key, nonce) pair used and asserts no nonce
 * repeats - evidence toward the nonce-safety gate (Requirement 2.4 / Task 2.3).
 *
 * This mirrors tests/walnut.ts conventions: x25519 key exchange with the MXE,
 * RescueCipher encrypt/decrypt, queue + awaitComputationFinalization, and local
 * decryption. No financial logic is exercised; deltas are plain additions.
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
  uploadCircuit,
  RescueCipher,
  deserializeLE,
  getMXEAccAddress,
  getMempoolAccAddress,
  getCompDefAccAddress,
  getExecutingPoolAccAddress,
  x25519,
  getComputationAccAddress,
  getMXEPublicKey,
  getClusterAccAddress,
  getLookupTableAddress,
  getArciumProgram,
} from "@arcium-hq/client";
import * as fs from "fs";
import * as os from "os";
import { expect } from "chai";

const POSITION_SEED = Buffer.from("spike_position");
const FIELD_COUNT = 5;

// Canonical PositionSecret field order (must match the Arcis struct):
// free_sol, free_usdc, collateral_sol, supply_shares_usdc, debt_shares_usdc.
const FIELD_NAMES = [
  "free_sol",
  "free_usdc",
  "collateral_sol",
  "supply_shares_usdc",
  "debt_shares_usdc",
];

describe("Walnut Task 2.2: persistent encrypted Position state", () => {
  anchor.setProvider(anchor.AnchorProvider.env());
  const program = anchor.workspace.Walnut as Program<Walnut>;
  const provider = anchor.getProvider();

  const arciumEnv = getArciumEnv();
  const clusterAccount = getClusterAccAddress(arciumEnv.arciumClusterOffset);

  it("stores, rereads, updates in randomized order, and decrypts locally", async () => {
    const owner = readKpJson(`${os.homedir()}/.config/solana/id.json`);

    const mxePublicKey = await getMXEPublicKeyWithRetry(
      provider as anchor.AnchorProvider,
      program.programId,
    );

    await initCompDef(
      owner,
      "init_position",
      program.methods.initPositionCompDef(),
    );
    await initCompDef(
      owner,
      "apply_position_delta",
      program.methods.applyPositionDeltaCompDef(),
    );

    // One Viewing Key for the whole Position payload (the Enc<Shared>
    // local-read model): the same x25519 keypair encrypts the init payload and
    // every delta, and decrypts every stored revision.
    const viewingPrivateKey = x25519.utils.randomSecretKey();
    const viewingPublicKey = x25519.getPublicKey(viewingPrivateKey);
    const sharedSecret = x25519.getSharedSecret(viewingPrivateKey, mxePublicKey);
    const cipher = new RescueCipher(sharedSecret);

    const positionPda = PublicKey.findProgramAddressSync(
      [POSITION_SEED, owner.publicKey.toBuffer()],
      program.programId,
    )[0];

    // Every nonce ever used under the Viewing Key, for the no-repeat assertion.
    const usedNonces = new Set<string>();
    const recordNonce = (nonce: Uint8Array) => {
      const key = Buffer.from(nonce).toString("hex");
      expect(usedNonces.has(key), `nonce reused: ${key}`).to.equal(false);
      usedNonces.add(key);
    };

    // ---- Step 1: store the initial (all-zero) Position payload ------------
    const expected = [0n, 0n, 0n, 0n, 0n];
    {
      const nonce = randomBytes(16);
      recordNonce(nonce);
      const ct = cipher.encrypt(expected, nonce);
      const computationOffset = new anchor.BN(randomBytes(8), "hex");

      await program.methods
        .initPosition(
          computationOffset,
          ct.map((c) => Array.from(c)),
          Array.from(viewingPublicKey),
          new anchor.BN(deserializeLE(nonce).toString()),
        )
        .accountsPartial({
          payer: owner.publicKey,
          computationAccount: getComputationAccAddress(
            arciumEnv.arciumClusterOffset,
            computationOffset,
          ),
          clusterAccount,
          mxeAccount: getMXEAccAddress(program.programId),
          mempoolAccount: getMempoolAccAddress(arciumEnv.arciumClusterOffset),
          executingPool: getExecutingPoolAccAddress(
            arciumEnv.arciumClusterOffset,
          ),
          compDefAccount: getCompDefAccAddress(
            program.programId,
            Buffer.from(getCompDefAccOffset("init_position")).readUInt32LE(),
          ),
          position: positionPda,
        })
        .signers([owner])
        .rpc({ skipPreflight: true, commitment: "confirmed" });

      await awaitComputationFinalization(
        provider as anchor.AnchorProvider,
        computationOffset,
        program.programId,
        "confirmed",
      );

      const decrypted = await fetchAndDecrypt(
        program,
        positionPda,
        cipher,
        recordNonce,
      );
      expect(decrypted).to.deep.equal(expected);

      const acct = await program.account.spikePosition.fetch(positionPda);
      expect(Buffer.from(acct.viewingPubkey)).to.deep.equal(
        Buffer.from(viewingPublicKey),
      );
      expect(acct.revision.toNumber()).to.equal(1);
    }

    // ---- Step 2: randomized-order field updates across computations -------
    // A deterministic but shuffled sequence that touches every field and
    // repeats one, proving order independence and cumulative consistency.
    const updates: Array<{ field: number; amount: bigint }> = [
      { field: 2, amount: 7_000_000_000n }, // collateral_sol
      { field: 0, amount: 1_500_000_000n }, // free_sol
      { field: 4, amount: 42_000_000_000_000_000_000n }, // debt_shares_usdc (u128)
      { field: 1, amount: 2_500_000n }, // free_usdc
      { field: 3, amount: 99_000_000_000_000_000_000n }, // supply_shares_usdc (u128)
      { field: 0, amount: 500_000_000n }, // free_sol again
    ];

    let revision = 1;
    for (const { field, amount } of updates) {
      const deltaPlain = [0n, 0n, 0n, 0n, 0n];
      deltaPlain[field] = amount;

      const nonce = randomBytes(16);
      recordNonce(nonce);
      const ct = cipher.encrypt(deltaPlain, nonce);
      const computationOffset = new anchor.BN(randomBytes(8), "hex");

      await program.methods
        .applyPositionDelta(
          computationOffset,
          ct.map((c) => Array.from(c)),
          Array.from(viewingPublicKey),
          new anchor.BN(deserializeLE(nonce).toString()),
        )
        .accountsPartial({
          payer: owner.publicKey,
          computationAccount: getComputationAccAddress(
            arciumEnv.arciumClusterOffset,
            computationOffset,
          ),
          clusterAccount,
          mxeAccount: getMXEAccAddress(program.programId),
          mempoolAccount: getMempoolAccAddress(arciumEnv.arciumClusterOffset),
          executingPool: getExecutingPoolAccAddress(
            arciumEnv.arciumClusterOffset,
          ),
          compDefAccount: getCompDefAccAddress(
            program.programId,
            Buffer.from(
              getCompDefAccOffset("apply_position_delta"),
            ).readUInt32LE(),
          ),
          position: positionPda,
        })
        .signers([owner])
        .rpc({ skipPreflight: true, commitment: "confirmed" });

      await awaitComputationFinalization(
        provider as anchor.AnchorProvider,
        computationOffset,
        program.programId,
        "confirmed",
      );

      expected[field] += amount;
      revision += 1;

      const decrypted = await fetchAndDecrypt(
        program,
        positionPda,
        cipher,
        recordNonce,
      );
      expect(
        decrypted,
        `after updating ${FIELD_NAMES[field]} the full payload must match`,
      ).to.deep.equal(expected);

      const acct = await program.account.spikePosition.fetch(positionPda);
      expect(acct.revision.toNumber()).to.equal(revision);
    }

    // Every nonce used across init + 6 updates + their stored outputs is unique.
    expect(usedNonces.size).to.be.greaterThan(updates.length + 1);
  });

  async function initCompDef(
    owner: anchor.web3.Keypair,
    circuitName: string,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    method: any,
  ): Promise<string> {
    const baseSeedCompDefAcc = getArciumAccountBaseSeed(
      "ComputationDefinitionAccount",
    );
    const offset = getCompDefAccOffset(circuitName);
    const compDefPDA = PublicKey.findProgramAddressSync(
      [baseSeedCompDefAcc, program.programId.toBuffer(), offset],
      getArciumProgramId(),
    )[0];

    const arciumProgram = getArciumProgram(provider as anchor.AnchorProvider);
    const mxeAccount = getMXEAccAddress(program.programId);
    const mxeAcc = await arciumProgram.account.mxeAccount.fetch(mxeAccount);
    const lutAddress = getLookupTableAddress(
      program.programId,
      mxeAcc.lutOffsetSlot,
    );

    const sig = await method
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
      provider as anchor.AnchorProvider,
      circuitName,
      program.programId,
      rawCircuit,
      true,
    );
    return sig;
  }
});

/** Fetches the stored Position account and decrypts its payload locally. */
async function fetchAndDecrypt(
  program: Program<Walnut>,
  positionPda: PublicKey,
  cipher: RescueCipher,
  recordNonce: (nonce: Uint8Array) => void,
): Promise<bigint[]> {
  const acct = await program.account.spikePosition.fetch(positionPda);
  const nonceBytes = new Uint8Array(
    acct.state.nonce.toArrayLike(Buffer, "le", 16),
  );
  recordNonce(nonceBytes);
  const ciphertexts = acct.state.ciphertexts.map((c: number[]) =>
    Uint8Array.from(c),
  );
  expect(ciphertexts.length).to.equal(FIELD_COUNT);
  return cipher.decrypt(ciphertexts, nonceBytes);
}

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
  throw new Error(`Failed to fetch MXE public key after ${maxRetries} attempts`);
}

function readKpJson(path: string): anchor.web3.Keypair {
  const file = fs.readFileSync(path);
  return anchor.web3.Keypair.fromSecretKey(
    new Uint8Array(JSON.parse(file.toString())),
  );
}
