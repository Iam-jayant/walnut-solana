#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
echo "=== Installing @coral-xyz/anchor ==="
npm install @coral-xyz/anchor@0.29.0 2>&1 || true
echo "=== Checking ==="
ls node_modules/@coral-xyz/anchor/package.json 2>&1 || echo "NOT installed"
