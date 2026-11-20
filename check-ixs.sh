#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
cat > check-ixs.mts << 'ENDSCRIPT'
import { readFileSync } from "fs";
const idl = JSON.parse(readFileSync("./target/idl/walnut.json", "utf8"));
const updatePoolBand = idl.instructions?.find((i: any) => i.name === "updatePoolBand");
if (updatePoolBand) {
  console.log("updatePoolBand accounts:");
  for (const acc of updatePoolBand.accounts) {
    console.log(`  ${acc.name}: ${acc.pda ? "PDA" : ""}, ${acc.address ? "address=" + acc.address : ""}`);
  }
  console.log("updatePoolBand args:");
  for (const arg of updatePoolBand.args) {
    console.log(`  ${arg.name}: ${JSON.stringify(arg.type)}`);
  }
} else {
  console.log("updatePoolBand not found in IDL");
}
const addTogether = idl.instructions?.find((i: any) => i.name === "addTogether");
if (addTogether) {
  console.log("\naddTogether accounts:");
  for (const acc of addTogether.accounts) {
    console.log(`  ${acc.name}: ${acc.pda ? "PDA" : ""}, ${acc.address ? "address=" + acc.address : ""}`);
  }
}
ENDSCRIPT
tsx check-ixs.mts 2>&1
rm -f check-ixs.mts
