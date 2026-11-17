// Typed terminal outcomes for an Operation Intent (Requirements 3.4, 14.8).
//
// The tags mirror programs/walnut/src/operation/intent.rs EXACTLY:
//   IntentStatus: Queued 0, Applied 1, Zero 2, Failed 3, Stale 4, Expired 5,
//                 Abandoned 6
//   OperationKind: Probe 0, PositionInit 1, Shield 2, Unshield 3, Supply 4,
//                  WithdrawSupply 5, Borrow 6, Repay 7, Health 8, Liquidation 9
// Both Rust enums are append-only and the tag is the Borsh variant index, so
// these tables must only ever gain entries at the end.
//
// `TimedOut` is client-only: it means the SDK stopped waiting. It has no
// on-chain tag, because the Intent is still `Queued` (or its state is unknown)
// until a callback or an abandonment lands.

/** On-chain `IntentStatus` tags. */
export const INTENT_STATUS_TAG = {
  Queued: 0,
  Applied: 1,
  Zero: 2,
  Failed: 3,
  Stale: 4,
  Expired: 5,
  Abandoned: 6,
} as const;

/** On-chain `OperationKind` tags. */
export const OPERATION_KIND_TAG = {
  Probe: 0,
  PositionInit: 1,
  Shield: 2,
  Unshield: 3,
  Supply: 4,
  WithdrawSupply: 5,
  Borrow: 6,
  Repay: 7,
  Health: 8,
  Liquidation: 9,
} as const;

export type IntentStatusName = keyof typeof INTENT_STATUS_TAG;
export type OperationKindName = keyof typeof OPERATION_KIND_TAG;

/** Names of the terminal outcomes that exist on chain (everything but Queued). */
export type OnChainOutcomeName = Exclude<IntentStatusName, "Queued">;

/** Every terminal outcome the SDK reports. */
export type OutcomeName = OnChainOutcomeName | "TimedOut";

/** A terminal outcome that is recorded on chain, with its status tag. */
export type OnChainOutcome = {
  [K in OnChainOutcomeName]: {
    readonly status: K;
    readonly tag: (typeof INTENT_STATUS_TAG)[K];
  };
}[OnChainOutcomeName];

/** The client-only outcome: the SDK stopped waiting. */
export interface TimedOutOutcome {
  readonly status: "TimedOut";
  readonly tag: null;
}

/** Discriminated union of every terminal outcome. */
export type OperationOutcome = OnChainOutcome | TimedOutOutcome;

/** Raised for a tag that is not a known `IntentStatus` or `OperationKind`. */
export class UnknownTagError extends Error {
  readonly tag: number;
  constructor(what: "status" | "kind", tag: number) {
    super(`unknown ${what} tag: ${tag}`);
    this.name = "UnknownTagError";
    this.tag = tag;
  }
}

/** Raised when `Queued` (tag 0) is used where a terminal status is required. */
export class NonTerminalStatusError extends Error {
  constructor() {
    super("status tag 0 (Queued) is not a terminal outcome");
    this.name = "NonTerminalStatusError";
  }
}

const OUTCOME_BY_TAG: ReadonlyMap<number, OnChainOutcome> = new Map(
  (Object.keys(INTENT_STATUS_TAG) as IntentStatusName[])
    .filter((name): name is OnChainOutcomeName => name !== "Queued")
    .map((name) => [
      INTENT_STATUS_TAG[name],
      { status: name, tag: INTENT_STATUS_TAG[name] } as OnChainOutcome,
    ]),
);

/** The `TimedOut` outcome (a shared constant). */
export const TIMED_OUT: TimedOutOutcome = Object.freeze({
  status: "TimedOut",
  tag: null,
});

/** Builds the on-chain outcome with the given name. */
export function outcomeOf(name: OnChainOutcomeName): OnChainOutcome {
  return fromStatusTag(INTENT_STATUS_TAG[name]);
}

/**
 * Converts an on-chain `IntentStatus` tag to a terminal outcome.
 *
 * Throws {@link NonTerminalStatusError} for `Queued` (0) and
 * {@link UnknownTagError} for any other value that is not a status tag.
 */
export function fromStatusTag(tag: number): OnChainOutcome {
  if (tag === INTENT_STATUS_TAG.Queued) throw new NonTerminalStatusError();
  const found = Number.isInteger(tag) ? OUTCOME_BY_TAG.get(tag) : undefined;
  if (!found) throw new UnknownTagError("status", tag);
  return found;
}

/**
 * Converts an on-chain outcome back to its `IntentStatus` tag.
 *
 * `TimedOut` has no on-chain tag, so it is rejected (statically and at run
 * time).
 */
export function toStatusTag(outcome: OnChainOutcome): number {
  if ((outcome as OperationOutcome).status === "TimedOut") {
    throw new UnknownTagError("status", Number.NaN);
  }
  return INTENT_STATUS_TAG[outcome.status];
}

/** Converts an `OperationKind` tag to its name. */
export function operationKindFromTag(tag: number): OperationKindName {
  for (const name of Object.keys(OPERATION_KIND_TAG) as OperationKindName[]) {
    if (OPERATION_KIND_TAG[name] === tag) return name;
  }
  throw new UnknownTagError("kind", tag);
}

/** Compile-time exhaustiveness guard for `switch` statements. */
export function assertNever(value: never): never {
  throw new Error(`unhandled variant: ${JSON.stringify(value)}`);
}

/** One handler per outcome; omitting any is a compile error. */
export type OutcomeHandlers<R> = {
  [K in OutcomeName]: (outcome: Extract<OperationOutcome, { status: K }>) => R;
};

/** Exhaustively dispatches on an outcome. */
export function matchOutcome<R>(
  outcome: OperationOutcome,
  handlers: OutcomeHandlers<R>,
): R {
  switch (outcome.status) {
    case "Applied":
      return handlers.Applied(outcome);
    case "Zero":
      return handlers.Zero(outcome);
    case "Failed":
      return handlers.Failed(outcome);
    case "Stale":
      return handlers.Stale(outcome);
    case "Expired":
      return handlers.Expired(outcome);
    case "Abandoned":
      return handlers.Abandoned(outcome);
    case "TimedOut":
      return handlers.TimedOut(outcome);
    default:
      return assertNever(outcome);
  }
}

/** What the user should do next. */
export type RecoveryAction =
  /** Nothing to do. */
  | "none"
  /** No value moved. It is safe to submit the operation again. */
  | "retry"
  /** Read the Operation Intent on chain, then reclaim fee or rent as the payer. */
  | "check-onchain-then-reclaim";

/** User-facing description of an outcome. */
export interface OutcomeDescription {
  readonly message: string;
  readonly recovery: RecoveryAction;
  /** True when submitting the operation again cannot double-apply it. */
  readonly safeToRetry: boolean;
}

const DESCRIPTIONS: { [K in OutcomeName]: OutcomeDescription } = {
  Applied: {
    message: "The operation was applied.",
    recovery: "none",
    safeToRetry: false,
  },
  Zero: {
    message:
      "The computation succeeded but approved no value (a precondition, cap, health, or rounding check did not pass). No funds moved.",
    recovery: "retry",
    safeToRetry: true,
  },
  Failed: {
    message:
      "The computation failed and the failure was recorded on chain. No funds moved.",
    recovery: "retry",
    safeToRetry: true,
  },
  Stale: {
    message:
      "Positions or market state changed while the computation ran, so its result was discarded. No funds moved.",
    recovery: "retry",
    safeToRetry: true,
  },
  Expired: {
    message:
      "The result arrived after the decision expiry and was discarded. No funds moved.",
    recovery: "retry",
    safeToRetry: true,
  },
  Abandoned: {
    message:
      "No valid callback arrived before the lock expired, so the operation was abandoned. A late result cannot change state. Check the Operation Intent on chain, then reclaim the computation fee or rent with the original payer.",
    recovery: "check-onchain-then-reclaim",
    safeToRetry: true,
  },
  TimedOut: {
    message:
      "The SDK stopped waiting. The operation may still complete. Check the Operation Intent on chain, then reclaim the computation fee or rent with the original payer before retrying.",
    recovery: "check-onchain-then-reclaim",
    safeToRetry: false,
  },
};

/** Returns the user-facing message and recovery hint for an outcome. */
export function describeOutcome(outcome: OperationOutcome): OutcomeDescription {
  return DESCRIPTIONS[outcome.status];
}
