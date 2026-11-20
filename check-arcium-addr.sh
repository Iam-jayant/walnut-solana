#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
node --input-type=commonjs -e '
const c = require("@arcium-hq/client");
const { PublicKey } = require("@solana/web3.js");
console.log("ARCIUM_ADDR:", c.ARCIUM_ADDR);
console.log("ARCIUM_STAKING_ADDR:", c.ARCIUM_STAKING_ADDR);
const expectedArcium = new PublicKey("Arcj82pX7HxYKLR92qvgZUAd7vGS1k4hQvAFcPATFdEQ");
console.log("Expected Arcium:", expectedArcium.toBase58());
console.log("Match:", c.ARCIUM_ADDR === expectedArcium.toBase58());
'
