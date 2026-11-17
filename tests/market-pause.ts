/**
 * Task 4.3: granular pause controls (Requirements 4.5, 4.7, 4.8, and 4.9).
 *
 * Proves on localnet, against the Market created by `market-init.ts`, that:
 *   1. only the guardian sets guardian pauses and only the risk authority sets
 *      durable pauses, renews, or clears guardian pauses;
 *   2. repay and collateral addition bits can never be set, and empty or
 *      undefined masks and out-of-range durations are rejected;
 *   3. a guardian pause expires on its own, the guardian cannot extend it,
 *      and after a natural expiry the guardian must wait for the cooldown;
 *   4. governance renews and clears guardian pauses and sets and clears
 *      durable pauses;
 *   5. every change emits `PauseUpdated` with the change, actor, mask, old and
 *      new state, and slot.
 *
 * Idempotent: the risk and guardian keys come from fixed seeds and are
 * installed by the provider wallet (the admin) in `before`. Leftover pauses
 * are cleared in `before` and `after`, so the Market ends unpaused. The
 * provider wallet pays every fee.
 */
import * as anchor from "@anchor-lang/core";
import { BN, Program } from "@anchor-lang/core";
import { Keypair, PublicKey } from "@solana/web3.js";
import { Walnut } from "../target/types/walnut";
import { createHash } from "crypto";
import * as fs from "fs";
import * as os from "os";
import { expect } from "chai";

// `market::pause_flags`.
const FLAGS = {
  supply: 1 << 0,
  withdrawSupply: 1 << 1,
  borrow: 1 << 2,
  repay: 1 << 3,
  addCollateral: 1 << 4,
  withdrawCollateral: 1 << 5,
  shield: 1 << 6,
  unshield: 1 << 7,
  liquidationStart: 1 << 8,
  liquidationComplete: 1 << 9,
};
const RISK_INCREASING =
  FLAGS.supply | FLAGS.borrow | FLAGS.withdrawCollateral | FLAGS.shield;
const WITHDRAWAL = FLAGS.withdrawSupply | FLAGS.unshield;
const SETTLEMENT = FLAGS.liquidationStart | FLAGS.liquidationComplete;
const PAUSABLE = RISK_INCREASING | WITHDRAWAL | SETTLEMENT;
// `market::GUARDIAN_MAX_PAUSE_SLOTS`.
const GUARDIAN_MAX_PAUSE_SLOTS = 216_000;

type ParsedEvent = { name: string; data: Record<string, unknown> };

describe("Walnut Task 4.3: pause controls", () => {
  anchor.setProvider(anchor.AnchorProvider.env());
  const program = anchor.workspace.Walnut as Program<Walnut>;
  const provider = anchor.getProvider() as anchor.AnchorProvider;
  const connection = provider.connection;
  const owner = readKpJson(`${os.homedir()}/.config/solana/id.json`);
  const stranger = Keypair.generate();
  const risk = seededKeypair("walnut-task-4.3-risk");
  const guardian = seededKeypair("walnut-task-4.3-guardian");

  const pda = (...seeds: Buffer[]) =>
    PublicKey.findProgramAddressSync(seeds, program.programId)[0];
  const market = pda(Buffer.from("market"));
  const assetState = pda(Buffer.from("asset_state"), market.toBuffer());
  const parser = new anchor.EventParser(program.programId, program.coder);
  const accounts = (signer: Keypair) => ({
    authority: signer.publicKey,
    market,
    assetState,
  });

  const guardianPause = (signer: Keypair, flags: number, duration: number) =>
    program.methods
      .guardianPause(flags, new BN(duration))
      .accountsPartial(accounts(signer))
      .signers([signer])
      .rpc({ commitment: "confirmed" });
  const setPause = (signer: Keypair, flags: number, paused: boolean) =>
    program.methods
      .setPause(flags, paused)
      .accountsPartial(accounts(signer))
      .signers([signer])
      .rpc({ commitment: "confirmed" });
  const renew = (signer: Keypair, expiry: BN) =>
    program.methods
      .renewGuardianPause(expiry)
      .accountsPartial(accounts(signer))
      .signers([signer])
      .rpc({ commitment: "confirmed" });
  const clear = (signer: Keypair) =>
    program.methods
      .clearGuardianPause()
      .accountsPartial(accounts(signer))
      .signers([signer])
      .rpc({ commitment: "confirmed" });

  const pauses = async () =>
    (await program.account.assetState.fetch(assetState, "confirmed")).pauses;

  async function onlyEvent(
    signature: string,
    change: string,
    actor: PublicKey,
  ): Promise<Record<string, unknown>> {
    const tx = await connection.getTransaction(signature, {
      commitment: "confirmed",
      maxSupportedTransactionVersion: 0,
    });
    expect(tx?.meta?.logMessages, "transaction logs").to.not.equal(undefined);
    const events = [
      ...parser.parseLogs(tx!.meta!.logMessages!),
    ] as ParsedEvent[];
    expect(events.map((e) => e.name)).to.deep.equal(["pauseUpdated"]);
    const data = events[0].data;
    expect((data.market as PublicKey).equals(market)).to.equal(true);
    expect((data.assetState as PublicKey).equals(assetState)).to.equal(true);
    expect((data.actor as PublicKey).equals(actor)).to.equal(true);
    expect(Object.keys(data.change as object)).to.deep.equal([change]);
    expect(Number(data.slot)).to.be.greaterThan(0);
    return data;
  }

  async function resetPauses() {
    const p = await pauses();
    if (p.guardianFlags !== 0 || !p.guardianExpirySlot.isZero()) {
      await clear(risk);
    }
    if (p.flags !== 0) {
      await setPause(risk, PAUSABLE, false);
    }
    const after = await pauses();
    expect(after.flags).to.equal(0);
    expect(after.guardianFlags).to.equal(0);
    expect(after.guardianExpirySlot.toNumber()).to.equal(0);
  }

  async function waitForSlot(target: number) {
    while ((await connection.getSlot("confirmed")) < target) {
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
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
    await resetPauses();
  });

  after(async () => {
    await resetPauses();
  });

  it("rejects unauthorized signers and invalid requests", async () => {
    for (const signer of [stranger, risk, owner]) {
      await expectRejected(
        guardianPause(signer, FLAGS.borrow, 10),
        "NotGuardianAuthority",
      );
    }
    for (const signer of [stranger, guardian, owner]) {
      await expectRejected(
        setPause(signer, FLAGS.borrow, true),
        "NotRiskAuthority",
      );
      await expectRejected(clear(signer), "NotRiskAuthority");
      await expectRejected(renew(signer, new BN(1)), "NotRiskAuthority");
    }
    for (const rescue of [FLAGS.repay, FLAGS.addCollateral]) {
      await expectRejected(
        guardianPause(guardian, rescue, 10),
        "PauseNotAllowed",
      );
      await expectRejected(setPause(risk, rescue, true), "PauseNotAllowed");
    }
    for (const bad of [0, 1 << 10]) {
      await expectRejected(
        guardianPause(guardian, bad, 10),
        "InvalidPauseFlags",
      );
      await expectRejected(setPause(risk, bad, false), "InvalidPauseFlags");
    }
    for (const duration of [0, GUARDIAN_MAX_PAUSE_SLOTS + 1]) {
      await expectRejected(
        guardianPause(guardian, FLAGS.borrow, duration),
        "InvalidPauseDuration",
      );
    }
    await expectRejected(clear(risk), "NoActiveGuardianPause");
    await expectRejected(renew(risk, new BN(1)), "NoActiveGuardianPause");
    const p = await pauses();
    expect([p.flags, p.guardianFlags]).to.deep.equal([0, 0]);
  });

  it("expires guardian pauses; only governance renews and clears", async () => {
    let data = await onlyEvent(
      await guardianPause(guardian, RISK_INCREASING, 15),
      "guardianPause",
      guardian.publicKey,
    );
    expect(data.flags).to.equal(RISK_INCREASING);
    expect(data.oldGuardianFlags).to.equal(0);
    expect(data.newGuardianFlags).to.equal(RISK_INCREASING);
    const expiry = Number(data.newGuardianExpirySlot);
    expect(expiry).to.equal(Number(data.slot) + 15);

    // While active, the guardian adds bits but cannot extend the expiry.
    data = await onlyEvent(
      await guardianPause(guardian, SETTLEMENT, GUARDIAN_MAX_PAUSE_SLOTS),
      "guardianPause",
      guardian.publicKey,
    );
    expect(data.newGuardianFlags).to.equal(RISK_INCREASING | SETTLEMENT);
    expect(Number(data.newGuardianExpirySlot)).to.equal(expiry);

    // After the expiry the guardian must wait for the cooldown.
    await waitForSlot(expiry);
    await expectRejected(
      guardianPause(guardian, FLAGS.borrow, 10),
      "GuardianPauseCooldown",
    );
    await expectRejected(
      renew(risk, new BN(expiry + 100)),
      "NoActiveGuardianPause",
    );

    // A governance clear resets the guardian state and ends the cooldown.
    data = await onlyEvent(await clear(risk), "guardianClear", risk.publicKey);
    expect(data.newGuardianFlags).to.equal(0);
    expect(Number(data.newGuardianExpirySlot)).to.equal(0);

    data = await onlyEvent(
      await guardianPause(guardian, WITHDRAWAL, 1_000),
      "guardianPause",
      guardian.publicKey,
    );
    const second = Number(data.newGuardianExpirySlot);
    await expectRejected(renew(risk, new BN(second)), "InvalidPauseExpiry");
    data = await onlyEvent(
      await renew(risk, new BN(second + 500)),
      "guardianRenew",
      risk.publicKey,
    );
    expect(Number(data.oldGuardianExpirySlot)).to.equal(second);
    expect(Number(data.newGuardianExpirySlot)).to.equal(second + 500);
    await onlyEvent(await clear(risk), "guardianClear", risk.publicKey);
  });

  it("sets and clears durable governance pauses", async () => {
    let data = await onlyEvent(
      await setPause(risk, SETTLEMENT | FLAGS.unshield, true),
      "governancePause",
      risk.publicKey,
    );
    expect(data.oldFlags).to.equal(0);
    expect(data.newFlags).to.equal(SETTLEMENT | FLAGS.unshield);
    data = await onlyEvent(
      await setPause(risk, FLAGS.liquidationStart, false),
      "governanceUnpause",
      risk.publicKey,
    );
    expect(data.newFlags).to.equal(FLAGS.liquidationComplete | FLAGS.unshield);
    expect((await pauses()).flags).to.equal(
      FLAGS.liquidationComplete | FLAGS.unshield,
    );
    data = await onlyEvent(
      await setPause(risk, PAUSABLE, false),
      "governanceUnpause",
      risk.publicKey,
    );
    expect(data.newFlags).to.equal(0);
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
