#!/usr/bin/env bash
# Builds the mainnet artifacts: the size-optimized program with only the
# confidential path, its IDL, and the circuit files to publish as a GitHub
# Release. Builds only; deploys nothing and sends no transaction.
#
# Usage (Linux or WSL2, from anywhere):
#   scripts/build-mainnet.sh              full build, including circuits
#   scripts/build-mainnet.sh --skip-circuits
#                                         reuse build/*.arcis and *.hash
#
# The program embeds each circuit's SHA-256, so publish the files from the same
# run that produced the program. See docs/MAINNET_DEPLOY.md.
set -euo pipefail
cd "$(dirname "$0")/.."

CIRCUITS=(init_pool_state init_position_state shield_credit unshield
  rotate_position_key lend_op disclose_position pool_op position_private
  liquidate_pool pool_band)
FEATURES=offchain-circuits,no-log-ix-name
OUT=artifacts/mainnet
TARGET="${CARGO_TARGET_DIR:-target}"

if [ "${1:-}" != "--skip-circuits" ]; then
  echo "== circuits =="
  arcium build --skip-program
fi
for c in "${CIRCUITS[@]}"; do
  [ -f "build/$c.arcis" ] && [ -f "build/$c.hash" ] ||
    { echo "missing build/$c.arcis or .hash" >&2; exit 1; }
  # build/<name>.hash is a JSON byte array of the file's SHA-256.
  want=$(node -e 'process.stdout.write(Buffer.from(JSON.parse(require("fs").readFileSync(process.argv[1]))).toString("hex"))' "build/$c.hash")
  got=$(sha256sum "build/$c.arcis" | cut -d' ' -f1)
  [ "$want" = "$got" ] || { echo "stale hash for $c; rebuild circuits" >&2; exit 1; }
done

echo "== program (opt-level z, features: $FEATURES) =="
# location-detail=none drops panic file and line strings (needs the unstable
# flag opt-in on the platform-tools compiler).
(
  cd programs/walnut
  CARGO_PROFILE_RELEASE_OPT_LEVEL=z RUSTC_BOOTSTRAP=1 \
    RUSTFLAGS="-Zlocation-detail=none" \
    cargo build-sbf --no-default-features --features "$FEATURES"
)

echo "== IDL =="
mkdir -p "$OUT/circuits"
anchor idl build -p walnut -o "$OUT/walnut.full.json"
node scripts/filter-idl.mjs "$OUT/walnut.full.json" "$OUT/walnut.json"
rm "$OUT/walnut.full.json"

cp "$TARGET/deploy/walnut.so" "$OUT/walnut.so"
for c in "${CIRCUITS[@]}"; do cp "build/$c.arcis" "$OUT/circuits/"; done
(cd "$OUT/circuits" && sha256sum ./*.arcis >SHA256SUMS)

bytes=$(stat -c %s "$OUT/walnut.so")
echo "== done =="
echo "program: $OUT/walnut.so ($bytes bytes)"
solana rent "$bytes" || true
echo "circuits to publish: $OUT/circuits/"
echo "NOTE: $TARGET/deploy/walnut.so is now the mainnet build. Run 'arcium build'"
echo "again before any devnet or localnet work."
