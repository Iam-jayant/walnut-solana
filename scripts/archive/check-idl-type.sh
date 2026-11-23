#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
node --input-type=commonjs -e '
const idl = require("./target/idl/walnut.json");
console.log("accounts is array:", Array.isArray(idl.accounts));
console.log("accounts type:", typeof idl.accounts);
console.log("accounts length:", idl.accounts?.length);
if (idl.accounts && !Array.isArray(idl.accounts)) {
  console.log("account keys:", Object.keys(idl.accounts));
}
'
