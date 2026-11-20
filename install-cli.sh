#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20

echo "=== Installing Solana CLI ==="
if ! command -v solana &> /dev/null; then
  sh -c "$(curl -sSf https://release.anza.xyz/stable)" 2>&1 || true
fi
export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"
solana --version 2>&1 || echo "solana install may have failed"

echo "=== Installing Arcium CLI ==="
if ! command -v arcium &> /dev/null; then
  curl -L https://github.com/arcium-network/arcium-cli/releases/latest/download/arcium-cli-linux-x86_64.tar.gz -o /tmp/arcium-cli.tar.gz 2>&1 || true
  tar -xzf /tmp/arcium-cli.tar.gz -C /tmp/ 2>&1 || true
  mkdir -p "$HOME/.local/bin"
  cp /tmp/arcium-cli "$HOME/.local/bin/arcium" 2>&1 || true
  chmod +x "$HOME/.local/bin/arcium" 2>&1 || true
fi
export PATH="$HOME/.local/bin:$PATH"
arcium --version 2>&1 || echo "arcium install may have failed"

echo "=== DONE ==="
