// Unit tests for typed terminal outcomes (Task 3.4).

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  describeOutcome,
  fromStatusTag,
  INTENT_STATUS_TAG,
  matchOutcome,
  NonTerminalStatusError,
  OPERATION_KIND_TAG,
  operationKindFromTag,
  outcomeOf,
  TIMED_OUT,
  toStatusTag,
  UnknownTagError,
  type OnChainOutcome,
  type OperationOutcome,
} from "../src/index.ts";

test("status tags mirror the Rust IntentStatus enum", () => {
  assert.deepEqual(INTENT_STATUS_TAG, {
    Queued: 0,
    Applied: 1,
    Zero: 2,
    Failed: 3,
    Stale: 4,
    Expired: 5,
    Abandoned: 6,
  });
});

test("kind tags mirror the Rust OperationKind enum", () => {
  assert.deepEqual(
    Object.entries(OPERATION_KIND_TAG).map(([name]) => name),
    [
      "Probe",
      "PositionInit",
      "Shield",
      "Unshield",
      "Supply",
      "WithdrawSupply",
      "Borrow",
      "Repay",
      "Health",
      "Liquidation",
    ],
  );
  Object.entries(OPERATION_KIND_TAG).forEach(([name, tag], index) => {
    assert.equal(tag, index);
    assert.equal(operationKindFromTag(tag), name);
  });
  assert.throws(() => operationKindFromTag(10), UnknownTagError);
});

test("fromStatusTag and toStatusTag are inverse for every terminal tag", () => {
  const expected = [
    "Applied",
    "Zero",
    "Failed",
    "Stale",
    "Expired",
    "Abandoned",
  ];
  expected.forEach((status, index) => {
    const tag = index + 1;
    const outcome = fromStatusTag(tag);
    assert.equal(outcome.status, status);
    assert.equal(outcome.tag, tag);
    assert.equal(toStatusTag(outcome), tag);
  });
});

test("Queued is not terminal and unknown tags are rejected", () => {
  assert.throws(() => fromStatusTag(0), NonTerminalStatusError);
  for (const tag of [7, 255, -1, 1.5, Number.NaN]) {
    assert.throws(() => fromStatusTag(tag), UnknownTagError);
  }
});

test("TimedOut has no on-chain tag", () => {
  assert.equal(TIMED_OUT.tag, null);
  assert.throws(
    () => toStatusTag(TIMED_OUT as unknown as OnChainOutcome),
    UnknownTagError,
  );
});

const ALL: OperationOutcome[] = [
  outcomeOf("Applied"),
  outcomeOf("Zero"),
  outcomeOf("Failed"),
  outcomeOf("Stale"),
  outcomeOf("Expired"),
  outcomeOf("Abandoned"),
  TIMED_OUT,
];

test("matchOutcome dispatches to exactly one handler per outcome", () => {
  const names = ALL.map((outcome) =>
    matchOutcome(outcome, {
      Applied: () => "Applied",
      Zero: () => "Zero",
      Failed: () => "Failed",
      Stale: () => "Stale",
      Expired: () => "Expired",
      Abandoned: () => "Abandoned",
      TimedOut: () => "TimedOut",
    }),
  );
  assert.deepEqual(
    names,
    ALL.map((o) => o.status),
  );
});

test("recovery hints: Zero/Failed/Stale/Expired retry; Abandoned/TimedOut check then reclaim", () => {
  for (const name of ["Zero", "Failed", "Stale", "Expired"] as const) {
    const d = describeOutcome(outcomeOf(name));
    assert.equal(d.recovery, "retry");
    assert.equal(d.safeToRetry, true);
    assert.ok(d.message.length > 0);
  }
  for (const outcome of [outcomeOf("Abandoned"), TIMED_OUT]) {
    assert.equal(
      describeOutcome(outcome).recovery,
      "check-onchain-then-reclaim",
    );
  }
  assert.equal(describeOutcome(TIMED_OUT).safeToRetry, false);
  assert.equal(describeOutcome(outcomeOf("Applied")).recovery, "none");
});
