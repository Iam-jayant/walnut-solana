#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
cat > check-ixs2.mts << 'ENDSCRIPT'
import { readFileSync } from "fs";
const idl = JSON.parse(readFileSync("./target/idl/walnut.json", "utf8"));
console.log("All instruction names:");
for (const i of idl.instructions || []) {
  console.log(`  ${i.name}`);
}
ENDSCRIPT
tsx check-ixs2.mts 2>&1
rm -f check-ixs2.mts
