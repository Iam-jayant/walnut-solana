#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana/packages/sdk
export ANCHOR_PROVIDER_URL="https://devnet.helius-rpc.com/?api-key=24db9681-90e5-4d92-bd8c-829d2db5f29e"
export ANCHOR_WALLET="$HOME/.config/solana/walnut-deployer.json"
export WALNUT_BENCH_CLUSTER_OFFSET=456
export WALNUT_PROGRAM_ID="5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea"
# Debug: print idl.address from loadWalnutIdl
node --input-type=commonjs -e '
const { loadWalnutIdl } = require("./bench/program.ts");
' 2>&1 || true
# Use tsx to run a debug script
cat > /tmp/debug-idl.mts << 'EOF'
import { loadWalnutIdl } from "./bench/config.ts";
const idl = loadWalnutIdl("/mnt/d/solana/walnut-solana");
console.log("idl.address:", idl.address);
console.log("Expected: 5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea");
EOF
echo "=== Debug IDL address ==="
cd /mnt/d/solana/walnut-solana/packages/sdk
tsx /tmp/debug-idl.mts 2>&1
