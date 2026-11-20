#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
node --input-type=commonjs -e '
const fs = require("fs");
const idl = JSON.parse(fs.readFileSync("./target/idl/walnut.json", "utf8"));
console.log("accounts:", idl.accounts?.length);
console.log("first:", JSON.stringify(idl.accounts?.[0]));
console.log("address:", idl.address);
'
