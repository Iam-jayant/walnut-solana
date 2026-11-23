#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
node --input-type=commonjs -e '
const idl = require("./target/idl/walnut.json");
console.log("First account:", JSON.stringify(idl.accounts?.[0]));
console.log("Has size:", idl.accounts?.[0]?.size);
'
