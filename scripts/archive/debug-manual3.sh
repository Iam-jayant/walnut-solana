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
cat > debug-manual3.mts << 'ENDSCRIPT'
import { Connection, Keypair, PublicKey, Transaction } from "@solana/web3.js";
import { AnchorProvider, Wallet } from "@anchor-lang/core";
import { readFileSync } from "fs";
import { homedir } from "os";
import * as crypto from "node:crypto";
import { createRequire } from "module";
const require = createRequire(import.meta.url);
const { BN } = require("@anchor-lang/core");

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
  const marketAcc = await provider.connection.getAccountInfo(market);
  if (!marketAcc) throw new Error("Market not found");
  const view = new DataView(marketAcc.data.buffer, marketAcc.data.byteOffset, marketAcc.data.byteLength);
  const assetState = new PublicKey(marketAcc.data.slice(41, 73));
  const poolState = PublicKey.findProgramAddressSync([Buffer.from("pool_state"), market.toBuffer()], programId)[0];
  const lendingPool = PublicKey.findProgramAddressSync([Buffer.from("lending_pool"), market.toBuffer()], programId)[0];

  const counterAddr = PublicKey.findProgramAddressSync([Buffer.from("operation_counter"), poolState.toBuffer()], programId)[0];
  const counterAcc = await provider.connection.getAccountInfo(counterAddr);
  let sequence = 0n;
  if (counterAcc) {
    const cv = new DataView(counterAcc.data.buffer, counterAcc.data.byteOffset, counterAcc.data.byteLength);
    sequence = cv.getBigUint64(41, false); // 8 disc + 32 owner + 1 bump = 41
  }
  console.log("sequence:", sequence);

  const offset = BigInt(Math.floor(Math.random() * 2 ** 48));
  console.log("offset:", offset);

  // Instruction discriminator
  const disc = crypto.createHash("sha256").update("global:update_pool_band").digest().slice(0, 8);
  
  // Encode args: computation_offset (u64 LE), sequence (u64 LE)
  const argsBuf = Buffer.alloc(16);
  new DataView(argsBuf.buffer, argsBuf.byteOffset, 16).setBigUint64(0, offset, true);
  new DataView(argsBuf.buffer, argsBuf.byteOffset, 16).setBigUint64(8, sequence, true);
  
  const ixData = Buffer.concat([Buffer.from(disc), argsBuf]);

  // Account metas
  const signPdaAddr = PublicKey.findProgramAddressSync([Buffer.from("ArciumSignerAccount")], programId)[0];
  const operationIntent = PublicKey.findProgramAddressSync(
    [Buffer.from("operation_intent"), poolState.toBuffer(), new BN(sequence).toArray("le", 8)],
    programId
  )[0];

  console.log("market:", market.toBase58());
  console.log("assetState:", assetState.toBase58());
  console.log("poolState:", poolState.toBase58());
  console.log("lendingPool:", lendingPool.toBase58());
  console.log("counterAddr:", counterAddr.toBase58());
  console.log("operationIntent:", operationIntent.toBase58());
  console.log("signPdaAddr:", signPdaAddr.toBase58());

  const ix = {
    programId,
    keys: [
      { pubkey: kp.publicKey, isSigner: true, isWritable: false },
      { pubkey: market, isSigner: false, isWritable: false },
      { pubkey: assetState, isSigner: false, isWritable: false },
      { pubkey: poolState, isSigner: false, isWritable: true },
      { pubkey: lendingPool, isSigner: false, isWritable: false },
      { pubkey: counterAddr, isSigner: false, isWritable: false },
      { pubkey: operationIntent, isSigner: false, isWritable: true },
      { pubkey: signPdaAddr, isSigner: false, isWritable: true },
    ],
    data: ixData,
  };

  const tx = new Transaction().add(ix);
  try {
    const sig = await provider.sendAndConfirm(tx, [kp], { commitment: "confirmed" });
    console.log("update_pool_band manual SUCCESS:", sig);
  } catch(e: any) {
    console.log("update_pool_band manual FAILED:", e.message);
    if (e.logs) console.log("Logs:", e.logs.join("\n"));
  }
  
  await conn.close();
  process.exit(0);
}
main().catch(e => {
  console.error("Error:", e.message);
  process.exit(1);
});
ENDSCRIPT
tsx debug-manual3.mts 2>&1
rm -f debug-manual3.mts
