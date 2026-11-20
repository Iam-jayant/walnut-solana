#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
cat > /tmp/debug-idl.mts << 'ENDSCRIPT'
import { readFileSync } from "fs";
const idl = JSON.parse(readFileSync("/mnt/d/solana/walnut-solana/target/idl/walnut.json", "utf8"));
console.log("idl.address from disk:", idl.address);
console.log("Expected: 5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea");
console.log("Match:", idl.address === "5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea");
ENDSCRIPT
tsx /tmp/debug-idl.mts
