# WSL2 development setup

This guide installs the Walnut-Solana toolchain on Windows through WSL2 (Windows Subsystem for Linux) with Ubuntu. It assumes no previous Solana or Rust experience. Linux and macOS users can skip the WSL sections and start at [Install the system packages](#3-install-the-system-packages).

Arcium does not support native Windows. Every Rust, Solana, Anchor, and Arcium command in this repository runs inside the Ubuntu terminal, not in PowerShell or `cmd`.

All tools in this guide are free. Docker Desktop has licence conditions for larger companies; see [Install Docker](#9-install-docker).

## Pinned versions

The file [`toolchain.versions`](../toolchain.versions) at the repository root is the single source of truth. Every other pin must match it, and `scripts/check-toolchain.sh` reads it.

| Tool                                                         | Version | Where else it is pinned                                                  |
| ------------------------------------------------------------ | ------- | ------------------------------------------------------------------------ |
| Arcium CLI, Arx node image, Rust crates, `@arcium-hq/client` | 0.16.0  | `programs/walnut/Cargo.toml`, `encrypted-ixs/Cargo.toml`, `package.json` |
| Anchor CLI, `anchor-lang`, `@anchor-lang/core`               | 1.0.2   | `Anchor.toml [toolchain]`, `programs/walnut/Cargo.toml`, `package.json`  |
| Solana (Agave) CLI                                           | 3.1.10  | `Anchor.toml [toolchain]`                                                |
| Rust                                                         | 1.99.1  | `rust-toolchain.toml`                                                    |
| Node.js                                                      | 26.13.1 | `.nvmrc`, `.node-version`, `package.json` engines                        |
| pnpm                                                         | 9.15.9  | `package.json` `packageManager` and engines                              |

The commands below read versions from `toolchain.versions` where they can, so you do not need to retype them.

## 1. Install WSL2 and Ubuntu

1. Open **PowerShell as Administrator** (right-click the Start button, choose "Terminal (Admin)").
2. Run:

   ```powershell
   wsl --install -d Ubuntu
   ```

3. Reboot when Windows asks.
4. After the reboot, Ubuntu opens and asks for a Linux username and password. This password is the one `sudo` asks for later. It does not need to match your Windows password.
5. Confirm that Ubuntu runs on WSL version 2:

   ```powershell
   wsl -l -v
   ```

   The `VERSION` column for `Ubuntu` must be `2`. If it shows `1`, run `wsl --set-version Ubuntu 2`.

From now on, open Ubuntu from the Start menu or run `wsl -d Ubuntu` in a Windows terminal. Commands in the rest of this guide go into that Ubuntu shell.

## 2. Decide where the repository lives

WSL2 has its own Linux disk. Windows drives appear inside it under `/mnt/c`, `/mnt/d`, and so on.

| Location                       | Example                       | Build speed                                                                                                                                                       | Windows editor access                                            |
| ------------------------------ | ----------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| Linux filesystem (recommended) | `~/code/walnut-solana`        | Fast. Native ext4.                                                                                                                                                | Through `\\wsl$\Ubuntu\home\<you>\code` or VS Code "WSL" remote. |
| Windows drive                  | `/mnt/d/solana/walnut-solana` | Slow. Every file access crosses the 9P bridge between Linux and Windows. Cargo and pnpm builds can take several times longer, and file watchers may miss changes. | Direct.                                                          |

Clone into the Linux filesystem for daily work:

```bash
mkdir -p ~/code && cd ~/code
git clone <repository-url> walnut-solana
cd walnut-solana
```

Working from `/mnt/d/...` is supported, and `check-toolchain.sh` prints a reminder when it detects it. Do not run `pnpm install` from both Windows and WSL against the same `node_modules`; native packages built for one system do not work on the other.

## 3. Install the system packages

These packages provide the C compiler and linker that Rust needs, plus the OpenSSL and udev headers that Solana and Anchor link against. They need `sudo`, which asks for the Ubuntu password from step 1.

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libssl-dev libudev-dev curl git ca-certificates
```

The package list comes from the [Arcium installation guide](https://docs.arcium.com/developers/installation). `curl`, `git`, and `ca-certificates` are added because the installers below need them.

### Optional: keep Windows tools off the Linux PATH

By default WSL appends the Windows `PATH`, so Windows copies of `node`, `pnpm`, or `docker` can shadow or confuse the Linux ones. The Linux tools installed below are placed first on `PATH`, so this is usually fine. If `which node` or `which pnpm` ever prints a `/mnt/c/...` path, stop appending the Windows PATH:

```bash
sudo tee -a /etc/wsl.conf > /dev/null << 'EOF'
[interop]
appendWindowsPath = false
EOF
```

Then run `wsl --shutdown` in PowerShell and reopen Ubuntu. If you use Docker Desktop, its WSL integration still works with this setting.

## 4. Load the pinned versions into your shell

Run this from the repository root. It exports `RUST_VERSION`, `SOLANA_CLI_VERSION`, `ANCHOR_VERSION`, `ARCIUM_VERSION`, `NODE_VERSION`, and `PNPM_VERSION` for the commands that follow:

```bash
set -a; source <(grep -E '^[A-Z_]+=' toolchain.versions); set +a
echo "$RUST_VERSION $SOLANA_CLI_VERSION $ANCHOR_VERSION $ARCIUM_VERSION $NODE_VERSION $PNPM_VERSION"
```

If you open a new terminal before finishing, run it again.

## 5. Install Rust

Rust is installed with [rustup](https://rustup.rs/), the official installer:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
  | sh -s -- -y --profile minimal --default-toolchain "$RUST_VERSION" -c rustfmt -c clippy
. "$HOME/.cargo/env"
rustc --version
```

`rust-toolchain.toml` in the repository also selects this version, so `cargo` uses it inside the repository even if your default changes later.

## 6. Install the Solana CLI

Install the pinned [Agave](https://github.com/anza-xyz/agave) release with the official Anza installer:

```bash
sh -c "$(curl -sSfL "https://release.anza.xyz/v${SOLANA_CLI_VERSION}/install")"
export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"
solana --version
```

Create a local development wallet. The Arcium prerequisites ask for this, and `Anchor.toml` points at `~/.config/solana/id.json`:

```bash
solana-keygen new
```

The keypair file is a plain-text secret. Keep it outside the repository. `.gitignore` excludes `id.json` and `*keypair*.json` in case one is copied in by mistake.

## 7. Install Anchor

Install the Anchor Version Manager (AVM), then the pinned Anchor CLI. This step compiles AVM from source, so step 3 must be complete:

```bash
cargo install --git https://github.com/otter-sec/anchor --tag "v${ANCHOR_VERSION}" avm --locked --force
avm install "$ANCHOR_VERSION"
avm use "$ANCHOR_VERSION"
anchor --version
```

If this fails with `could not exec the linker cc`, the `build-essential` package from step 3 is missing.

Anchor's own documentation suggests the latest Anchor. Use the pinned version instead: `arcium-anchor 0.16.0` requires `anchor-lang` 1.0.2 exactly.

## 8. Install Node.js and pnpm

Install Node with [nvm](https://github.com/nvm-sh/nvm), then install pnpm through Corepack:

```bash
NVM_TAG="$(curl -sSfL https://api.github.com/repos/nvm-sh/nvm/releases/latest | sed -nE 's/.*"tag_name": *"([^"]+)".*/\1/p')"
curl -sSfL "https://raw.githubusercontent.com/nvm-sh/nvm/${NVM_TAG}/install.sh" | bash
. "$HOME/.nvm/nvm.sh"
nvm install "$NODE_VERSION"
nvm alias default "$NODE_VERSION"
npm install -g corepack
corepack enable pnpm
corepack prepare "pnpm@${PNPM_VERSION}" --activate
hash -r
which node pnpm      # both must be under ~/.nvm, not /mnt/c
node --version
pnpm --version
```

Node 25 and newer no longer ship Corepack, so the pinned Node 26 needs the `npm install -g corepack` line. Without it, `corepack` and `pnpm` either fail with `command not found` or fall through to the Windows copies under `/mnt/c/...`, which do not work in Linux (a typical error is `/bin/sh^M: bad interpreter`). See the [Node.js distribution notes](https://github.com/nodejs/node/blob/main/doc/contributing/distribution.md).

The repository's `.nvmrc` lets you run `nvm use` inside the repository to switch to the pinned Node. `package.json` sets `packageManager`, so Corepack runs the pinned pnpm in this repository.

## 9. Install Docker

`arcium test` runs local Arx (MPC) nodes in Docker containers, and `arcup install` pulls the Arx node image. You need a Docker daemon that the Ubuntu shell can reach, plus the Docker Compose plugin. Choose one option.

The Arcium installation guide links the [Docker Engine install page](https://docs.docker.com/engine/install/) and does not name Docker Desktop, so Option B matches the Arcium docs most closely. Option A is easier if Docker Desktop is already on your PC.

### Option A: Docker Desktop with WSL integration

1. Install [Docker Desktop for Windows](https://docs.docker.com/desktop/setup/install/windows-install/) and start it.
2. Open **Settings > Resources > WSL integration**, turn on **Ubuntu**, and click **Apply & restart**.
3. In Ubuntu, run `docker info` and `docker compose version`.

Licence: Docker Desktop is free for personal use, education, non-commercial open-source projects, and companies with fewer than 250 employees and less than US$10 million in annual revenue. Larger companies need a paid subscription. See the [Docker Desktop licence agreement](https://docs.docker.com/subscription-billing/desktop-license/).

### Option B: Docker Engine inside Ubuntu

Docker Engine is open source (Apache 2.0) with no licence conditions. Do not run it alongside Docker Desktop's WSL integration for the same distro.

1. Confirm systemd is running (current Ubuntu WSL images enable it by default):

   ```bash
   ps -p 1 -o comm=   # must print "systemd"
   ```

   If it prints `init`, add `[boot]` and `systemd=true` to `/etc/wsl.conf`, then run `wsl --shutdown` in PowerShell and reopen Ubuntu.

2. Install Docker Engine and the Compose plugin from Docker's apt repository. Follow the current [Ubuntu instructions](https://docs.docker.com/engine/install/ubuntu/); at the time of writing they are:

   ```bash
   sudo install -m 0755 -d /etc/apt/keyrings
   sudo curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
   sudo chmod a+r /etc/apt/keyrings/docker.asc
   echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu $(. /etc/os-release && echo "${UBUNTU_CODENAME:-$VERSION_CODENAME}") stable" \
     | sudo tee /etc/apt/sources.list.d/docker.list > /dev/null
   sudo apt-get update
   sudo apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
   ```

3. Let your user run Docker without `sudo`, then open a new Ubuntu terminal:

   ```bash
   sudo usermod -aG docker "$USER"
   ```

4. Check: `docker run --rm hello-world`.

The `docker` group grants root-equivalent access to the machine. That is normal for a personal development box but not for shared servers.

## 10. Install Arcium

Install `arcup`, the Arcium version manager, then the pinned Arcium release. Docker from step 9 must be running because `arcup install` pulls the `arcium/arx-node` image.

```bash
mkdir -p ~/.cargo/bin
curl -sSfL "https://bin.arcium.com/download/arcup_x86_64_linux_${ARCIUM_VERSION}" -o ~/.cargo/bin/arcup
chmod +x ~/.cargo/bin/arcup
arcup install "$ARCIUM_VERSION"
arcium --version
arcup version
```

This is the manual install from the [Arcium installation guide](https://docs.arcium.com/developers/installation#manual-installation). The guide's one-line quick installer installs the latest release, which may differ from the pin. Use the commands above.

## 11. Set up PATH

The installers usually add themselves to `~/.bashrc`. Check that it contains these lines and add any that are missing:

```bash
. "$HOME/.cargo/env"                                                  # Rust, arcup, arcium
export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"  # Solana CLI
export PATH="$HOME/.avm/bin:$PATH"                                    # Anchor (avm)
export NVM_DIR="$HOME/.nvm"; [ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh"  # Node, pnpm
```

Open a new terminal (or run `source ~/.bashrc`) and confirm each tool resolves to a Linux path, not `/mnt/c/...`:

```bash
which rustc cargo solana anchor arcium node pnpm docker
```

## 12. Install dependencies and verify

From the repository root:

```bash
pnpm install --frozen-lockfile
bash scripts/check-toolchain.sh
```

`check-toolchain.sh` checks every pin in the repository against `toolchain.versions` and every installed tool against the same file. It lists all mismatches and missing tools, not only the first, and exits with status 1 if any check fails. It ends with `All N checks passed.` when the setup is complete.

Use `bash scripts/check-toolchain.sh --files-only` to check only the repository files, for example on a machine without the tools.

Then build:

```bash
arcium build
```

`arcium test` additionally needs a higher open-file limit on Linux. If it fails with "too many open files", run `ulimit -n 1048576` (or `sudo prlimit --pid $$ --nofile=1048576:1048576`) in the same shell first.

## Compatibility matrix

Verified on 2026-11-15.

| Pin                                                                                                                                                                                                                                                                                                            | Source                                                                                                                                              |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| Arcium 0.16.0 is the latest coordinated release (2026-10-23).                                                                                                                                                                                                                                                  | [Arcium release notes](https://docs.arcium.com/developers/release-notes)                                                                            |
| CLI, crates, TypeScript packages, and Arx image must share one release.                                                                                                                                                                                                                                        | [Arcium versioning](https://docs.arcium.com/developers/versioning), [arcup](https://docs.arcium.com/developers/installation/arcup)                  |
| Solana CLI 3.1.10 and Anchor 1.0.2 are the listed prerequisites.                                                                                                                                                                                                                                               | [Arcium installation](https://docs.arcium.com/developers/installation)                                                                              |
| Node 26.13.1 is the version on the reference WSL2 machine. Arcium publishes no Node requirement; its `setup-arcium` CI action defaults to 24.10.0 and asks projects to pass their own pin, so CI must set `node-version: 26.13.1`.                                                                             | [Arcium continuous integration](https://docs.arcium.com/developers/continuous-integration)                                                          |
| `arcium-anchor 0.16.0` depends on `anchor-lang =1.0.2`, so Anchor cannot move without Arcium.                                                                                                                                                                                                                  | crates.io dependency metadata, enforced by `Cargo.lock`                                                                                             |
| Rust 1.99.1 is the latest stable patch at the Arcium 0.16.0 release date (released 2026-10-15). Arcium does not publish a Rust pin; Anchor 0.32+ needs Rust 1.89 or newer for IDL builds.                                                                                                                      | [Rust release notes](https://doc.rust-lang.org/releases.html), [Anchor 0.32.0 notes](https://www.anchor-lang.com/docs/updates/release-notes/0-32-0) |
| pnpm 9.15.9 is the workspace package manager. Arcium's docs use Yarn; pnpm is a project choice, set in `Anchor.toml` `package_manager`.                                                                                                                                                                        | Project decision (task 1.1)                                                                                                                         |
| `@anchor-lang/core`, `@anchor-lang/borsh`, `@anchor-lang/errors` are forced to 1.0.2 through `pnpm.overrides`. `@arcium-hq/client@0.16.0` declares `@anchor-lang/core: ^1.0.2`, which otherwise resolves to the npm `latest` tag (1.2.2). 1.0.2 is the npm release that matches the `anchor-lang` 1.0.2 crate. | `package.json`, `pnpm-lock.yaml`                                                                                                                    |

On-chain SBF builds use the platform tools bundled with the Solana CLI, not the host Rust toolchain.

## Changing a pinned version

1. Read the Arcium [release notes](https://docs.arcium.com/developers/release-notes) from the current version to the target version.
2. Edit `toolchain.versions`.
3. Update the files listed in the [Pinned versions](#pinned-versions) table, then run `cargo update` and `pnpm install` to refresh the lockfiles.
4. Install the new tools (`rustup toolchain install`, the Anza installer, `avm install`, `arcup install`, `nvm install`, `corepack prepare`).
5. Run `bash scripts/check-toolchain.sh`, `arcium build`, and `arcium test`.

## Troubleshooting

| Symptom                                                        | Fix                                                                                                                                     |
| -------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| `could not exec the linker cc`                                 | Install `build-essential` (step 3).                                                                                                     |
| `command not found` for a tool you installed                   | Check PATH (step 11) and open a new terminal.                                                                                           |
| `which pnpm` or `which node` prints `/mnt/c/...`               | The Linux tool is missing or not first on PATH. Repeat step 8, or disable `appendWindowsPath`.                                          |
| `corepack: /bin/sh^M: bad interpreter`                         | That is the Windows Corepack. Node 25+ has no bundled Corepack; run `npm install -g corepack` inside WSL (step 8).                      |
| `The command 'docker' could not be found in this WSL 2 distro` | Turn on WSL integration for Ubuntu in Docker Desktop, or install Docker Engine (step 9).                                                |
| `docker: permission denied ... docker.sock`                    | Run `sudo usermod -aG docker "$USER"` and open a new terminal.                                                                          |
| Builds are very slow                                           | Move the repository from `/mnt/...` to the Linux filesystem (step 2).                                                                   |
| `check-toolchain.sh` shows `$'\r': command not found`          | The script was checked out with Windows line endings. `.gitattributes` forces LF; run `git add --renormalize .` or re-clone inside WSL. |
