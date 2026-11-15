// Unit tests for the pure statistics helpers (Task 2.6 harness).
//
// These are devnet-free and use Node's built-in test runner, so they can be
// run locally with `pnpm bench:selftest` to sanity-check the math before
// spending devnet SOL. They are intentionally NOT wired into any `test:*`
// script, so CI (which has no devnet wallet) never needs them; CI only
// type-checks this file via tsconfig.bench.json.

import { test } from "node:test";
import assert from "node:assert/strict";
import { percentile, mean, stddev, summarize, round } from "./stats.ts";

test("percentile returns NaN for empty input", () => {
  assert.ok(Number.isNaN(percentile([], 50)));
});

test("percentile on a single value returns that value", () => {
  assert.equal(percentile([42], 95), 42);
});

test("percentile interpolates linearly like NumPy's default", () => {
  const data = [1, 2, 3, 4]; // already ascending
  assert.equal(percentile(data, 0), 1);
  assert.equal(percentile(data, 100), 4);
  // rank = 0.5 * 3 = 1.5 -> between index 1 (2) and 2 (3) -> 2.5
  assert.equal(percentile(data, 50), 2.5);
});

test("p95 of 1..100 is 95.05", () => {
  const data = Array.from({ length: 100 }, (_, i) => i + 1);
  assert.equal(round(percentile(data, 95), 2), 95.05);
});

test("mean and stddev of a known set", () => {
  const data = [2, 4, 4, 4, 5, 5, 7, 9];
  assert.equal(mean(data), 5);
  assert.equal(stddev(data), 2); // population standard deviation
});

test("summarize reports count and ordered extremes", () => {
  const s = summarize([5, 1, 3, 2, 4]);
  assert.equal(s.count, 5);
  assert.equal(s.min, 1);
  assert.equal(s.max, 5);
  assert.equal(s.p50, 3);
});

test("summarize of empty set is all NaN with zero count", () => {
  const s = summarize([]);
  assert.equal(s.count, 0);
  assert.ok(Number.isNaN(s.mean));
});

test("round preserves NaN and trims decimals", () => {
  assert.ok(Number.isNaN(round(NaN)));
  assert.equal(round(1.23456, 2), 1.23);
});
