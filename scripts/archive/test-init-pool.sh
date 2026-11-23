#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"
cd /mnt/d/solana/walnut-solana/packages/sdk
RPC="$(solana config get | awk '/RPC URL/ {print $3}')"
echo "RPC URL: $RPC"
ANCHOR_PROVIDER_URL="$RPC" ANCHOR_WALLET="$HOME/.config/solana/walnut-deployer.json" WALNUT_BENCH_CLUSTER_OFFSET=456 WALNUT_PROGRAM_ID="5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea" \
  tsx scripts/lend-deploy.ts init-pool 2>&1
