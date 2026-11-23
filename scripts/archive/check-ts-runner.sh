#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
echo "=== dist ==="
ls packages/sdk/dist/ 2>&1 | head -10
echo "=== tsx ==="
npx tsx --version 2>&1 || echo 'tsx not found'
echo "=== ts-node ==="
npx ts-node --version 2>&1 || echo 'ts-node not found'
