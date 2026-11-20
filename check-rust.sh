#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
export PATH="$HOME/.cargo/bin:$PATH"
echo "=== cargo ===" && cargo --version
echo "=== rustc ===" && rustc --version
echo "=== targets ===" && rustup target list --installed 2>&1
