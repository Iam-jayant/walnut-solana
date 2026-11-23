#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"
cd /mnt/d/solana/walnut-solana
echo "=== sign_pda_account under Walnut ==="
solana account 8noo8125cUvAgcFE4pPJGX9Ci28BAW4LFCpMXwsGAEiW --url https://devnet.helius-rpc.com/?api-key=24db9681-90e5-4d92-bd8c-829d2db5f29e 2>&1
echo ""
echo "=== sign_pda_account under Arcium ==="
solana account 3iJUqefe12B14HVnedJGhmbS9uTGvdy1vnxD5kXVtAj4 --url https://devnet.helius-rpc.com/?api-key=24db9681-90e5-4d92-bd8c-829d2db5f29e 2>&1
