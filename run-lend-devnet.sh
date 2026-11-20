#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
export PATH="$HOME/.local/share/solana/install/active_release/bin:$HOME/.local/share/pnpm:$HOME/.cargo/bin:$PATH"
cd /mnt/d/solana/walnut-solana
bash scripts/lend-devnet.sh 2>&1 | tee /mnt/d/solana/walnut-solana/lend.log
