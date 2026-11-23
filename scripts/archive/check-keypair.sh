#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"
echo "=== keypair pubkey ==="
solana-keygen pubkey /mnt/d/solana/walnut-solana/target/deploy/walnut-keypair.json 2>&1
echo "=== expected ==="
echo "5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea"
