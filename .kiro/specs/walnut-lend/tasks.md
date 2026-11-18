# Implementation Plan: Walnut-Lend

## Overview

Build order: circuits (`lend_op`, `disclose_position`), Program state and math (`LendingPool`, `math/lend.rs`), Program instructions (lend, liquidate, health check, disclose), SDK, web, deploy script and e2e demo, then one final verification pass and one devnet run. Names follow `design.md`: circuits `lend_op`, `disclose_position` and the existing `rotate_position_key`; accounts `LendingPool` (PDA `["lending_pool", market]`, which also holds the oracle account, feed id and max price age) and `Disclosure` (PDA `["disclosure", position, auditor]`).

Each code task only compile-checks: `arcium build` / `cargo check` for circuit and program tasks, `tsc` (`pnpm typecheck`) for TypeScript, `next build` for the web app. No test suites run before task 7. Unit and property test code named in the design's Testing Strategy is written inside the code tasks (not as separate or optional tasks) and first runs in task 7.1.

## Tasks

- [x] 1. Circuits: `lend_op` and `disclose_position`
  - [x] 1.1 Write the `lend_op` circuit and report its weight
    - Replace `position_op` as the Position mutation circuit. Signature: `lend_op(state: Enc<Shared, PositionSecret>, owner: Shared, op, amount, shares, borrow_index_1e9, price, ltv_bps, liq_threshold_bps, position_debt_cap) -> (Enc<Shared, PositionSecret>, bool, u64, u64, u64)` returning new state, `applied`, and echoes of `op`, `amount`, `shares`
    - Ops 0 to 7: add collateral, withdraw collateral, borrow, repay, supply, withdraw supply, health check, liquidate, following the rules table in `design.md`
    - Only multiplication and comparison on secret values, using `u128` for the scaled products (`debt_shares * index * 1e4` vs `collateral * price * bps`); no secret division; a refusal returns the input state unchanged, resealed to `owner`
    - Health check never changes state and sets `applied = unhealthy`; liquidate re-checks unhealthy inside the same run, enforces the close factor (`shares*2 <= debt_shares` or full close) and `amount <= collateral`
    - Run `arcium build` and report the `lend_op` weight against the ~1.5e9 CU target (reference `position_op` 5.3e8)
    - If the weight exceeds the target, apply the design's fallback: move ops 4 and 5 (supply, withdraw supply) into a separate small circuit, keep `lend_op` for the rest, rebuild and report both weights. Carry the extra circuit name into tasks 3.1 and 6.1
    - _Requirements: 3.1, 3.2, 3.3, 4.4, 4.5, 4.6, 5.1, 5.4, 5.6, 9.1, 9.2, 9.3, 9.6_
  - [x] 1.2 Write the `disclose_position` circuit
    - Signature: `disclose_position(state: Enc<Shared, PositionSecret>, auditor: Shared, mask: u64) -> Enc<Shared, PositionSecret>`; mask bits 0 to 4 select free_sol, free_usdc, collateral, supply_shares, debt_shares; unselected fields are zero; the Position is not changed
    - Confirm `rotate_position_key` is already present in the circuits directory and needs no code change (it is only registered and exposed in the App)
    - Run `arcium build` and report the `disclose_position` weight
    - _Requirements: 7.1, 7.2, 7.5, 9.1, 9.2, 9.3_

- [x] 2. Program state, math and market rules
  - [x] 2.1 Add `LendingPool` state, accrual and share math
    - `state/lending_pool.rs`: `LendingPool` with `market`, `bump`, `oracle`, `feed_id`, `max_price_age_seconds`, `total_supply_shares`, `total_debt_shares`, `supply_index_1e9`, `borrow_index_1e9`, `last_accrual_ts`, `reserves_usdc`, `reserved`; `accrue(&mut self, now, rates, reserve_factor_bps)` using `math/rate.rs` (`borrow_rate_bps`, `rate_per_second_wad`, `project_index`), splitting interest between suppliers and reserves; reject index above `1e10` or overflow
    - `math/lend.rs`: borrow mint (ceil), repay burn (floor), supply shares (floor), withdraw-supply burn (ceil), liquidation burn and seize, bound checks, and the plain-Rust mirror `lend_op_rules` of the circuit
    - Write the unit tests and `proptest` property tests from the design's Testing Strategy in the same files, compile-checked only with `cargo check --tests`: Properties 1 to 6 and 11
    - _Requirements: 4.1, 4.2, 4.3, 4.5, 4.6, 3.1, 3.2, 5.2, 5.3_
  - [x] 2.2 Add pool init, Pyth price, pause and caps
    - `instructions/lending_pool.rs`: `init_lending_pool(max_price_age_seconds, oracle, feed_id)`, admin only, idempotent, seeds `total_supply_shares` from the USDC vault balance at `supply_index = 1e9`
    - Comp-def init instructions: `init_lend_op_comp_def`, `init_disclose_position_comp_def`, `init_rotate_position_key_comp_def` (plus the fallback circuit's init if task 1.1 split one out)
    - Price helper: require `account == pool.oracle` (`OracleAccountMismatch`), run `validate_price` with `PriceUse::Collateral` and the low bound (`price - conf`) using the pool's max age, normalize to USDC base units per SOL, clamp to `1e10`; reject before the Position is locked
    - Pause per op via `market/pause.rs` (`ensure_not_paused`, `ensure_action_allowed`): supply, borrow, add collateral and liquidation blocked; repay, withdraw supply, withdraw collateral, unshield allowed; health check never paused
    - Caps: supply cap on `total_supply_value + amount`, total-debt cap on `total_debt_value + amount`, per-asset cap on shield and add collateral (bounds collateral to 1e15 lamports); remove the caller-supplied price argument
    - Add `OperationKind::LendOp` and `OperationKind::Disclose`; unit tests for the pause matrix, caps and price rejection paths (Properties 7 and 8), compile-checked only
    - Verify with `cargo check`
    - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 2.1, 2.2, 2.3, 2.4, 2.5, 3.5, 9.3, 9.4_

- [x] 3. Program instructions
  - [x] 3.1 Add `lend_op` queue, callback and abandon
    - `state/lend_op.rs` and `instructions/lend_op.rs`, shaped like `state/position_op.rs`, replacing `position_op` as the mutation path: `lend_op(computation_offset, sequence, op, amount)` for ops 0 to 5 (owner signer; price account for ops 1 and 2)
    - Queue flow: pause and caps, accrue, price, compute shares publicly, check liquidity (`amount <= vault_usdc_balance - reserves` for borrow and withdraw supply), lock the Position, create the Operation Intent with the inputs commitment, queue `lend_op`
    - `lend_op_callback`: verify output; on `applied` store the payload, bump the generation, update pool totals and move USDC and wSOL; on refusal release the Lock; failed vault transfer leaves the Intent queued; `abandon_lend_op` releases the Lock after expiry
    - Existing shield, unshield, deposit, withdraw, collateral, borrow and repay flows keep working through the new path
    - Verify with `arcium build` and `cargo check`
    - _Requirements: 1.1, 1.5, 2.1, 2.2, 3.1, 3.2, 3.3, 3.4, 3.5, 4.1, 4.6, 9.4, 9.6_
  - [x] 3.2 Add `liquidate` and `check_health`
    - `check_health(computation_offset, sequence)`: anyone; queues op 6 with the low-bound price and current Borrow_Index; callback emits `HealthChecked { position, unhealthy }` and never changes the Position
    - `liquidate(computation_offset, sequence, repay_amount)`: Liquidator signer, permissionless, paused-market rejection; take the repay USDC into the vault at queue time; compute burn and seize publicly; lock the target Position and reseal it under its own stored Viewing Key
    - Box the custom accounts and pass the liquidator USDC account, liquidator wSOL account and token program through `remaining_accounts`, with their addresses bound in the Intent account commitment, to stay inside the SBF stack limit
    - Callback on `applied`: send `seize` wSOL from the wSOL vault to the Liquidator and update totals; on refusal (healthy, burn above debt, close factor, seize above collateral): refund the USDC and release the Lock; `abandon_lend_op` refunds the Liquidator after Lock expiry
    - Verify with `arcium build` and `cargo check`
    - _Requirements: 2.1, 5.1, 5.2, 5.3, 5.4, 5.5, 5.6, 9.6_
  - [x] 3.3 Add `Disclosure` account and disclose instructions
    - `Disclosure` account (`position`, `owner`, `auditor`, `mask`, `nonce`, `ciphertexts: [[u8; 32]; 5]`, `position_generation`, `slot`, `bump`), PDA `["disclosure", position, auditor]`
    - `instructions/disclose.rs`: `disclose_position(computation_offset, auditor_pubkey, mask)` (owner; Position not locked) and `disclose_position_callback` that writes or overwrites the Disclosure; a failed callback leaves the previous Disclosure in place
    - Expose the existing `rotate_position_key` instruction through the new comp-def registration only; no behaviour change
    - Verify with `arcium build` and `cargo check`
    - _Requirements: 6.1, 7.1, 7.2, 7.3, 7.5, 9.3_

- [x] 4. SDK
  - [x] 4.1 Lend-op and lending-pool helpers
    - `src/lend-op.ts`: `LendOp` enum (0 to 7), `applyLendOp(state, op, amount, shares, ctx)` mirror of the rules, share helpers, `buildLendOpInstruction`, `buildLiquidateInstruction`, `buildCheckHealthInstruction` (price account from `pyth.ts` and `oracle.ts`, no price argument)
    - `src/lending-pool.ts`: `fetchLendingPool`, `projectIndexes(pool, now)`, `supplyApyBps`, `borrowAprBps`, `utilizationBps`, `accruedDebt(shares, index)`, `accruedSupply(shares, index)`
    - Write `fast-check` property tests for Properties 3, 4 and 5 and golden-vector unit tests shared with the Rust mirror, compile-checked only
    - Verify with `pnpm typecheck`
    - _Requirements: 1.1, 1.6, 3.1, 3.2, 4.2, 4.3, 4.6, 5.1, 5.2, 5.3_
  - [x] 4.2 Disclosure and rotation helpers
    - `src/disclosure.ts`: `buildDiscloseInstruction`, `decryptDisclosure(disclosure, auditorSecret)` returning only selected fields
    - Rotation builder: generate a new Viewing Key, submit `rotate_position_key`, decrypt with the new key to confirm balances, and replace the stored key only after success
    - Verify with `pnpm typecheck`
    - _Requirements: 6.1, 6.2, 6.3, 6.4, 6.5, 7.1, 7.4, 7.5_

- [x] 5. Web app and landing page
  - [x] 5.1 Routing and landing page
    - Move the existing app under `web/app/app/` and update all links, navigation and redirects to `/app`
    - Port `components/landing/*` and `app/page.tsx` from `d:\solana\walnut` to `/` (hero, problem, solution, features, how-it-works, security, metrics strip, CTA, footer, page loader, scroll progress, magic bento), same layout and design, copy rewritten for Solana and Arcium MPC with no FHE wording; every launch CTA links to `/app`
    - Remove any text about reduced scope, demo limits or test-only status
    - Add example checks (no "FHE", CTA targets `/app`, nav links use `/app`) as compile-checked tests
    - Verify with `pnpm typecheck`
    - _Requirements: 8.1, 8.2, 8.3, 8.4, 8.5_
  - [x] 5.2 Lender and market UI
    - Supply and Withdraw supply cards; APR and APY from on-chain indexes and utilization; live accrued interest (decrypted shares times the public index, refreshed); Health check button; Liquidations panel (borrower address, check health, liquidate with a USDC amount)
    - Surface pause, cap, liquidity and missing or stale Oracle account rejections as clear errors with a retry message
    - Verify with `pnpm typecheck`
    - _Requirements: 1.5, 2.1, 2.2, 2.3, 2.4, 2.5, 3.1, 3.2, 3.3, 3.4, 4.2, 4.3, 5.1, 5.2, 5.5, 8.5_
  - [x] 5.3 Rotate Viewing Key, disclose and `/app/audit`
    - Rotate Viewing Key control: new key, submit, show the same balances after completion, keep the old key and show an error on failure; existing relock unchanged
    - Disclose to auditor form (auditor public key, field checkboxes) and `/app/audit`, which takes the auditor secret locally and shows the decrypted fields
    - Verify with `pnpm typecheck`, then `next build`
    - _Requirements: 6.1, 6.2, 6.3, 6.4, 6.5, 7.1, 7.3, 7.4, 7.5, 8.3, 8.5_

- [x] 6. Deploy script, e2e demo and docs
  - [x] 6.1 Write `scripts/lend-devnet.sh`
    - One resumable command run by the user from their terminal, recording progress in `artifacts/lend-deploy-state.json`; each step checks on-chain state first and is skipped when done; before each spending step compare the balance with the step cost plus a reserve and stop with a clear message when short
    - Steps in order: (1) build program and circuits; (2) reclaim rent from unused comp defs via `packages/sdk/src/reclaim.ts` (`position_op`, `add_together`, `init_position`, `apply_position_delta`, `liquidation_receipts`, `oversized_output`, `rotate_viewing_key`) before any upload; (3) single program upgrade: extend, write-buffer, deploy; (4) `init_lending_pool` (idempotent, seeds from the vault balance); (5) register and upload `lend_op`, `disclose_position`, `rotate_position_key` together (plus the split circuit if task 1.1 used the fallback); (6) print summary with the final balance
    - Budget per design (deployer about 23.9 SOL): extend 1 to 3, buffer about 16 held and refunded, `lend_op` 3.5, `disclose_position` 2, `rotate_position_key` 2, `LendingPool` and fees 0.1; program id, cluster 456 and branch `v2` unchanged
    - _Requirements: 9.3, 9.5_
  - [x] 6.2 Write the devnet e2e demo `demo/lend-e2e.ts`
    - Steps: supply, borrow, interest accrual (Supply_Index rises), repay, withdraw supply (more USDC than supplied), health check, liquidation refused on a healthy Position, liquidation applied on a deliberately unhealthy Position, Viewing Key rotation, disclosure and auditor decrypt
    - Each step compares decrypted or public values with expected values and fails with a message naming the step
    - Verify with `pnpm typecheck`
    - _Requirements: 5.2, 5.3, 5.5, 6.2, 7.4, 10.2, 10.3, 10.4, 10.5_
  - [x] 6.3 Update docs
    - Extend `docs/DEVNET_LOAN_DEMO.md` (or add a lend doc) with the new instructions, `scripts/lend-devnet.sh` and the e2e run
    - _Requirements: 9.5, 10.2_

- [ ] 7. Final verification, devnet run and push
  - [x] 7.1 Run the single full local verification pass and fix failures
    - Result: all checks green. Weights: `lend_op` 558,589,580; `disclose_position` 445,345,216; `rotate_position_key` 445,340,864
    - `arcium build`, `cargo fmt --check`, `cargo clippy`, `cargo test` (unit and `proptest` tests), `pnpm typecheck`, lint, format check, web `next build`
    - Fix all failures; record the final `lend_op` and `disclose_position` circuit weights
    - _Requirements: 9.2, 10.1_
  - [ ] 7.2 Devnet deployment, end-to-end run, fixes, commit and push to `v2`
    - Deployed (slot 523696369); LendingPool `EFEgr1aypvqqY6dje2afQjDufo957pYNGim65ZM32AiR`; max price age raised to 86,400 s because the devnet Pyth SOL/USD feed stopped publishing
    - `demo:lend` exit 0, all steps ok; `liquidation_applied` skipped (frozen price, no unhealthy Position; risk params are behind a 1-day timelock). Committed locally; push waits for approval
    - The user runs `scripts/lend-devnet.sh` from their terminal (one program upgrade, all new circuits registered together), then `demo/lend-e2e.ts`
    - Fix anything the run reveals, re-run only the affected checks, commit and push to branch `v2`
    - _Requirements: 9.5, 10.1, 10.2, 10.3, 10.4, 10.5_

## Notes

- No optional test tasks. Test code is written inside the code tasks and first runs in task 7.1; tasks 1 to 6 only compile-check.
- One devnet deployment, in task 7.2. Do not deploy or register circuits earlier. Agents cannot use the deployer keypair, so the user runs the script.
- Task 1.1 must report the `lend_op` weight against the ~1.5e9 CU target. If exceeded, split supply and withdraw supply into a separate small circuit (design fallback) before continuing, and add its comp def and upload to tasks 2.2, 3.1 and 6.1.
- `rotate_position_key` is existing: it is registered on devnet and exposed in the App, with no circuit change.
- Liquidation passes liquidator accounts through `remaining_accounts` with addresses bound in the Intent account commitment.
- The oracle config (account, feed id, max price age) lives on `LendingPool`; the `Market` layout is unchanged.
- Property numbers refer to `design.md`; requirement numbers refer to `requirements.md`. Properties 9 and 10 are covered by the devnet run, not by iterated tests.

## Task Dependency Graph

```json
{
  "waves": [
    { "id": 0, "tasks": ["1.1", "5.1"] },
    { "id": 1, "tasks": ["1.2", "2.1"] },
    { "id": 2, "tasks": ["2.2"] },
    { "id": 3, "tasks": ["3.1"] },
    { "id": 4, "tasks": ["3.2"] },
    { "id": 5, "tasks": ["3.3"] },
    { "id": 6, "tasks": ["4.1"] },
    { "id": 7, "tasks": ["4.2"] },
    { "id": 8, "tasks": ["5.2"] },
    { "id": 9, "tasks": ["5.3"] },
    { "id": 10, "tasks": ["6.1", "6.3"] },
    { "id": 11, "tasks": ["6.2"] },
    { "id": 12, "tasks": ["7.1"] },
    { "id": 13, "tasks": ["7.2"] }
  ]
}
```
