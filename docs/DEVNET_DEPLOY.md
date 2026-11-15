# Devnet deploy and benchmark runbook (Task 2.6)

This runbook deploys the Walnut program to the shared **Arcium devnet cluster
(offset 456)**, initializes its MXE and computation definitions, and runs the
existing Task 2.6 benchmark harness (`packages/sdk/bench/`) against devnet to
produce the p50/p95 latency, fee, compute-unit, failure-rate, and
operations-per-minute numbers ADR-0003 (Task 2.7) records.

All Arcium / Solana / Anchor commands run in **Linux or WSL2**. Everything here
uses only free, open-source tooling and the free public devnet RPC. No paid RPC
or hosting is required (an optional faster RPC override is noted where the large
program deploy is likely to need one).

> The cluster offset for devnet is **456** (mainnet is 2026). Every cluster PDA
> the client/tests/bench derive — `getClusterAccAddress(456)`,
> `getComputationAccAddress(456, …)`, `getMempoolAccAddress(456)`,
> `getExecutingPoolAccAddress(456)` — uses this offset. `getMXEAccAddress` and
> `getCompDefAccAddress` use the **program id**, not the offset.

---

## 0. Prerequisites

- WSL2 / Linux with the pinned toolchain (`pnpm check:toolchain` passes).
- Arcium CLI 0.16.0, Solana CLI, Anchor per `toolchain.versions`.
- Dependencies installed: `pnpm install`.

---

## 1. Point the Solana CLI at devnet

```bash
solana config set --url https://api.devnet.solana.com
solana config get          # confirm RPC URL = devnet and keypair path
```

The default CLI keypair is `~/.config/solana/id.json`. Create one if needed:

```bash
solana-keygen new -o ~/.config/solana/id.json      # back this up; it is gitignored
```

This same keypair is the **program upgrade authority** and the benchmark payer
unless you override `--keypair-path` / `ANCHOR_WALLET`.

---

## 2. Fund the deploy/payer wallet

Devnet SOL is free but the faucet is **rate-limited** (small amounts per
request, throttled per IP/time window). A program deploy plus a benchmark run
needs several SOL, so expect to airdrop multiple times or use a web faucet.

```bash
PUBKEY="$(solana-keygen pubkey ~/.config/solana/id.json)"
solana airdrop 2 "$PUBKEY" -u devnet     # repeat as needed; 2 SOL is a typical per-request cap
solana balance "$PUBKEY" -u devnet
```

If the CLI airdrop is throttled, use a public devnet faucet (e.g. the official
Solana faucet) for the same pubkey. See the cost estimate in section 8 for how
much to fund.

---

## 3. Keep the program id stable (program keypair + `declare_id!`)

The deployed program id **must stay stable** and must match
`declare_id!("5Eunnix…jTcjea")` in `programs/walnut/src/lib.rs` (and
`[programs.localnet]` in `Anchor.toml`). Stability comes from the program
keypair at:

```
target/deploy/walnut-keypair.json
```

- This file is **gitignored** (`*keypair*.json`). **Back it up securely.** If it
  is lost, a redeploy gets a _new_ program id, which then mismatches
  `declare_id!` and breaks every derived PDA and the existing localnet tests/CI.
- `arcium build` **re-syncs `declare_id!` to this keypair.** As long as the same
  `target/deploy/walnut-keypair.json` persists across builds, the id is stable
  and devnet == declared == localnet id. Do **not** edit the program id by hand.
- First-time only: if `target/deploy/walnut-keypair.json` does not yet exist and
  you must preserve the current id `5Eunnix…jTcjea`, restore the matching keypair
  to that path before building. Otherwise `arcium build` will generate a new
  keypair/id and you must not commit that change for this feasibility deploy.

---

## 4. Build

```bash
arcium build
```

Produces `target/deploy/walnut.so`, `target/idl/walnut.json` (the harness reads
this IDL), and the circuit artifacts under `build/` (`add_together.arcis`,
`init_position.arcis`, `apply_position_delta.arcis`) the harness uploads when
registering comp defs. `arcium build` re-syncs `declare_id!` to
`target/deploy/walnut-keypair.json` (section 3).

---

## 5. Deploy to devnet (offset 456)

`arcium deploy` deploys the program **and initializes the MXE account** in one
step.

```bash
arcium deploy \
  --cluster-offset 456 \
  --recovery-set-size 4 \
  --keypair-path ~/.config/solana/id.json \
  --rpc-url https://api.devnet.solana.com
```

- `--cluster-offset 456` — the shared Arcium devnet cluster.
- `--keypair-path` — must be the program's **upgrade authority** (the funded
  wallet from section 2).
- `--rpc-url` — defaults to the free public devnet RPC. It is rate-limited, and
  deploying a large program over it may fail partway and need a retry. If it
  keeps failing, pass any standard Solana JSON-RPC devnet endpoint instead (a
  free alternative is fine — do **not** hard-code or require a paid one). A
  partially deployed buffer can be resumed/recovered with
  `solana program deploy --buffer …`; keep the buffer keypair the CLI prints.

Record the program id the deploy prints; it must equal `5Eunnix…jTcjea`.

---

## 6. Record the devnet offset in config

Already committed in this repo, but confirm:

- `Arcium.toml` has:

  ```toml
  [clusters.devnet]
  offset = 456
  ```

- `config/networks/devnet.json` records offset 456, the default free RPC, and
  the program id (public configuration only).

The benchmark harness resolves the offset from these automatically (section 9).

---

## 7. Initialize the computation definitions on devnet

Comp defs persist on-chain and are initialized **once per deployment** (the
`init_*_comp_def` instructions), after which client/test/bench calls run against
the cluster. Provision exactly the two benchmarked ops (and the
`init_position` dependency of the stateful op):

```bash
export ANCHOR_PROVIDER_URL=https://api.devnet.solana.com
export ANCHOR_WALLET=~/.config/solana/id.json

pnpm --filter @walnut/sdk bench:init-devnet
```

This is idempotent (skip-if-exists): it registers `add_together`,
`init_position`, and `apply_position_delta` and uploads their circuits, leaving
any already-registered definition untouched. The benchmarks also do this inline
by default (`--init-compdefs`); provisioning here lets you run them later with
`--no-init-compdefs`.

> Alternatively, run inside an `arcium test --cluster devnet` context, where the
> provider URL, wallet, and offset are already set.

---

## 8. Verify MXE keygen finalized and the public key is fetchable

After deploy, the MPC nodes run distributed key generation for the MXE; the x25519
public key becomes fetchable only once that finalizes (this can take a short
while — the harness and `bench:init-devnet` already retry the fetch).

`pnpm --filter @walnut/sdk bench:init-devnet` prints
`MXE public key fetched: keygen has finalized on this cluster.` on success,
which is the verification. (Internally it calls `getMXEPublicKey(provider,
programId)`.) If it fails after retries, wait and re-run; keygen has not
finalized yet.

---

## 9. Run the benchmark harness against devnet

The environment from section 7 is sufficient. The harness reads the RPC/wallet
from the Anchor provider env and the **cluster offset 456** automatically (see
"How the offset resolves" below).

```bash
# ≥ 50 latency runs: p50/p95 latency, fees, callback CU, failure rate, throughput.
pnpm --filter @walnut/sdk bench:latency -- --runs 50 --reclaim --no-init-compdefs

# Stateful persisted path (one Position PDA).
pnpm --filter @walnut/sdk bench:latency -- --op apply_position_delta --runs 50 --no-init-compdefs

# 10 concurrent writers on ONE shared Position, then sustained Pool-like contention.
pnpm --filter @walnut/sdk bench:contention -- --concurrency 10 --duration 120 --no-init-compdefs
```

Each run writes a JSON artifact under `packages/sdk/bench/results/`; paste/link
those numbers into ADR-0003 (Task 2.7). Use `--reclaim` on the sequential
latency run so computation-account rent is recovered and separated from the
Arcium computation fee.

### How the offset resolves (devnet → 456)

`packages/sdk/bench/config.ts` → `resolveClusterOffset` picks the offset in this
order:

1. `--cluster-offset <n>` flag;
2. `WALNUT_BENCH_CLUSTER_OFFSET` or `ARCIUM_CLUSTER_OFFSET` env;
3. `getArciumEnv()` (set inside `arcium test`);
4. **`Arcium.toml` `[clusters.<network>]`**, where the network is inferred from
   the RPC URL — a devnet RPC selects `[clusters.devnet]` → **456**;
5. `0`.

So with the default/any devnet RPC and the committed `Arcium.toml`, the harness
uses offset 456 with no extra flags. Every cluster PDA in `bench/measure.ts`
(`getClusterAccAddress`, `getComputationAccAddress`, `getMempoolAccAddress`,
`getExecutingPoolAccAddress`, and `claimComputationRent`) uses that offset; the
MXE and comp-def PDAs use the program id. To force it explicitly:
`--cluster-offset 456` or `export ARCIUM_CLUSTER_OFFSET=456`.

---

## 10. Cost / SOL estimate (disclose, do not hide)

Open-source tooling does not make devnet operation free of SOL. Devnet SOL has
no monetary value, but the deploy and runs still consume it. **Estimates (not
measured — confirm on-chain):**

- **Program deployment rent: ~2–5 SOL.** Scales with `walnut.so` size;
  rent-exempt and reclaimable if the program is later closed. A buffer account
  transiently doubles this during the deploy itself.
- **MXE init / comp-def init (3 defs) + rent: ~0.1–0.3 SOL.** One-time per
  deployment; circuit uploads plus comp-def account rent.
- **Per operation (queue fee + Arcium computation fee): ~0.001–0.01 SOL each.**
  The queue tx fee is tiny; the Arcium computation fee is small on devnet.
- **Per-operation computation-account rent: ~0.002–0.01 SOL each.**
  **Reclaimable** with `--reclaim` / `claimComputationRent`.
- **A full pass (≥ 50 latency runs + 10-writer contention): ~2 SOL total.**
  Matches the bench README budget; most of it is reclaimable rent.

**Fund roughly 7–10 devnet SOL** to cover a sized program deploy plus a full
benchmark pass with headroom for rate-limited airdrop retries. Mainnet (Task
17/18) will add real, non-reclaimable costs — deployment, rent, per-computation
Arcium fees across many runs, keeper funds, and review — which the project
discloses rather than claiming do not exist.

---

## CI-safe verification (does not touch devnet)

These run with no wallet/cluster and are what CI enforces:

```bash
pnpm --filter @walnut/sdk typecheck     # includes tsconfig.bench.json (bench/ + init.ts)
pnpm lint                               # eslint --max-warnings 0
pnpm format:check                       # prettier
pnpm --filter @walnut/sdk bench:selftest   # pure-stats unit tests, no network
```

The `bench:*` run scripts are not `test:*` scripts, so the suite runner never
executes them in CI.

---

## Known uncertainties against Arcium 0.16.0

These could not be executed here (Windows PowerShell; arcium/solana/anchor hang),
so verify on devnet:

- **Deploy reliability on the public RPC.** Large program deploys over
  `https://api.devnet.solana.com` commonly fail partway due to rate limits;
  budget retries or switch `--rpc-url` to any free standard JSON-RPC devnet
  endpoint. Resume via `solana program deploy --buffer <buffer-keypair>`.
- **`--recovery-set-size 4`.** Used exactly as the 0.16.0 deploy docs specify; its
  effect on the node recovery set was taken from the docs, not re-derived.
- **Keygen finalization time.** The delay between a successful `arcium deploy`
  and `getMXEPublicKey` returning is not fixed; the harness/init retry, but if it
  keeps failing, wait longer and re-run section 8 before benchmarking.
