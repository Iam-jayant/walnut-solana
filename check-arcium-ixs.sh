#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
cat > check-arcium-ixs.mts << 'ENDSCRIPT'
import { readFileSync } from "fs";
const idl = JSON.parse(readFileSync("./node_modules/@arcium-hq/client/build/ARCIUM_IDL.json", "utf8"));
console.log("Arcium IDL instructions:");
for (const i of idl.instructions || []) {
  console.log(`  ${i.name}`);
}
ENDSCRIPT
tsx check-arcium-ixs.mts 2>&1 || echo "ARCIUM_IDL.json not found, checking other locations"
ls node_modules/@arcium-hq/client/build/ | grep -i idl
