// Event listening for the localnet tests without `anchor.IdlEvents<...>`.
//
// The generated IDL is now large enough that `anchor.IdlEvents<typeof idl>`
// exceeds TypeScript's instantiation depth limit (TS2589), and it would get
// worse as the program grows. Tests declare the payloads they read by hand
// and talk to the program through this minimal interface instead.

type Listener = (payload: unknown) => void;

interface EventSource {
  addEventListener(name: string, listener: Listener): number;
  removeEventListener(id: number): Promise<void>;
}

/** Payload of `SumEvent`, emitted by the add_together callbacks. */
export interface SumEventPayload {
  sum: number[];
  nonce: number[];
}

/** Payload of `LiquidationReceiptsEmitted` (Task 2.3 two-owner outputs). */
export interface LiquidationReceiptsPayload {
  borrowerKey: number[];
  borrowerNonce: number[];
  borrowerCiphertexts: number[][];
  liquidatorKey: number[];
  liquidatorNonce: number[];
  liquidatorCiphertexts: number[][];
}

export interface WatchedEvent<T> {
  state: { fired: boolean; payload: T | undefined };
  stop: () => Promise<void>;
}

/** Records whether an event fired, for negative controls. Call `stop()` after. */
export function watchEvent<T>(program: unknown, name: string): WatchedEvent<T> {
  const source = program as EventSource;
  const state: { fired: boolean; payload: T | undefined } = {
    fired: false,
    payload: undefined,
  };
  const id = source.addEventListener(name, (payload) => {
    state.fired = true;
    state.payload = payload as T;
  });
  return {
    state,
    stop: async () => {
      await source.removeEventListener(id);
    },
  };
}

/** Resolves with the next event of this name, or rejects after `timeoutMs`. */
export async function awaitEvent<T>(
  program: unknown,
  name: string,
  timeoutMs = 120000,
): Promise<T> {
  const source = program as EventSource;
  let id = -1;
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await new Promise<T>((resolve, reject) => {
      id = source.addEventListener(name, (payload) => resolve(payload as T));
      timer = setTimeout(
        () => reject(new Error(`Event ${name} timed out after ${timeoutMs}ms`)),
        timeoutMs,
      );
    });
  } finally {
    clearTimeout(timer);
    if (id >= 0) await source.removeEventListener(id);
  }
}
