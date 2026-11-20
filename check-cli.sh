#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
export PATH="$HOME/.local/share/solana/install/active_release/bin:$HOME/.local/bin:$PATH"
echo "=== solana ===" && (solana --version 2>&1 || echo "solana NOT found")
echo "=== arcium ===" && (arcium --version 2>&1 || echo "arcium NOT found")
echo "=== cargo ===" && (cargo --version 2>&1 || echo "cargo NOT found")
echo "=== anchor ===" && (anchor --version 2>&1 || echo "anchor NOT found")
echo "=== rustup ===" && (rustup --version 2>&1 || echo "rustup NOT found")
