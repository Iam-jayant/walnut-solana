#!/usr/bin/env bash
# Run cargo-audit and pnpm audit, then apply the advisory policy.
#
# Usage: bash scripts/audit.sh
#
# Needs cargo-audit (cargo install cargo-audit --locked), pnpm, and node.
# Reports are written to $AUDIT_OUT_DIR (default target/audit). The pass/fail
# decision is made by scripts/advisories.mjs using
# security/advisory-exceptions.toml; see docs/CI.md#advisory-exceptions.
#
# Exit status: 0 pass, 1 policy failure, 2 missing tools or unreadable reports.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || exit 2

for tool in cargo-audit pnpm node; do
  if ! command -v "$tool" > /dev/null 2>&1; then
    printf 'audit: %s is not installed\n' "$tool" >&2
    exit 2
  fi
done

out="${AUDIT_OUT_DIR:-target/audit}"
mkdir -p "$out" || exit 2

# Both tools exit non-zero when they find advisories. Their exit codes are
# informational; advisories.mjs decides, and it fails on missing or invalid
# JSON, so a crashed audit cannot pass.
printf '== cargo-audit ==\n'
cargo-audit audit --json --file Cargo.lock > "$out/cargo-audit.json"
printf 'cargo-audit exit status %d\n' "$?"

printf '\n== pnpm audit ==\n'
pnpm audit --json > "$out/pnpm-audit.json"
printf 'pnpm audit exit status %d\n\n' "$?"

node scripts/advisories.mjs evaluate --cargo "$out/cargo-audit.json" --npm "$out/pnpm-audit.json"
