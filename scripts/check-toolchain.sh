#!/usr/bin/env bash
# Verify that the repository pins and the installed tools match toolchain.versions.
#
# Usage:
#   bash scripts/check-toolchain.sh               # check pinned files and installed tools
#   bash scripts/check-toolchain.sh --files-only  # check pinned files only
#
# Runs in Linux or WSL2. Reports every mismatch (it does not stop at the first)
# and exits 1 if any check fails, 0 if all pass, 2 on usage errors.

set -u -o pipefail

usage() { sed -n '2,9p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; }

files_only=0
for arg in "$@"; do
  case "$arg" in
    --files-only) files_only=1 ;;
    -h | --help) usage; exit 0 ;;
    *) printf 'unknown argument: %s\n' "$arg" >&2; usage >&2; exit 2 ;;
  esac
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || exit 2

# Keep the check free of side effects and network noise.
export ARCIUM_NO_UPDATE_CHECK=1
export RUSTUP_AUTO_INSTALL=0
export COREPACK_ENABLE_DOWNLOAD_PROMPT=0

mismatches=()
passes=0

pass() { passes=$((passes + 1)); printf '  ok    %s\n' "$1"; }
fail() { mismatches+=("$1"); printf '  FAIL  %s\n' "$1"; }
section() { printf '\n== %s ==\n' "$1"; }

# expect LABEL EXPECTED ACTUAL
expect() {
  if [[ "$3" == "$2" ]]; then
    pass "$1 = $2"
  else
    fail "$1: expected $2, found ${3:-<none>}"
  fi
}

# First X.Y.Z in stdin.
semver() { grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -n 1; }

# toml_raw FILE SECTION KEY -> raw value text after "=", comments stripped.
toml_raw() {
  awk -v sec="[$2]" -v key="$3" '
    /^[[:space:]]*\[/ { h = $0; gsub(/[[:space:]]/, "", h); insec = (h == sec); next }
    insec {
      line = $0
      sub(/[[:space:]]#.*$/, "", line)
      if (match(line, "^[[:space:]]*\"?" key "\"?[[:space:]]*=[[:space:]]*")) {
        v = substr(line, RLENGTH + 1)
        sub(/[[:space:]]+$/, "", v)
        print v
        exit
      }
    }' "$1"
}

# toml_get FILE SECTION KEY -> unquoted string value.
toml_get() {
  local v
  v="$(toml_raw "$@")"
  v="${v#\"}"
  printf '%s' "${v%\"}"
}

# cargo_dep FILE CRATE -> version requirement from [dependencies].
cargo_dep() {
  local raw
  raw="$(toml_raw "$1" dependencies "$2")"
  if [[ "$raw" =~ ^\"([^\"]*)\" ]]; then
    printf '%s' "${BASH_REMATCH[1]}"
  elif [[ "$raw" =~ version[[:space:]]*=[[:space:]]*\"([^\"]*)\" ]]; then
    printf '%s' "${BASH_REMATCH[1]}"
  fi
}

# ---------------------------------------------------------------------------
section "Pinned versions (toolchain.versions)"

VERSIONS_FILE="toolchain.versions"
declare -A want=()
keys=(ARCIUM_VERSION ANCHOR_VERSION SOLANA_CLI_VERSION RUST_VERSION NODE_VERSION PNPM_VERSION)
if [[ -f "$VERSIONS_FILE" ]]; then
  while IFS='=' read -r k v; do
    want["$k"]="$v"
  done < <(sed -nE 's/^([A-Z_]+)=([^[:space:]#]+).*$/\1=\2/p' "$VERSIONS_FILE")
else
  fail "$VERSIONS_FILE: file not found"
fi
for k in "${keys[@]}"; do
  if [[ -n "${want[$k]:-}" ]]; then
    pass "$k = ${want[$k]}"
  else
    fail "$VERSIONS_FILE: $k is not set"
    want["$k"]="<unset>"
  fi
done

ARCIUM="${want[ARCIUM_VERSION]}"
ANCHOR="${want[ANCHOR_VERSION]}"
SOLANA="${want[SOLANA_CLI_VERSION]}"
RUST="${want[RUST_VERSION]}"
NODE="${want[NODE_VERSION]}"
PNPM="${want[PNPM_VERSION]}"

# ---------------------------------------------------------------------------
section "Repository pins"

# check_file_value LABEL FILE EXPECTED VALUE
check_file_value() {
  if [[ ! -f "$2" ]]; then
    fail "$1: $2 not found"
  else
    expect "$1" "$3" "$4"
  fi
}

check_file_value "rust-toolchain.toml channel" rust-toolchain.toml "$RUST" \
  "$( [[ -f rust-toolchain.toml ]] && toml_get rust-toolchain.toml toolchain channel )"
check_file_value "Anchor.toml [toolchain] anchor_version" Anchor.toml "$ANCHOR" \
  "$( [[ -f Anchor.toml ]] && toml_get Anchor.toml toolchain anchor_version )"
check_file_value "Anchor.toml [toolchain] solana_version" Anchor.toml "$SOLANA" \
  "$( [[ -f Anchor.toml ]] && toml_get Anchor.toml toolchain solana_version )"

for f in .nvmrc .node-version; do
  check_file_value "$f" "$f" "$NODE" "$( [[ -f "$f" ]] && tr -d '[:space:]' < "$f" | sed 's/^v//' )"
done

# Cargo manifests: every Anchor and Arcium crate must use an exact "=" pin.
expected_crate_req() {
  case "$1" in
    anchor-lang | anchor-spl) printf '=%s' "$ANCHOR" ;;
    arcium-client | arcium-macros | arcium-anchor | arcis) printf '=%s' "$ARCIUM" ;;
  esac
}
pinned_crates=(anchor-lang anchor-spl arcium-client arcium-macros arcium-anchor arcis)
manifests=()
for m in programs/*/Cargo.toml encrypted-ixs/Cargo.toml; do
  [[ -f "$m" ]] && manifests+=("$m")
done
if [[ ${#manifests[@]} -eq 0 ]]; then
  fail "Cargo manifests: none found under programs/*/ or encrypted-ixs/"
fi
for m in "${manifests[@]}"; do
  for c in "${pinned_crates[@]}"; do
    req="$(cargo_dep "$m" "$c")"
    [[ -z "$req" ]] && continue
    expect "$m $c" "$(expected_crate_req "$c")" "$req"
  done
done
# Crates each manifest must declare.
require_crate() {
  if [[ -f "$1" && -z "$(cargo_dep "$1" "$2")" ]]; then
    fail "$1: missing dependency $2 (expected $(expected_crate_req "$2"))"
  fi
}
for c in anchor-lang arcium-client arcium-macros arcium-anchor; do
  require_crate programs/walnut/Cargo.toml "$c"
done
require_crate encrypted-ixs/Cargo.toml arcis

# Cargo.lock: resolved versions.
if [[ -f Cargo.lock ]]; then
  for c in "${pinned_crates[@]}"; do
    resolved="$(awk -v c="$c" '
      /^\[\[package\]\]/ { n = "" }
      /^name = / { n = $3; gsub(/"/, "", n) }
      /^version = / { v = $3; gsub(/"/, "", v); if (n == c) print v }' Cargo.lock | sort -u | paste -sd ',' -)"
    [[ -z "$resolved" ]] && continue
    want_req="$(expected_crate_req "$c")"
    expect "Cargo.lock $c" "${want_req#=}" "$resolved"
  done
else
  printf '  info  Cargo.lock not generated yet; resolved crate versions not checked\n'
fi

# package.json files: packageManager, engines, overrides, and every
# @arcium-hq/* and @anchor-lang/* dependency must be an exact version.
if command -v node > /dev/null 2>&1; then
  pkg_files=(package.json)
  for p in packages/*/package.json keeper/package.json web/package.json; do
    [[ -f "$p" ]] && pkg_files+=("$p")
  done
  while IFS=$'\t' read -r label expected actual; do
    [[ -z "$label" ]] && continue
    expect "$label" "$expected" "$actual"
  done < <(
    WANT_ARCIUM="$ARCIUM" WANT_ANCHOR="$ANCHOR" WANT_NODE="$NODE" WANT_PNPM="$PNPM" \
      node - "${pkg_files[@]}" << 'NODE_EOF'
const fs = require("fs");
const env = process.env;
const out = [];
const row = (label, expected, actual) =>
  out.push([label, expected, actual === undefined ? "" : String(actual)].join("\t"));
const wantFor = (name) =>
  name.startsWith("@arcium-hq/") ? env.WANT_ARCIUM
  : name.startsWith("@anchor-lang/") ? env.WANT_ANCHOR
  : undefined;
for (const file of process.argv.slice(2)) {
  let pkg;
  try {
    pkg = JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (e) {
    row(`${file}: unreadable JSON`, "valid JSON", e.message.replace(/\s+/g, " "));
    continue;
  }
  if (file === "package.json") {
    row("package.json packageManager", `pnpm@${env.WANT_PNPM}`, pkg.packageManager);
    row("package.json engines.node", env.WANT_NODE, pkg.engines && pkg.engines.node);
    row("package.json engines.pnpm", env.WANT_PNPM, pkg.engines && pkg.engines.pnpm);
    row("package.json dependencies @arcium-hq/client", env.WANT_ARCIUM,
      pkg.dependencies && pkg.dependencies["@arcium-hq/client"]);
    row("package.json dependencies @anchor-lang/core", env.WANT_ANCHOR,
      pkg.dependencies && pkg.dependencies["@anchor-lang/core"]);
    const ov = (pkg.pnpm && pkg.pnpm.overrides) || {};
    for (const name of ["@anchor-lang/core", "@anchor-lang/borsh", "@anchor-lang/errors", "@arcium-hq/client"]) {
      row(`package.json pnpm.overrides ${name}`, wantFor(name), ov[name]);
    }
  }
  for (const section of ["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"]) {
    for (const [name, spec] of Object.entries(pkg[section] || {})) {
      const want = wantFor(name);
      if (want === undefined) continue;
      if (file === "package.json" && section === "dependencies" &&
          (name === "@arcium-hq/client" || name === "@anchor-lang/core")) continue;
      row(`${file} ${section} ${name}`, want, spec);
    }
  }
}
process.stdout.write(out.join("\n") + "\n");
NODE_EOF
  )
else
  fail "package.json checks: node is not installed, cannot parse package.json files"
fi

# pnpm-lock.yaml: every resolved @arcium-hq/* and @anchor-lang/* package.
if [[ -f pnpm-lock.yaml ]]; then
  lock_entries="$(grep -oE "^  '?@(anchor-lang|arcium-hq)/[a-z0-9-]+@[0-9][^(':]*" pnpm-lock.yaml \
    | sed -E "s/^  '?//" | sort -u)"
  if [[ -z "$lock_entries" ]]; then
    fail "pnpm-lock.yaml: no @arcium-hq/* or @anchor-lang/* packages resolved"
  fi
  while IFS= read -r entry; do
    [[ -z "$entry" ]] && continue
    name="${entry%@*}"
    version="${entry##*@}"
    case "$name" in
      @arcium-hq/*) expect "pnpm-lock.yaml $name" "$ARCIUM" "$version" ;;
      @anchor-lang/*) expect "pnpm-lock.yaml $name" "$ANCHOR" "$version" ;;
    esac
  done <<< "$lock_entries"
else
  fail "pnpm-lock.yaml: not found (run pnpm install)"
fi

# ---------------------------------------------------------------------------
if [[ $files_only -eq 0 ]]; then
  section "Host"

  os="$(uname -s 2> /dev/null || echo unknown)"
  if [[ "$os" == "Linux" ]]; then
    if grep -qi microsoft /proc/version 2> /dev/null; then
      pass "host OS = Linux (WSL2)"
      case "$ROOT" in
        /mnt/*) printf '  info  repository is on a Windows drive (%s); clone into the Linux filesystem (~/) for much faster builds\n' "$ROOT" ;;
      esac
    else
      pass "host OS = Linux"
    fi
  else
    fail "host OS: expected Linux or WSL2, found $os"
  fi

  section "Installed tools"

  # check_tool LABEL EXPECTED COMMAND...
  check_tool() {
    local label="$1" expected="$2"
    shift 2
    local path
    if ! path="$(command -v "$1" 2> /dev/null)"; then
      fail "$label: not installed (expected $expected)"
      return
    fi
    if [[ "$path" == /mnt/* ]]; then
      fail "$label: resolves to a Windows binary ($path); install the Linux version (expected $expected)"
      return
    fi
    local out
    out="$("$@" 2>&1)"
    local status=$?
    if [[ $status -ne 0 ]]; then
      fail "$label: '$*' failed (expected $expected): $(printf '%s' "$out" | head -n 1)"
      return
    fi
    expect "$label" "$expected" "$(printf '%s' "$out" | semver)"
  }

  # System packages (apt): C toolchain and headers needed by cargo, avm, Solana, and Anchor.
  for bin in cc make pkg-config; do
    if command -v "$bin" > /dev/null 2>&1; then
      pass "$bin installed"
    else
      fail "$bin: not installed (sudo apt-get install -y build-essential pkg-config)"
    fi
  done
  if command -v pkg-config > /dev/null 2>&1; then
    for lib in openssl libudev; do
      if pkg-config --exists "$lib"; then
        pass "$lib development headers installed"
      else
        fail "$lib: development headers missing (sudo apt-get install -y libssl-dev libudev-dev)"
      fi
    done
  fi

  check_tool "rustc" "$RUST" rustc --version
  check_tool "cargo" "$RUST" cargo --version
  if command -v cargo > /dev/null 2>&1; then
    if cargo fmt --version > /dev/null 2>&1; then pass "rustfmt component installed"; else fail "rustfmt: component missing for $RUST (rustup component add rustfmt)"; fi
    if cargo clippy --version > /dev/null 2>&1; then pass "clippy component installed"; else fail "clippy: component missing for $RUST (rustup component add clippy)"; fi
  fi
  check_tool "solana (Agave CLI)" "$SOLANA" solana --version
  check_tool "cargo-build-sbf" "$SOLANA" cargo-build-sbf --version
  check_tool "anchor" "$ANCHOR" anchor --version
  check_tool "arcium" "$ARCIUM" arcium --version
  if command -v arcup > /dev/null 2>&1; then
    arcup_out="$(arcup version 2>&1)"
    if grep -q "$ARCIUM" <<< "$arcup_out"; then
      pass "arcup active release includes $ARCIUM"
    else
      fail "arcup: active release does not include $ARCIUM: $(head -n 1 <<< "$arcup_out")"
    fi
  else
    fail "arcup: not installed (expected Arcium release $ARCIUM)"
  fi
  check_tool "node" "$NODE" node --version
  check_tool "pnpm" "$PNPM" pnpm --version
  # Node 25+ no longer bundles Corepack, so pnpm is often missing or falls
  # through to the Windows copy on WSL2.
  pnpm_path="$(command -v pnpm 2> /dev/null || true)"
  if [[ -z "$pnpm_path" || "$pnpm_path" == /mnt/* ]]; then
    printf '  info  install Linux pnpm: npm install -g corepack && corepack enable pnpm && corepack prepare pnpm@%s --activate\n' "$PNPM"
  fi

  # Docker is required by `arcium test` (local Arx nodes).
  docker_path="$(command -v docker 2> /dev/null || true)"
  if [[ -z "$docker_path" ]]; then
    fail "docker: not installed (required by arcium test)"
  elif [[ "$docker_path" == /mnt/* ]]; then
    fail "docker: resolves to the Windows CLI ($docker_path); enable Docker Desktop WSL integration for this distro or install Docker Engine"
  elif ! docker info > /dev/null 2>&1; then
    fail "docker: daemon not reachable (start Docker Engine or enable Docker Desktop WSL integration)"
  else
    pass "docker daemon reachable"
    if docker compose version > /dev/null 2>&1; then
      pass "docker compose plugin installed"
    else
      fail "docker compose: plugin not installed"
    fi
    if docker image inspect "arcium/arx-node:v$ARCIUM" > /dev/null 2>&1; then
      pass "Arx node image arcium/arx-node:v$ARCIUM present"
    else
      fail "Arx node image: arcium/arx-node:v$ARCIUM not found (run arcup install $ARCIUM)"
    fi
  fi
fi

# ---------------------------------------------------------------------------
section "Summary"
if [[ ${#mismatches[@]} -eq 0 ]]; then
  printf 'All %d checks passed.\n' "$passes"
  exit 0
fi
printf '%d check(s) failed, %d passed:\n' "${#mismatches[@]}" "$passes"
for m in "${mismatches[@]}"; do
  printf '  - %s\n' "$m"
done
printf 'See docs/WSL2_SETUP.md to install the pinned toolchain.\n'
exit 1
