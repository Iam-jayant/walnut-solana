#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
echo "=== @anchor-lang/core dist ==="
ls node_modules/@anchor-lang/core/dist/ 2>&1
echo "=== @anchor-lang/core types ==="
ls node_modules/@anchor-lang/core/types/ 2>&1
echo "=== @anchor-lang/core package.json ==="
cat node_modules/@anchor-lang/core/package.json 2>&1
