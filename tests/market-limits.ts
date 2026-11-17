/**
 * Task 4.5: launch caps and public lifecycle counts (Requirements 4.5, 4.6,
 * 4.10, and 4.11).
 *
 * Proves on localnet, against the Market created by `market-init.ts`, that:
 *   1. a cap raise (USDC caps or count caps) through `decrease_caps` is
 *      rejected with `CapChangeNotDecrease`, and only the risk authority may
 *      call it;
 *   2. a cap decrease applies immediately, bumps the Asset State generation
 *      once, and emits `CapsDecreased` with old and new values;
 *   3. a cap raise goes through `propose_config_change`, cannot execute before
 *      the delay (`TimelockNotElapsed`), and a raise past the per-change limit
 *      or outside the bounds is rejected at proposal time;
 *   4. the Market starts with zero counts and a zero reservation count, and
 *      the Recovery Position has `has_debt == false`.
 *
 * Named `market-limits.ts` (not `market-caps.ts`) because the Anchor test
 * glob runs files alphabetically and this file needs `market-init.ts` to have
 * run first.
 *
 * Idempotent: every run lowers the supply cap by one base unit (and the
 * Position count cap by one while it stays above the debt-Position count
 * cap), and every proposal it creates is cancelled; `before` and `after` also
 * cancel any proposal an interrupted run left pending. Successful timelocked
 * execution is covered by the Rust tests in `programs/walnut/src/market/`.
 */
import * as anchor from "@anchor-lang/core";
import { BN, Program } from "@anchor-lang/core";
import { Keypair, PublicKey } from "@solana/web3.js";
import { Walnut } from "../target/types/walnut";
import { createHash } from "crypto";
import * as fs from "fs";
import * as os from "os";
import { expect } from "chai";

// `market::caps`.
const MAX_USDC_CAP = new BN("1000000000000000");
const USDC_CAP_RAISE_FLOOR = new BN("100000000000");

type ParsedEvent = { name: string; data: Record<string, unknown> };

describe("Walnut Task 4.5: launch caps and lifecycle counts", () => {
  anchor.setProvider(anchor.AnchorProvider.env());
  const program = anchor.workspace.Walnut as Program<Walnut>;
  const provider = anchor.getProvider() as anchor.AnchorProvider;
  const connection = provider.connection;
  const owner = readKpJson(`${os.homedir()}/.config/solana/id.json`);
  const stranger = Keypair.generate();
  // Same seeds as `market-timelock.ts`, so both files agree on the roles.
  const risk = seededKeypair("walnut-task-4.4-risk");
  const guardian = seededKeypair("walnut-task-4.4-guardian");

  const pda = (...seeds: Buffer[]) =>
    PublicKey.findProgramAddressSync(seeds, program.programId)[0];
  const market = pda(Buffer.from("market"));
  const assetState = pda(Buffer.from("asset_state"), market.toBuffer());
  const recoveryPosition = pda(
    Buffer.from("recovery_position"),
    market.toBuffer(),
  );
  const proposalAddress = (id: BN) =>
    pda(
      Buffer.from("config_proposal"),
      market.toBuffer(),
      id.toArrayLike(Buffer, "le", 8),
    );
  const parser = new anchor.EventParser(program.programId, program.coder);

  type ConfigValue = Parameters<typeof program.methods.decreaseCaps>[0];

  const asset = () => program.account.assetState.fetch(assetState, "confirmed");
  const marketAccount = () => program.account.market.fetch(market, "confirmed");

  const decrease = (signer: Keypair, change: ConfigValue) =>
    program.methods
      .decreaseCaps(change)
      .accountsPartial({ authority: signer.publicKey, market, assetState })
      .signers([signer])
      .rpc({ commitment: "confirmed" });
  const propose = (id: BN, change: ConfigValue) =>
    program.methods
      .proposeConfigChange(id, change)
      .accountsPartial({
        proposer: risk.publicKey,
        payer: owner.publicKey,
        market,
        assetState,
        proposal: proposalAddress(id),
      })
      .signers([risk])
      .rpc({ commitment: "confirmed" });
  const execute = (id: BN) =>
    program.methods
      .executeConfigChange(id)
      .accountsPartial({
        executor: stranger.publicKey,
        market,
        assetState,
        proposal: proposalAddress(id),
      })
      .signers([stranger])
      .rpc({ commitment: "confirmed" });
  const cancel = (id: BN) =>
    program.methods
      .cancelConfigChange(id)
      .accountsPartial({
        authority: guardian.publicKey,
        market,
        proposal: proposalAddress(id),
      })
      .signers([guardian])
      .rpc({ commitment: "confirmed" });

  async function onlyEvent(
    signature: string,
    name: string,
  ): Promise<Record<string, unknown>> {
    const tx = await connection.getTransaction(signature, {
      commitment: "confirmed",
      maxSupportedTransactionVersion: 0,
    });
    expect(tx?.meta?.logMessages, "transaction logs").to.not.equal(undefined);
    const events = [
      ...parser.parseLogs(tx!.meta!.logMessages!),
    ] as ParsedEvent[];
    expect(events.map((e) => e.name)).to.deep.equal([name]);
    const data = events[0].data;
    expect((data.market as PublicKey).equals(market)).to.equal(true);
    return data;
  }

  before(async () => {
    const m = await program.account.market.fetchNullable(market);
    expect(m, "run market-init.ts first: the Market must exist").to.not.equal(
      null,
    );
    expect(
      m!.admin.equals(owner.publicKey),
      "provider wallet is admin",
    ).to.equal(true);
    if (!m!.riskAuthority.equals(risk.publicKey)) {
      await program.methods
        .setRiskAuthority(risk.publicKey)
        .accountsPartial({ admin: owner.publicKey, market })
        .rpc({ commitment: "confirmed" });
    }
    if (!m!.guardianAuthority.equals(guardian.publicKey)) {
      await program.methods
        .setGuardianAuthority(guardian.publicKey)
        .accountsPartial({ admin: owner.publicKey, market })
        .rpc({ commitment: "confirmed" });
    }
    await cancelPendingProposals();
  });

  // Leave no pending proposal behind, even when a test stopped midway.
  after(async () => {
    await cancelPendingProposals();
  });

  async function cancelPendingProposals() {
    const next = (await asset()).timelock.nextProposalId.toNumber();
    const ids = Array.from({ length: next }, (_, i) => new BN(i));
    const stored = await program.account.configProposal.fetchMultiple(
      ids.map(proposalAddress),
      "confirmed",
    );
    for (const [i, proposal] of stored.entries()) {
      if (proposal && "pending" in proposal.status) await cancel(ids[i]);
    }
  }

  it("exposes public counts and has_debt", async () => {
    const m = await marketAccount();
    expect(m.debtPositionCount.lte(m.debtPositionCountCap)).to.equal(true);
    expect(m.positionCount.lte(m.positionCountCap)).to.equal(true);
    expect(m.debtPositionReservations.toNumber()).to.equal(0);
    expect(m.debtPositionCountCap.lte(m.positionCountCap)).to.equal(true);
    const recovery = await program.account.position.fetch(
      recoveryPosition,
      "confirmed",
    );
    expect(recovery.hasDebt).to.equal(false);
  });

  it("rejects immediate raises and unauthorized decreases", async () => {
    const a = await asset();
    const m = await marketAccount();
    const lower: ConfigValue = {
      caps: {
        caps: { ...a.caps, supplyCapUsdc: a.caps.supplyCapUsdc.subn(1) },
      },
    };
    for (const signer of [stranger, guardian, owner]) {
      await expectRejected(decrease(signer, lower), "NotRiskAuthority");
    }
    await expectRejected(
      decrease(risk, {
        caps: {
          caps: { ...a.caps, supplyCapUsdc: a.caps.supplyCapUsdc.addn(1) },
        },
      }),
      "CapChangeNotDecrease",
    );
    await expectRejected(
      decrease(risk, {
        countCaps: {
          caps: {
            positionCountCap: m.positionCountCap.addn(1),
            debtPositionCountCap: m.debtPositionCountCap,
          },
        },
      }),
      "CapChangeNotDecrease",
    );
    await expectRejected(
      decrease(risk, {
        timelockDelay: { delaySeconds: a.timelock.delaySeconds },
      }),
      "CapChangeNotDecrease",
    );
    await expectRejected(
      decrease(risk, { caps: { caps: a.caps } }),
      "ConfigUnchanged",
    );
    await expectRejected(
      decrease(risk, {
        caps: { caps: { ...a.caps, supplyCapUsdc: new BN(0) } },
      }),
      "ConfigOutOfBounds",
    );
    const after = await asset();
    expect(after.generation.eq(a.generation)).to.equal(true);
    expect(after.caps).to.deep.equal(a.caps);
  });

  it("applies decreases immediately and bumps the generation", async () => {
    const a = await asset();
    const caps = { ...a.caps, supplyCapUsdc: a.caps.supplyCapUsdc.subn(1) };
    const data = await onlyEvent(
      await decrease(risk, { caps: { caps } }),
      "capsDecreased",
    );
    expect(Object.keys(data.group as object)).to.deep.equal(["caps"]);
    expect(data.oldValue).to.deep.equal({ caps: { caps: a.caps } });
    expect(data.newValue).to.deep.equal({ caps: { caps } });
    expect(new BN(data.newGeneration as BN).eq(a.generation.addn(1))).to.equal(
      true,
    );
    const after = await asset();
    expect(after.caps).to.deep.equal(caps);
    expect(after.generation.eq(a.generation.addn(1))).to.equal(true);

    const m = await marketAccount();
    if (m.positionCountCap.gt(m.debtPositionCountCap)) {
      const countCaps = {
        positionCountCap: m.positionCountCap.subn(1),
        debtPositionCountCap: m.debtPositionCountCap,
      };
      await onlyEvent(
        await decrease(risk, { countCaps: { caps: countCaps } }),
        "capsDecreased",
      );
      const updated = await marketAccount();
      expect(updated.positionCountCap.eq(countCaps.positionCountCap)).to.equal(
        true,
      );
    }
  });

  it("requires the timelock for raises", async () => {
    const a = await asset();
    const m = await marketAccount();
    let id = a.timelock.nextProposalId;

    await expectRejected(
      propose(id, {
        caps: {
          caps: { ...a.caps, supplyCapUsdc: MAX_USDC_CAP.addn(1) },
        },
      }),
      "ConfigOutOfBounds",
    );
    await expectRejected(
      propose(id, {
        caps: {
          caps: {
            ...a.caps,
            // One base unit past `old + USDC_CAP_RAISE_FLOOR` (100,000 USDC),
            // which exceeds `2 × old` for the market-init caps.
            positionDebtCapUsdc: a.caps.positionDebtCapUsdc.add(
              USDC_CAP_RAISE_FLOOR.addn(1),
            ),
          },
        },
      }),
      "ConfigMovementTooLarge",
    );

    const raisedCounts = {
      positionCountCap: m.positionCountCap.addn(10),
      debtPositionCountCap: m.debtPositionCountCap.addn(10),
    };
    const raise: ConfigValue = { countCaps: { caps: raisedCounts } };
    const data = await onlyEvent(
      await propose(id, raise),
      "configChangeProposed",
    );
    expect(Object.keys(data.group as object)).to.deep.equal(["countCaps"]);
    expect(data.newValue).to.deep.equal(raise);
    const stored = await program.account.configProposal.fetch(
      proposalAddress(id),
      "confirmed",
    );
    expect(stored.newCountCaps).to.deep.equal(raisedCounts);
    expect(
      stored.oldCountCaps.positionCountCap.eq(m.positionCountCap),
    ).to.equal(true);
    await expectRejected(execute(id), "TimelockNotElapsed");
    await onlyEvent(await cancel(id), "configChangeCancelled");

    id = id.addn(1);
    const capsRaise: ConfigValue = {
      caps: {
        caps: { ...a.caps, supplyCapUsdc: a.caps.supplyCapUsdc.addn(1) },
      },
    };
    await onlyEvent(await propose(id, capsRaise), "configChangeProposed");
    await expectRejected(execute(id), "TimelockNotElapsed");
    await onlyEvent(await cancel(id), "configChangeCancelled");

    const after = await asset();
    const afterMarket = await marketAccount();
    expect(after.caps).to.deep.equal(a.caps);
    expect(after.generation.eq(a.generation)).to.equal(true);
    expect(afterMarket.positionCountCap.eq(m.positionCountCap)).to.equal(true);
  });
});

function seededKeypair(label: string): Keypair {
  return Keypair.fromSeed(createHash("sha256").update(label).digest());
}

/** Asserts that `promise` rejects and that the error names one of `codes`. */
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
