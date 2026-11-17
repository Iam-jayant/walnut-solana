/**
 * Task 4.4: timelocked configuration (Requirements 4.5, 4.6, 4.12, and 4.13).
 *
 * Proves on localnet, against the Market created by `market-init.ts`, that:
 *   1. only the risk authority proposes, the proposal id must be the next id,
 *      and unchanged, out-of-bounds, too-large, or below-minimum-delay values
 *      are rejected;
 *   2. a proposal stores its group, old and new values, proposer, times, and
 *      Asset State generation, and emits `ConfigChangeProposed`;
 *   3. execution before the delay fails with `TimelockNotElapsed` and changes
 *      nothing;
 *   4. only the risk authority or the guardian cancels, a cancelled proposal
 *      never executes, and cancellation emits `ConfigChangeCancelled`.
 *
 * The delay is at least 24 hours (`MIN_TIMELOCK_DELAY_SECONDS`) and there is
 * no localnet-only shortcut, so a successful execution is covered by the Rust
 * unit and property tests in `programs/walnut/src/market/config.rs`.
 *
 * Idempotent: proposal ids come from `asset_state.timelock.next_proposal_id`,
 * the risk and guardian keys come from fixed seeds and are installed by the
 * provider wallet (the admin) in `before`, and nothing is ever executed, so
 * the configuration and Asset State generation are unchanged. `before` and
 * `after` cancel any proposal still pending (from this file, an interrupted
 * run, or another file). The provider wallet pays every fee and every
 * proposal's rent.
 */
import * as anchor from "@anchor-lang/core";
import { BN, Program } from "@anchor-lang/core";
import { Keypair, PublicKey } from "@solana/web3.js";
import { Walnut } from "../target/types/walnut";
import { createHash } from "crypto";
import * as fs from "fs";
import * as os from "os";
import { expect } from "chai";

// `market::config`.
const MIN_TIMELOCK_DELAY_SECONDS = 86_400;
const EXECUTION_GRACE_SECONDS = 1_209_600;

type ParsedEvent = { name: string; data: Record<string, unknown> };

describe("Walnut Task 4.4: timelocked configuration", () => {
  anchor.setProvider(anchor.AnchorProvider.env());
  const program = anchor.workspace.Walnut as Program<Walnut>;
  const provider = anchor.getProvider() as anchor.AnchorProvider;
  const connection = provider.connection;
  const owner = readKpJson(`${os.homedir()}/.config/solana/id.json`);
  const stranger = Keypair.generate();
  const risk = seededKeypair("walnut-task-4.4-risk");
  const guardian = seededKeypair("walnut-task-4.4-guardian");

  const pda = (...seeds: Buffer[]) =>
    PublicKey.findProgramAddressSync(seeds, program.programId)[0];
  const market = pda(Buffer.from("market"));
  const assetState = pda(Buffer.from("asset_state"), market.toBuffer());
  const proposalAddress = (id: BN) =>
    pda(
      Buffer.from("config_proposal"),
      market.toBuffer(),
      id.toArrayLike(Buffer, "le", 8),
    );
  const parser = new anchor.EventParser(program.programId, program.coder);

  type ConfigValue = Parameters<typeof program.methods.proposeConfigChange>[1];

  const asset = () => program.account.assetState.fetch(assetState, "confirmed");

  const propose = (signer: Keypair, id: BN, change: ConfigValue) =>
    program.methods
      .proposeConfigChange(id, change)
      .accountsPartial({
        proposer: signer.publicKey,
        payer: owner.publicKey,
        market,
        assetState,
        proposal: proposalAddress(id),
      })
      .signers([signer])
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
  const cancel = (signer: Keypair, id: BN) =>
    program.methods
      .cancelConfigChange(id)
      .accountsPartial({
        authority: signer.publicKey,
        market,
        proposal: proposalAddress(id),
      })
      .signers([signer])
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
    expect(Number(data.slot)).to.be.greaterThan(0);
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
      if (proposal && "pending" in proposal.status) {
        await cancel(guardian, ids[i]);
      }
    }
  }

  it("rejects unauthorized proposals and invalid values", async () => {
    const a = await asset();
    const id = a.timelock.nextProposalId;
    const withRisk = (patch: Partial<typeof a.risk>): ConfigValue => ({
      risk: { params: { ...a.risk, ...patch } },
    });

    for (const signer of [stranger, guardian, owner]) {
      await expectRejected(
        propose(signer, id, withRisk({ ltvBps: a.risk.ltvBps + 100 })),
        "NotRiskAuthority",
      );
    }
    await expectRejected(
      propose(risk, id.addn(5), withRisk({ ltvBps: a.risk.ltvBps + 100 })),
      "ConfigProposalIdMismatch",
    );
    await expectRejected(propose(risk, id, withRisk({})), "ConfigUnchanged");
    await expectRejected(
      propose(risk, id, withRisk({ ltvBps: 9_000 })),
      "ConfigOutOfBounds",
    );
    await expectRejected(
      propose(risk, id, withRisk({ ltvBps: a.risk.liquidationThresholdBps })),
      "ConfigOutOfBounds",
    );
    await expectRejected(
      propose(risk, id, withRisk({ ltvBps: a.risk.ltvBps + 501 })),
      "ConfigMovementTooLarge",
    );
    await expectRejected(
      propose(risk, id, {
        rates: { params: { ...a.rates, slope2Bps: a.rates.slope2Bps + 5_001 } },
      }),
      "ConfigMovementTooLarge",
    );
    await expectRejected(
      propose(risk, id, {
        timelockDelay: { delaySeconds: MIN_TIMELOCK_DELAY_SECONDS - 1 },
      }),
      "TimelockDelayOutOfBounds",
    );

    const after = await asset();
    expect(after.timelock.nextProposalId.eq(id)).to.equal(true);
    expect(after.generation.eq(a.generation)).to.equal(true);
    expect(after.risk).to.deep.equal(a.risk);
  });

  it("proposes, rejects early execution, and cancels", async () => {
    const a = await asset();
    const id = a.timelock.nextProposalId;
    const params = { ...a.risk, ltvBps: a.risk.ltvBps + 100 };

    let data = await onlyEvent(
      await propose(risk, id, { risk: { params } }),
      "configChangeProposed",
    );
    expect(new BN(data.proposalId as BN).eq(id)).to.equal(true);
    expect((data.proposer as PublicKey).equals(risk.publicKey)).to.equal(true);
    expect(Object.keys(data.group as object)).to.deep.equal(["risk"]);
    expect(data.oldValue).to.deep.equal({ risk: { params: a.risk } });
    expect(data.newValue).to.deep.equal({ risk: { params } });
    const proposedAt = Number(data.proposedAt);
    expect(Number(data.earliestExecuteAt)).to.equal(
      proposedAt + a.timelock.delaySeconds,
    );
    expect(Number(data.expiresAt)).to.equal(
      proposedAt + a.timelock.delaySeconds + EXECUTION_GRACE_SECONDS,
    );
    expect(new BN(data.proposalGeneration as BN).eq(a.generation)).to.equal(
      true,
    );

    const stored = await program.account.configProposal.fetch(
      proposalAddress(id),
      "confirmed",
    );
    expect(Object.keys(stored.status)).to.deep.equal(["pending"]);
    expect(stored.newValues.risk).to.deep.equal(params);
    expect(stored.oldValues.risk).to.deep.equal(a.risk);
    expect(stored.proposer.equals(risk.publicKey)).to.equal(true);

    await expectRejected(execute(id), "TimelockNotElapsed");
    for (const signer of [stranger, owner]) {
      await expectRejected(cancel(signer, id), "NotConfigCanceller");
    }

    data = await onlyEvent(await cancel(guardian, id), "configChangeCancelled");
    expect((data.cancelledBy as PublicKey).equals(guardian.publicKey)).to.equal(
      true,
    );
    await expectRejected(execute(id), "ConfigProposalNotPending");
    await expectRejected(cancel(risk, id), "ConfigProposalNotPending");

    const after = await asset();
    expect(after.timelock.nextProposalId.eq(id.addn(1))).to.equal(true);
    expect(after.generation.eq(a.generation)).to.equal(true);
    expect(after.risk).to.deep.equal(a.risk);
  });

  it("lets the risk authority cancel a timelock-delay change", async () => {
    const a = await asset();
    const id = a.timelock.nextProposalId;
    const delaySeconds = a.timelock.delaySeconds + 3_600;
    await onlyEvent(
      await propose(risk, id, { timelockDelay: { delaySeconds } }),
      "configChangeProposed",
    );
    await expectRejected(execute(id), "TimelockNotElapsed");
    const data = await onlyEvent(
      await cancel(risk, id),
      "configChangeCancelled",
    );
    expect((data.cancelledBy as PublicKey).equals(risk.publicKey)).to.equal(
      true,
    );
    const after = await asset();
    expect(after.timelock.delaySeconds).to.equal(a.timelock.delaySeconds);
    expect(after.generation.eq(a.generation)).to.equal(true);
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
