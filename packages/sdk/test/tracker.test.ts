// Unit tests for the queue and callback tracker (Task 3.4), using fake
// injected sources. No cluster is needed.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  INTENT_STATUS_TAG,
  OperationTracker,
  type OperationState,
  type TrackerOptions,
  type TrackerSources,
} from "../src/index.ts";

interface FakeScript {
  /** Per attempt (1-based) behavior; the last entry repeats. */
  queue?: (attempt: number) => Promise<string>;
  finalize?: (attempt: number) => Promise<string>;
  status?: (attempt: number) => Promise<number>;
}

const never = <T>(): Promise<T> => new Promise<T>(() => undefined);

function fakes(script: FakeScript): {
  sources: TrackerSources;
  offsets: bigint[];
  options: TrackerOptions;
} {
  const offsets: bigint[] = [];
  const attemptOf = new Map<bigint, number>();
  let counter = 100n;
  const sources: TrackerSources = {
    queue: ({ attempt, offset }) => {
      offsets.push(offset);
      attemptOf.set(offset, attempt);
      return (script.queue ?? (() => Promise.resolve("queue-sig")))(attempt);
    },
    awaitFinalization: (offset) =>
      (script.finalize ?? (() => Promise.resolve("callback-sig")))(
        attemptOf.get(offset) ?? 0,
      ),
    watchEvent: (offset) =>
      (script.status ?? (() => never<number>()))(attemptOf.get(offset) ?? 0),
  };
  const options: TrackerOptions = {
    newOffset: () => (counter += 1n),
    sleep: () => Promise.resolve(),
    finalizeTimeoutMs: 20,
    eventGraceMs: 20,
  };
  return { sources, offsets, options };
}

const phases = (states: readonly OperationState[]): string[] =>
  states.map((s) => (s.phase === "Terminal" ? s.outcome.status : s.phase));

for (const [name, tag] of [
  ["Applied", INTENT_STATUS_TAG.Applied],
  ["Zero", INTENT_STATUS_TAG.Zero],
  ["Failed", INTENT_STATUS_TAG.Failed],
  ["Stale", INTENT_STATUS_TAG.Stale],
  ["Expired", INTENT_STATUS_TAG.Expired],
  ["Abandoned", INTENT_STATUS_TAG.Abandoned],
] as const) {
  test(`reaches the ${name} outcome from the reported status tag`, async () => {
    const { sources, options } = fakes({ status: () => Promise.resolve(tag) });
    const tracker = new OperationTracker(sources, options);
    const end = await tracker.run();
    assert.deepEqual(phases(tracker.history), [
      "Signing",
      "Queued",
      "Computing",
      name,
    ]);
    assert.equal(end.outcome.status, name);
    assert.equal(end.statusTag, tag);
    assert.equal(end.queueSignature, "queue-sig");
    assert.equal(end.attempt, 1);
    assert.deepEqual(end.attemptErrors, []);
  });
}

test("records the callback signature when the computation finalized first", async () => {
  const { sources, options } = fakes({
    status: () =>
      new Promise((resolve) =>
        setTimeout(() => resolve(INTENT_STATUS_TAG.Applied), 5),
      ),
  });
  const end = await new OperationTracker(sources, options).run();
  assert.equal(end.outcome.status, "Applied");
  assert.equal(end.finalizationSignature, "callback-sig");
});

test("finalized but no terminal status within the grace period is TimedOut", async () => {
  const { sources, options } = fakes({});
  const end = await new OperationTracker(sources, options).run();
  assert.equal(end.outcome.status, "TimedOut");
  assert.equal(end.finalizationSignature, "callback-sig");
  assert.match(end.detail ?? "", /stays Queued/);
  assert.equal(end.attempt, 1);
});

test("a reported Queued or unknown tag is not a terminal outcome", async () => {
  for (const tag of [0, 9]) {
    const { sources, options } = fakes({ status: () => Promise.resolve(tag) });
    const end = await new OperationTracker(sources, options).run();
    assert.equal(end.outcome.status, "TimedOut");
    assert.match(end.detail ?? "", /unusable Intent status/);
  }
});

test("a finalization timeout retries with fresh offsets, then ends TimedOut", async () => {
  const { sources, options, offsets } = fakes({ finalize: () => never() });
  const tracker = new OperationTracker(sources, { ...options, maxAttempts: 3 });
  const end = await tracker.run();
  assert.equal(end.outcome.status, "TimedOut");
  assert.equal(end.attempt, 3);
  assert.equal(new Set(offsets).size, 3);
  assert.equal(end.attemptErrors.length, 3);
  assert.deepEqual(
    phases(tracker.history).filter((p) => p === "Signing").length,
    3,
  );
});

test("retryOnTimeout=false stops at the first timeout", async () => {
  const { sources, options, offsets } = fakes({ finalize: () => never() });
  const end = await new OperationTracker(sources, {
    ...options,
    retryOnTimeout: false,
  }).run();
  assert.equal(end.outcome.status, "TimedOut");
  assert.equal(offsets.length, 1);
});

test("queue errors retry with fresh offsets, then end Abandoned", async () => {
  const { sources, options, offsets } = fakes({
    queue: () => Promise.reject(new Error("blockhash expired")),
  });
  const end = await new OperationTracker(sources, {
    ...options,
    maxAttempts: 3,
  }).run();
  assert.equal(end.outcome.status, "Abandoned");
  assert.equal(end.statusTag, null);
  assert.equal(end.attempt, 3);
  assert.equal(new Set(offsets).size, 3);
  assert.equal(end.attemptErrors.length, 3);
  assert.match(end.attemptErrors[0] ?? "", /blockhash expired/);
});

test("a later attempt can succeed after an earlier one failed", async () => {
  const { sources, options, offsets } = fakes({
    queue: (attempt) =>
      attempt === 1
        ? Promise.reject(new Error("rpc 429"))
        : Promise.resolve("queue-sig-2"),
    status: () => Promise.resolve(INTENT_STATUS_TAG.Applied),
  });
  const end = await new OperationTracker(sources, options).run();
  assert.equal(end.outcome.status, "Applied");
  assert.equal(end.attempt, 2);
  assert.equal(end.queueSignature, "queue-sig-2");
  assert.equal(end.computationOffset, offsets[1]);
  assert.notEqual(offsets[0], offsets[1]);
  assert.equal(end.attemptErrors.length, 1);
});

test("a terminal status wins over a failed finalization", async () => {
  const { sources, options } = fakes({
    finalize: () => Promise.reject(new Error("computation failed")),
    status: () =>
      new Promise((resolve) =>
        setTimeout(() => resolve(INTENT_STATUS_TAG.Failed), 5),
      ),
  });
  const end = await new OperationTracker(sources, options).run();
  assert.equal(end.outcome.status, "Failed");
  assert.equal(end.attempt, 1);
});

test("a failed finalization with no status exhausts attempts as Abandoned", async () => {
  const { sources, options } = fakes({
    finalize: () => Promise.reject(new Error("computation failed")),
  });
  const end = await new OperationTracker(sources, {
    ...options,
    maxAttempts: 2,
  }).run();
  assert.equal(end.outcome.status, "Abandoned");
  assert.equal(end.attemptErrors.length, 2);
});

test("a failing status watcher with a finalized computation is TimedOut", async () => {
  const { sources, options } = fakes({
    status: () => Promise.reject(new Error("ws closed")),
  });
  const end = await new OperationTracker(sources, options).run();
  assert.equal(end.outcome.status, "TimedOut");
  assert.match(end.detail ?? "", /ws closed/);
});

test("the async iterator and onChange listeners see every state in order", async () => {
  const { sources, options } = fakes({
    status: () => Promise.resolve(INTENT_STATUS_TAG.Zero),
  });
  const tracker = new OperationTracker(sources, options);
  const seen: string[] = [];
  const unsubscribe = tracker.onChange((s) => {
    seen.push(s.phase);
  });
  const iterated: OperationState[] = [];
  for await (const state of tracker) iterated.push(state);
  unsubscribe();
  assert.deepEqual(phases(iterated), [
    "Signing",
    "Queued",
    "Computing",
    "Zero",
  ]);
  assert.deepEqual(seen, ["Signing", "Queued", "Computing", "Terminal"]);
  assert.equal(tracker.state, iterated[iterated.length - 1]);
  // Iterating again replays the finished history; run() is idempotent.
  const replay: OperationState[] = [];
  for await (const state of tracker) replay.push(state);
  assert.deepEqual(phases(replay), phases(iterated));
  assert.equal(await tracker.run(), await tracker.run());
});

test("a throwing listener does not disturb tracking", async () => {
  const { sources, options } = fakes({
    status: () => Promise.resolve(INTENT_STATUS_TAG.Applied),
  });
  const tracker = new OperationTracker(sources, options);
  tracker.onChange(() => {
    throw new Error("listener bug");
  });
  assert.equal((await tracker.run()).outcome.status, "Applied");
});

test("maxAttempts must be a positive integer", () => {
  const { sources } = fakes({});
  assert.throws(
    () => new OperationTracker(sources, { maxAttempts: 0 }),
    RangeError,
  );
});
