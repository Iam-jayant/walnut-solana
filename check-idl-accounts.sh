#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
node --input-type=commonjs -e '
const idl = require("./target/idl/walnut.json");
if (idl.accounts) {
  for (const acc of idl.accounts) {
    const hasSize = "size" in acc;
    console.log(acc.name, "size:", acc.size, "hasSize:", hasSize);
  }
} else {
  console.log("No accounts in IDL");
}
console.log("Total accounts:", idl.accounts?.length || 0);
'
