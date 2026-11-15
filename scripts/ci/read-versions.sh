#!/usr/bin/env bash
# Print the pinned versions from toolchain.versions as KEY=VALUE lines.
#
# Usage: bash scripts/ci/read-versions.sh
#
# CI appends the output to $GITHUB_OUTPUT and $GITHUB_ENV so every job reads
# versions from the single source of truth. Fails if a required key is
# missing or a value contains characters outside [0-9A-Za-z.+-].

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FILE="$ROOT/toolchain.versions"
REQUIRED=(ARCIUM_VERSION ANCHOR_VERSION SOLANA_CLI_VERSION RUST_VERSION NODE_VERSION PNPM_VERSION)

if [[ ! -f "$FILE" ]]; then
  printf 'read-versions: %s not found\n' "$FILE" >&2
  exit 1
fi

declare -A seen=()
status=0
while IFS= read -r line; do
  line="${line%$'\r'}"
  [[ -z "$line" || "$line" == \#* ]] && continue
  if [[ ! "$line" =~ ^([A-Z_]+)=([0-9A-Za-z.+-]+)$ ]]; then
    printf 'read-versions: malformed line: %s\n' "$line" >&2
    status=1
    continue
  fi
  seen["${BASH_REMATCH[1]}"]="${BASH_REMATCH[2]}"
done < "$FILE"

for key in "${REQUIRED[@]}"; do
  if [[ -z "${seen[$key]:-}" ]]; then
    printf 'read-versions: %s is not set in toolchain.versions\n' "$key" >&2
    status=1
  fi
done
[[ $status -eq 0 ]] || exit "$status"

for key in "${REQUIRED[@]}"; do
  printf '%s=%s\n' "$key" "${seen[$key]}"
done
