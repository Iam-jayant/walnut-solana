#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
cat > check-metadata.mts << 'ENDSCRIPT'
import { readFileSync } from "fs";
const idl = JSON.parse(readFileSync("./target/idl/walnut.json", "utf8"));
console.log("top-level address:", idl.address);
console.log("metadata:", JSON.stringify(idl.metadata, null, 2));
console.log("All keys:", Object.keys(idl));
ENDSCRIPT
tsx check-metadata.mts 2>&1
rm -f check-metadata.mts
