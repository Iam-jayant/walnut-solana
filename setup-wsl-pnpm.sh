#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
npm install -g pnpm@9.15.9
which pnpm
pnpm --version
echo "=== pnpm install in workspace ==="
cd /mnt/d/solana/walnut-solana
pnpm install 2>&1
echo "=== DONE ==="
