#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
node --input-type=commonjs -e '
try {
  const BN = require("bn.js");
  console.log("bn.js BN:", typeof BN);
  const b = new BN(123);
  console.log("BN instance:", b.toString());
} catch(e) {
  console.log("Error:", e.message);
}
'
