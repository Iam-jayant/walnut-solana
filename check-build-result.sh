#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"
cd /mnt/d/solana/walnut-solana
echo "=== Program keypair pubkey ==="
solana-keygen pubkey target/deploy/walnut-keypair.json 2>&1
echo "=== IDL address ==="
node --input-type=commonjs -e '
const idl = require("./target/idl/walnut.json");
console.log(idl.address);
'
echo "=== deploy state ==="
cat artifacts/lend-deploy-state.json 2>&1 || echo "No state file"
