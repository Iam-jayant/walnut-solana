#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"
echo "=== Solana config ==="
solana config get 2>&1
echo "=== Default keypair address ==="
solana address 2>&1
echo "=== Deployer keypair address ==="
solana address -k ~/.config/solana/walnut-deployer.json 2>&1
echo "=== Deployer balance ==="
solana balance -k ~/.config/solana/walnut-deployer.json -u https://api.devnet.solana.com 2>&1
