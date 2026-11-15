# ADR-0003: Arcium feasibility gate and encryption model

- Status: Proposed — provisional, conditional GO (final decision pending the Task 2.6 devnet benchmark)
- Date: 2026-11-16
- Related: Requirement 2; [ADR-0001](0001-stack-and-scope.md); [SPEC.md](../SPEC.md); [DEVNET_DEPLOY.md](../DEVNET_DEPLOY.md)

## Context

ADR-0001 chose Arcium MPC and deliberately left one question open: the Position ciphertext layout (one `Enc<Shared>` structure, or `Enc<Mxe>` plus an owner-authorized view computation). Requirement 2 makes Task 2 a blocking gate: no financial instruction beyond a test counter is implemented until a compiled spike proves the encrypted-state model and an ADR records the evidence.

This ADR records the result of Task 2.2 through Task 2.5 against the pinned Arcium 0.16.0 release and states a provisional decision. The Task 2.6 devnet benchmark has **not** been run yet; its empirical latency, fee, throughput, and failure-rate numbers are explicitly pending below. The decision is therefore conditional: the static feasibility criteria are met, and the final go/no-go is gated on the 2.6 numbers meeting the Requirement 2 targets.

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
- Devnet-pending (cannot be produced on localnet):
  - forged/garbled BLS signature → live `verify_output` rejection (item 3; state invariant is proven in Rust);
  - live SPL token-CPI callback failure (item 5; CPI wiring lands with vaults in Task 5);
  - live 180-slot mempool expiry and the matching expired-fee return (items 8/13). The `reclaimExpiredComputationFee` instruction is proven to build against the 0.16.0 IDL.
- Reclaim APIs recorded: `claimComputationRent` (finalized-computation rent) and `reclaimExpiredComputationFee` (expired-computation fee).

### 6. Transaction envelopes and the ALT constraint (Task 2.5)

`packages/sdk/tx-size/` builds the realistic queue and callback transaction for every v1 operation (actual Arcium standard accounts + all custom accounts + compute-budget instructions), compiles them as v0 messages with and without an address lookup table, and measures exact serialized size. It runs in CI with no cluster (`pnpm test:tx-size`), because size depends only on account count, signer/writable split, instruction-data length, and ALT membership. The authoritative per-operation byte table is printed by that command.

- **All queue and callback envelopes are < 1,232 bytes** (the hard limit), the asserted gate pass.
- All queue transactions fit comfortably under the 1,150-byte target without an ALT.
- **Key design constraint:** the two largest callbacks require an **Address Lookup Table** to stay under the target/limit — the **two-Position liquidation callback** (the tightest envelope, ≈1,176 B in the committed model; it carries both fixed-size Receipt ciphertexts per `design.md`) and the **Absorb callback**. Arcium already provisions a per-MXE ALT. If more headroom is needed, the liquidation Receipt ciphertexts can be emitted in the callback event instead of carried as instruction data (removing ≈290 B).
- Modeled (not measured) callback compute-unit limits used by the size model: light 200,000; position 400,000; pool 600,000; CPI 700,000; absorb 1,000,000; liquidation 1,200,000. Actual consumed CU is a Task 2.6 measurement.

### 7. Stop/go decision — PROVISIONAL / CONDITIONAL GO

All static feasibility criteria are met:

- the exact account-backed encrypted-input API compiles against pinned 0.16.0;
- persistent encrypted Position state stores, rereads, updates, and decrypts across computations;
- nonce/ownership safety (no repeated effective key/nonce pair; output nonce = input nonce + 1; two-owner outputs; rotation) holds;
- every transaction envelope is < 1,232 bytes, with ALTs required only for the liquidation and Absorb callbacks;
- the failure matrix behaves correctly for every localnet- and Rust-provable item.

**The final go/no-go is explicitly conditional on the Task 2.6 devnet benchmark meeting the Requirement 2 latency, fee, and throughput targets.** Per Requirement 2.10 and the Task 2.7 rule, **Task 3 MUST NOT begin until (a) CI passes AND (b) this ADR records a final GO backed by the 2.6 devnet numbers.** If any 2.6 gate fails, the design and requirements are updated before Task 3; the gate is not bypassed for schedule.

### PENDING — Task 2.6 devnet benchmark

The following metrics are not yet filled in. They will be produced on the shared Arcium devnet cluster (offset 456) by the harness in `packages/sdk/bench/` following `docs/DEVNET_DEPLOY.md`, and pasted/linked here (JSON artifacts under `packages/sdk/bench/results/`) before this ADR is moved to a final GO:

- p50 and p95 queue-to-callback latency over ≥ 50 runs;
- SOL fee per operation (queue tx fee) and Arcium computation fee per operation, with computation-account rent separated via `--reclaim`;
- callback compute units actually consumed;
- failure rate;
- accepted operations per minute (throughput);
- 10-concurrent-writer contention outcomes (success/retry/abandon) and sustained Pool-like contention.

Harness commands (run in Linux/WSL2, never in CI):

- `pnpm --filter @walnut/sdk bench:init-devnet`
- `pnpm --filter @walnut/sdk bench:latency -- --runs 50 --reclaim --no-init-compdefs`
- `pnpm --filter @walnut/sdk bench:latency -- --op apply_position_delta --runs 50 --no-init-compdefs`
- `pnpm --filter @walnut/sdk bench:contention -- --concurrency 10 --duration 120 --no-init-compdefs`

### Requirements and design reconciliation

No measured divergence is known yet (2.6 has not run). Once the devnet numbers exist, reconcile them against the latency/fee/throughput assumptions in `requirements.md` (Requirement 2.8) and `design.md`, and confirm these two static findings that Task 2.5/2.3 already established are reflected:

- the **liquidation and Absorb callbacks require an ALT** (a design constraint on those operations' account lists / Receipt-emission choice);
- the **output nonce = owning-input nonce + 1** client rule (fresh random per-encryption nonces).

No divergence requiring a `requirements.md`/`design.md` edit was found in this task, so none is made here; `design.md` already defers `EncGroup` to this ADR, and this ADR records that layout. Any reconciliation that the 2.6 numbers force is made before Task 3.

## Consequences

- Task 3 may be scheduled but not started: it is blocked until CI passes and this ADR is updated to a final GO with the 2.6 devnet numbers.
- The v1 encrypted model is fixed to single-structure `Enc<Shared>` Position state with local reads and `Enc<Mxe>` Pool state; the `Enc<Mxe>` owner-view fallback is not carried.
- Liquidation and Absorb callback transactions are built against an Address Lookup Table from the start; the liquidation callback is the tightest envelope and must stay within budget as its account list evolves.
- Clients and the SDK must generate a fresh random nonce per encryption and never sequential nonces under one key.
- CI (native Linux) is the authoritative environment for the Arcium localnet tests; local WSL runs may hang on the Docker/MPC UDP path and are not a gate signal.
