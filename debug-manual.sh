#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana/packages/sdk
export ANCHOR_PROVIDER_URL="https://devnet.helius-rpc.com/?api-key=24db9681-90e5-4d92-bd8c-829d2db5f29e"
export ANCHOR_WALLET="$HOME/.config/solana/walnut-deployer.json"
export WALNUT_BENCH_CLUSTER_OFFSET=456
export WALNUT_PROGRAM_ID="5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea"
cat > debug-manual.mts << 'ENDSCRIPT'
import { Connection, Keypair, PublicKey, Transaction, SystemProgram } from "@solana/web3.js";
import { AnchorProvider, Wallet } from "@anchor-lang/core";
import { readFileSync } from "fs";
import { homedir } from "os";
import { createRequire } from "module";
const require = createRequire(import.meta.url);
const { sha256 } = require("@noble/hashes/sha256");
const { createPacker } = require("@arcium-hq/client");

async function main() {
  const programId = new PublicKey("5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea");
  const conn = new Connection("https://devnet.helius-rpc.com/?api-key=24db9681-90e5-4d92-bd8c-829d2db5f29e", "confirmed");
  const kp = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(readFileSync(homedir() + "/.config/solana/walnut-deployer.json", "utf-8")))
  );
  const wallet = new Wallet(kp);
  const provider = new AnchorProvider(conn, wallet, { commitment: "confirmed" });

  // Derive PDAs
  const market = PublicKey.findProgramAddressSync([Buffer.from("market")], programId)[0];
  const marketAcc = await (provider as any).connection.getAccountInfo(market);
  if (!marketAcc) throw new Error("Market not found");
  // Parse market account to get asset_state
  // Market layout: 8 (discriminator) + ...
  // Actually let's just use the program to fetch
  
  console.log("Manual instruction building not yet implemented - need to parse market");
  await conn.close();
  process.exit(0);
}
main().catch(console.error);
ENDSCRIPT
tsx debug-manual.mts 2>&1
rm -f debug-manual.mts
