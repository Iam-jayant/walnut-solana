// Read-only devnet check of the Web App's chain module: loads the Market, the
// AssetState LTV, the MXE key and the Pyth SOL price. No keys, no signing.
//
//   pnpm --filter @walnut/web smoke
import { readFileSync } from "node:fs";
import type { Idl } from "@anchor-lang/core";
import {
  createChain,
  createConnection,
  fetchSolPrice,
  loadProtocol,
  solUsdPriceAccount,
} from "../lib/chain.ts";
import { rpcUrl } from "../lib/config.ts";

const idl = JSON.parse(
  readFileSync(new URL("../lib/idl/walnut.json", import.meta.url), "utf8"),
) as Idl;
const chain = createChain(createConnection(rpcUrl()), idl);
const protocol = await loadProtocol(chain);
const price = await fetchSolPrice(chain.connection);
console.log(`market: ${protocol.marketAddress.toBase58()}`);
console.log(`asset state: ${protocol.market.assetState.toBase58()}`);
console.log(`usdc vault: ${protocol.market.usdcVault.toBase58()}`);
console.log(`ltv_bps: ${protocol.ltvBps}`);
console.log(`mxe public key: ${protocol.mxePublicKey.length} bytes`);
console.log(`pyth account: ${solUsdPriceAccount().toBase58()}`);
console.log(
  `SOL price (${price.source}): ${Number(price.price) / 1e6} USDC` +
    (price.publishTime !== undefined ? ` @ ${price.publishTime}` : ""),
);
