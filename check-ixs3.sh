#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
cat > check-ixs3.mts << 'ENDSCRIPT'
import { readFileSync } from "fs";
const idl = JSON.parse(readFileSync("./target/idl/walnut.json", "utf8"));
const ix = idl.instructions?.find((i: any) => i.name === "update_pool_band");
if (ix) {
  console.log("update_pool_band accounts:");
  for (const acc of ix.accounts) {
    console.log(`  ${acc.name}: writable=${acc.writable}, signer=${acc.signer}`);
    if (acc.pda) console.log(`    PDA: ${JSON.stringify(acc.pda)}`);
    if (acc.address) console.log(`    address: ${acc.address}`);
  }
  console.log("update_pool_band args:");
  for (const arg of ix.args) {
    console.log(`  ${arg.name}: ${JSON.stringify(arg.type)}`);
  }
} else {
  console.log("update_pool_band not found");
}
const setRate = idl.instructions?.find((i: any) => i.name === "set_lending_rate_source");
if (setRate) {
  console.log("\nset_lending_rate_source accounts:");
  for (const acc of setRate.accounts) {
    console.log(`  ${acc.name}: writable=${acc.writable}, signer=${acc.signer}`);
  }
}
ENDSCRIPT
tsx check-ixs3.mts 2>&1
rm -f check-ixs3.mts
