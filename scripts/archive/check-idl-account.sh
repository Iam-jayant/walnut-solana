#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"
cd /mnt/d/solana/walnut-solana
# Check if there's an IDL account on-chain for this program
# IDL account address = findProgramAddress(["idl", programId], IDL_PROGRAM_ID)
echo "=== Checking for IDL account ==="
# The IDL program is Anchor's IDL program
IDL_PROGRAM="AnL1zKjFv2QbScl574rQVxRRyNfZyJpN5Mwyz8k2gXVz"  # Anchor IDL program (approximate)
# Actually, let's just try to fetch the program data
solana program show 5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea --url https://devnet.helius-rpc.com/?api-key=24db9681-90e5-4d92-bd8c-829d2db5f29e 2>&1 | head -20
