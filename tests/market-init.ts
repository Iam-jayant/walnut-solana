/**
 * Task 4.1: market initialization (Requirements 4.1 and 4.2).
 *
 * Proves on localnet that `initialize_market`:
 *   1. rejects a signer that is not the program upgrade authority, a zero
 *      authority key, a wrong token program, a mint owned by another program,
 *      a wSOL mint that is not the native mint, a native USDC mint, and a
 *      USDC mint without 6 decimals;
 *   2. creates the Market, Asset State, Pool State (`Initializing`), vault
 *      authority, wSOL and USDC vaults, and the Recovery Position;
 *   3. rejects a second initialization with `MarketAlreadyInitialized`;
 * and that the `init_pool_state` computation stores the `Enc<Mxe>` zero Pool
 * payload, activates the Pool once, and rejects a later queue.
 *
 * The Market is a singleton PDA, so the file is idempotent: when another run
 * already initialized the Market on this validator, it skips creation and
 * still asserts the stored state and every rejection. Values that later files
 * change (generation, caps, roles) are only checked on the creating run.
 *
 * Shared-state contract for the market-* files (run alphabetically in one
 * mocha process: init, limits, pause, roles, timelock): each file installs
 * the roles it needs through the provider wallet (the admin) in `before`,
 * and leaves the provider wallet as admin, no pending admin, no pauses, and
 * no pending configuration proposal in `after`.
 */
import * as anchor from "@anchor-lang/core";
import { Program } from "@anchor-lang/core";
import {
  Keypair,
  LAMPORTS_PER_SOL,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";
import { Walnut } from "../target/types/walnut";
import { randomBytes } from "crypto";
import {
  awaitComputationFinalization,
  getArciumEnv,
  getCompDefAccOffset,
  getArciumAccountBaseSeed,
  getArciumProgramId,
  uploadCircuit,
  getMXEAccAddress,
  getMempoolAccAddress,
  getCompDefAccAddress,
  getExecutingPoolAccAddress,
  getComputationAccAddress,
  getClusterAccAddress,
  getLookupTableAddress,
  getArciumProgram,
} from "@arcium-hq/client";
import * as fs from "fs";
import * as os from "os";
import { expect } from "chai";

const TOKEN_PROGRAM_ID = new PublicKey(
  "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
);
const TOKEN_2022_PROGRAM_ID = new PublicKey(
  "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
);
const NATIVE_MINT = new PublicKey(
  "So11111111111111111111111111111111111111112",
);
const BPF_LOADER_UPGRADEABLE_ID = new PublicKey(
  "BPFLoaderUpgradeab1e11111111111111111111111",
);
const MINT_SIZE = 82;

// Seeds mirror programs/walnut/src/market/mod.rs.
const seed = (s: string) => Buffer.from(s);

describe("Walnut Task 4.1: market initialization", () => {
  anchor.setProvider(anchor.AnchorProvider.env());
  const program = anchor.workspace.Walnut as Program<Walnut>;
  const provider = anchor.getProvider() as anchor.AnchorProvider;
  const connection = provider.connection;
  const arciumEnv = getArciumEnv();
  const owner = readKpJson(`${os.homedir()}/.config/solana/id.json`);

  const pda = (...seeds: Buffer[]) =>
    PublicKey.findProgramAddressSync(seeds, program.programId)[0];
  const market = pda(seed("market"));
  const assetState = pda(seed("asset_state"), market.toBuffer());
  const poolState = pda(seed("pool_state"), market.toBuffer());
  const vaultAuthority = pda(seed("vault_authority"), market.toBuffer());
  const recoveryPosition = pda(seed("recovery_position"), market.toBuffer());
  const vaultFor = (mint: PublicKey) =>
    pda(seed("vault"), market.toBuffer(), mint.toBuffer());
  const programData = PublicKey.findProgramAddressSync(
    [program.programId.toBuffer()],
    BPF_LOADER_UPGRADEABLE_ID,
  )[0];

  const riskAuthority = Keypair.generate().publicKey;
  const treasuryAuthority = Keypair.generate().publicKey;
  const guardianAuthority = Keypair.generate().publicKey;
  const caps = {
    supplyCapUsdc: new anchor.BN(1_000_000_000_000),
    totalDebtCapUsdc: new anchor.BN(500_000_000_000),
    positionDebtCapUsdc: new anchor.BN(10_000_000_000),
  };

  let usdcMint: PublicKey;

  type InitOverrides = {
    signer?: Keypair;
    wsolMint?: PublicKey;
    usdcMint?: PublicKey;
    tokenProgram?: PublicKey;
    riskAuthority?: PublicKey;
  };

  function initializeMarket(o: InitOverrides = {}) {
    const signer = o.signer ?? owner;
    const wsolMint = o.wsolMint ?? NATIVE_MINT;
    const usdc = o.usdcMint ?? usdcMint;
    return program.methods
      .initializeMarket({
        riskAuthority: o.riskAuthority ?? riskAuthority,
        treasuryAuthority,
        guardianAuthority,
        caps,
        positionCountCap: new anchor.BN(1_000),
        debtPositionCountCap: new anchor.BN(250),
      })
      .accountsPartial({
        authority: signer.publicKey,
        walnutProgram: program.programId,
        programData,
        wsolMint,
        usdcMint: usdc,
        tokenProgram: o.tokenProgram ?? TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        market,
        assetState,
        poolState,
        vaultAuthority,
        wsolVault: vaultFor(wsolMint),
        usdcVault: vaultFor(usdc),
        recoveryPosition,
      })
      .signers([signer])
      .rpc({ commitment: "confirmed" });
  }

  before(async () => {
    // Initialization is gated on the program upgrade authority, which must be
    // the provider wallet on localnet (Arcium's MXE init has the same rule).
    const info = await connection.getAccountInfo(programData);
    expect(info, "program-data account").to.not.equal(null);
    expect(info!.data[12], "program must be upgradeable").to.equal(1);
    expect(
      new PublicKey(info!.data.subarray(13, 45)).equals(owner.publicKey),
      "the provider wallet must be the upgrade authority",
    ).to.equal(true);
    expect(
      await connection.getAccountInfo(NATIVE_MINT),
      "native mint must exist on localnet",
    ).to.not.equal(null);

    const existing = await program.account.market.fetchNullable(market);
    usdcMint = existing
      ? existing.usdcMint
      : await createMint(provider, owner, 6);
  });

  it("rejects invalid signers, authorities, mints, and token programs", async () => {
    const stranger = Keypair.generate();
    const airdrop = await connection.requestAirdrop(
      stranger.publicKey,
      2 * LAMPORTS_PER_SOL,
    );
    await connection.confirmTransaction(airdrop, "confirmed");

    await expectRejected(
      initializeMarket({ signer: stranger }),
      "InitializerNotUpgradeAuthority",
    );
    await expectRejected(
      initializeMarket({ riskAuthority: PublicKey.default }),
      "InvalidAuthority",
    );
    // `Program<Token>` rejects Token-2022 (InvalidProgramExecutable only if the
    // validator does not ship Token-2022).
    await expectRejected(
      initializeMarket({ tokenProgram: TOKEN_2022_PROGRAM_ID }),
      "InvalidProgramId",
      "InvalidProgramExecutable",
    );
    // A "mint" owned by the System Program, not SPL Token.
    await expectRejected(
      initializeMarket({ usdcMint: owner.publicKey }),
      "AccountOwnedByWrongProgram",
    );
    const fakeWsol = await createMint(provider, owner, 9);
    await expectRejected(
      initializeMarket({ wsolMint: fakeWsol }),
      "WsolMintMismatch",
    );
    // Both vaults then share one address, so Anchor's duplicate-mutable check
    // may fire before the handler's own check.
    await expectRejected(
      initializeMarket({ usdcMint: NATIVE_MINT }),
      "UsdcMintIsNative",
      "ConstraintDuplicateMutableAccount",
    );
    const nineDecimals = await createMint(provider, owner, 9);
    await expectRejected(
      initializeMarket({ usdcMint: nineDecimals }),
      "InvalidUsdcDecimals",
    );
  });

  it("creates every market account once", async () => {
    // Later files (market-limits) bump the generation and change caps and
    // roles, so the launch values are only asserted when this run created
    // the Market.
    const created = !(await program.account.market.fetchNullable(market));
    if (created) {
      await initializeMarket();
    }

    const m = await program.account.market.fetch(market);
    expect(m.layoutVersion).to.equal(1);
    expect("active" in m.status).to.equal(true);
    expect(m.admin.equals(owner.publicKey)).to.equal(true);
    expect(m.upgradeAuthority.equals(owner.publicKey)).to.equal(true);
    expect(m.pendingAdmin.equals(PublicKey.default)).to.equal(true);
    expect(m.assetState.equals(assetState)).to.equal(true);
    expect(m.poolState.equals(poolState)).to.equal(true);
    expect(m.recoveryPosition.equals(recoveryPosition)).to.equal(true);
    expect(m.vaultAuthority.equals(vaultAuthority)).to.equal(true);
    expect(m.wsolMint.equals(NATIVE_MINT)).to.equal(true);
    expect(m.usdcMint.equals(usdcMint)).to.equal(true);
    expect(m.wsolVault.equals(vaultFor(NATIVE_MINT))).to.equal(true);
    expect(m.usdcVault.equals(vaultFor(usdcMint))).to.equal(true);
    expect(m.positionCount.toNumber()).to.equal(0);
    expect(m.debtPositionCount.toNumber()).to.equal(0);
    expect(m.unsettledRecoveryClaimCount.toNumber()).to.equal(0);

    const a = await program.account.assetState.fetch(assetState);
    expect(a.layoutVersion).to.equal(1);
    expect(a.market.equals(market)).to.equal(true);
    if (created) {
      expect(a.generation.toNumber()).to.equal(0);
      expect(a.caps.supplyCapUsdc.eq(caps.supplyCapUsdc)).to.equal(true);
      expect(m.riskAuthority.equals(riskAuthority)).to.equal(true);
    }
    expect(a.accountingBorrowIndex.toString()).to.equal("1000000000000000000");
    expect(a.pauses.flags).to.equal(0);
    expect(a.pauses.guardianFlags).to.equal(0);

    const pool = await program.account.poolState.fetch(poolState);
    expect(pool.layoutVersion).to.equal(1);
    expect(pool.market.equals(market)).to.equal(true);
    expect(pool.coordination.lock.active).to.equal(false);
    expect("initializing" in pool.status || "active" in pool.status).to.equal(
      true,
    );

    const va = await program.account.vaultAuthority.fetch(vaultAuthority);
    expect(va.market.equals(market)).to.equal(true);

    const recovery = await program.account.position.fetch(recoveryPosition);
    expect(recovery.owner.equals(market)).to.equal(true);
    expect(recovery.coordination.generation.toNumber()).to.equal(0);

    for (const mint of [NATIVE_MINT, usdcMint]) {
      const vault = await connection.getAccountInfo(vaultFor(mint));
      expect(vault, "vault token account").to.not.equal(null);
      expect(vault!.owner.equals(TOKEN_PROGRAM_ID)).to.equal(true);
      // SPL token account: mint 0..32, owner (authority) 32..64.
      expect(new PublicKey(vault!.data.subarray(0, 32)).equals(mint)).to.equal(
        true,
      );
      expect(
        new PublicKey(vault!.data.subarray(32, 64)).equals(vaultAuthority),
      ).to.equal(true);
    }
  });

  it("rejects repeat initialization", async () => {
    await expectRejected(initializeMarket(), "MarketAlreadyInitialized");
  });

  it("activates the Pool through init_pool_state exactly once", async () => {
    await initCompDef(
      "init_pool_state",
      program.methods.initPoolStateCompDef(),
    );

    const queue = (payer: Keypair) => {
      const computationOffset = new anchor.BN(randomBytes(8), "hex");
      const tx = program.methods
        .queuePoolStateInit(computationOffset)
        .accountsPartial({
          payer: payer.publicKey,
          computationAccount: getComputationAccAddress(
            arciumEnv.arciumClusterOffset,
            computationOffset,
          ),
          clusterAccount: getClusterAccAddress(arciumEnv.arciumClusterOffset),
          mxeAccount: getMXEAccAddress(program.programId),
          mempoolAccount: getMempoolAccAddress(arciumEnv.arciumClusterOffset),
          executingPool: getExecutingPoolAccAddress(
            arciumEnv.arciumClusterOffset,
          ),
          compDefAccount: getCompDefAccAddress(
            program.programId,
            Buffer.from(getCompDefAccOffset("init_pool_state")).readUInt32LE(),
          ),
          market,
          poolState,
        })
        .signers([payer])
        .rpc({ skipPreflight: false, commitment: "confirmed" });
      return { computationOffset, tx };
    };

    const stranger = Keypair.generate();
    const airdrop = await connection.requestAirdrop(
      stranger.publicKey,
      LAMPORTS_PER_SOL,
    );
    await connection.confirmTransaction(airdrop, "confirmed");
    await expectRejected(queue(stranger).tx, "NotMarketAdmin");

    let pool = await program.account.poolState.fetch(poolState);
    if ("initializing" in pool.status) {
      const { computationOffset, tx } = queue(owner);
      await tx;
      await awaitComputationFinalization(
        provider,
        computationOffset,
        program.programId,
        "confirmed",
      );
      pool = await program.account.poolState.fetch(poolState);
    }

    expect("active" in pool.status).to.equal(true);
    expect(pool.coordination.generation.toNumber()).to.equal(1);
    expect(pool.encrypted.nonce.isZero()).to.equal(false);
    expect(pool.encrypted.ciphertexts.length).to.equal(4);
    for (const ciphertext of pool.encrypted.ciphertexts) {
      expect(ciphertext.length).to.equal(32);
    }

    await expectRejected(queue(owner).tx, "PoolStateNotInitializing");
  });

  async function initCompDef(
    circuitName: string,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    method: any,
  ): Promise<void> {
    const offset = getCompDefAccOffset(circuitName);
    const compDefPDA = PublicKey.findProgramAddressSync(
      [
        getArciumAccountBaseSeed("ComputationDefinitionAccount"),
        program.programId.toBuffer(),
        offset,
      ],
      getArciumProgramId(),
    )[0];
    // Shared validator across test files: register once.
    if (await connection.getAccountInfo(compDefPDA)) return;

    const arciumProgram = getArciumProgram(provider);
    const mxeAccount = getMXEAccAddress(program.programId);
    const mxeAcc = await arciumProgram.account.mxeAccount.fetch(mxeAccount);
    await method
      .accounts({
        compDefAccount: compDefPDA,
        payer: owner.publicKey,
        mxeAccount,
        addressLookupTable: getLookupTableAddress(
          program.programId,
          mxeAcc.lutOffsetSlot,
        ),
      })
      .signers([owner])
      .rpc({ preflightCommitment: "confirmed", commitment: "confirmed" });

    await uploadCircuit(
      provider,
      circuitName,
      program.programId,
      fs.readFileSync(`build/${circuitName}.arcis`),
      true,
    );
  }
});

/** Creates a classic SPL Token mint with `decimals`; `payer` is the authority. */
async function createMint(
  provider: anchor.AnchorProvider,
  payer: Keypair,
  decimals: number,
): Promise<PublicKey> {
  const mint = Keypair.generate();
  const lamports =
    await provider.connection.getMinimumBalanceForRentExemption(MINT_SIZE);
  // InitializeMint2: tag 20 | decimals | mint authority | freeze authority None.
  const data = Buffer.alloc(35);
  data.writeUInt8(20, 0);
  data.writeUInt8(decimals, 1);
  payer.publicKey.toBuffer().copy(data, 2);
  data.writeUInt8(0, 34);
  const tx = new Transaction().add(
    SystemProgram.createAccount({
      fromPubkey: payer.publicKey,
      newAccountPubkey: mint.publicKey,
      lamports,
      space: MINT_SIZE,
      programId: TOKEN_PROGRAM_ID,
    }),
    new TransactionInstruction({
      programId: TOKEN_PROGRAM_ID,
      keys: [{ pubkey: mint.publicKey, isSigner: false, isWritable: true }],
      data,
    }),
  );
  await provider.sendAndConfirm(tx, [payer, mint], { commitment: "confirmed" });
  return mint.publicKey;
}

/**
 * Asserts that `promise` rejects and that the error names one of `codes`.
 * Several codes are accepted only where the runtime, not Walnut, decides which
 * Anchor check fires first.
 */
async function expectRejected(
  promise: Promise<unknown>,
  ...codes: string[]
): Promise<void> {
  let error: unknown;
  try {
    await promise;
  } catch (caught) {
    error = caught;
  }
  const expected = codes.join(" or ");
  expect(error, `expected rejection with ${expected}`).to.not.equal(undefined);
  const e = error as {
    message?: string;
    logs?: string[];
    error?: { errorCode?: { code?: string } };
  };
  const text = [e.message ?? "", ...(e.logs ?? []), e.error?.errorCode?.code]
    .filter(Boolean)
    .join("\n");
  expect(
    codes.some((code) => text.includes(code)),
    `rejection should name ${expected}, got: ${text}`,
  ).to.equal(true);
}

function readKpJson(path: string): Keypair {
  return Keypair.fromSecretKey(
    new Uint8Array(JSON.parse(fs.readFileSync(path).toString())),
  );
}
