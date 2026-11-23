#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
echo "=== Checking @coral-xyz/anchor ==="
ls node_modules/@coral-xyz/anchor/package.json 2>&1 || echo "NOT installed"
echo "=== Checking if @anchor-lang/core has issues ==="
node --input-type=commonjs -e '
const c = require("@anchor-lang/core");
console.log("Program:", typeof c.Program);
console.log("Program keys:", Object.getOwnPropertyNames(c.Program.prototype));
const idl = { version: "0.1.0", name: "test", instructions: [], accounts: [], types: [], address: "11111111111111111111111111111111" };
const { Connection } = require("@solana/web3.js");
const conn = new Connection("https://api.devnet.solana.com");
try {
  const p = new c.Program(idl, conn);
  console.log("Program created OK");
} catch(e) {
  console.log("Program creation failed:", e.message);
  console.log("Stack:", e.stack.split("\n").slice(0, 5).join("\n"));
}
'
