#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20

echo "=== Installing Arcium CLI ==="
mkdir -p "$HOME/.local/bin"

# Download Arcium CLI 0.16.0 directly
curl -L -o /tmp/arcium-cli.tar.gz "https://github.com/arcium-network/arcium-cli/releases/download/v0.16.0/arcium-cli-linux-x86_64.tar.gz" 2>&1
echo "Download complete, extracting..."
tar -xzf /tmp/arcium-cli.tar.gz -C /tmp/ 2>&1
ls -la /tmp/arcium-cli 2>&1
cp /tmp/arcium-cli "$HOME/.local/bin/arcium"
chmod +x "$HOME/.local/bin/arcium"
echo "=== Verifying ==="
export PATH="$HOME/.local/bin:$PATH"
"$HOME/.local/bin/arcium" --version 2>&1
