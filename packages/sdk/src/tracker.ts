// Queue and callback tracker (Requirements 14.7, 14.8).
//
// Tracks one operation through
//
//   Signing -> Queued -> Computing -> Terminal(outcome)
//
// where the outcome is one of Applied, Zero, Failed, Stale, Expired,
// Abandoned, or TimedOut. The tracker owns no RPC, wallet, or cluster code:
// every external effect is an injected function, so it is unit-testable with
// fakes and works with any standard Solana JSON-RPC provider.
//
// Injected sources (see TrackerSources):
//   queue(attempt)           signs and sends the queue transaction for one
//                            attempt and resolves to its signature.
//   awaitFinalization(...)   resolves with the callback transaction signature
//                            once the Arcium computation finalizes. In
//                            production this is @arcium-hq/client's
//                            awaitComputationFinalization (see
//                            arciumFinalizationSource below).
//   watchEvent(...)          resolves with the Operation Intent status tag the
//                            program reports for the offset, from a program
//                            event or an Intent account read. Only terminal
//                            tags (1..6) decide an outcome.
//
// Attempts and retries follow packages/sdk/bench/measure.ts runOp: each attempt
// uses a FRESH computation offset; an attempt that fails (queue error,
// finalization error, or timeout) is retried after a backoff until
// `maxAttempts` is reached.
//   * If the last failure was a timeout the outcome is TimedOut: the SDK
//     stopped waiting and the computation may still land.
//   * Otherwise (nothing was queued or finalized) the outcome is Abandoned,
//     the client-side analogue of the on-chain timeout recovery.
// A tag reported by watchEvent always wins over a failure, because the program
// is the authority on the Intent's status. If the computation finalized but no
// terminal tag arrives within `eventGraceMs`, the outcome is TimedOut: the
// Intent stays Queued for on-chain timeout recovery (Requirement 3.6).

import { BN } from "./anchor-shim.js";
import type { AnchorProvider } from "./anchor-shim.js";
import { awaitComputationFinalization } from "@arcium-hq/client";
import type { Finality, PublicKey } from "@solana/web3.js";
import { randomBytes } from "./encryption.ts";
import {
  fromStatusTag,
  TIMED_OUT,
  outcomeOf,
  type OperationOutcome,
} from "./outcome.ts";

/** One queue attempt. */
export interface QueueAttempt {
  /** 1-based attempt number. */
  readonly attempt: number;
  /** A fresh u64 computation offset for this attempt. */
  readonly offset: bigint;
}

export interface TrackerSources {
  /** Signs and sends the queue transaction. Resolves to its signature. */
  queue(attempt: QueueAttempt): Promise<string>;
  /**
   * Resolves with the callback signature when the computation finalizes.
   * Rejects on error. The tracker enforces its own timeout as well.
   */
  awaitFinalization(offset: bigint, timeoutMs: number): Promise<string>;
  /**
   * Resolves with the Operation Intent status tag for this computation. Reject
   * or stay pending when no status is known yet. `signal` aborts when the
   * attempt is finished.
   */
  watchEvent(offset: bigint, signal: AbortSignal): Promise<number>;
}

export interface TrackerOptions {
  /** Attempts before giving up. Default 3, as in bench/measure.ts. */
  readonly maxAttempts?: number;
  /** Per-attempt wait for finalization. Default 120 000 ms. */
  readonly finalizeTimeoutMs?: number;
  /** How long to wait for a status tag after finalization. Default 5 000 ms. */
  readonly eventGraceMs?: number;
  /** Retry an attempt that timed out. Default true, as in bench/measure.ts. */
  readonly retryOnTimeout?: boolean;
  /** Delay before attempt `n + 1`. Default `500 * n` ms, as in bench/measure.ts. */
  readonly backoffMs?: (failedAttempt: number) => number;
  /** Produces a fresh u64 offset. Default: 8 random bytes. */
  readonly newOffset?: () => bigint;
  /** Replaces the backoff sleep (tests). */
  readonly sleep?: (ms: number) => Promise<void>;
}

export type InFlightPhase = "Signing" | "Queued" | "Computing";

/** A tracker state. `Terminal` is final and carries the outcome. */
export type OperationState =
  | {
      readonly phase: InFlightPhase;
      readonly attempt: number;
      readonly computationOffset: bigint;
      readonly queueSignature: string | null;
    }
  | {
      readonly phase: "Terminal";
      readonly outcome: OperationOutcome;
      readonly attempt: number;
      readonly computationOffset: bigint;
      readonly queueSignature: string | null;
      /** The callback transaction signature, when the computation finalized. */
      readonly finalizationSignature: string | null;
      /** The Intent status tag the program reported, when one was seen. */
      readonly statusTag: number | null;
      /** One message per failed attempt, oldest first. */
      readonly attemptErrors: readonly string[];
      /** Extra context for TimedOut and Abandoned. */
      readonly detail: string | null;
    };

export type TerminalState = Extract<OperationState, { phase: "Terminal" }>;

type Listener = (state: OperationState) => void;

interface AttemptResult {
  outcome: OperationOutcome;
  finalizationSignature: string | null;
  statusTag: number | null;
  detail: string | null;
}

type FinalizeResult =
  | { kind: "finalized"; signature: string }
  | { kind: "finalize-failed"; error: unknown };

type StatusResult =
  { kind: "status"; tag: number } | { kind: "watch-failed"; error: unknown };

function timedOut(
  finalizationSignature: string | null,
  reason: string,
): AttemptResult {
  return {
    outcome: TIMED_OUT,
    finalizationSignature,
    statusTag: null,
    detail: `the computation finalized but ${reason}; the Intent stays Queued until timeout recovery`,
  };
}
class TimeoutError extends Error {
  constructor(what: string, ms: number) {
    super(`${what} timed out after ${ms} ms`);
    this.name = "TimeoutError";
  }
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function defaultOffset(): bigint {
  let n = 0n;
  for (const byte of randomBytes(8)) n = (n << 8n) | BigInt(byte);
  return n;
}

/** Races `promise` against a timer. The timer is always cleared. */
function withTimeout<T>(
  promise: Promise<T>,
  ms: number,
  what: string,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new TimeoutError(what, ms)), ms);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

/** Tracks one operation. Call {@link OperationTracker.run} once. */
export class OperationTracker {
  readonly #sources: TrackerSources;
  readonly #maxAttempts: number;
  readonly #finalizeTimeoutMs: number;
  readonly #eventGraceMs: number;
  readonly #retryOnTimeout: boolean;
  readonly #backoffMs: (failedAttempt: number) => number;
  readonly #newOffset: () => bigint;
  readonly #sleep: (ms: number) => Promise<void>;
  readonly #history: OperationState[] = [];
  readonly #listeners = new Set<Listener>();
  #running: Promise<TerminalState> | undefined;

  constructor(sources: TrackerSources, options: TrackerOptions = {}) {
    this.#sources = sources;
    this.#maxAttempts = options.maxAttempts ?? 3;
    if (!Number.isInteger(this.#maxAttempts) || this.#maxAttempts < 1) {
      throw new RangeError("maxAttempts must be an integer >= 1");
    }
    this.#finalizeTimeoutMs = options.finalizeTimeoutMs ?? 120_000;
    this.#eventGraceMs = options.eventGraceMs ?? 5_000;
    this.#retryOnTimeout = options.retryOnTimeout ?? true;
    this.#backoffMs = options.backoffMs ?? ((n) => 500 * n);
    this.#newOffset = options.newOffset ?? defaultOffset;
    this.#sleep =
      options.sleep ?? ((ms) => new Promise((r) => setTimeout(r, ms)));
  }

  /** Every state emitted so far, oldest first. */
  get history(): readonly OperationState[] {
    return this.#history;
  }

  /** The latest state, or undefined before {@link run} starts. */
  get state(): OperationState | undefined {
    return this.#history[this.#history.length - 1];
  }

  /** Subscribes to future state changes. Returns an unsubscribe function. */
  onChange(listener: Listener): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  /**
   * Yields every state from the first one (including states already emitted)
   * and finishes after the terminal state. Starts the tracker if needed.
   */
  async *[Symbol.asyncIterator](): AsyncGenerator<OperationState, void> {
    const queue: OperationState[] = [...this.#history];
    let wake: (() => void) | undefined;
    const unsubscribe = this.onChange((s) => {
      queue.push(s);
      wake?.();
    });
    const done = this.run().then(
      () => undefined,
      () => undefined,
    );
    try {
      for (;;) {
        const next = queue.shift();
        if (next) {
          yield next;
          if (next.phase === "Terminal") return;
        } else {
          await Promise.race([
            new Promise<void>((resolve) => {
              wake = resolve;
            }),
            done,
          ]);
          wake = undefined;
          if (queue.length === 0 && this.state?.phase === "Terminal") return;
        }
      }
    } finally {
      unsubscribe();
    }
  }

  /** Runs the operation to a terminal state. Calling it again returns the same promise. */
  run(): Promise<TerminalState> {
    this.#running ??= this.#execute();
    return this.#running;
  }

  #emit(state: OperationState): void {
    this.#history.push(state);
    for (const listener of [...this.#listeners]) {
      try {
        listener(state);
      } catch {
        // A faulty listener must not disturb tracking.
      }
    }
  }

  async #execute(): Promise<TerminalState> {
    const attemptErrors: string[] = [];
    let lastWasTimeout = false;
    let lastOffset = 0n;
    let lastQueueSignature: string | null = null;
    let lastAttempt = 0;

    for (let attempt = 1; attempt <= this.#maxAttempts; attempt += 1) {
      lastAttempt = attempt;
      const offset = this.#newOffset();
      lastOffset = offset;
      lastQueueSignature = null;
      this.#emit({
        phase: "Signing",
        attempt,
        computationOffset: offset,
        queueSignature: null,
      });

      const controller = new AbortController();
      try {
        const signature = await this.#sources.queue({ attempt, offset });
        lastQueueSignature = signature;
        this.#emit({
          phase: "Queued",
          attempt,
          computationOffset: offset,
          queueSignature: signature,
        });
        this.#emit({
          phase: "Computing",
          attempt,
          computationOffset: offset,
          queueSignature: signature,
        });

        const result = await this.#awaitResult(offset, controller.signal);
        controller.abort();
        return this.#terminal({
          ...result,
          attempt,
          offset,
          queueSignature: signature,
          attemptErrors,
        });
      } catch (error) {
        controller.abort();
        lastWasTimeout = error instanceof TimeoutError;
        attemptErrors.push(`attempt ${attempt}: ${message(error)}`);
        if (lastWasTimeout && !this.#retryOnTimeout) break;
        if (attempt < this.#maxAttempts) {
          await this.#sleep(this.#backoffMs(attempt));
        }
      }
    }

    return this.#terminal({
      outcome: lastWasTimeout ? TIMED_OUT : outcomeOf("Abandoned"),
      finalizationSignature: null,
      statusTag: null,
      detail: lastWasTimeout
        ? "the SDK stopped waiting; the computation may still land"
        : `no attempt was queued and finalized after ${lastAttempt} attempt(s)`,
      attempt: lastAttempt,
      offset: lastOffset,
      queueSignature: lastQueueSignature,
      attemptErrors,
    });
  }

  /**
   * Waits for a terminal status. Throws on error or timeout (the attempt
   * failed). Returns a TimedOut result when the computation finalized but the
   * program reported no usable terminal status.
   */
  async #awaitResult(
    offset: bigint,
    signal: AbortSignal,
  ): Promise<AttemptResult> {
    let finalizedSignature: string | null = null;
    // Both sources are folded into never-rejecting results so a late rejection
    // can never become an unhandled rejection.
    const finalize: Promise<FinalizeResult> = this.#sources
      .awaitFinalization(offset, this.#finalizeTimeoutMs)
      .then(
        (signature): FinalizeResult => {
          finalizedSignature = signature;
          return { kind: "finalized", signature };
        },
        (error: unknown): FinalizeResult => ({
          kind: "finalize-failed",
          error,
        }),
      );
    const status: Promise<StatusResult> = this.#sources
      .watchEvent(offset, signal)
      .then(
        (tag): StatusResult => ({ kind: "status", tag }),
        (error: unknown): StatusResult => ({ kind: "watch-failed", error }),
      );
    const statusWithin = (ms: number): Promise<StatusResult | null> =>
      withTimeout(status, ms, "awaiting the Intent status").catch(() => null);

    const first = await withTimeout(
      Promise.race([finalize, status]),
      this.#finalizeTimeoutMs,
      "awaiting the computation",
    );

    // The program is the authority: a terminal tag wins over everything else.
    if (first.kind === "status") {
      return this.#fromTag(first.tag, finalizedSignature);
    }
    if (first.kind === "watch-failed") {
      const done = await withTimeout(
        finalize,
        this.#finalizeTimeoutMs,
        "awaiting the computation",
      );
      if (done.kind === "finalize-failed") throw done.error;
      return timedOut(
        done.signature,
        `the Intent status could not be read (${message(first.error)})`,
      );
    }

    // Finalization settled first. Give the program a grace period to report.
    const reported = await statusWithin(this.#eventGraceMs);
    if (reported?.kind === "status") {
      return this.#fromTag(reported.tag, finalizedSignature);
    }
    if (first.kind === "finalize-failed") throw first.error;
    return timedOut(
      first.signature,
      reported
        ? `the Intent status could not be read (${message(reported.error)})`
        : "no terminal Intent status was reported",
    );
  }

  #fromTag(tag: number, finalizationSignature: string | null): AttemptResult {
    try {
      return {
        outcome: fromStatusTag(tag),
        finalizationSignature,
        statusTag: tag,
        detail: null,
      };
    } catch (error) {
      // Queued or an unknown tag is not a terminal outcome.
      return {
        outcome: TIMED_OUT,
        finalizationSignature,
        statusTag: Number.isInteger(tag) ? tag : null,
        detail: `unusable Intent status: ${message(error)}`,
      };
    }
  }
  #terminal(parts: {
    outcome: OperationOutcome;
    finalizationSignature: string | null;
    statusTag: number | null;
    detail: string | null;
    attempt: number;
    offset: bigint;
    queueSignature: string | null;
    attemptErrors: string[];
  }): TerminalState {
    const state: TerminalState = {
      phase: "Terminal",
      outcome: parts.outcome,
      attempt: parts.attempt,
      computationOffset: parts.offset,
      queueSignature: parts.queueSignature,
      finalizationSignature: parts.finalizationSignature,
      statusTag: parts.statusTag,
      attemptErrors: [...parts.attemptErrors],
      detail: parts.detail,
    };
    this.#emit(state);
    return state;
  }
}

/**
 * Adapts @arcium-hq/client's awaitComputationFinalization to the tracker's
 * `awaitFinalization` source. Needs a live cluster, so it is not unit-tested.
 */
export function arciumFinalizationSource(
  provider: AnchorProvider,
  mxeProgramId: PublicKey,
  commitment: Finality = "confirmed",
): TrackerSources["awaitFinalization"] {
  return (offset, timeoutMs) =>
    awaitComputationFinalization(
      provider,
      new BN(offset.toString()),
      mxeProgramId,
      commitment,
      timeoutMs,
    );
}
