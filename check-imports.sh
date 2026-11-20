#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
for f in packages/sdk/src/pda.ts packages/sdk/src/private-op.ts packages/sdk/src/tracker.ts packages/sdk/demo/supply-demo.ts packages/sdk/demo/private-e2e.ts packages/sdk/demo/loan-demo.ts packages/sdk/demo/lend-e2e.ts packages/sdk/demo/balance-demo.ts; do
  echo "=== $f ==="
  head -45 "$f" | grep -n 'from "@anchor-lang/core"'
done
