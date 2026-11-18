#!/usr/bin/env bash
# Walnut-Diffn: reclaim unused circuit rent, upgrade the devnet program, run the private loan demo.
# Run from WSL:  bash scripts/diffn-devnet.sh 2>&1 | tee ~/diffn.log
set -uo pipefail
export NVM_DIR="$HOME/.nvm"; . "$NVM_DIR/nvm.sh"
export PATH="$HOME/.local/share/pnpm:$PATH"
cd "$(dirname "$0")/.."

ID=5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea
KP="${KP:-$HOME/.config/solana/walnut-deployer.json}"
BUF="$HOME/.config/solana/walnut-buffer.json"
RPC="$(solana config get | awk '/RPC URL/ {print $3}')"
BAND_OFFSET=1361508387   # comp-def offset of the unused update_utilization_band circuit
f() { grep -v -i "api-key"; }
lamports() { solana balance --lamports | awk '{print $1}'; }

echo "start balance: $(solana balance)"

echo "== 1. reclaim rent of the unused update_utilization_band circuit =="
if solana account "$(node -e '
  const c=require("@arcium-hq/client");const {PublicKey}=require("@solana/web3.js");
  const p=new PublicKey(process.argv[1]);
  console.log(c.getRawCircuitAccAddress(c.getCompDefAccAddress(p,Number(process.argv[2])),0).toBase58());
' "$ID" "$BAND_OFFSET" 2>/dev/null)" > /dev/null 2>&1; then
  arcium close-computation-definition-buffers -o $BAND_OFFSET -p $ID -i 0 -k "$KP" --rpc-url "$RPC" 2>&1 | f
  if [ "${PIPESTATUS[0]}" -ne 0 ]; then
    echo "comp def is finalized: deactivating, then waiting 90 s for the 180-slot TTL"
    arcium deactivate-computation-definition -o $BAND_OFFSET -p $ID -k "$KP" --rpc-url "$RPC" 2>&1 | f
    sleep 90
    arcium close-computation-definition-buffers -o $BAND_OFFSET -p $ID -i 0 -k "$KP" --rpc-url "$RPC" 2>&1 | f
  fi
  arcium close-computation-definition -o $BAND_OFFSET -p $ID -c 456 -k "$KP" --rpc-url "$RPC" 2>&1 | f
else
  echo "raw circuit account already gone, skipping"
fi
echo "balance after reclaim: $(solana balance)"
if [ "$(lamports)" -lt 21000000000 ]; then
  echo "STOP: need at least 21 SOL for the upgrade (buffer ~16 SOL is refunded after deploy). Fund the wallet and re-run."
  exit 1
fi

echo "== 2. upgrade the program =="
for b in $(solana program show --buffers 2>/dev/null | awk 'NR>2 && $1 ~ /^[1-9A-HJ-NP-Za-km-z]{32,}$/ {print $1}'); do
  echo "closing stray buffer $b"; solana program close "$b" --recipient "$(solana address)" 2>&1 | f
done
cur=$(solana program show $ID | awk '/Data Length/ {print $3}')
so=$(stat -c %s target/deploy/walnut.so)
if [ "$so" -gt "$cur" ]; then
  solana program extend $ID $(( so - cur + 20000 )) 2>&1 | f
fi
[ -f "$BUF" ] || solana-keygen new --no-bip39-passphrase -s -o "$BUF" > /dev/null
for i in 1 2 3 4 5 6; do
  solana program write-buffer target/deploy/walnut.so --buffer "$BUF" --use-rpc \
    --with-compute-unit-price 50000 --max-sign-attempts 500 2>&1 | f | tail -2
  solana program show "$(solana address -k "$BUF")" > /dev/null 2>&1 && break
  echo "write-buffer attempt $i incomplete, retrying"; sleep 5
done
solana program deploy --program-id target/deploy/walnut-keypair.json --buffer "$BUF" \
  --upgrade-authority "$KP" --use-rpc --with-compute-unit-price 50000 2>&1 | f | tail -3
solana program show $ID | grep -E "Data Length|Last Deployed"
echo "balance after upgrade: $(solana balance)"

echo "== 3. private loan demo =="
export ANCHOR_PROVIDER_URL="$RPC"
export ANCHOR_WALLET="$KP"
export WALNUT_BENCH_CLUSTER_OFFSET=456
pnpm --filter @walnut/sdk demo:loan 2>&1 | f
echo "demo exit: ${PIPESTATUS[0]}"
echo "final balance: $(solana balance)"