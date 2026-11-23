#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
cat > test-convert.mts << 'ENDSCRIPT'
import { convertIdlToCamelCase } from "@anchor-lang/core";
import { readFileSync } from "fs";
const idl = JSON.parse(readFileSync("./target/idl/walnut.json", "utf8"));
console.log("Before - accounts:", idl.accounts?.length);
console.log("Before - first account name:", idl.accounts?.[0]?.name);
console.log("Before - first account size:", idl.accounts?.[0]?.size);
console.log("Before - address:", idl.address);

const camelIdl = convertIdlToCamelCase(idl);
console.log("\nAfter - accounts:", camelIdl.accounts?.length);
console.log("After - first account name:", camelIdl.accounts?.[0]?.name);
console.log("After - first account size:", camelIdl.accounts?.[0]?.size);
console.log("After - address:", camelIdl.address);
console.log("After - types length:", camelIdl.types?.length);
console.log("After - first type name:", camelIdl.types?.[0]?.name);
ENDSCRIPT
tsx test-convert.mts 2>&1
rm -f test-convert.mts
