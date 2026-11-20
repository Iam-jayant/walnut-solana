#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana/packages/sdk
export WALNUT_PROGRAM_ID="5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea"
cat > debug-prog4.mts << 'ENDSCRIPT'
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import { AnchorProvider, Wallet, Program } from "@anchor-lang/core";
import { readFileSync } from "fs";
import { homedir } from "os";
import { createRequire } from "module";
const require = createRequire(import.meta.url);
const idlRaw = require("/mnt/d/solana/walnut-solana/target/idl/walnut.json");

async function main() {
  const programId = new PublicKey("5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea");
  const conn = new Connection("https://devnet.helius-rpc.com/?api-key=24db9681-90e5-4d92-bd8c-829d2db5f29e", "confirmed");
  const kp = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(readFileSync(homedir() + "/.config/solana/walnut-deployer.json", "utf-8")))
  );
  const wallet = new Wallet(kp);
  const provider = new AnchorProvider(conn, wallet, { commitment: "confirmed" });

  console.log("Raw idl.address:", idlRaw.address);
  
  const program = new Program(idlRaw, provider, programId);
  console.log("program.programId:", program.programId.toBase58());
  console.log("program.idl.address:", program.idl.address);
  console.log("Match:", program.idl.address === program.programId.toBase58());

  // Now test building an instruction - check what program ID is used
  const methods = program.methods as any;
  const initPool = methods["initLendingPool"];
  console.log("\nHas initLendingPool:", !!initPool);
  
  await conn.close();
  process.exit(0);
}
main().catch(console.error);
ENDSCRIPT
tsx debug-prog4.mts 2>&1
rm -f debug-prog4.mts
