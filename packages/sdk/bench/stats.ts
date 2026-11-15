// Pure statistics helpers for the Task 2.6 benchmark harness.
//
// These are deliberately dependency-free and side-effect-free so they can be
// unit-tested without a cluster (see stats.test.ts) and reused by the latency
// and contention benchmarks. All inputs are plain numbers (milliseconds,
// lamports, or compute units); callers decide the unit.

/** Summary statistics for a set of samples. */
export interface Summary {
  count: number;
  min: number;
  max: number;
  mean: number;
  p50: number;
  p95: number;
  p99: number;
  stddev: number;
}

/**
 * Linear-interpolated percentile over an ascending-sorted array.
 *
 * `p` is a percentage in [0, 100]. Uses the same interpolation convention as
 * NumPy's default ("linear"): rank = p/100 * (n - 1), interpolating between the
 * two closest ranks. Returns `NaN` for an empty input.
 */
export function percentile(sortedAsc: readonly number[], p: number): number {
  const n = sortedAsc.length;
  if (n === 0) return NaN;
  if (n === 1) return sortedAsc[0] as number;
  const clamped = Math.min(100, Math.max(0, p));
  const rank = (clamped / 100) * (n - 1);
  const lo = Math.floor(rank);
  const hi = Math.ceil(rank);
  const loVal = sortedAsc[lo] as number;
  if (lo === hi) return loVal;
  const hiVal = sortedAsc[hi] as number;
  return loVal + (hiVal - loVal) * (rank - lo);
}

/** Arithmetic mean of a non-empty array; `NaN` if empty. */
export function mean(values: readonly number[]): number {
  if (values.length === 0) return NaN;
  let total = 0;
  for (const v of values) total += v;
  return total / values.length;
}

/** Population standard deviation; `NaN` if empty. */
export function stddev(values: readonly number[]): number {
  if (values.length === 0) return NaN;
  const m = mean(values);
  let acc = 0;
  for (const v of values) acc += (v - m) * (v - m);
  return Math.sqrt(acc / values.length);
}

/** Compute the full summary for a set of samples. Input is copied and sorted. */
export function summarize(values: readonly number[]): Summary {
  if (values.length === 0) {
    return {
      count: 0,
      min: NaN,
      max: NaN,
      mean: NaN,
      p50: NaN,
      p95: NaN,
      p99: NaN,
      stddev: NaN,
    };
  }
  const sorted = [...values].sort((a, b) => a - b);
  return {
    count: sorted.length,
    min: sorted[0] as number,
    max: sorted[sorted.length - 1] as number,
    mean: mean(sorted),
    p50: percentile(sorted, 50),
    p95: percentile(sorted, 95),
    p99: percentile(sorted, 99),
    stddev: stddev(sorted),
  };
}

/** Round to a fixed number of decimals, preserving `NaN`. */
export function round(value: number, decimals = 2): number {
  if (!Number.isFinite(value)) return value;
  const factor = 10 ** decimals;
  return Math.round(value * factor) / factor;
}
