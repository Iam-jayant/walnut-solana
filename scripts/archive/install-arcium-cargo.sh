#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
export PATH="$HOME/.cargo/bin:$PATH"

echo "=== Installing arcium-cli via cargo ==="
cargo install arcium-cli --version 0.16.0 --locked --force 2>&1 || true
echo "=== Checking ==="
export PATH="$HOME/.cargo/bin:$PATH"
which arcium 2>&1 || arcium --version 2>&1 || echo "arcium not found via cargo"
