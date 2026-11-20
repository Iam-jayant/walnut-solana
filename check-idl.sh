#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
node --input-type=commonjs -e '
const idl = require("./target/idl/walnut.json");
console.log("IDL address:", idl.address || idl.metadata?.address || "NOT FOUND");
console.log("IDL name:", idl.name);
console.log("Expected: 5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea");
'
