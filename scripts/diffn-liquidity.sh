#!/usr/bin/env bash
# Walnut-Diffn: mint test USDC straight into the market USDC vault so borrowers
# have liquidity to draw on.
#
# Run from WSL with the deployer keypair (the test USDC mint authority):
#   bash scripts/diffn-liquidity.sh            # mints 10,000 USDC
#   bash scripts/diffn-liquidity.sh 25000      # mints 25,000 USDC
#
# Environment overrides: KP (mint authority keypair, default
# ~/.config/solana/walnut-deployer.json), VAULT (vault token account).
# The RPC URL comes from `solana config`; it is never printed and api-key
# values are filtered from all output.
set -uo pipefail
export NVM_DIR="$HOME/.nvm"; [ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh"
cd "$(dirname "$0")/.."
KP="${KP:-$HOME/.config/solana/walnut-deployer.json}"
MINT=GHLMq3gCUHkb9sGPouXNo4Lt9QpGCmz5UMrtF7EwVDLT
AMOUNT="${1:-10000}"
f() { grep -v -i "api-key"; }

if ! [[ "$AMOUNT" =~ ^[0-9]+(\.[0-9]{1,6})?$ ]]; then
  echo "usage: bash scripts/diffn-liquidity.sh [USDC_AMOUNT]  (a positive number, up to 6 decimals)"
  exit 2
fi
for tool in solana spl-token node; do
  command -v "$tool" > /dev/null 2>&1 || { echo "$tool is not installed"; exit 2; }
done
[ -f "$KP" ] || { echo "mint authority keypair not found: $KP"; exit 2; }

# The market USDC vault from the public devnet profile.
VAULT="${VAULT:-$(node -e 'console.log(JSON.parse(require("fs").readFileSync("config/networks/devnet.json","utf8")).vaults.usdc)')}"
RPC="$(solana config get | awk '/RPC URL/ {print $3}')"
echo "test USDC mint: $MINT"
echo "market USDC vault: $VAULT"
echo "amount: $AMOUNT USDC"

echo "vault before: $(spl-token balance --address "$VAULT" --url "$RPC" 2>&1 | f)"
spl-token mint "$MINT" "$AMOUNT" "$VAULT" \
  --mint-authority "$KP" --fee-payer "$KP" --url "$RPC" 2>&1 | f
status=${PIPESTATUS[0]}
echo "vault after: $(spl-token balance --address "$VAULT" --url "$RPC" 2>&1 | f)"
if [ "$status" -ne 0 ]; then
  echo "mint failed (exit $status)"
  exit "$status"
fi
echo "done"