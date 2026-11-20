#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"
cd /mnt/d/solana/walnut-solana
echo "=== Checking program data ==="
solana program show 5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea --url https://devnet.helius-rpc.com/?api-key=24db9681-90e5-4d92-bd8c-829d2db5f29e 2>&1
