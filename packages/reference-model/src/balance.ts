// Plaintext reference model of the balance layer (Task 5.6; Requirements 5.1
// to 5.12, 3.1 to 3.10, 4.8).
//
// The program keeps every balance encrypted, so tests need an oracle that
// holds the same numbers in the clear. This model is that oracle: free
// balances per asset, both vault totals, the Pending Shields, the Operation
// Intents, the Position Lock and generation, the Viewing Key and epoch, the
// pause state, and the slot. It is the TypeScript twin of the reference model
// in programs/walnut/src/state/balance_checkpoint.rs, and both are checked
// against the shared scenarios in vectors/balance.vectors.
//
// It models rules, not accounts: a callback is delivered by name (honest,
// unauthenticated, wrong binding) and the model decides what the production
// rules do with it. Nothing here touches a network, a key, or a ciphertext.

/** The two shieldable assets. */
export type Asset = "sol" | "usdc";
/** Position lifecycle. */
export type PositionStatus =
  "uninitialized" | "initializing" | "active" | "init_failed";
/** Operation Intent status. */
export type IntentStatus =
  "queued" | "applied" | "zero" | "stale" | "expired" | "abandoned";
/** The Intent kinds the balance layer queues. */
export type IntentKind = "init" | "shield" | "unshield" | "rotate";
/** Pending Shield status. */
export type ShieldStatus = "deposited" | "credited" | "refunded";
/** Which Pending Shield a retry or refund names. */
export type ShieldSelector = "first" | "last";
/** Which Intent a callback is delivered for. */
export type CallbackTarget = "cur" | "first" | "prev";
/** What the callback carries. */
export type CallbackVariant =
  "honest" | "unauth" | "wrong_key" | "wrong_nonce" | "wrong_offset";
/** Why an action was rejected, coarsely. */
export type RejectReason =
  "paused" | "cooldown" | "unauthenticated" | "binding" | "invalid";

/** The result of one action. */
export type Outcome =
  | {
      type:
        | "done"
        | "skipped"
        | "applied"
        | "zero"
        | "stale"
        | "expired"
        | "ignored"
        | "abandoned";
    }
  | { type: "refund" | "approved"; amount: bigint }
  | { type: "rejected"; why: RejectReason };

/** One Operation Intent. */
export interface ModelIntent {
  kind: IntentKind;
  status: IntentStatus;
  generation: bigint;
  decisionExpiry: bigint;
  lockExpiry: bigint;
  shield: number | null;
  asset: Asset;
  amount: bigint;
  newKey: number;
}

/** One Pending Shield. */
export interface ModelShield {
  asset: Asset;
  amount: bigint;
  status: ShieldStatus;
  attempt: bigint;
  /** Index of the active attempt in `intents`. */
  active: number;
}

/** A copy of everything observable. */
export interface BalanceSnapshot {
  slot: bigint;
  status: PositionStatus;
  key: number;
  epoch: bigint;
  generation: bigint;
  lock: number | null;
  /** `[sol, usdc]`; null until the Position is active. */
  free: [bigint, bigint] | null;
  vault: [bigint, bigint];
  wallet: [bigint, bigint];
  out: [bigint, bigint];
  intents: ModelIntent[];
  shields: ModelShield[];
  blocked: { shield: boolean; unshield: boolean };
}

export const U64_MAX = (1n << 64n) - 1n;
/** Slots from queueing to Decision Expiry. */
export const DECISION_SLOTS = 1_500n;
/** Slots from Decision Expiry to Lock expiry. */
export const LOCK_GRACE_SLOTS = 300n;
/** Wait after a guardian pause expires before the guardian may pause again. */
export const GUARDIAN_COOLDOWN_SLOTS = 9_000n;
/** The wallet starts with this many base units of each asset. */
export const WALLET_START = 1n << 100n;
/** The first Viewing Key id a Position registers. */
export const INITIAL_KEY = 1;
/** Pause bits the guardian and governance can set. */
export const PAUSE_SHIELD = 1;
export const PAUSE_UNSHIELD = 2;

const idx = (asset: Asset): 0 | 1 => (asset === "sol" ? 0 : 1);
const done = (): Outcome => ({ type: "done" });
const rejected = (why: RejectReason): Outcome => ({ type: "rejected", why });

function pick(target: CallbackTarget, length: number): number | null {
  if (length === 0) return null;
  if (target === "cur") return length - 1;
  if (target === "first") return 0;
  return length === 1 ? 0 : length - 2;
}

function pickShield(selector: ShieldSelector, length: number): number | null {
  if (length === 0) return null;
  return selector === "first" ? 0 : length - 1;
}

/** The reference model. All amounts are exact `bigint` base units. */
export class BalanceModel {
  slot = 10n;
  generation = 0n;
  status: PositionStatus = "uninitialized";
  key = 0;
  epoch = 0n;
  free: [bigint, bigint] = [0n, 0n];
  lock: number | null = null;
  intents: ModelIntent[] = [];
  shields: ModelShield[] = [];
  vault: [bigint, bigint] = [0n, 0n];
  wallet: [bigint, bigint] = [WALLET_START, WALLET_START];
  out: [bigint, bigint] = [0n, 0n];
  durable = 0;
  guardian = 0;
  guardianExpiry = 0n;

  /** Whether `bit` blocks new queueing at the current slot. */
  blocked(bit: number): boolean {
    const live = this.guardian !== 0 && this.slot < this.guardianExpiry;
    return ((this.durable | (live ? this.guardian : 0)) & bit) !== 0;
  }

  /** A deep copy of the observable state. */
  snapshot(): BalanceSnapshot {
    return {
      slot: this.slot,
      status: this.status,
      key: this.key,
      epoch: this.epoch,
      generation: this.generation,
      lock: this.lock,
      free: this.status === "active" ? [this.free[0], this.free[1]] : null,
      vault: [this.vault[0], this.vault[1]],
      wallet: [this.wallet[0], this.wallet[1]],
      out: [this.out[0], this.out[1]],
      intents: this.intents.map((i) => ({ ...i })),
      shields: this.shields.map((s) => ({ ...s })),
      blocked: {
        shield: this.blocked(PAUSE_SHIELD),
        unshield: this.blocked(PAUSE_UNSHIELD),
      },
    };
  }

  #queue(
    kind: IntentKind,
    shield: number | null,
    asset: Asset,
    amount: bigint,
    newKey: number,
  ): number {
    this.intents.push({
      kind,
      status: "queued",
      generation: this.generation,
      decisionExpiry: this.slot + DECISION_SLOTS,
      lockExpiry: this.slot + DECISION_SLOTS + LOCK_GRACE_SLOTS,
      shield,
      asset,
      amount,
      newKey,
    });
    this.lock = this.intents.length - 1;
    return this.lock;
  }

  #activeIntent(shield: number): ModelIntent {
    const s = this.shields[shield] as ModelShield;
    return this.intents[s.active] as ModelIntent;
  }

  /** The owner creates the Position and queues its zero state. */
  initPosition(): Outcome {
    if (this.status !== "uninitialized") return { type: "skipped" };
    this.status = "initializing";
    this.key = INITIAL_KEY;
    this.generation = 0n;
    this.#queue("init", null, "sol", 0n, 0);
    return done();
  }

  /** The owner retries a failed initialization. */
  retryInit(): Outcome {
    if (this.status !== "init_failed" || this.lock !== null) {
      return rejected("invalid");
    }
    this.status = "initializing";
    this.#queue("init", null, "sol", 0n, 0);
    return done();
  }

  /** The owner shields `amount`: tokens move to the vault, the credit queues. */
  shield(asset: Asset, amount: bigint): Outcome {
    if (this.blocked(PAUSE_SHIELD)) return rejected("paused");
    if (amount === 0n || this.status !== "active" || this.lock !== null) {
      return rejected("invalid");
    }
    const a = idx(asset);
    this.wallet[a] -= amount;
    this.vault[a] += amount;
    const index = this.shields.length;
    const intent = this.#queue("shield", index, asset, amount, 0);
    this.shields.push({
      asset,
      amount,
      status: "deposited",
      attempt: 1n,
      active: intent,
    });
    return done();
  }

  /** A shield into an occupied Pending Shield account never succeeds. */
  shieldOverwrite(selector: ShieldSelector): Outcome {
    if (pickShield(selector, this.shields.length) === null) {
      return { type: "skipped" };
    }
    return rejected(this.blocked(PAUSE_SHIELD) ? "paused" : "invalid");
  }

  /** The owner retries a `deposited` Pending Shield. Not pausable. */
  retryShield(selector: ShieldSelector): Outcome {
    const s = pickShield(selector, this.shields.length);
    if (s === null) return { type: "skipped" };
    const shield = this.shields[s] as ModelShield;
    const usable =
      shield.status === "deposited" &&
      this.#activeIntent(s).status !== "queued" &&
      this.status === "active" &&
      this.lock === null;
    if (!usable) return rejected("invalid");
    const intent = this.#queue("shield", s, shield.asset, shield.amount, 0);
    shield.attempt += 1n;
    shield.active = intent;
    return done();
  }

  /** The owner refunds a `deposited` Pending Shield. Not pausable. */
  refund(selector: ShieldSelector): Outcome {
    const s = pickShield(selector, this.shields.length);
    if (s === null) return { type: "skipped" };
    const shield = this.shields[s] as ModelShield;
    if (
      shield.status !== "deposited" ||
      this.#activeIntent(s).status === "queued"
    ) {
      return rejected("invalid");
    }
    const a = idx(shield.asset);
    shield.status = "refunded";
    this.vault[a] -= shield.amount;
    this.wallet[a] += shield.amount;
    return { type: "refund", amount: shield.amount };
  }

  /** The owner unshields `amount` (the approval comes with the callback). */
  unshield(asset: Asset, amount: bigint): Outcome {
    if (this.blocked(PAUSE_UNSHIELD)) return rejected("paused");
    if (amount === 0n || this.status !== "active" || this.lock !== null) {
      return rejected("invalid");
    }
    this.#queue("unshield", null, asset, amount, 0);
    return done();
  }

  /** The owner rotates the Viewing Key to key id `newKey` (nonzero). */
  rotate(newKey: number): Outcome {
    if (
      this.status !== "active" ||
      this.lock !== null ||
      newKey === 0 ||
      newKey === this.key
    ) {
      return rejected("invalid");
    }
    this.#queue("rotate", null, "sol", 0n, newKey);
    return done();
  }

  /** A rotation request with a wrong epoch expectation is always rejected. */
  rotateStale(): Outcome {
    return rejected("invalid");
  }

  /** Delivers a callback for the targeted Intent. */
  callback(
    target: CallbackTarget = "cur",
    variant: CallbackVariant = "honest",
  ): Outcome {
    const i = pick(target, this.intents.length);
    if (i === null) return { type: "skipped" };
    if (variant === "unauth") return rejected("unauthenticated");
    const intent = this.intents[i] as ModelIntent;
    if (intent.status !== "queued") return { type: "ignored" };
    if (variant !== "honest") return rejected("binding");

    const a = idx(intent.asset);
    let status: IntentStatus;
    if (intent.generation !== this.generation) {
      status = "stale";
    } else if (this.slot > intent.decisionExpiry) {
      status = "expired";
    } else {
      const approves =
        intent.kind === "shield"
          ? this.free[a] + intent.amount <= U64_MAX
          : intent.kind === "unshield"
            ? intent.amount <= this.free[a]
            : true;
      status = approves ? "applied" : "zero";
    }
    intent.status = status;
    this.lock = null;
    if (status === "stale" || status === "expired") {
      if (intent.kind === "init") this.status = "init_failed";
      return { type: status };
    }
    if (status === "zero") return { type: "zero" };

    this.generation += 1n;
    switch (intent.kind) {
      case "init":
        this.status = "active";
        return { type: "applied" };
      case "shield":
        this.free[a] += intent.amount;
        (this.shields[intent.shield as number] as ModelShield).status =
          "credited";
        return { type: "applied" };
      case "unshield":
        this.free[a] -= intent.amount;
        this.vault[a] -= intent.amount;
        this.wallet[a] += intent.amount;
        this.out[a] += intent.amount;
        return { type: "approved", amount: intent.amount };
      case "rotate":
        this.key = intent.newKey;
        this.epoch += 1n;
        return { type: "applied" };
    }
  }

  /** Anyone abandons the newest Intent after its Lock expiry. */
  abandon(): Outcome {
    const i = this.intents.length - 1;
    if (i < 0) return { type: "skipped" };
    const intent = this.intents[i] as ModelIntent;
    if (intent.status !== "queued" || this.slot <= intent.lockExpiry) {
      return rejected("invalid");
    }
    intent.status = "abandoned";
    this.generation += 1n;
    this.lock = null;
    if (intent.kind === "init") this.status = "init_failed";
    return { type: "abandoned" };
  }

  /** Governance sets or clears durable pause bits. */
  setPause(bits: number, paused: boolean): Outcome {
    this.durable = paused ? this.durable | bits : this.durable & ~bits;
    return done();
  }

  /** The guardian pauses `bits` for `duration` slots. */
  guardianPause(bits: number, duration: bigint): Outcome {
    if (this.guardian !== 0 && this.slot < this.guardianExpiry) {
      this.guardian |= bits;
      return done();
    }
    if (
      this.guardianExpiry !== 0n &&
      this.slot < this.guardianExpiry + GUARDIAN_COOLDOWN_SLOTS
    ) {
      return rejected("cooldown");
    }
    this.guardian = bits;
    this.guardianExpiry = this.slot + duration;
    return done();
  }

  /** Governance clears every guardian pause. */
  guardianClear(): Outcome {
    if (this.guardian === 0 && this.guardianExpiry === 0n) {
      return rejected("invalid");
    }
    this.guardian = 0;
    this.guardianExpiry = 0n;
    return done();
  }

  /** Time passes. */
  advance(slots: bigint): Outcome {
    this.slot += slots;
    return done();
  }

  /** Another write moves the Position generation. */
  externalBump(): Outcome {
    if (this.status === "uninitialized") return { type: "skipped" };
    this.generation += 1n;
    return done();
  }
}
