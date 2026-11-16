# ADR-0003: Arcium feasibility gate and encryption model

- Status: Accepted — FINAL GO for Task 3 (backed by the Task 2.6 devnet benchmark)
- Date: 2026-11-16 (decision finalized with the Task 2.6 devnet results)
- Related: Requirement 2; [ADR-0001](0001-stack-and-scope.md); [SPEC.md](../SPEC.md); [DEVNET_DEPLOY.md](../DEVNET_DEPLOY.md)

## Context

ADR-0001 chose Arcium MPC and deliberately left one question open: the Position ciphertext layout (one `Enc<Shared>` structure, or `Enc<Mxe>` plus an owner-authorized view computation). Requirement 2 makes Task 2 a blocking gate: no financial instruction beyond a test counter is implemented until a compiled spike proves the encrypted-state model and an ADR records the evidence.

This ADR records the result of Task 2.2 through Task 2.6 against the pinned Arcium 0.16.0 release. Tasks 2.2 through 2.5 established the static feasibility criteria. The Task 2.6 benchmark has since been run against real Solana devnet (Arcium cluster offset 456), and its measured latency, fee, callback compute-unit, throughput, and failure-rate numbers are recorded below. The decision is final: **GO for Task 3**, with the residual items listed in the decision section tracked.

All feasibility code runs green in CI on native Linux (the `rust-arcium` and JS-checks jobs). The local WSL Arcium test hangs on the Docker/MPC UDP path, so CI is the authoritative environment for the localnet Arcium integration tests; this is a known environment limitation, not a code result.

## Decision

### 1. Encryption model: `Enc<Shared>` with local reads (primary), `Enc<Mxe>` view fallback not required

Walnut v1 persists each Position as a single `Enc<Shared, PositionSecret>` structure sealed to the owner's Viewing Key, and the client decrypts it locally. The `Enc<Mxe>` + owner-authorized-view fallback that Requirement 2.5 allows was evaluated and is **not** required.

This resolves the layout `EncGroup` left open in `design.md` (the "exact public key metadata, nonce, and ciphertext format approved by ADR-0003").

Evidence (all compiled and asserted against pinned 0.16.0):

- **Persistent account-backed state across computations (Task 2.2).** `encrypted-ixs/src/lib.rs` defines `PositionSecret` with five scalars (`free_sol: u64`, `free_usdc: u64`, `collateral_sol: u64`, `supply_shares_usdc: u128`, `debt_shares_usdc: u128`). `init_position` seals the payload with `initial.owner.from_arcis(...)`; `apply_position_delta` takes the stored ciphertext as an account-backed `Enc<Shared, PositionSecret>` input, applies a delta, and re-seals to the same owner. `tests/position-state.ts` stores, rereads in a later computation, updates every field in randomized order, and decrypts locally after each step with the exact expected value.
- **Two owners from one computation + rotation (Task 2.3).** `liquidation_receipts` returns `(Enc<Shared, (u64, u64)>, Enc<Shared, (u64, u64)>)`, one Receipt sealed to the borrower key and one to the liquidator key; `tests/nonce-ownership.ts` proves each owner decrypts only its own and the wrong key yields garbage. `rotate_viewing_key` re-encrypts the stored payload to a new key; the callback swaps `viewing_pubkey` and bumps `viewing_key_epoch` 0 → 1 atomically, and the payload then decrypts under the new key, not the old.

Because the single-structure `Enc<Shared>` layout stores, rereads, updates, seals to two owners, and rotates — all within account and transaction budgets — the conditions in Requirement 2.5 that would force the `Enc<Mxe>` view fallback did not occur. Aggregate Pool state remains `Enc<Mxe, PoolSecret>` (per `design.md`), because no individual owns it; that is unchanged by this ADR.

### 2. Exact account-backed encrypted-input API (pinned 0.16.0)

Recorded so Task 3+ builds on the proven surface (`programs/walnut/src/lib.rs`, `encrypted-ixs/src/lib.rs`):

- **Circuit side (Arcis):** `use arcis::*;` with `#[encrypted] mod circuits`, `#[instruction]` functions; inputs are `Enc<Shared, T>` (and `Enc<Mxe, T>` for Pool), read with `input.to_arcis()` and sealed with `owner.from_arcis(value)`. Tuple returns produce one `SharedEncryptedStruct<N>` per owner, each with its own `encryption_key` and `nonce`.
- **Program side (Anchor + Arcium):** `#[arcium_program]`; `arcium_client::idl::arcium::types::CallbackAccount { pubkey, is_writable }` names callback-writable custom accounts; `init_computation_def(ctx.accounts, None)` registers comp defs.
- **Argument marshalling:** `ArgBuilder::new().x25519_pubkey(pub_key).plaintext_u128(nonce).encrypted_u64(..)/.encrypted_u128(..).build()`, in exact circuit-signature order (pubkey, nonce, then one ciphertext per scalar in field order).
- **Queue:** `queue_computation(ctx.accounts, computation_offset, args, vec![<Callback>::callback_ix(computation_offset, &ctx.accounts.mxe_account, &[CallbackAccount{..}])?], 1, cu_price_micro, callback_cu_limit)`. A `callback_cu_limit` of `0` derives the limit from simulation; a fixed non-zero value disables the retry-on-exhaustion.
- **Callback:** `#[arcium_callback(encrypted_ix = "...")]`; `SignedComputationOutputs<T>::verify_output(&cluster_account, &computation_account)` returns the typed output, whose field carries `encryption_key`, `nonce`, and `ciphertexts`; an `Err` leaves the Intent queued for recovery (no financial write).
- **Client (`@arcium-hq/client` 0.16.0):** `RescueCipher` + `x25519` for sealing, `getMXEPublicKey`, `awaitComputationFinalization`, `getArciumEnv`, the cluster/mempool/executing-pool/computation/comp-def PDA derivations, `uploadCircuit`, `claimComputationRent`, and the Arcium program's `reclaimExpiredComputationFee` instruction (built directly; no client helper in 0.16.0).

### 3. Persisted Position encryption layout

- `PositionSecret` holds 5 scalar fields (above). The `Enc<Shared>` output is a `SharedEncryptedStruct<5>` = `encryption_key` (32, the Viewing public key) + `nonce` (16) + 5 × 32-byte ciphertexts.
- The long-lived `encryption_key` is stored once on the account as `viewing_pubkey`; the per-revision `EncryptedPositionState` (`EncGroup`) stores `nonce: u128` + `[[u8; 32]; 5]`, a fixed **176-byte** payload (`programs/walnut/src/position.rs`, asserted).
- The whole `SpikePosition` account is **265 bytes** (8 discriminator + 32 owner + 32 viewing key + 8 epoch + 8 revision + 176 payload + 1 bump), far under account and transaction limits — asserted in `position.rs` unit tests.
- Store → read → update → decrypt is proven end-to-end in `tests/position-state.ts`; the queue re-feeds the five ciphertexts in canonical field order and the client decrypts in the same order.

### 4. Nonce and ownership safety (Task 2.3)

- Two-owner `Enc<Shared>` tuple outputs are each sealed to their own key; `tests/nonce-ownership.ts` records every effective `(key, nonce)` pair and fails loudly on any repeat.
- The MXE derives each **output nonce as the owning input's nonce + 1** (Arcium encryption model). Clients therefore MUST pick a fresh random 16-byte nonce per encryption and never pick sequential nonces under one key, so no effective `(key, nonce)` pair repeats across inputs and outputs. Distinct owners always have distinct effective keys, so their pairs never collide.
- Viewing Key rotation commits old/new epoch and public key and re-encrypts atomically; the payload decrypts under the new key and not the old (epoch 0 → 1 asserted).

### 5. Failure matrix (Task 2.4)

Thirteen conditions are split between a localnet Arcium layer (`tests/failure-matrix.ts`) and a pure-Rust state-machine layer (`programs/walnut/src/operation/failure_matrix.rs`, one `#[test]` per item plus a 5,000-iteration property test):

- Localnet-proven: success callback (Applied); `OutputTooLarge` via the `oversized_output` circuit; callback compute-unit exhaustion via a fixed tiny `callback_cu_limit`; finalized-rent claim via `claimComputationRent`.
- Pure-Rust-proven coordination invariants: static-failure → `Failed`; invalid output left queued then abandoned; callback custom-account failure; callback CPI failure (revert leaves nothing); decision-expiry → `Expired` and mempool expiry → abandonment; abandon-before-late-execution; duplicate callback no-op; late callback no-op; and the cross-cutting property that no failing path ever moves a generation.
- Not observable on localnet and **not covered by the Task 2.6 benchmark** (residual items, not passed; see section 7):
  - forged/garbled BLS signature → live `verify_output` rejection (item 3; state invariant is proven in Rust);
  - live SPL token-CPI callback failure (item 5; CPI wiring lands with vaults in Task 5);
  - live 180-slot mempool expiry and the matching expired-fee return (items 8/13). The `reclaimExpiredComputationFee` instruction is proven to build against the 0.16.0 IDL.
- Reclaim APIs recorded: `claimComputationRent` (finalized-computation rent) and `reclaimExpiredComputationFee` (expired-computation fee).

### 6. Transaction envelopes and the ALT constraint (Task 2.5)

`packages/sdk/tx-size/` builds the realistic queue and callback transaction for every v1 operation (actual Arcium standard accounts + all custom accounts + compute-budget instructions), compiles them as v0 messages with and without an address lookup table, and measures exact serialized size. It runs in CI with no cluster (`pnpm test:tx-size`), because size depends only on account count, signer/writable split, instruction-data length, and ALT membership. The authoritative per-operation byte table is printed by that command.

- **All queue and callback envelopes are < 1,232 bytes** (the hard limit), the asserted gate pass.
- All queue transactions fit comfortably under the 1,150-byte target without an ALT.
- **Key design constraint:** the two largest callbacks require an **Address Lookup Table** to stay under the target/limit — the **two-Position liquidation callback** (the tightest envelope, ≈1,176 B in the committed model; it carries both fixed-size Receipt ciphertexts per `design.md`) and the **Absorb callback**. Arcium already provisions a per-MXE ALT. If more headroom is needed, the liquidation Receipt ciphertexts can be emitted in the callback event instead of carried as instruction data (removing ≈290 B).
- Modeled (not measured) callback compute-unit limits used by the size model: light 200,000; position 400,000; pool 600,000; CPI 700,000; absorb 1,000,000; liquidation 1,200,000. Actual consumed callback CU was measured in Task 2.6 (see section 8).

### 7. Stop/go decision — FINAL GO for Task 3

**Decision: FINAL GO. Task 3 may begin because (a) CI passed and (b) this ADR now records a final GO backed by the Task 2.6 devnet numbers in section 8.**

Rationale:

- Requirement 2.8 requires that latency, callback compute units, fees, failure rate, and sustained operations per minute be measured. It sets no numeric ceiling, and all of them were measured on devnet (section 8).
- No gate failed: 0% failure across 321 devnet operations (50 + 50 + 221).
- Every transaction envelope is under 1,232 bytes. An Address Lookup Table is needed for the liquidation and Absorb callbacks.
- `Enc<Shared>` persistent state and two-owner outputs work, so the `Enc<Mxe>` fallback is not carried.
- Requirement 2.10 (update the design and requirements when a gate fails) is therefore not triggered.

Static feasibility criteria (established in Tasks 2.2 through 2.5):

- the exact account-backed encrypted-input API compiles against pinned 0.16.0;
- persistent encrypted Position state stores, rereads, updates, and decrypts across computations;
- nonce/ownership safety (no repeated effective key/nonce pair; output nonce = input nonce + 1; two-owner outputs; rotation) holds;
- every transaction envelope is < 1,232 bytes, with ALTs required only for the liquidation and Absorb callbacks;
- the failure matrix behaves correctly for every localnet- and Rust-provable item.

Residual items, tracked and **not** passed by this ADR:

- Forged BLS signature rejection, real token-CPI callback failure, live 180-slot queued expiry, and expired-fee return are not observable on localnet and were not exercised by the benchmark. They stay deferred to the tasks that introduce those paths (Task 5 for token CPI; the keeper and recovery tasks for expiry and fee return).
- Lock-conflict behavior under contention. The spike program has no real financial Lock, so the 10-writer and sustained runs measured MPC mempool and executing-pool serialization on one account, not on-chain Lock rejection. Lock conflicts are covered by the Task 2.1 pure-Rust state machine and must be re-measured in Task 3 once Locks gate real accounts.
- Devnet numbers are not mainnet numbers (cluster size, load, and fee schedule differ). Re-measure before mainnet in the Task 16/17 soak.
- The callback compute-unit p95 spike seen under contention (section 8.3) needs investigation.

### 8. Task 2.6 devnet benchmark results

Measured on Solana devnet against Arcium cluster offset 456, with at least 50 runs per scenario, by the harness in `packages/sdk/bench/` (see `docs/DEVNET_DEPLOY.md`). Raw JSON is written under `packages/sdk/bench/results/`, which is gitignored, so the numbers are reproduced here.

Deployment facts:

- The program is deployed to Solana devnet at `5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea` (matches `config/networks/devnet.json`). Program size is 749,200 bytes and program rent is about 3.8 SOL (reclaimable if the program is closed).
- The MXE was initialized on Arcium devnet cluster offset 456 with recovery-set-size 4. MXE keygen finalized, the MXE x25519 public key was fetchable, and key-recovery material was computed.

#### 8.1 Latency: `add_together` (stateless)

50 sequential runs with `--reclaim`.

- Queue-to-callback latency: p50 4,696 ms; p95 5,081 ms; p99 5,385 ms; mean 4,683 ms; min 3,927 ms; max 5,523 ms.
- Callback CU consumed: 23,274 (constant).
- Queue transaction CU: p50 112,098; p95 125,602.
- Queue transaction fee: 5,000 lamports.
- Arcium fee plus computation-account rent per operation: p50 3,225,949 lamports (about 0.0032 SOL); mean 0.003304 SOL. Total user cost per operation is about 0.0033 SOL.
- Throughput: 5.45 accepted ops/min sequential, over 550 s.
- Outcomes: 50/50 ok, 0 abandoned, 1 retry, failure rate 0.00%.

#### 8.2 Latency: `apply_position_delta` (stateful)

50 sequential runs, no reclaim. Each run reads the persisted Position and writes it back.

- Queue-to-callback latency: p50 4,644 ms; p95 5,333 ms; p99 5,837 ms; mean 4,680 ms; min 3,901 ms; max 6,067 ms.
- Callback CU consumed: 24,595 (constant).
- Queue transaction CU: p50 120,493; p95 129,714.
- Queue transaction fee: 5,000 lamports.
- Arcium fee plus rent per operation: p50 5,039,700 lamports (about 0.0050 SOL); mean 0.005141 SOL. Total is about 0.005146 SOL per operation.
- Throughput: 7.07 accepted ops/min sequential, over 424 s.
- Outcomes: 50/50 ok, 0 abandoned, 1 retry, failure rate 0.00%.

#### 8.3 Contention: `apply_position_delta`

Phase 1 is 10 concurrent writers on one shared Position. Phase 2 is a sustained 120 s run with 10 in flight.

- Outcomes: 221 operations, 221 ok, 0 abandoned, 3 retries, failure rate 0.00%.
- Latency: p50 4,905 ms; p95 5,938 ms; p99 7,255 ms; max 9,854 ms.
- Callback CU: p50 24,595, but p95 about 132,999 and max about 136,672. A minority of callbacks consumed far more CU than the median. Size `callback_cu_limit` and the Task 3 callback budget against the p95/max figure, not the median, and investigate why those callbacks cost more.
- Queue CU: p50 116,597.
- Sustained throughput: 55.51 accepted ops/min with 10 in flight, over 239 s wall clock.
- Per-op Arcium fee is n/a in this phase, because the balance-delta fee derivation is only valid for sequential runs.

#### 8.4 Reading the numbers

- Sequential throughput (5 to 7 ops/min) is latency-bound: one operation at a time, about 4.7 s each, plus reclaim and harness overhead. The 55.5 ops/min concurrency-10 sustained figure is the better capacity input for keeper and cap sizing (Requirement 14, Task 14.5).
- The contention run measured MPC mempool and executing-pool serialization, not on-chain Lock rejection (see residual items in section 7).
- Failure rate was 0.00% across all 321 operations. The 5 retries (1 + 1 + 3) all resolved without abandonment.

Harness commands used (run in Linux/WSL2, never in CI):

- `pnpm --filter @walnut/sdk bench:init-devnet`
- `pnpm --filter @walnut/sdk bench:latency -- --runs 50 --reclaim --no-init-compdefs`
- `pnpm --filter @walnut/sdk bench:latency -- --op apply_position_delta --runs 50 --no-init-compdefs`
- `pnpm --filter @walnut/sdk bench:contention -- --concurrency 10 --duration 120 --no-init-compdefs`

### 9. Operational findings

- **RPC reliability for deployment.** The public `api.devnet.solana.com` endpoint could not land the 781 program-write transactions (blockhash expired, 0% progress). The deploy succeeded through a free-tier RPC endpoint (Helius free tier) using a resumable buffer. Program deployment needs a reliable RPC. The optional-adapter policy is unchanged: a free-tier or paid RPC is a deploy-time convenience, not a requirement of the product.
- **`ComputationDefinitionNotCompleted` (Arcium error 6300).** The comp-def init skipped circuit upload when the comp-def account already existed. An interrupted upload therefore left `init_position` incomplete, and every queue failed with error 6300. `ensureCompDef` now resumes the upload. Lesson: treat comp-def existence and circuit-upload completeness as separate states.
- **Bench process exit.** Open websocket subscriptions kept Node alive, so bench entry points did not exit. They now call `process.exit(0)`.
- **Callback CU p95 spike.** Under contention a minority of `apply_position_delta` callbacks consumed about 133,000 to 137,000 CU against a 24,595 median (section 8.3). The cause is not yet known. It is the figure to size `callback_cu_limit` and the Task 3 callback budget against.

### Requirements and design reconciliation

No measured value contradicts `requirements.md` or `design.md`, so neither is edited. Requirement 2.8 asks for measurement and sets no numeric ceiling, and Requirement 2.10 is not triggered. The two static findings from Tasks 2.3 and 2.5 are already reflected as design constraints:

- the **liquidation and Absorb callbacks require an ALT** (a constraint on those operations' account lists and the Receipt-emission choice);
- the **output nonce = owning-input nonce + 1** client rule (fresh random per-encryption nonces).

`design.md` already defers `EncGroup` to this ADR, and this ADR records that layout. Capacity inputs for Requirement 14 and Task 14.5 should use the section 8.3 concurrency-10 figure, and Task 3 should size its callback CU budget from the section 8.3 p95/max.

## Consequences

- Task 3 may begin. The two conditions are met: CI passed, and this ADR records a final GO backed by the Task 2.6 devnet numbers.
- The residual items in section 7 are tracked and must be closed or re-measured in the tasks that introduce those paths. They are not passed by this ADR.
- Callback compute-unit limits in Task 3 are sized from the measured p95/max (about 133,000 to 137,000 CU for a stateful callback under contention), not from the 24,595 median.
- Capacity and keeper/cap sizing use the concurrency-10 sustained figure (55.51 ops/min), and every mainnet-facing number is re-measured in the Task 16/17 soak.
- Program deploy needs a reliable RPC. This is a deploy-time convenience only, and no paid RPC is mandatory for the product.
- The v1 encrypted model is fixed to single-structure `Enc<Shared>` Position state with local reads and `Enc<Mxe>` Pool state; the `Enc<Mxe>` owner-view fallback is not carried.
- Liquidation and Absorb callback transactions are built against an Address Lookup Table from the start; the liquidation callback is the tightest envelope and must stay within budget as its account list evolves.
- Clients and the SDK must generate a fresh random nonce per encryption and never sequential nonces under one key.
- CI (native Linux) is the authoritative environment for the Arcium localnet tests; local WSL runs may hang on the Docker/MPC UDP path and are not a gate signal.
