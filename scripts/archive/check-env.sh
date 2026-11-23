#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
export PATH="$HOME/.local/share/solana/install/active_release/bin:$HOME/.local/share/pnpm:$HOME/.local/bin:$PATH"
echo "HOME=$HOME"
echo "PATH=$PATH"
echo "arcium path: $(which arcium 2>&1 || echo 'not found')"
ls -la "$HOME/.local/bin/" 2>&1 || echo "no .local/bin"
ls -la ~/.local/bin/ 2>&1 || echo "no ~/.local/bin"
