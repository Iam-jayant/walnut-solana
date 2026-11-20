#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20

echo "=== Installing Rust toolchain ==="
if ! command -v cargo &> /dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none 2>&1
fi
export PATH="$HOME/.cargo/bin:$PATH"
rustup --version 2>&1 || echo "rustup install may have failed"

echo "=== Installing Rust toolchain components ==="
export PATH="$HOME/.cargo/bin:$PATH"
rustup toolchain install stable --profile minimal 2>&1 || true
rustup default stable 2>&1 || true
rustup target add solana 2>&1 || true
cargo --version 2>&1 || echo "cargo may not be available"

echo "=== Installing Anchor CLI ==="
if ! command -v anchor &> /dev/null; then
  cargo install anchor-cli --locked --force 2>&1 || true
fi
anchor --version 2>&1 || echo "anchor install may have failed"

echo "=== Installing Arcium CLI ==="
if ! command -v arcium &> /dev/null; then
  ARCIUM_LATEST=$(curl -s https://api.github.com/repos/arcium-network/arcium-cli/releases/latest | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/' 2>&1 || echo "v0.16.0")
  echo "Latest Arcium CLI: $ARCIUM_LATEST"
  curl -L "https://github.com/arcium-network/arcium-cli/releases/download/${ARCIUM_LATEST}/arcium-cli-linux-x86_64.tar.gz" -o /tmp/arcium-cli.tar.gz 2>&1 || true
  tar -xzf /tmp/arcium-cli.tar.gz -C /tmp/ 2>&1 || true
  mkdir -p "$HOME/.local/bin"
  cp /tmp/arcium-cli "$HOME/.local/bin/arcium" 2>&1 || true
  chmod +x "$HOME/.local/bin/arcium" 2>&1 || true
fi
export PATH="$HOME/.local/bin:$PATH"
arcium --version 2>&1 || echo "arcium install may have failed"

echo "=== DONE ==="
