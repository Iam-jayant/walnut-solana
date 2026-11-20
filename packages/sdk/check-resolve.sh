#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana/packages/sdk
node --input-type=commonjs -e '
const path = require.resolve("@anchor-lang/core");
console.log("Resolved:", path);
const c = require("@anchor-lang/core");
console.log("Has Program:", typeof c.Program);
'
