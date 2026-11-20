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
console.log("Current idl.address:", idl.address);
idl.address = "5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea";
fs.writeFileSync(path, JSON.stringify(idl, null, 2) + "\n");
console.log("Updated idl.address to:", idl.address);
'
