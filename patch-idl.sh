#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
node --input-type=commonjs -e '
const fs = require("fs");
const path = "./target/idl/walnut.json";
const idl = JSON.parse(fs.readFileSync(path, "utf8"));
let count = 0;
if (idl.accounts) {
  for (const acc of idl.accounts) {
    if (acc.size === undefined) {
      acc.size = 0;
      count++;
    }
  }
}
fs.writeFileSync(path, JSON.stringify(idl, null, 2) + "\n");
console.log("Patched", count, "accounts with size=0");
'
