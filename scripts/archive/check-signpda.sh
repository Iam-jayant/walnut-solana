#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
cat > check-signpda.mts << 'ENDSCRIPT'
import { readFileSync } from "fs";
const idl = JSON.parse(readFileSync("./target/idl/walnut.json", "utf8"));
// Find instructions that use sign_pda_account
for (const ix of idl.instructions || []) {
  const hasSignPda = ix.accounts?.some((a: any) => a.name === "sign_pda_account");
  if (hasSignPda) {
    console.log(`${ix.name}:`);
    for (const acc of ix.accounts) {
      if (acc.name === "sign_pda_account") {
        console.log(`  sign_pda_account: ${JSON.stringify(acc)}`);
      }
    }
  }
}
ENDSCRIPT
tsx check-signpda.mts 2>&1
rm -f check-signpda.mts
