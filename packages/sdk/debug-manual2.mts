import { Connection, Keypair, PublicKey, Transaction, SystemProgram } from "@solana/web3.js";
import { AnchorProvider, Wallet } from "@anchor-lang/core";
import { readFileSync } from "fs";
import { homedir } from "os";
import { createRequire } from "module";
const require = createRequire(import.meta.url);
const { sha256 } = require("@noble/hashes/sha256");
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
  // Market layout: 8 (disc) + 32 (pubkey) + 1 (bump) + 32 (oracle) + 32 (feed_id) + 4 (u32) + 16 (u128) + 16 (u128) + 16 (u128) + 16 (u128) + 8 (i64) + 8 (u64) + 1 (u8) + reserved
  const view = new DataView(marketAcc.data.buffer, marketAcc.data.byteOffset, marketAcc.data.byteLength);
  const assetState = new PublicKey(marketAcc.data.slice(41, 73));
  const poolState = PublicKey.findProgramAddressSync([Buffer.from("pool_state"), market.toBuffer()], programId)[0];
  const lendingPool = PublicKey.findProgramAddressSync([Buffer.from("lending_pool"), market.toBuffer()], programId)[0];

  const counter = await provider.connection.getAccountInfo(
    PublicKey.findProgramAddressSync([Buffer.from("operation_counter"), poolState.toBuffer()], programId)[0]
  );
  let sequence = 0n;
  if (counter) {
    // operation_counter layout: 8 (disc) + 32 (owner) + 1 (bump) + 8 (next_sequence)
    sequence = view.getBigUint64(41, false); // approximate
  }
  // Actually let's just read it properly
  const counterAddr = PublicKey.findProgramAddressSync([Buffer.from("operation_counter"), poolState.toBuffer()], programId)[0];
  const counterAcc = await provider.connection.getAccountInfo(counterAddr);
  if (counterAcc) {
    const cv = new DataView(counterAcc.data.buffer, counterAcc.data.byteOffset, counterAcc.data.byteLength);
    sequence = cv.getBigUint64(41, false); // 8 disc + 32 owner + 1 bump = 41
  }
  console.log("sequence:", sequence);

  const offset = BigInt(Math.floor(Math.random() * 2 ** 48));
  console.log("offset:", offset);

  // Instruction discriminator: SHA-256("global:update_pool_band")[0..8]
  const disc = new Uint8Array(await sha256(new TextEncoder().encode("global:update_pool_band"))).slice(0, 8);
  
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
  } catch(e) {
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
