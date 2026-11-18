#!/usr/bin/env bash
# Walnut-Lend: one resumable devnet deployment (walnut-lend task 6.1).
#
# Run from WSL:  bash scripts/lend-devnet.sh 2>&1 | tee ~/lend.log
#
# Steps (each checks on-chain state first and skips when done):
#   1. build program and circuits (pnpm run build)
#   2. reclaim rent from unused comp defs, BEFORE any upload
#   3. single program upgrade (stray buffers, extend, write-buffer, deploy)
#   4. init_lending_pool (idempotent)
#   5. register and upload lend_op, disclose_position, rotate_position_key
#   6. private mode (ADR-0004): band rate source and a first band observation
#   7. optional: reclaim the position_op comp def (RECLAIM_POSITION_OP=1)
#   8. summary with the final balance
#
# Progress: artifacts/lend-deploy-state.json. Re-run the same command after a
# stop (low balance, RPC error); finished steps are skipped.
#
# Flags (environment variables):
#   KP=<path>               deployer keypair (default ~/.config/solana/walnut-deployer.json)
#   REBUILD=1               rebuild even when the state file says the build is done
#   RECLAIM_POSITION_OP=1   after a verified upgrade, also reclaim the position_op comp def.
#                           Default off: the CURRENT web app still queues position_op.
#                           Only set it when the new build no longer needs position_op.
#
# The script never prints the RPC URL, keys or api-key values.
set -uo pipefail
export NVM_DIR="$HOME/.nvm"; [ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh"
export PATH="$HOME/.local/share/pnpm:$PATH"
cd "$(dirname "$0")/.."

ID=5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea
CLUSTER=456
KP="${KP:-$HOME/.config/solana/walnut-deployer.json}"
BUF="$HOME/.config/solana/walnut-buffer.json"
SO=target/deploy/walnut.so
STATE=artifacts/lend-deploy-state.json
DUMP=artifacts/.lend-dump.so
RPC="$(solana config get | awk '/RPC URL/ {print $3}')"
RESERVE=1000000000          # 1 SOL kept free before any spending step
FEE_ONLY=50000000           # 0.05 SOL, enough for reclaim transaction fees
LAMPORTS_PER_BYTE=6960      # 6.96e-6 SOL per byte (rent-exempt, 2 years)
# Comp defs reclaimed before any upload (position_op is handled by step 6).
RECLAIM_NAMES=(add_together init_position apply_position_delta liquidation_receipts oversized_output rotate_viewing_key)
NEW_CIRCUITS=(lend_op disclose_position rotate_position_key pool_op position_private liquidate_pool pool_band)

mkdir -p artifacts

# Filter api-key lines and hide the RPC URL in everything printed.
f() {
  local line
  grep -v -i "api-key" | while IFS= read -r line; do echo "${line//"$RPC"/<rpc>}"; done
}
lamports() { solana balance --lamports | awk '{print $1}'; }
sol() { awk -v l="$1" 'BEGIN { printf "%.3f", l / 1e9 }'; }
acct_exists() { solana account "$1" > /dev/null 2>&1; }

# ---- state file -------------------------------------------------------------
state_get() {
  node -e '
    const fs = require("fs");
    try {
      const s = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
      const v = s.steps && s.steps[process.argv[2]];
      process.stdout.write(v ? v.status : "");
    } catch (e) {}
  ' "$STATE" "$1"
}
state_set() { # step status note
  node -e '
    const fs = require("fs");
    const [file, step, status, note, id] = process.argv.slice(1);
    let s = { version: 1, programId: id, cluster: 456, steps: {} };
    try { s = JSON.parse(fs.readFileSync(file, "utf8")); } catch (e) {}
    s.steps = s.steps || {};
    s.steps[step] = { status, at: new Date().toISOString(), note };
    s.updatedAt = new Date().toISOString();
    fs.writeFileSync(file, JSON.stringify(s, null, 2) + "\n");
  ' "$STATE" "$1" "$2" "${3:-}" "$ID"
}
die() { # step message
  echo "STOP [$1]: $2"
  state_set "$1" stopped "$2"
  exit 1
}
# Stop with a clear message when the balance cannot cover cost + 1 SOL reserve.
require_funds() { # step label cost_lamports
  local bal need
  bal=$(lamports); need=$(( $3 + RESERVE ))
  if [ "$bal" -lt "$need" ]; then
    die "$1" "$2 needs about $(sol "$3") SOL plus a 1 SOL reserve ($(sol "$need") SOL); balance is $(sol "$bal") SOL. Fund the deployer and re-run this command."
  fi
  echo "  funds ok for $2: balance $(sol "$bal") SOL, need $(sol "$need") SOL"
}

# ---- helpers ----------------------------------------------------------------
# Prints "<offset> <comp def address> <raw circuit 0 address>" for a circuit.
# Runs from packages/sdk so @arcium-hq/client and @solana/web3.js resolve.
compdef_addrs() {
  (cd packages/sdk && node --input-type=commonjs -e '
    const c = require("@arcium-hq/client");
    const { PublicKey } = require("@solana/web3.js");
    const off = Buffer.from(c.getCompDefAccOffset(process.argv[1])).readUInt32LE(0);
    const cd = c.getCompDefAccAddress(new PublicKey(process.argv[2]), off);
    console.log(off, cd.toBase58(), c.getRawCircuitAccAddress(cd, 0).toBase58());
  ' "$1" "$ID")
}
arc() { arcium "$@" -k "$KP" --rpc-url "$RPC" 2>&1 | f; return "${PIPESTATUS[0]}"; }

# Reclaim one comp def. Returns 0 done/absent, 2 when it is finalized and has
# been deactivated (caller waits once for the 180-slot TTL, then calls finish).
reclaim_try() {
  local name=$1 off cd raw
  read -r off cd raw < <(compdef_addrs "$name") || return 1
  if ! acct_exists "$cd"; then echo "  $name: comp def absent, skipping"; return 0; fi
  echo "  $name: reclaiming (offset $off)"
  local ok=1
  if acct_exists "$raw"; then
    # Incomplete upload: buffers close directly. Finalized: this fails.
    arc close-computation-definition-buffers -o "$off" -p "$ID" -i 0 && ok=0 || ok=1
  else
    ok=0
  fi
  if [ "$ok" -eq 0 ]; then
    arc close-computation-definition -o "$off" -p "$ID" -c "$CLUSTER" && ok=0 || ok=1
  fi
  if [ "$ok" -eq 0 ] && ! acct_exists "$cd"; then echo "  $name: closed"; return 0; fi
  echo "  $name: finalized, deactivating (needs the 180-slot TTL before closing)"
  arc deactivate-computation-definition -o "$off" -p "$ID" || true
  return 2
}
reclaim_finish() {
  local name=$1 off cd raw
  read -r off cd raw < <(compdef_addrs "$name") || return 1
  acct_exists "$cd" || return 0
  if acct_exists "$raw"; then
    arc close-computation-definition-buffers -o "$off" -p "$ID" -i 0 || true
  fi
  arc close-computation-definition -o "$off" -p "$ID" -c "$CLUSTER" || true
  if acct_exists "$cd"; then echo "  $name: still present after close"; return 1; fi
  echo "  $name: closed"
}
# Reclaim every name given. Waits once for all deactivated comp defs.
reclaim_all() {
  local name rc waiting=() failed=0
  for name in "$@"; do
    reclaim_try "$name"; rc=$?
    if [ "$rc" -eq 2 ]; then waiting+=("$name"); elif [ "$rc" -ne 0 ]; then failed=1; fi
  done
  if [ "${#waiting[@]}" -gt 0 ]; then
    echo "  waiting 90 s for the deactivation TTL of: ${waiting[*]}"
    sleep 90
    for name in "${waiting[@]}"; do reclaim_finish "$name" || failed=1; done
  fi
  return "$failed"
}

# True when the on-chain program bytes equal the local .so.
program_matches_local() {
  local so; so=$(stat -c %s "$SO")
  rm -f "$DUMP"
  solana program dump "$ID" "$DUMP" > /dev/null 2>&1 || return 1
  cmp -s -n "$so" "$DUMP" "$SO"; local rc=$?
  rm -f "$DUMP"
  return "$rc"
}

# Run the TypeScript helper from packages/sdk (needs the deployer keypair).
helper() {
  (
    cd packages/sdk || exit 1
    ANCHOR_PROVIDER_URL="$RPC" ANCHOR_WALLET="$KP" WALNUT_BENCH_CLUSTER_OFFSET="$CLUSTER" \
      node scripts/lend-deploy.ts "$@" 2>&1 | f
    exit "${PIPESTATUS[0]}"
  )
}

# ---- preflight --------------------------------------------------------------
[ -f "$KP" ] || { echo "STOP: deployer keypair not found at $KP (set KP=...)"; exit 1; }
if [ "$(solana address)" != "$(solana address -k "$KP")" ]; then
  echo "STOP: the Solana CLI default keypair is not the deployer keypair. Run: solana config set -k $KP"
  exit 1
fi
START_BAL=$(lamports)
echo "start balance: $(sol "$START_BAL") SOL"

# ---- 1. build ---------------------------------------------------------------
echo "== 1. build program and circuits =="
if [ "$(state_get build)" = "done" ] && [ -f "$SO" ] && [ "${REBUILD:-0}" != "1" ]; then
  echo "  already built (REBUILD=1 to force)"
else
  pnpm run build 2>&1 | f
  [ "${PIPESTATUS[0]}" -eq 0 ] || die build "pnpm run build failed"
  state_set build done "$(stat -c %s "$SO") bytes"
fi
[ -f "$SO" ] || die build "$SO missing after build"
for c in "${NEW_CIRCUITS[@]}"; do
  [ -f "build/$c.arcis" ] || die build "build/$c.arcis missing after build"
done

# ---- 2. reclaim unused comp defs -------------------------------------------
echo "== 2. reclaim rent from unused comp defs =="
require_funds_fee() {
  [ "$(lamports)" -ge "$FEE_ONLY" ] || die "$1" "balance below $(sol "$FEE_ONLY") SOL, too low even for transaction fees"
}
if [ "$(state_get reclaim)" = "done" ]; then
  echo "  already done"
else
  require_funds_fee reclaim
  before=$(lamports)
  reclaim_all "${RECLAIM_NAMES[@]}" || die reclaim "a comp def could not be closed; see messages above, then re-run"
  state_set reclaim done "freed $(sol $(( $(lamports) - before ))) SOL"
fi
echo "balance after reclaim: $(sol "$(lamports)") SOL"

# ---- 3. program upgrade -----------------------------------------------------
echo "== 3. upgrade the program =="
if program_matches_local; then
  echo "  on-chain program already matches the local build, skipping"
  state_set upgrade done "on-chain matches local build"
else
  so=$(stat -c %s "$SO")
  cur=$(solana program show "$ID" | awk '/Data Length/ {print $3}')
  ext=0
  [ "$so" -gt "$cur" ] && ext=$(( so - cur + 20000 ))
  BUFADDR=""
  [ -f "$BUF" ] || solana-keygen new --no-bip39-passphrase -s -o "$BUF" > /dev/null
  BUFADDR=$(solana address -k "$BUF")
  buf_cost=$(( (so + 37) * LAMPORTS_PER_BYTE ))
  acct_exists "$BUFADDR" && buf_cost=0   # buffer rent already paid on a previous run
  cost=$(( buf_cost + ext * LAMPORTS_PER_BYTE ))
  echo "  .so $so bytes, on-chain $cur bytes, extend by $ext bytes"
  echo "  peak need: buffer $(sol "$buf_cost") SOL (refunded on deploy) + extend $(sol $(( ext * LAMPORTS_PER_BYTE ))) SOL"
  require_funds upgrade "the program upgrade" "$cost"

  # Close stray buffers from earlier attempts, but keep our resumable buffer.
  for b in $(solana program show --buffers 2>/dev/null | awk 'NR>2 && $1 ~ /^[1-9A-HJ-NP-Za-km-z]{32,}$/ {print $1}'); do
    [ "$b" = "$BUFADDR" ] && continue
    echo "  closing stray buffer $b"
    solana program close "$b" --recipient "$(solana address)" 2>&1 | f
  done

  if [ "$ext" -gt 0 ]; then
    solana program extend "$ID" "$ext" 2>&1 | f
    [ "${PIPESTATUS[0]}" -eq 0 ] || die upgrade "program extend failed"
  fi

  buffer_complete() {
    solana program dump "$BUFADDR" "$DUMP" > /dev/null 2>&1 || { rm -f "$DUMP"; return 1; }
    cmp -s -n "$so" "$DUMP" "$SO"; local rc=$?
    rm -f "$DUMP"; return "$rc"
  }
  written=0
  for i in 1 2 3 4 5 6; do
    if buffer_complete; then written=1; break; fi
    solana program write-buffer "$SO" --buffer "$BUF" --use-rpc \
      --with-compute-unit-price 50000 --max-sign-attempts 500 2>&1 | f | tail -2
    if buffer_complete; then written=1; break; fi
    echo "  write-buffer attempt $i incomplete, retrying"; sleep 5
  done
  [ "$written" -eq 1 ] || die upgrade "buffer upload did not complete after 6 attempts; re-run to resume"

  deployed=0
  for i in 1 2 3; do
    solana program deploy --program-id target/deploy/walnut-keypair.json --buffer "$BUF" \
      --upgrade-authority "$KP" --use-rpc --with-compute-unit-price 50000 2>&1 | f | tail -3
    if program_matches_local; then deployed=1; break; fi
    echo "  deploy attempt $i not confirmed, retrying"; sleep 5
  done
  [ "$deployed" -eq 1 ] || die upgrade "deploy did not take effect; re-run to resume"
  state_set upgrade done "deployed $so bytes"
fi
solana program show "$ID" | grep -E "Data Length|Last Deployed"
echo "balance after upgrade: $(sol "$(lamports)") SOL"

# ---- 4. init_lending_pool ---------------------------------------------------
echo "== 4. init_lending_pool =="
if [ "$(state_get init_pool)" = "done" ]; then
  echo "  already done"
else
  require_funds init_pool "init_lending_pool (LendingPool account and fees)" 100000000
  helper init-pool || die init_pool "init_lending_pool failed; see messages above"
  state_set init_pool done "LendingPool initialised or already present"
fi

# ---- 5. new comp defs and circuit uploads ----------------------------------
echo "== 5. register and upload ${NEW_CIRCUITS[*]} =="
need=0; pending=()
while read -r circ st; do
  [ -z "$circ" ] && continue
  echo "  $circ: $st"
  if [ "$st" != "finalized" ]; then
    pending+=("$circ")
    need=$(( need + $(stat -c %s "build/$circ.arcis") * LAMPORTS_PER_BYTE ))
  fi
done < <(helper comp-defs status | grep -E '^[a-z_]+ (absent|pending|finalized)$')
if [ "${#pending[@]}" -eq 0 ] && [ "$(helper comp-defs status | grep -c ' finalized$')" -eq "${#NEW_CIRCUITS[@]}" ]; then
  echo "  all circuits finalized, skipping"
  state_set circuits done "${NEW_CIRCUITS[*]}"
else
  require_funds circuits "uploading ${pending[*]:-circuits}" "$need"
  ok=0
  for i in 1 2 3; do
    if helper comp-defs run; then ok=1; break; fi
    echo "  upload attempt $i incomplete, resuming"; sleep 5
  done
  [ "$ok" -eq 1 ] || die circuits "circuit upload did not finish; re-run to resume"
  fin=$(helper comp-defs status | grep -c ' finalized$')
  [ "$fin" -eq "${#NEW_CIRCUITS[@]}" ] || die circuits "not every new comp def is finalized after upload; re-run to resume"
  state_set circuits done "${NEW_CIRCUITS[*]}"
fi

# ---- 6. private mode ---------------------------------------------------------
echo "== 6. private mode: band rate source and first band =="
if [ "$(state_get private_setup)" = "done" ]; then
  echo "  already done"
else
  require_funds private_setup "band rate source and band update" 100000000
  ok=0
  for i in 1 2; do
    if helper private-setup; then ok=1; break; fi
    echo "  private-setup attempt $i failed, retrying"; sleep 10
  done
  [ "$ok" -eq 1 ] || die private_setup "private-setup failed; see messages above, then re-run"
  state_set private_setup done "rate source band, band observed"
fi

# ---- 7. optional: reclaim position_op --------------------------------------
echo "== 7. reclaim position_op (optional, RECLAIM_POSITION_OP=1) =="
if [ "${RECLAIM_POSITION_OP:-0}" != "1" ]; then
  echo "  skipped (flag off): the current web app still uses position_op"
elif [ "$(state_get reclaim_position_op)" = "done" ]; then
  echo "  already done"
elif ! program_matches_local; then
  echo "  skipped: the upgrade is not verified on chain, position_op may still be needed"
else
  require_funds_fee reclaim_position_op
  before=$(lamports)
  reclaim_all position_op || die reclaim_position_op "position_op comp def could not be closed; see messages above"
  state_set reclaim_position_op done "freed $(sol $(( $(lamports) - before ))) SOL"
fi

# ---- 8. summary -------------------------------------------------------------
echo "== 8. summary =="
echo "program:  $ID"
solana program show "$ID" | grep -E "Data Length|Last Deployed"
echo "steps (artifacts/lend-deploy-state.json):"
node -e '
  const s = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
  for (const [k, v] of Object.entries(s.steps)) console.log("  " + k.padEnd(22) + v.status.padEnd(8) + (v.note || ""));
' "$STATE"
echo "start balance: $(sol "$START_BAL") SOL"
echo "final balance: $(sol "$(lamports)") SOL"
