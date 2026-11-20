#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
export PATH="$HOME/.cargo/bin:$HOME/.local/share/solana/install/active_release/bin:$PATH"
export PATH="$HOME/.local/bin:$PATH"
cd /mnt/d/solana/walnut-solana
echo "=== arcium build ==="
arcium build 2>&1
