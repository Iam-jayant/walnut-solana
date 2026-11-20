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
cat > debug-prog2.mts << 'ENDSCRIPT'
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import { AnchorProvider, Wallet } from "@anchor-lang/core";
import { readFileSync } from "fs";
import { homedir } from "os";
import { createRequire } from "module";
const require = createRequire(import.meta.url);
const idl = require("/mnt/d/solana/walnut-solana/target/idl/walnut.json");

async function main() {
  const programId = new PublicKey("5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea");
  const conn = new Connection("https://devnet.helius-rpc.com/?api-key=24db9681-90e5-4d92-bd8c-829d2db5f29e", "confirmed");
  const kp = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(readFileSync(homedir() + "/.config/solana/walnut-deployer.json", "utf-8")))
  );
  const wallet = new Wallet(kp);
  const provider = new AnchorProvider(conn, wallet, { commitment: "confirmed" });

  console.log("idl.address from JSON:", idl.address);
  const program = new (require("@anchor-lang/core").Program)(idl, provider, programId);
  console.log("program.programId:", program.programId.toBase58());
  console.log("idl.address field:", program.idl.address);
  console.log("Match:", program.idl.address === program.programId.toBase58());

  // Now try to call update_pool_band
  console.log("\nAttempting update_pool_band...");
  try {
    const { buildUpdatePoolBandInstruction, derivePoolStatePda, deriveLendingPoolPda, derivePoolCounterPda, deriveIntentPda, RATE_SOURCE_BAND } = await import("./src/index.ts");
    const market = PublicKey.findProgramAddressSync([Buffer.from("market")], programId)[0];
    const marketAcc = await (program.account as any)["market"].fetchNullable(market);
    if (!marketAcc) throw new Error("Market not found");
    const assetState = marketAcc.assetState;
    const poolState = derivePoolStatePda(programId, market);
    const lendingPool = deriveLendingPoolPda(programId, market);
    const counter = await (program.account as any)["operationCounter"].fetchNullable(derivePoolCounterPda(programId, poolState));
    const sequence = BigInt(String((counter as any)?.nextSequence ?? 0));
    const offset = BigInt(Math.floor(Math.random() * 2 ** 48));
    const ix = await buildUpdatePoolBandInstruction({
      program,
      payer: ctxPayer.publicKey,
      market,
      assetState,
      poolState,
      sequence,
      computationOffset: offset,
      clusterOffset: 456,
    });
    console.log("Instruction built successfully");
    const sig = await provider.sendAndConfirm(new (require("@solana/web3.js").Transaction()).add(ix), [kp], { commitment: "confirmed" });
    console.log("Signature:", sig);
  } catch(e) {
    console.log("Error:", e.message);
  }
  
  await conn.close();
  process.exit(0);
}

const ctxPayer = kp;
main().catch(console.error);
ENDSCRIPT
tsx debug-prog2.mts 2>&1
rm -f debug-prog2.mts
