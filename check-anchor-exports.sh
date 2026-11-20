#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
node --input-type=commonjs -e '
const c = require("@anchor-lang/core");
console.log("BN:", typeof c.BN);
console.log("AnchorProvider:", typeof c.AnchorProvider);
console.log("Wallet:", typeof c.Wallet);
console.log("Provider:", typeof c.Provider);
'
