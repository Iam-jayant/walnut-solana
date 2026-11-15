# Continuous integration

CI runs in GitHub Actions on `ubuntu-24.04` runners. The workflow is [`.github/workflows/ci.yml`](../.github/workflows/ci.yml). It runs on pushes to `main`, on pull requests, daily at 05:17 UTC, and on manual dispatch.

All toolchain versions come from [`toolchain.versions`](../toolchain.versions). Each job reads them with `scripts/ci/read-versions.sh`, which writes them to the step outputs and the job environment. To change a version, follow [Changing a pinned version](WSL2_SETUP.md#changing-a-pinned-version); the workflow needs no edit. The only versions in the workflow file are the third-party actions (pinned by commit SHA) and the CI-only scanners `cargo-audit` and `gitleaks`.

## Jobs

| Job           | What it runs                                                                                                                                                                                                                                                  | Runs on schedule |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------- |
| `js`          | `check-toolchain.sh --files-only`, advisory-exception validation, Prettier, ESLint, workspace type-check, unit tests, TypeScript property tests, fixture-import check                                                                                         | yes              |
| `rust-arcium` | rustfmt, `check-toolchain.sh` (installed tools), `arcium build` (Anchor program and Arcis circuits), clippy with `-D warnings`, Rust unit tests, Rust property tests, type-check of `tests/`, transaction-size tests, `arcium test` on a local Arcium cluster | no               |
| `audit`       | `cargo audit` and `pnpm audit`, evaluated against the [advisory exceptions](#advisory-exceptions)                                                                                                                                                             | yes              |
| `secrets`     | gitleaks over the full git history                                                                                                                                                                                                                            | yes              |

The jobs run in parallel. `rust-arcium` runs the build before clippy and the Rust tests because the program's Arcium callback types are generated from the compiled circuits in `build/`.

### Toolchain setup in `rust-arcium`

1. `rustup` installs `RUST_VERSION` (selected by `rust-toolchain.toml`) and `stable`. The Anchor IDL build invokes `cargo +stable`.
2. [`arcium-hq/setup-arcium`](https://github.com/arcium-hq/setup-arcium), the action recommended by the [Arcium CI guide](https://docs.arcium.com/developers/continuous-integration), installs Node, the Solana CLI, Anchor, `arcup`, Arcium, and the Arx node image, and creates `~/.config/solana/id.json`. Its inputs come from `toolchain.versions`.
3. pnpm is installed with `npm install -g pnpm@$PNPM_VERSION`. Node 25 and newer do not bundle Corepack.
4. `scripts/check-toolchain.sh` verifies every installed tool against `toolchain.versions` before the build.

`arcium test` runs in the same shell as `sudo prlimit --nofile=1048576:1048576`, as the Arcium CI guide requires.

### Caches

| Cache                 | Paths                                                       | Key                              |
| --------------------- | ----------------------------------------------------------- | -------------------------------- |
| `Swatinem/rust-cache` | `~/.cargo/registry`, `~/.cargo/git`, `target/`              | rustc version, `Cargo.lock`, job |
| setup-arcium (Solana) | `~/.cache/solana` (platform-tools), `~/.local/share/solana` | Solana CLI version               |
| setup-arcium (Anchor) | `~/.cargo/bin/anchor`                                       | Anchor version                   |
| pnpm store            | `pnpm store path`                                           | pnpm version, `pnpm-lock.yaml`   |

The Solana cache is saved at the end of the first job that misses it, so it includes the platform-tools that `cargo build-sbf` downloads during `arcium build`. The Arx node Docker image is pulled on every `rust-arcium` run.

## Running the checks locally

Run these in WSL2 or Linux from the repository root after following [WSL2_SETUP.md](WSL2_SETUP.md).

| CI step                | Command                                                        |
| ---------------------- | -------------------------------------------------------------- |
| Pin check              | `bash scripts/check-toolchain.sh`                              |
| Prettier               | `pnpm format:check` (fix with `pnpm format`)                   |
| ESLint                 | `pnpm lint`                                                    |
| TypeScript             | `pnpm typecheck`, and after a build `pnpm typecheck:tests`     |
| rustfmt                | `pnpm format:rust:check`                                       |
| Build                  | `arcium build`                                                 |
| Clippy                 | `pnpm lint:rust` (after `arcium build`)                        |
| Unit tests             | `pnpm test:unit`, `pnpm test:rust`                             |
| Property tests         | `pnpm test:property`, `pnpm test:rust:property`                |
| Transaction-size tests | `pnpm test:tx-size`                                            |
| Local Arcium tests     | `ulimit -n 1048576 && arcium test`                             |
| Dependency audit       | `pnpm audit:deps` (needs `cargo install cargo-audit --locked`) |
| Fixture imports        | `pnpm check:fixtures`                                          |
| Exceptions only        | `pnpm check:advisories`                                        |
| Secret scan            | `gitleaks git --redact .`                                      |

## Test suites

Test suites are found by naming convention, so adding a test needs no workflow change.

| Suite                     | Convention                                                                                      | Root command              |
| ------------------------- | ----------------------------------------------------------------------------------------------- | ------------------------- |
| Script unit tests         | `scripts/tests/*.test.mjs` (Node test runner)                                                   | `pnpm test:scripts`       |
| TypeScript unit tests     | a `test` script in a workspace package                                                          | `pnpm test:unit`          |
| TypeScript property tests | a `test:property` script in a workspace package                                                 | `pnpm test:property`      |
| Transaction-size tests    | a `test:tx-size` script in a workspace package                                                  | `pnpm test:tx-size`       |
| Rust unit tests           | any `#[test]` whose name does not start with `prop_`                                            | `pnpm test:rust`          |
| Rust property tests       | `#[test]` functions named `prop_*` (for example with `proptest`); CI sets `PROPTEST_CASES=1024` | `pnpm test:rust:property` |
| Local Arcium tests        | `tests/**/*.ts`, run by `arcium test` through `Anchor.toml [scripts]`                           | `arcium test`             |

`scripts/run-suite.mjs` runs a package-level suite in every workspace package that defines it. When no package defines it yet, it prints `0 suites run` and exits 0. The `test:property` and `test:tx-size` suites have no tests yet: the first property tests arrive with the reference model and the first transaction-size tests with Task 2.5. The size limit (1,232 bytes, target 1,150) is enforced by those tests, not by this workflow.

## Advisory exceptions

High and critical advisories fail CI unless a committed exception covers them (Requirement 1.5).

- `cargo audit`: every RustSec vulnerability blocks. RustSec entries often have no CVSS score, so all are treated as high. Warnings (unmaintained, unsound, yanked) are printed but do not block.
- `pnpm audit`: `high` and `critical` advisories block. `moderate`, `low`, and `info` are printed but do not block. An unknown severity blocks.

Exceptions live in [`security/advisory-exceptions.toml`](../security/advisory-exceptions.toml). Each entry needs:

| Field       | Rule                                                                                                                |
| ----------- | ------------------------------------------------------------------------------------------------------------------- |
| `id`        | `RUSTSEC-…`, `GHSA-…`, or `CVE-…`; matches the advisory ID or any alias                                             |
| `ecosystem` | `cargo` or `npm`                                                                                                    |
| `package`   | must match the advisory's package                                                                                   |
| `reason`    | at least 40 characters, no placeholders; why it is not exploitable here or cannot be fixed yet, and what removes it |
| `owner`     | GitHub handle accountable for the exception                                                                         |
| `added`     | `YYYY-MM-DD`, not in the future                                                                                     |
| `expires`   | `YYYY-MM-DD`, at most 90 days after `added`; valid through that day (UTC)                                           |

`scripts/advisories.mjs` enforces these rules. The `js` job validates the file on every run, and the daily schedule makes an expired exception fail CI even without a code change. An exception that no longer matches any advisory produces a warning so it can be removed. When an advisory blocks, the `audit` job log prints a pre-filled exception template.

To renew an exception, re-review the advisory, then move both `added` and `expires`.

## Fixture-import check

`scripts/check-fixture-imports.mjs` enforces that production code contains no fixture or mock imports (Requirement 1.8). It scans `programs/`, `encrypted-ixs/`, `packages/`, `keeper/`, and `web/`, skipping `tests/`, `test/`, `__tests__/`, `*.test.*`, `*.spec.*`, `tests.rs`, and Rust items gated by `#[cfg(test)]`. It fails on:

- imports of modules named `fixture(s)`, `mock(s)`, `__mocks__`, `fake(s)`, `stub(s)`, `test-utils`, `test-helpers`, or `testing`, or files such as `*.fixture.ts` and `*.mock.ts`;
- relative imports into the repository `tests/` directory;
- test-only packages (`sinon`, `nock`, `msw`, `vitest`, `mocha`, `chai`, `fast-check`, and others) and test-only crates (`proptest`, `mockall`, `solana_program_test`, `litesvm`, and others);
- Rust `include_bytes!`, `include_str!`, and `#[path]` pointing at fixture or test files.

## Secret scanning

The `secrets` job downloads the pinned gitleaks release, verifies its SHA-256, and scans every commit with the default rules. gitleaks is MIT-licensed and needs no licence key when run as a binary.
