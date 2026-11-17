// Unit and property tests of the balance reference model (Task 5.6). The
// property run mirrors the Rust checkpoint: random action sequences, with the
// invariants checked on the model after every action.
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  BalanceModel,
  GUARDIAN_COOLDOWN_SLOTS,
  PAUSE_SHIELD,
  PAUSE_UNSHIELD,
  U64_MAX,
  WALLET_START,
  type Asset,
  type BalanceSnapshot,
  type Outcome,
  type ShieldSelector,
} from "../src/index.ts";

const active = (): BalanceModel => {
  const model = new BalanceModel();
  model.initPosition();
  model.callback();
  return model;
};

test("shield credits the free balance once and keeps the vault backed", () => {
  const model = active();
  assert.deepEqual(model.shield("sol", 100n), { type: "done" });
  assert.deepEqual(model.callback(), { type: "applied" });
  assert.deepEqual(model.snapshot().free, [100n, 0n]);
  assert.deepEqual(model.snapshot().vault, [100n, 0n]);
  // A duplicate callback is ignored.
  assert.deepEqual(model.callback(), { type: "ignored" });
  assert.deepEqual(model.snapshot().free, [100n, 0n]);
});

test("a credit that would overflow u64 stays deposited and is refundable once", () => {
  const model = active();
  model.shield("usdc", U64_MAX);
  model.callback();
  model.shield("usdc", 1n);
  assert.deepEqual(model.callback(), { type: "zero" });
  assert.equal(model.snapshot().shields[1]?.status, "deposited");
  assert.deepEqual(model.refund("last"), { type: "refund", amount: 1n });
  assert.deepEqual(model.refund("last"), { type: "rejected", why: "invalid" });
  assert.deepEqual(model.snapshot().vault, [0n, U64_MAX]);
});

test("an oversized unshield returns zero and the exact balance leaves zero", () => {
  const model = active();
  model.shield("sol", 50n);
  model.callback();
  model.unshield("sol", 51n);
  assert.deepEqual(model.callback(), { type: "zero" });
  assert.deepEqual(model.snapshot().free, [50n, 0n]);
  model.unshield("sol", 50n);
  assert.deepEqual(model.callback(), { type: "approved", amount: 50n });
  assert.deepEqual(model.snapshot().free, [0n, 0n]);
  assert.deepEqual(model.snapshot().vault, [0n, 0n]);
  assert.deepEqual(model.snapshot().out, [50n, 0n]);
});

test("rotation bumps the epoch only when applied and keeps the balances", () => {
  const model = active();
  model.shield("sol", 9n);
  model.callback();
  model.rotate(2);
  model.advance(2_000n);
  assert.deepEqual(model.callback(), { type: "expired" });
  assert.equal(model.epoch, 0n);
  assert.equal(model.key, 1);
  model.rotate(2);
  assert.deepEqual(model.callback(), { type: "applied" });
  assert.equal(model.epoch, 1n);
  assert.equal(model.key, 2);
  assert.deepEqual(model.snapshot().free, [9n, 0n]);
  assert.deepEqual(model.rotate(2), { type: "rejected", why: "invalid" });
  assert.deepEqual(model.rotateStale(), { type: "rejected", why: "invalid" });
});

test("pause blocks only new shield and unshield; an expired guardian pause blocks nothing", () => {
  const model = active();
  model.shield("sol", 5n);
  model.advance(2_000n);
  model.callback();
  model.guardianPause(PAUSE_SHIELD | PAUSE_UNSHIELD, 300n);
  assert.deepEqual(model.shield("sol", 1n), {
    type: "rejected",
    why: "paused",
  });
  assert.deepEqual(model.unshield("sol", 1n), {
    type: "rejected",
    why: "paused",
  });
  assert.equal(model.retryShield("last").type, "done");
  assert.equal(model.callback().type, "applied");
  assert.equal(model.rotate(3).type, "done");
  assert.equal(model.callback().type, "applied");
  model.advance(300n);
  assert.equal(model.shield("sol", 1n).type, "done");
  // The guardian waits out the cooldown before pausing again.
  model.callback();
  assert.deepEqual(model.guardianPause(PAUSE_SHIELD, 10n), {
    type: "rejected",
    why: "cooldown",
  });
  model.advance(GUARDIAN_COOLDOWN_SLOTS);
  assert.equal(model.guardianPause(PAUSE_SHIELD, 10n).type, "done");
});

test("failed and late callbacks change nothing", () => {
  const model = active();
  model.shield("usdc", 10n);
  const before = model.snapshot();
  for (const variant of [
    "unauth",
    "wrong_key",
    "wrong_nonce",
    "wrong_offset",
  ] as const) {
    assert.equal(model.callback("cur", variant).type, "rejected");
  }
  assert.deepEqual(model.snapshot(), before);
  model.externalBump();
  assert.deepEqual(model.callback(), { type: "stale" });
  model.advance(2_000n);
  model.retryShield("last");
  model.advance(2_000n);
  assert.deepEqual(model.abandon(), { type: "abandoned" });
  assert.deepEqual(model.callback(), { type: "ignored" });
  assert.deepEqual(model.callback("first"), { type: "ignored" });
  assert.deepEqual(model.snapshot().free, [0n, 0n]);
  assert.deepEqual(model.snapshot().vault, [0n, 10n]);
});

// ---- Property test -------------------------------------------------------------

/** Deterministic linear-congruential sampler (no external library). */
class Lcg {
  #state: bigint;
  constructor(seed: bigint) {
    this.#state = seed;
  }
  next(): bigint {
    this.#state =
      (this.#state * 6364136223846793005n + 1442695040888963407n) &
      ((1n << 64n) - 1n);
    return this.#state >> 33n;
  }
  below(n: number): number {
    return Number(this.next() % BigInt(n));
  }
  pick<T>(items: readonly T[]): T {
    return items[this.below(items.length)] as T;
  }
}

const AMOUNTS = [1n, 7n, 100n, 500n, 1_000n, U64_MAX, U64_MAX - 1n, 1n << 63n];

interface Step {
  name: string;
  outcome: Outcome;
}

function randomStep(model: BalanceModel, rng: Lcg): Step {
  const asset: Asset = rng.pick(["sol", "usdc"] as const);
  const selector: ShieldSelector = rng.pick(["first", "last", "last"] as const);
  const roll = rng.below(10);
  if (model.status === "uninitialized") {
    return { name: "init", outcome: model.initPosition() };
  }
  if (model.lock !== null && roll < 6) {
    return { name: "callback", outcome: model.callback() };
  }
  switch (rng.below(20)) {
    case 0:
      return { name: "retry_init", outcome: model.retryInit() };
    case 1:
    case 2:
    case 3:
    case 4:
      return {
        name: "shield",
        outcome: model.shield(asset, rng.pick(AMOUNTS)),
      };
    case 5:
    case 6:
    case 7: {
      const free = model.free[asset === "sol" ? 0 : 1];
      const amount = rng.pick([free, free + 1n, free / 2n + 1n, U64_MAX, 1n]);
      return { name: "unshield", outcome: model.unshield(asset, amount) };
    }
    case 8:
      return { name: "rotate", outcome: model.rotate(1 + rng.below(3)) };
    case 9:
      return { name: "refund", outcome: model.refund(selector) };
    case 10:
      return { name: "retry_shield", outcome: model.retryShield(selector) };
    case 11:
      return { name: "abandon", outcome: model.abandon() };
    case 12:
      return {
        name: "advance",
        outcome: model.advance(rng.pick([10n, 2_000n])),
      };
    case 13:
      return { name: "bump", outcome: model.externalBump() };
    case 14:
      return {
        name: "pause",
        outcome: model.setPause(
          rng.pick([PAUSE_SHIELD, PAUSE_UNSHIELD]),
          rng.below(2) === 0,
        ),
      };
    case 15:
      return {
        name: "guardian",
        outcome: model.guardianPause(PAUSE_SHIELD | PAUSE_UNSHIELD, 300n),
      };
    case 16:
      return {
        name: "callback",
        outcome: model.callback(
          rng.pick(["cur", "first", "prev"] as const),
          rng.pick(["honest", "unauth", "wrong_key"] as const),
        ),
      };
    case 17:
      return { name: "overwrite", outcome: model.shieldOverwrite(selector) };
    default:
      return { name: "callback", outcome: model.callback() };
  }
}

const sum = (values: bigint[]): bigint => values.reduce((a, b) => a + b, 0n);

function checkInvariants(
  step: Step,
  before: BalanceSnapshot,
  after: BalanceSnapshot,
): void {
  const where = `after ${step.name} -> ${JSON.stringify(step.outcome, (_, v: unknown) => (typeof v === "bigint" ? v.toString() : v))}`;
  for (const a of [0, 1] as const) {
    const of = (status: string): bigint =>
      sum(
        after.shields
          .filter(
            (s) =>
              s.asset === (a === 0 ? "sol" : "usdc") && s.status === status,
          )
          .map((s) => s.amount),
      );
    // VAULT CONSERVATION
    assert.equal(
      after.vault[a],
      of("credited") + of("deposited") - after.out[a],
      `vault conservation, asset ${a} ${where}`,
    );
    assert.equal(
      after.wallet[a] + after.vault[a],
      WALLET_START,
      `no token created or destroyed ${where}`,
    );
    // The encrypted claim is fully backed.
    if (after.free !== null) {
      assert.equal(
        after.free[a] + after.out[a],
        of("credited"),
        `balance backing, asset ${a} ${where}`,
      );
    }
  }
  // PENDING SHIELD EXCLUSIVITY: legal transitions only; finals never change.
  before.shields.forEach((old, i) => {
    const now = after.shields[i];
    assert.ok(now, `a shield disappeared ${where}`);
    if (old.status !== now.status) {
      assert.equal(old.status, "deposited", `final shield changed ${where}`);
      assert.ok(
        (now.status === "credited" && step.outcome.type === "applied") ||
          (now.status === "refunded" && step.outcome.type === "refund"),
        `illegal shield transition ${where}`,
      );
    }
  });
  // FAILURE / LATE CALLBACK NEUTRALITY and the oversized rule.
  const neutral = [
    "zero",
    "stale",
    "expired",
    "ignored",
    "abandoned",
    "skipped",
    "rejected",
  ];
  if (neutral.includes(step.outcome.type)) {
    assert.deepEqual(after.free, before.free, `balances changed ${where}`);
    assert.deepEqual(after.vault, before.vault, `vault changed ${where}`);
    assert.deepEqual(after.out, before.out, `out changed ${where}`);
    assert.equal(after.epoch, before.epoch, `epoch changed ${where}`);
    assert.equal(after.key, before.key, `key changed ${where}`);
  }
  // ROTATION: epoch moves only on an applied rotation, by one.
  const rotated =
    step.outcome.type === "applied" && after.epoch === before.epoch + 1n;
  assert.ok(
    after.epoch === before.epoch || rotated,
    `epoch moved without an applied rotation ${where}`,
  );
  if (after.epoch !== before.epoch) {
    assert.deepEqual(
      after.free,
      before.free,
      `rotation changed balances ${where}`,
    );
  }
  // Terminal Intents never change; at most one is queued; the Lock tracks it.
  before.intents.forEach((old, i) => {
    if (old.status !== "queued") {
      assert.equal(
        after.intents[i]?.status,
        old.status,
        `terminal Intent changed ${where}`,
      );
    }
  });
  const queued = after.intents.filter((i) => i.status === "queued").length;
  assert.equal(queued, after.lock === null ? 0 : 1, `lock safety ${where}`);
}

test("prop: random sequences keep every balance-layer invariant", () => {
  const rng = new Lcg(0x5eed_ba1an);
  const seen = new Set<string>();
  let actions = 0;
  for (let n = 0; n < 3_000; n++) {
    const model = new BalanceModel();
    const length = 1 + rng.below(40);
    for (let i = 0; i < length; i++) {
      const before = model.snapshot();
      const step = randomStep(model, rng);
      checkInvariants(step, before, model.snapshot());
      seen.add(`${step.name}:${step.outcome.type}`);
      actions++;
    }
  }
  assert.ok(actions > 50_000);
  for (const outcome of [
    "callback:applied",
    "callback:zero",
    "callback:approved",
    "callback:stale",
    "callback:expired",
    "callback:ignored",
    "refund:refund",
    "abandon:abandoned",
    "shield:rejected",
  ]) {
    assert.ok(seen.has(outcome), `the random run never produced ${outcome}`);
  }
});

test("a defective model is caught by the invariant checks", () => {
  const model = active();
  model.shield("sol", 10n);
  model.callback();
  // A refund that forgets to debit the vault must violate conservation.
  model.shield("usdc", 5n);
  model.advance(2_000n);
  model.callback();
  const mid = model.snapshot();
  const step = { name: "refund", outcome: model.refund("last") };
  const after = model.snapshot();
  after.vault[1] += 5n;
  after.wallet[1] -= 5n;
  assert.throws(() => checkInvariants(step, mid, after));
});
