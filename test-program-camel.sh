#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
cat > test-program-camel.mts << 'ENDSCRIPT'
import { Connection } from "@solana/web3.js";
import { Program, convertIdlToCamelCase } from "@anchor-lang/core";
import { readFileSync } from "fs";
const idl = JSON.parse(readFileSync("./target/idl/walnut.json", "utf8"));
const camelIdl = convertIdlToCamelCase(idl);
console.log("camelIdl.accounts[0].name:", camelIdl.accounts[0].name);
console.log("camelIdl.accounts[0].size:", camelIdl.accounts[0].size);
// Check if type exists for first account
const typeDef = camelIdl.types?.find((t: any) => t.name === camelIdl.accounts[0].name);
console.log("Type for first account:", typeDef ? "FOUND" : "NOT FOUND");

const conn = new Connection("https://api.devnet.solana.com");
try {
  const program = new Program(camelIdl, conn);
  console.log("Program created OK!");
  console.log("program.account keys:", Object.keys(program.account || {}));
} catch(e) {
  console.log("Error:", e.message);
  console.log("Stack:", e.stack.split("\n").slice(0, 8).join("\n"));
}
ENDSCRIPT
tsx test-program-camel.mts 2>&1
rm -f test-program-camel.mts
