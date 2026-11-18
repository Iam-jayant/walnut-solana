// Read-side chain access: connection with rate-limit backoff, the Walnut
// program client, the Market, the LTV from AssetState, the Pyth SOL price and
// Position reads. No signing happens here, so the module also runs in Node
// (the read-only smoke check).
import "./polyfill.ts";
import { AnchorProvider, BN, Program, type Idl } from "@anchor-lang/core";
import { getMXEPublicKey } from "@arcium-hq/client";
import {
  decodeLendingPool,
  deriveLendingPoolPda,
  hexToBytes,
  parsePriceUpdateV2,
  PYTH_FEED_IDS,
  u64ToLeBytes,
  type LendingPool,
  type RateParams,
} from "@walnut/sdk";
import {
  Connection,
  PublicKey,
  type Commitment,
  type Transaction,
  type VersionedTransaction,
} from "@solana/web3.js";
import { FALLBACK_SOL_PRICE, MARKET_ADDRESS, PROGRAM_ID } from "./config.ts";

// ------------------------------------------------------------------ retry

export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

const TRANSIENT =
  /429|too many|fetch failed|failed to fetch|networkerror|network request|econn|etimedout|enotfound|timed out|socket|503|502|504|failed to get|rate limit|load failed|und_err/i;

/** Retries transient RPC failures with exponential backoff. */
export async function withRetry<T>(
  fn: () => Promise<T>,
  attempts = 6,
): Promise<T> {
  let last: unknown;
  for (let i = 0; i < attempts; i += 1) {
    try {
      return await fn();
    } catch (err) {
      last = err;
      const text = err instanceof Error ? err.message : String(err);
      if (!TRANSIENT.test(text) || i === attempts - 1) break;
      await sleep(Math.min(800 * 2 ** i, 12_000));
    }
  }
  throw last;
}

/** fetch with backoff on HTTP 429 and 5xx, for the Solana JSON-RPC client. */
const retryingFetch: typeof fetch = async (input, init) => {
  let delay = 700;
  for (let attempt = 0; ; attempt += 1) {
    try {
      const response = await fetch(input, init);
      if ((response.status === 429 || response.status >= 502) && attempt < 6) {
        await sleep(delay);
        delay = Math.min(delay * 2, 10_000);
        continue;
      }
      return response;
    } catch (err) {
      if (attempt >= 6) throw err;
      await sleep(delay);
      delay = Math.min(delay * 2, 10_000);
    }
  }
};

export function createConnection(url: string): Connection {
  return new Connection(url, {
    commitment: "confirmed",
    fetch: retryingFetch,
    disableRetryOnRateLimit: true,
  });
}

// ---------------------------------------------------------------- program

interface ReadOnlyWallet {
  publicKey: PublicKey;
  signTransaction<T extends Transaction | VersionedTransaction>(
    tx: T,
  ): Promise<T>;
  signAllTransactions<T extends Transaction | VersionedTransaction>(
    txs: T[],
  ): Promise<T[]>;
}

const readOnlyWallet: ReadOnlyWallet = {
  publicKey: PublicKey.default,
  signTransaction: () => Promise.reject(new Error("read-only provider")),
  signAllTransactions: () => Promise.reject(new Error("read-only provider")),
};

export interface Chain {
  connection: Connection;
  provider: AnchorProvider;
  program: Program;
}

export function createChain(connection: Connection, idl: Idl): Chain {
  const provider = new AnchorProvider(connection, readOnlyWallet, {
    commitment: "confirmed",
  });
  return { connection, provider, program: new Program(idl, provider) };
}

// ------------------------------------------------------- typed account reads

type Json = Record<string, unknown>;

export interface MethodBuilder {
  accountsPartial(accounts: Record<string, PublicKey>): MethodBuilder;
  instruction(): Promise<import("@solana/web3.js").TransactionInstruction>;
}
type MethodFn = (...args: unknown[]) => MethodBuilder;

/** A typed handle on `program.methods.<name>`. */
export function method(program: Program, name: string): MethodFn {
  const methods = program.methods as unknown as Record<
    string,
    MethodFn | undefined
  >;
  const fn = methods[name];
  if (fn === undefined)
    throw new Error(`The program has no instruction ${name}`);
  return (...args) => fn.call(program.methods, ...args);
}

interface AccountClient {
  fetchNullable(address: PublicKey, commitment?: Commitment): Promise<unknown>;
}

export async function fetchAccount<T>(
  program: Program,
  name: string,
  address: PublicKey,
): Promise<T | null> {
  const client = (
    program.account as unknown as Record<string, AccountClient | undefined>
  )[name];
  if (client === undefined)
    throw new Error(`The program has no account ${name}`);
  return withRetry(
    async () => (await client.fetchNullable(address, "confirmed")) as T | null,
  );
}

export interface MarketAccount {
  assetState: PublicKey;
  vaultAuthority: PublicKey;
  wsolMint: PublicKey;
  usdcMint: PublicKey;
  wsolVault: PublicKey;
  usdcVault: PublicKey;
}

export interface AssetStateAccount {
  risk: {
    ltvBps: number;
    liquidationThresholdBps: number;
    liquidationBonusBps: number;
    closeFactorBps: number;
    reserveFactorBps: number;
  };
  rates: {
    baseRateBps: number;
    slope1Bps: number;
    slope2Bps: number;
    optimalUtilizationBps: number;
  };
  caps: { positionDebtCapUsdc: BN };
}

export interface LockView {
  active: boolean;
  computationOffset: BN;
  operationIntent: PublicKey;
  expirySlot: BN;
}

export interface PositionAccount {
  owner: PublicKey;
  status: Json;
  viewingPubkey: number[];
  viewingKeyEpoch: BN;
  nextShieldSeq: BN;
  coordination: { generation: BN; lock: LockView };
  encryptedState: { nonce: BN; ciphertexts: number[][] };
}

export interface IntentAccount {
  kind: Json;
  payer: PublicKey;
  status: Json;
  sequence: BN;
  lockExpirySlot: BN;
}

export interface OperationCounterAccount {
  nextSequence: BN;
}

export interface PendingShieldAccount {
  status: Json;
}

/** The single variant name of an Anchor-decoded enum, such as `active`. */
export function variantName(value: Json): string {
  return Object.keys(value)[0] ?? "unknown";
}

// ----------------------------------------------------------------- protocol

export interface Protocol {
  marketAddress: PublicKey;
  market: MarketAccount;
  ltvBps: bigint;
  liquidationThresholdBps: bigint;
  liquidationBonusBps: bigint;
  closeFactorBps: bigint;
  reserveFactorBps: bigint;
  /** Borrow rate curve of the AssetState (used for APR and APY). */
  rates: RateParams;
  /** Per-Position debt cap in USDC base units. */
  positionDebtCap: bigint;
  mxePublicKey: Uint8Array;
}

/** Reads the Market, the LTV from AssetState, and the MXE public key. */
export async function loadProtocol(chain: Chain): Promise<Protocol> {
  const market = await fetchAccount<MarketAccount>(
    chain.program,
    "market",
    MARKET_ADDRESS,
  );
  if (market === null) throw new Error("Market account not found");
  const assetState = await fetchAccount<AssetStateAccount>(
    chain.program,
    "assetState",
    market.assetState,
  );
  if (assetState === null) throw new Error("Asset State account not found");
  let mxePublicKey: Uint8Array | null = null;
  for (let i = 0; i < 6 && mxePublicKey === null; i += 1) {
    try {
      mxePublicKey = await getMXEPublicKey(chain.provider, PROGRAM_ID);
    } catch {
      /* the MXE key may not be readable for a moment; retry */
    }
    if (mxePublicKey === null) await sleep(1000 * (i + 1));
  }
  if (mxePublicKey === null) throw new Error("MXE public key unavailable");
  return {
    marketAddress: MARKET_ADDRESS,
    market,
    ltvBps: BigInt(assetState.risk.ltvBps),
    liquidationThresholdBps: BigInt(assetState.risk.liquidationThresholdBps),
    liquidationBonusBps: BigInt(assetState.risk.liquidationBonusBps),
    closeFactorBps: BigInt(assetState.risk.closeFactorBps),
    reserveFactorBps: BigInt(assetState.risk.reserveFactorBps),
    rates: {
      baseRateBps: BigInt(assetState.rates.baseRateBps),
      slope1Bps: BigInt(assetState.rates.slope1Bps),
      slope2Bps: BigInt(assetState.rates.slope2Bps),
      optimalUtilizationBps: BigInt(assetState.rates.optimalUtilizationBps),
    },
    positionDebtCap: BigInt(assetState.caps.positionDebtCapUsdc.toString()),
    mxePublicKey,
  };
}

// ------------------------------------------------------------- lending pool

/** The Market's LendingPool with the USDC the vault can pay out now. */
export interface LendingView {
  address: PublicKey;
  pool: LendingPool;
  /** Vault USDC minus protocol reserves, in base units. */
  liquidity: bigint;
}

/** Reads the LendingPool; null while it has not been created on chain. */
export async function loadLendingView(
  connection: Connection,
  protocol: Protocol,
): Promise<LendingView | null> {
  const address = deriveLendingPoolPda(PROGRAM_ID, protocol.marketAddress);
  const info = await withRetry(() =>
    connection.getAccountInfo(address, "confirmed"),
  );
  if (info === null) return null;
  const pool = decodeLendingPool(info.data);
  const vault = await tokenBalanceOf(connection, protocol.market.usdcVault);
  const held = vault ?? 0n;
  return {
    address,
    pool,
    liquidity: held > pool.reservesUsdc ? held - pool.reservesUsdc : 0n,
  };
}

// -------------------------------------------------------------------- price

/** Pyth push-oracle program: owns the sponsored feed account of a feed id. */
const PUSH_ORACLE_PROGRAM_ID = new PublicKey(
  "pythWSnswVUd12oZpeFP8e9CVaEqJg25g1Vtc2biRsT",
);

export interface PriceInfo {
  /** USDC base units (6 decimals) per 1 SOL. */
  price: bigint;
  source: "pyth" | "fallback";
  /** Unix seconds of the Pyth print, when read from chain. */
  publishTime?: number;
}

/** The shard-0 sponsored SOL/USD PriceUpdateV2 account. */
export function solUsdPriceAccount(): PublicKey {
  return PublicKey.findProgramAddressSync(
    [new Uint8Array([0, 0]), hexToBytes(PYTH_FEED_IDS["SOL/USD"])],
    PUSH_ORACLE_PROGRAM_ID,
  )[0];
}

/** Converts a Pyth mantissa and exponent to USDC base units per SOL. */
export function pythToUsdcPerSol(price: bigint, exponent: number): bigint {
  const scaled = price * 1_000_000n;
  return exponent >= 0
    ? scaled * 10n ** BigInt(exponent)
    : scaled / 10n ** BigInt(-exponent);
}

/** Reads the Pyth devnet SOL/USD price; falls back to 150 USDC. */
export async function fetchSolPrice(
  connection: Connection,
): Promise<PriceInfo> {
  try {
    const info = await withRetry(() =>
      connection.getAccountInfo(solUsdPriceAccount(), "confirmed"),
    );
    if (info === null) throw new Error("price account missing");
    const parsed = parsePriceUpdateV2(info.data);
    const value = pythToUsdcPerSol(parsed.price, parsed.exponent);
    if (value < 1_000_000n || value > 10_000_000_000n) {
      throw new Error("price out of range");
    }
    return {
      price: value,
      source: "pyth",
      publishTime: Number(parsed.publishTime),
    };
  } catch {
    return { price: FALLBACK_SOL_PRICE, source: "fallback" };
  }
}

// ---------------------------------------------------------------- addresses

const textEncoder = new TextEncoder();

export function positionAddress(owner: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [textEncoder.encode("position"), MARKET_ADDRESS.toBytes(), owner.toBytes()],
    PROGRAM_ID,
  )[0];
}

export function pendingShieldAddress(
  position: PublicKey,
  shieldSequence: bigint,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [
      textEncoder.encode("pending_shield"),
      position.toBytes(),
      u64ToLeBytes(shieldSequence),
    ],
    PROGRAM_ID,
  )[0];
}

const TOKEN_PROGRAM_ID = new PublicKey(
  "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
);
const ATA_PROGRAM_ID = new PublicKey(
  "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
);
export { ATA_PROGRAM_ID, TOKEN_PROGRAM_ID };

export function ataAddress(owner: PublicKey, mint: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [owner.toBytes(), TOKEN_PROGRAM_ID.toBytes(), mint.toBytes()],
    ATA_PROGRAM_ID,
  )[0];
}

/** SPL token account amount (little-endian u64 at offset 64). */
export function tokenAmountOf(data: Uint8Array): bigint {
  return new DataView(
    data.buffer,
    data.byteOffset,
    data.byteLength,
  ).getBigUint64(64, true);
}

export async function tokenBalanceOf(
  connection: Connection,
  address: PublicKey,
): Promise<bigint | null> {
  const info = await withRetry(() =>
    connection.getAccountInfo(address, "confirmed"),
  );
  return info === null ? null : tokenAmountOf(info.data);
}

/** A random u64 as BN, used for computation offsets. */
export function randomU64(): BN {
  const bytes = new Uint8Array(8);
  globalThis.crypto.getRandomValues(bytes);
  return new BN(
    Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join(""),
    16,
  );
}

/** A random u128 as BN, used for the Position initialization nonce. */
export function randomU128(): BN {
  const bytes = new Uint8Array(16);
  globalThis.crypto.getRandomValues(bytes);
  return new BN(
    Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join(""),
    16,
  );
}
