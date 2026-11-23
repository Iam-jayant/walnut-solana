#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
cat > test-borsh.mts << 'ENDSCRIPT'
import { BorshCoder } from "@anchor-lang/core";
import { readFileSync } from "fs";
const idl = JSON.parse(readFileSync("./target/idl/walnut.json", "utf8"));
try {
  const coder = new BorshCoder(idl);
  console.log("coder.accounts:", typeof coder.accounts);
  console.log("coder.accounts.size:", typeof coder.accounts?.size);
  const s = coder.accounts.size("ArciumSignerAccount");
  console.log("size(ArciumSignerAccount):", s);
} catch(e) {
  console.log("Error:", e.message);
  console.log("Stack:", e.stack.split("\n").slice(0, 5).join("\n"));
}
ENDSCRIPT
tsx test-borsh.mts 2>&1
rm -f test-borsh.mts
