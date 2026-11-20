#!/usr/bin/env bash
# Phase 1: WSL environment setup for Walnut devnet deployment
set -euo pipefail

export NVM_DIR="$HOME/.nvm"
[ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh"

echo "=== Installing Node.js via nvm ==="
nvm install 20 2>&1 || nvm install --lts 2>&1
nvm use 20 2>&1 || nvm use --lts 2>&1
nvm alias default 20 2>&1 || nvm alias default --lts 2>&1
echo "node: $(node --version)"
echo "npm: $(npm --version)"

echo "=== Installing pnpm 9.15.9 ==="
npm install -g pnpm@9.15.9 2>&1
echo "pnpm: $(pnpm --version)"

echo "=== Verifying workspace node_modules ==="
ls /mnt/d/solana/walnut-solana/node_modules/@arcium-hq/client/package.json 2>&1 || echo "arcium client not in workspace node_modules"

echo "=== Done Phase 1a (Node/pnpm) ==="
