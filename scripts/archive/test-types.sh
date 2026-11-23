#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
cat > test-types.mts << 'ENDSCRIPT'
import { readFileSync } from "fs";
const idl = JSON.parse(readFileSync("./target/idl/walnut.json", "utf8"));
console.log("Types length:", idl.types?.length);
const accountNames = idl.accounts?.map(a => a.name);
console.log("Account names:", accountNames);
// Check if each account name exists as a type
for (const accName of accountNames || []) {
  const typeDef = idl.types?.find((t: any) => t.name === accName);
  console.log(`  Type "${accName}":`, typeDef ? "FOUND" : "NOT FOUND");
}
ENDSCRIPT
tsx test-types.mts 2>&1
rm -f test-types.mts
