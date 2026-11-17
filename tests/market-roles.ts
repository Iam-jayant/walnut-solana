/**
 * Task 4.2: roles (Requirements 4.3 and 4.4).
 *
 * Proves on localnet, against the Market created by `market-init.ts`, that:
 *   1. the admin transfer takes two steps: only the pending admin accepts, a
 *      re-proposal overwrites the pending admin, and cancel clears it;
 *   2. every role instruction rejects a signer that is not the admin (or the
 *      pending admin for accept);
 *   3. the risk, treasury, and guardian setters reject zero keys, the
 *      Market's own PDAs, unchanged keys, and keys held by another role, and
 *      accept an off-curve multisig PDA;
 *   4. `set_upgrade_authority` accepts only the key the loader names;
 *   5. each change emits its event with the old key, the new key, and a slot.
 *
 * Idempotent: the temporary admins come from fixed seeds, so a run that
 * stopped mid-transfer is repaired in `before` and `after`, and every run ends
 * with the provider wallet as admin, no pending admin, and the risk,
 * treasury, and guardian keys it started with. The provider wallet pays every
 * fee, so the other signers need no lamports.
 */
import * as anchor from "@anchor-lang/core";
import { Program } from "@anchor-lang/core";
import { Keypair, PublicKey } from "@solana/web3.js";
import { Walnut } from "../target/types/walnut";
import { createHash } from "crypto";
import * as fs from "fs";
import * as os from "os";
import { expect } from "chai";

const BPF_LOADER_UPGRADEABLE_ID = new PublicKey(
  "BPFLoaderUpgradeab1e11111111111111111111111",
);
// Squads v4. Only used to derive an off-curve vault-style address.
const SQUADS_PROGRAM_ID = new PublicKey(
  "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf",
);

type ParsedEvent = { name: string; data: Record<string, unknown> };

describe("Walnut Task 4.2: roles", () => {
  anchor.setProvider(anchor.AnchorProvider.env());
  const program = anchor.workspace.Walnut as Program<Walnut>;
  const provider = anchor.getProvider() as anchor.AnchorProvider;
  const connection = provider.connection;
  const owner = readKpJson(`${os.homedir()}/.config/solana/id.json`);
  const stranger = Keypair.generate();
  // Fixed seeds so an interrupted run can hand the admin back.
  const adminA = seededKeypair("walnut-task-4.2-admin-a");
  const adminB = seededKeypair("walnut-task-4.2-admin-b");

  const pda = (...seeds: Buffer[]) =>
    PublicKey.findProgramAddressSync(seeds, program.programId)[0];
  const market = pda(Buffer.from("market"));
  const vaultAuthority = pda(Buffer.from("vault_authority"), market.toBuffer());
  const programData = PublicKey.findProgramAddressSync(
    [program.programId.toBuffer()],
    BPF_LOADER_UPGRADEABLE_ID,
  )[0];
  const parser = new anchor.EventParser(program.programId, program.coder);

  const propose = (signer: Keypair, newAdmin: PublicKey) =>
    program.methods
      .proposeAdmin(newAdmin)
      .accountsPartial({ admin: signer.publicKey, market })
      .signers([signer])
      .rpc({ commitment: "confirmed" });
  const accept = (signer: Keypair) =>
    program.methods
      .acceptAdmin()
      .accountsPartial({ pendingAdmin: signer.publicKey, market })
      .signers([signer])
      .rpc({ commitment: "confirmed" });
  const cancel = (signer: Keypair) =>
    program.methods
      .cancelAdminTransfer()
      .accountsPartial({ admin: signer.publicKey, market })
      .signers([signer])
      .rpc({ commitment: "confirmed" });
  const setters = {
    risk: (signer: Keypair, key: PublicKey) =>
      program.methods
        .setRiskAuthority(key)
        .accountsPartial({ admin: signer.publicKey, market })
        .signers([signer])
        .rpc({ commitment: "confirmed" }),
    treasury: (signer: Keypair, key: PublicKey) =>
      program.methods
        .setTreasuryAuthority(key)
        .accountsPartial({ admin: signer.publicKey, market })
        .signers([signer])
        .rpc({ commitment: "confirmed" }),
    guardian: (signer: Keypair, key: PublicKey) =>
      program.methods
        .setGuardianAuthority(key)
        .accountsPartial({ admin: signer.publicKey, market })
        .signers([signer])
        .rpc({ commitment: "confirmed" }),
  };
  const setUpgrade = (signer: Keypair, key: PublicKey) =>
    program.methods
      .setUpgradeAuthority(key)
      .accountsPartial({
        admin: signer.publicKey,
        market,
        walnutProgram: program.programId,
        programData,
      })
      .signers([signer])
      .rpc({ commitment: "confirmed" });

  async function eventsOf(signature: string): Promise<ParsedEvent[]> {
    const tx = await connection.getTransaction(signature, {
      commitment: "confirmed",
      maxSupportedTransactionVersion: 0,
    });
    expect(tx?.meta?.logMessages, "transaction logs").to.not.equal(undefined);
    return [...parser.parseLogs(tx!.meta!.logMessages!)] as ParsedEvent[];
  }

  async function onlyEvent(
    signature: string,
    name: string,
  ): Promise<Record<string, unknown>> {
    const events = await eventsOf(signature);
    expect(events.map((e) => e.name)).to.deep.equal([name]);
    const data = events[0].data;
    expect((data.market as PublicKey).equals(market)).to.equal(true);
    expect(Number(data.slot)).to.be.greaterThan(0);
    return data;
  }

  const same = (value: unknown, key: PublicKey) =>
    expect((value as PublicKey).equals(key), `${value} = ${key}`).to.equal(
      true,
    );

  // The separated roles at the start, restored in `after` so later files
  // (market-timelock) and reruns see the same keys.
  let original: Record<keyof typeof setters, PublicKey>;

  /** Hands the admin back to the provider wallet and clears a pending admin. */
  async function restoreAdmin() {
    let m = await program.account.market.fetch(market, "confirmed");
    // Repair a run that stopped while a temporary admin held the Market.
    for (const temp of [adminA, adminB]) {
      if (m.admin.equals(temp.publicKey)) {
        await propose(temp, owner.publicKey);
        await accept(owner);
      }
    }
    m = await program.account.market.fetch(market, "confirmed");
    if (!m.pendingAdmin.equals(PublicKey.default)) {
      await cancel(owner);
    }
    m = await program.account.market.fetch(market, "confirmed");
    expect(
      m.admin.equals(owner.publicKey),
      "provider wallet is admin",
    ).to.equal(true);
    expect(m.pendingAdmin.equals(PublicKey.default)).to.equal(true);
  }

  before(async () => {
    const m = await program.account.market.fetchNullable(market);
    expect(m, "run market-init.ts first: the Market must exist").to.not.equal(
      null,
    );
    await restoreAdmin();
    const start = await program.account.market.fetch(market, "confirmed");
    original = {
      risk: start.riskAuthority,
      treasury: start.treasuryAuthority,
      guardian: start.guardianAuthority,
    };
  });

  after(async () => {
    await restoreAdmin();
    if (!original) return;
    const m = await program.account.market.fetch(market, "confirmed");
    const current = {
      risk: m.riskAuthority,
      treasury: m.treasuryAuthority,
      guardian: m.guardianAuthority,
    };
    for (const role of ["risk", "treasury", "guardian"] as const) {
      if (!current[role].equals(original[role])) {
        await setters[role](owner, original[role]);
      }
    }
  });

  it("transfers the admin in two steps and restores it", async () => {
    await expectRejected(propose(stranger, adminA.publicKey), "NotMarketAdmin");
    await expectRejected(accept(stranger), "NoPendingAdmin");
    await expectRejected(cancel(owner), "NoPendingAdmin");
    await expectRejected(propose(owner, PublicKey.default), "InvalidAuthority");
    await expectRejected(propose(owner, market), "InvalidRoleKey");
    await expectRejected(propose(owner, vaultAuthority), "InvalidRoleKey");
    await expectRejected(propose(owner, owner.publicKey), "RoleKeyUnchanged");

    let data = await onlyEvent(
      await propose(owner, adminA.publicKey),
      "adminTransferProposed",
    );
    same(data.admin, owner.publicKey);
    same(data.oldPendingAdmin, PublicKey.default);
    same(data.newPendingAdmin, adminA.publicKey);
    await expectRejected(accept(stranger), "NotPendingAdmin");

    // A second proposal overwrites the first.
    data = await onlyEvent(
      await propose(owner, adminB.publicKey),
      "adminTransferProposed",
    );
    same(data.oldPendingAdmin, adminA.publicKey);
    same(data.newPendingAdmin, adminB.publicKey);
    await expectRejected(accept(adminA), "NotPendingAdmin");

    // Cancel clears it; only the admin may cancel.
    await expectRejected(cancel(stranger), "NotMarketAdmin");
    await expectRejected(cancel(adminB), "NotMarketAdmin");
    data = await onlyEvent(await cancel(owner), "adminTransferCancelled");
    same(data.oldPendingAdmin, adminB.publicKey);
    same(data.newPendingAdmin, PublicKey.default);
    let m = await program.account.market.fetch(market);
    same(m.pendingAdmin, PublicKey.default);
    await expectRejected(accept(adminB), "NoPendingAdmin");

    // Propose and accept.
    await propose(owner, adminB.publicKey);
    data = await onlyEvent(await accept(adminB), "adminTransferAccepted");
    same(data.oldAdmin, owner.publicKey);
    same(data.newAdmin, adminB.publicKey);
    m = await program.account.market.fetch(market);
    same(m.admin, adminB.publicKey);
    same(m.pendingAdmin, PublicKey.default);

    // The old admin lost admin powers, including the role setters.
    await expectRejected(propose(owner, adminA.publicKey), "NotMarketAdmin");
    await expectRejected(
      setters.risk(owner, Keypair.generate().publicKey),
      "NotMarketAdmin",
    );

    // Hand the admin back.
    await propose(adminB, owner.publicKey);
    await accept(owner);
    m = await program.account.market.fetch(market);
    same(m.admin, owner.publicKey);
    same(m.pendingAdmin, PublicKey.default);
  });

  it("sets each separated role through the admin only", async () => {
    for (const [role, set] of Object.entries(setters)) {
      await expectRejected(
        set(stranger, Keypair.generate().publicKey),
        "NotMarketAdmin",
      );
      const before = await program.account.market.fetch(market);
      const old = {
        risk: before.riskAuthority,
        treasury: before.treasuryAuthority,
        guardian: before.guardianAuthority,
      }[role]!;
      const fresh = Keypair.generate().publicKey;
      const data = await onlyEvent(
        await set(owner, fresh),
        "roleAuthorityChanged",
      );
      expect(Object.keys(data.role as object)).to.deep.equal([role]);
      same(data.oldAuthority, old);
      same(data.newAuthority, fresh);
      same(data.changedBy, owner.publicKey);
    }

    // A Squads-style vault PDA is off-curve and still accepted.
    const multisigVault = PublicKey.findProgramAddressSync(
      [
        Buffer.from("multisig"),
        Keypair.generate().publicKey.toBuffer(),
        Buffer.from("vault"),
        Buffer.from([0]),
      ],
      SQUADS_PROGRAM_ID,
    )[0];
    expect(PublicKey.isOnCurve(multisigVault.toBytes())).to.equal(false);
    await setters.guardian(owner, multisigVault);

    const m = await program.account.market.fetch(market);
    same(m.guardianAuthority, multisigVault);
    await expectRejected(
      setters.treasury(owner, m.riskAuthority),
      "RoleKeyNotDistinct",
    );
    await expectRejected(
      setters.risk(owner, m.riskAuthority),
      "RoleKeyUnchanged",
    );
    await expectRejected(
      setters.risk(owner, PublicKey.default),
      "InvalidAuthority",
    );
    await expectRejected(setters.risk(owner, market), "InvalidRoleKey");
    await expectRejected(
      setters.guardian(owner, vaultAuthority),
      "InvalidRoleKey",
    );
  });

  it("records only the upgrade authority the loader names", async () => {
    const m = await program.account.market.fetch(market);
    same(m.upgradeAuthority, owner.publicKey);
    await expectRejected(
      setUpgrade(stranger, Keypair.generate().publicKey),
      "NotMarketAdmin",
    );
    await expectRejected(
      setUpgrade(owner, Keypair.generate().publicKey),
      "UpgradeAuthorityMismatch",
    );
    await expectRejected(
      setUpgrade(owner, owner.publicKey),
      "RoleKeyUnchanged",
    );
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
