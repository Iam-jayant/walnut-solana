#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
node --input-type=commonjs -e '
const idl = require("./target/idl/walnut.json");
console.log("Has types:", !!idl.types);
console.log("Types length:", idl.types?.length || 0);
console.log("Has accounts:", !!idl.accounts);
console.log("Accounts length:", idl.accounts?.length || 0);
if (idl.types) {
  console.log("First 3 types:", idl.types.slice(0, 3).map(t => t.name));
}
if (idl.accounts) {
  console.log("All account names:", idl.accounts.map(a => a.name));
}
'
