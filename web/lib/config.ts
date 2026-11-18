// Public configuration of the Walnut devnet deployment. No secrets live here.
import { PublicKey } from "@solana/web3.js";

export const NETWORK = "devnet";
export const NETWORK_LABEL = "Solana Devnet";
export const PROGRAM_ID = new PublicKey(
  "5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea",
);
export const MARKET_ADDRESS = new PublicKey(
  "Du5ZWegZJj1RvpZ2GHVNgsy2MXAhiThpP3AHWRkJEEGr",
);
export const USDC_MINT = new PublicKey(
  "GHLMq3gCUHkb9sGPouXNo4Lt9QpGCmz5UMrtF7EwVDLT",
);
export const WSOL_MINT = new PublicKey(
  "So11111111111111111111111111111111111111112",
);
/** Shared Arcium devnet cluster. */
export const CLUSTER_OFFSET = 456;

export const DEFAULT_RPC_URL = "https://api.devnet.solana.com";
export const FAUCET_URL = "https://faucet.solana.com";
export const SOL_DECIMALS = 9;
export const USDC_DECIMALS = 6;
/** Used when the Pyth price account cannot be read: 150 USDC per SOL. */
export const FALLBACK_SOL_PRICE = 150_000_000n;
/** Compute-unit limit of every transaction that queues an MPC computation. */
export const QUEUE_COMPUTE_UNITS = 600_000;
/** SOL kept in the wallet for fees and token account rent when using Max. */
export const SOL_FEE_RESERVE = 10_000_000n;

/** The RPC endpoint: NEXT_PUBLIC_RPC_URL when set, else the public devnet RPC. */
export function rpcUrl(): string {
  // Dot access so Next.js inlines the value into the browser bundle.
  const fromEnv = process.env.NEXT_PUBLIC_RPC_URL;
  return fromEnv !== undefined && fromEnv.trim() !== ""
    ? fromEnv.trim()
    : DEFAULT_RPC_URL;
}

export function explorerTx(signature: string): string {
  return `https://explorer.solana.com/tx/${signature}?cluster=devnet`;
}
