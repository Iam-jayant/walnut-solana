#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
export RPC_URL="https://devnet.helius-rpc.com/?api-key=24db9681-90e5-4d92-bd8c-829d2db5f29e"
node scripts/init-mxe-devnet.mjs
