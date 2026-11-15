# Platform benchmark harness (Task 2.6)

This is the throughput / latency / cost half of the Arcium feasibility gate
(Requirement 2.8). It measures, against a configurable cluster, the numbers
ADR-0003 (Task 2.7) needs:

- **p50 and p95 callback latency** over at least 50 runs (queue → callback
  finalization), using the existing `add_together` / `apply_position_delta`
  computations;
- **fees per operation** (Solana tx fee + Arcium computation fee), **callback
  compute units actually consumed**, **failure rate**, and **accepted
  operations per minute** (throughput);
- **10 concurrent writers** contending on one shared account, with retry and
  abandonment, reporting success / retry / abandon counts and latency under
  contention;
- **sustained Pool-like contention** (many operations serialized through one
  locked account over a time window).

It is a **measurement tool, not production code**. It sends real transactions
and spends real (dev)net SOL. It does not run in CI (CI has no devnet wallet),
but it type-checks, lints, and formats as part of the normal workspace checks.

## Requirements

- The program deployed to the target cluster with its MXE initialized.
- `target/idl/walnut.json` present (produced by `arcium build`).
- `build/add_together.arcis` and, for the stateful op, `build/init_position.arcis`
  and `build/apply_position_delta.arcis` present (also from `arcium build`), so
  the harness can register any missing computation definitions.
- A funded wallet keypair on the target cluster.

## Environment

The harness reads the RPC URL and wallet exactly like `arcium test --cluster
devnet` does — from the Anchor provider environment — and the Arcium cluster
offset from the environment or `Arcium.toml`. No private endpoint or key is ever
hard-coded; the only built-in default is the **free public devnet RPC**
(`https://api.devnet.solana.com`) and the standard Solana CLI keypair path.

| Variable                      | Purpose                          | Default                         |
| ----------------------------- | -------------------------------- | ------------------------------- |
| `ANCHOR_PROVIDER_URL`         | RPC URL (devnet by default)      | `https://api.devnet.solana.com` |
| `ANCHOR_WALLET`               | Path to the payer keypair JSON   | `~/.config/solana/id.json`      |
| `ARCIUM_CLUSTER_OFFSET`       | Arcium cluster offset            | from `Arcium.toml` / `0`        |
| `WALNUT_BENCH_CLUSTER_OFFSET` | Overrides the cluster offset     | —                               |
| `WALNUT_BENCH_RUNS`           | Latency run count                | `50`                            |
| `WALNUT_BENCH_CONCURRENCY`    | Concurrent writers               | `10`                            |
| `WALNUT_BENCH_DURATION`       | Sustained window, seconds        | `120`                           |
| `WALNUT_BENCH_MAX_ATTEMPTS`   | Attempts before abandonment      | `3`                             |
| `WALNUT_BENCH_TIMEOUT_MS`     | Finalization timeout per attempt | `120000`                        |
| `WALNUT_BENCH_OUT`            | Artifact output directory        | `packages/sdk/bench/results`    |

Every env var has an equivalent CLI flag (below), and flags win over env.

### Funding a devnet keypair

```bash
# Create a keypair if you do not have one, then point the harness at it.
solana-keygen new -o ~/.config/solana/id.json          # or any path
export ANCHOR_WALLET=~/.config/solana/id.json
export ANCHOR_PROVIDER_URL=https://api.devnet.solana.com

# Airdrop devnet SOL (repeat / use a faucet if rate-limited).
solana airdrop 2 "$(solana-keygen pubkey "$ANCHOR_WALLET")" --url devnet
solana balance "$(solana-keygen pubkey "$ANCHOR_WALLET")" --url devnet
```

Budget roughly one queue transaction fee + one Arcium computation fee + the
(reclaimable) computation-account rent per run; 50 latency runs plus a contention
pass comfortably fit in ~2 devnet SOL. Use `--reclaim` to recover computation
rent as you go.

## Commands

Run from the repository root. Flags after `--` are forwarded to the harness.

### Latency, fees, CU, failure rate, throughput (≥ 50 runs)

```bash
# add_together (stateless; cleanest pure-latency + fee/CU numbers)
pnpm --filter @walnut/sdk bench:latency -- --runs 50 --reclaim

# apply_position_delta (stateful persisted path, one Position PDA)
pnpm --filter @walnut/sdk bench:latency -- --op apply_position_delta --runs 50
```

### 10 concurrent writers (burst) + sustained Pool-like contention

```bash
# 10 writers contend on ONE shared Position account; then sustain for 120s.
pnpm --filter @walnut/sdk bench:contention -- --concurrency 10 --duration 120

# Burst only (no sustained window).
pnpm --filter @walnut/sdk bench:contention -- --concurrency 10 --duration 0
```

### Pointing at localnet

```bash
pnpm --filter @walnut/sdk bench:latency -- \
  --rpc http://127.0.0.1:8899 --cluster-offset 0 --runs 50
```

Or run it the same way the tests run, inside an `arcium test --cluster <net>`
context, where `ANCHOR_PROVIDER_URL`, `ANCHOR_WALLET`, and the cluster offset are
already set; then `--no-init-compdefs` if the definitions are already provisioned.

### All flags

| Flag                              | Meaning                                          |
| --------------------------------- | ------------------------------------------------ |
| `latency` \| `contention`         | Subcommand (first positional)                    |
| `--op <name>`                     | `add_together` or `apply_position_delta`         |
| `--runs <n>`                      | Latency run count (≥ 50 to satisfy the task)     |
| `--concurrency <n>`               | Concurrent writers (10 to satisfy the task)      |
| `--duration <sec>`                | Sustained-contention window (0 = burst only)     |
| `--max-attempts <n>`              | Attempts before an operation is abandoned        |
| `--timeout <ms>`                  | Finalization timeout per attempt                 |
| `--reclaim`                       | Reclaim computation rent and measure the refund  |
| `--no-init-compdefs`              | Skip comp-def registration (already provisioned) |
| `--rpc <url>` / `--wallet <path>` | Override RPC / keypair without env vars          |
| `--cluster-offset <n>`            | Override the Arcium cluster offset               |
| `--out <dir>`                     | Artifact output directory                        |

## Output

Each run prints a human-readable table (latency, callback CU, queue CU, fees;
p50/p95/p99/mean/min/max) plus throughput and outcome counts, and writes a
machine-readable JSON artifact to the output directory
(`<kind>-<op>-<timestamp>.json`). Paste or link the JSON numbers into ADR-0003.

### JSON artifact schema

```jsonc
{
  "meta": {
    "kind": "latency" | "contention",
    "op": "add_together" | "apply_position_delta",
    "timestamp": "ISO-8601",
    "rpcUrl": "…",
    "clusterOffset": 0,
    "programId": "…",
    "walletPubkey": "…",
    "params": { "runs", "concurrency", "durationSec", "maxAttempts",
                "finalizeTimeoutMs", "reclaim" }
  },
  "wallClockSec": 0,
  "throughputPerMin": 0,                 // accepted (ok) ops per minute
  "latencyMs":  { "count","min","max","mean","p50","p95","p99","stddev" },
  "callbackCu": { … same summary shape … }, // callback CU actually consumed
  "queueCu":    { … },                       // queue tx CU
  "fees": {
    "samples": 0,
    "queueTxFee":        { …summary (lamports)… },  // user-paid Solana fee
    "callbackTxFee":     { … },                     // node-paid; informational
    "arciumFeePlusRent": { … },                     // computation fee + comp rent
    "totalUserCost":     { … },                     // payer balance delta
    "meanQueueTxFeeSol": 0,
    "meanArciumFeePlusRentSol": 0,
    "meanTotalUserCostSol": 0
  },
  "outcomes": {
    "total","ok","abandoned","retried","totalRetries","failureRate"
  },
  "samples": [ /* every raw OpSample: latency, sigs, CU, fees, attempts … */ ]
}
```

## How fees and compute units are read (Arcium 0.16.0)

There is no dedicated fee/CU introspection helper in `@arcium-hq/client` 0.16.0,
so the harness reads realized values straight from transaction metadata:

- **queue fee + queue CU** — `connection.getTransaction(queueSig).meta.fee` and
  `.meta.computeUnitsConsumed`, the user-paid Solana cost of enqueuing.
- **callback CU actually consumed + callback fee** —
  `getTransaction(finalizeSig).meta.computeUnitsConsumed` / `.fee`, where
  `finalizeSig` is the signature returned by `awaitComputationFinalization`
  (documented to return the finalization transaction signature). The callback
  fee is node-paid and recorded only for completeness.
- **Arcium computation fee** — measured as the payer balance delta across the
  operation minus the queue tx fee (sequential latency runs only; balance
  deltas are meaningless when operations overlap). This bundles the computation
  fee and the computation-account rent; `--reclaim` calls
  `claimComputationRent` and measures the refund so the rent can be separated
  from the fee.

`@arcium-hq/client` also exposes `getMempoolPriorityFeeStats`, which can be used
later to report mempool priority-fee pressure; it is not needed for the v1
numbers.

## Resilience, retry, and abandonment

Each operation runs the Task 2.1 lifecycle at the client level: on a queue or
finalization error (RPC fault, timeout, finalization failure) the attempt is
retried with a **fresh computation offset** (mirroring the design's "retry
installs a new offset") up to `--max-attempts`; if every attempt fails the
operation is recorded as **abandoned** rather than aborting the whole run. This
is the client-side analogue of the design's permissionless timeout abandonment —
the test-only program carries no financial Lock enforcement (the Task 2.1 Locks
live in the Rust layer), so true Lock-contention failures surface instead as
mempool / executing-pool capacity limits and RPC faults, which the retry /
abandon counters capture.

## Localnet vs devnet

- **Measurable on localnet:** the full mechanics — queue → finalization latency,
  callback CU consumed, queue/callback fees, retry/abandon behavior, and
  single-account serialization. Localnet latency and fees are **not**
  representative of production.
- **Needs devnet (Requirement 2.8):** the p50/p95 latency, fees, failure rate,
  and sustained operations-per-minute that ADR-0003 records must come from
  devnet. Minimums to satisfy the task: **≥ 50 latency runs** and **10 concurrent
  writers**; scale `--runs`, `--concurrency`, and `--duration` to the time and
  funds available.

## CI safety

- Type-checked via `tsconfig.bench.json` (wired into `@walnut/sdk`'s `typecheck`
  script, run by `pnpm typecheck` in the CI "JS checks" job).
- Linted by `pnpm lint` and formatted by Prettier like the rest of the workspace.
- **Not executed in CI:** the `bench` / `bench:latency` / `bench:contention`
  scripts are not `test:*` scripts, so the suite runner never invokes them.
- `pnpm --filter @walnut/sdk bench:selftest` runs the pure-stats unit tests with
  Node's built-in test runner (no cluster, no wallet) for a quick sanity check.
