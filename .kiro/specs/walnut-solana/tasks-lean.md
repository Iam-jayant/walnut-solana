# Lean Plan: Walnut-Solana Tasks 8-14 (time-boxed)

The original `tasks.md` is unchanged and remains the full spec. This file is a faster execution path for Tasks 8-14 only.
Tasks 1-7 are completed on the original list (in progress by the user) and are not touched here.
Start this plan only after the user reports that Task 7 is done.

## Rules for this plan

- Build first, test once at the end. During a Pass, write code only.
- End of each Pass: run `cargo check` and `tsc --noEmit` only. No test suites, no devnet.
- Do not write property tests, adversarial suites, or demos inside a Pass.
- Keep these safety rules in the code from the start, because they are expensive to retrofit:
  - Locks and Intents are used for every state change.
  - A failed or stale operation moves zero value.
  - Rounding always favors the protocol.
  - A liquidation must improve health.
  - The first-supplier rule applies only in the canonical empty state.
- Anything cut goes into `docs/adr/0002-deferred-features.md` as one line, not a document.
- Keep the Rust, Arcis, and TypeScript reference-model code in step by writing them in the same Pass.
- If a Pass runs 30 minutes over its budget, cut scope from that Pass. Do not extend the budget.

## Time budget (about 4 hours total)

| Step | Budget |
| --- | --- |
| Pass A: Supply and withdraw (Task 8 only) | 25 min |
| Pass B: Borrow, repay | 50 min |
| Pass C: Health, liquidation, recovery | 60 min |
| Pass D: Disclosure, SDK, minimal keeper | 40 min |
| Verification and bug-fix (V) | 60 min |
| Total | 3 h 55 min |

Tasks 15-18 (web app, devnet soak, security review, mainnet beta) do not fit in this window.
Tasks 15.2-15.6 can only start if the V step finishes early.
Tasks 16-18 need a multi-day soak and independent reviews, so they are outside this plan.

## Pass A: Supply and withdraw (original task 8; Task 7 done on the original list)

Pool accounting (original Task 7) is done on the original list and is not repeated here.

- [ ] A6. First-supplier rule (8.1): the canonical-empty check and `unsettled_recovery_claim_count`.
- [ ] A7. Supply and withdraw supply (8.2, 8.3): mint against encrypted NAV, zero on failure, atomic Position and Pool update.
- [ ] A-check. `cargo check` and `tsc --noEmit` pass.

Deferred to V or cut: 8.4, 8.5.

## Pass B: Collateral, borrow, repay (original tasks 9 and 10)

- [ ] B1. Normalized value and health math (9.1): one USD scale, floor for SOL collateral, ceil for USDC debt, LTV and Liquidation Threshold.
- [ ] B2. Collateral addition (9.2): allowed while frozen.
- [ ] B3. Borrow (9.3): fresh Pyth and band, interest checkpoint, all caps, health at the projected index, debt shares rounded against the borrower, and the `has_debt` and count transition.
- [ ] B4. Repay, full and partial (10.1): burn shares, update Pool Cash, and set the `has_debt` and count transition on full repayment.
- [ ] B5. Collateral withdrawal (10.2): post-withdraw LTV health, rejected while frozen, and debt-free full withdrawal.
- [ ] B-check. `cargo check` and `tsc --noEmit` pass.

Deferred to V or cut: 9.4, 9.5, 10.3, 10.4.

## Pass C: Health, liquidation, recovery (original tasks 11 and 12)

- [ ] C1. Health Intents (11.1, 11.2): non-locking, reveals only the unhealthy boolean, and freezes on a current-generation result.
- [ ] C2. `open_liquidation` (11.3): Liquidation Record keyed by Position and freeze epoch.
- [ ] C3. Liquidation parameter bounds and algebra (12.1).
- [ ] C4. Partial-liquidation circuit (12.2): lock three accounts, checkpoint, recheck health, bound the burn, require health improvement, return zero otherwise, and produce two encrypted Receipts.
- [ ] C5. Liquidation callback and record lifecycle (12.3).
- [ ] C6. Absorb and Recovery Claims (12.4): includes Pending Recovery Settlement, buyer escrow, and the one-time refund.
- [ ] C-check. `cargo check` and `tsc --noEmit` pass.

Deferred to V or cut: 11.4, 12.5, 12.6, 12.7. A health-improvement unit test remains in V.

## Pass D: Disclosure, SDK, minimal keeper (original tasks 13 and 14)

- [ ] D1. Disclosure accounts, circuit, and callback (13.1, 13.2).
- [ ] D2. Auditor CLI (13.3).
- [ ] D3. SDK action builders for every v1 instruction, Pyth update path, and typed errors (14.1, 14.2).
- [ ] D4. Minimal keeper (14.3): index, band, and health scheduler, Liquidation Record creation, and Intent/Lock recovery with rent reclaim.
- [ ] D-check. `cargo check` and `tsc --noEmit` pass.

Deferred to V or cut: 13.4, 14.4 (Prometheus and Docker), 14.5 (capacity stress test). Basic JSON logging stays in D4.

## V: Verification and bug-fix (one pass over everything)

- [ ] V1. Full build, then clippy and rustfmt once.
- [ ] V2. Reference-model differential tests: fixed-point, rate, checkpoint, supply, borrow, repay, liquidation. Fix failures.
- [ ] V3. One integration test per feature on the local Arcium setup. Include the failure path (zero on failure) for supply, borrow, and liquidation.
- [ ] V4. One devnet demo script covering: shield, supply, borrow, accrue, partial repay, freeze, liquidate, full repay, withdraw.
- [ ] V5. Mark the original `tasks.md` items complete where verified. List what was deferred.

## Mapping to the original spec

| Original | Lean |
| --- | --- |
| 7.x | Done on the original list |
| 8.1-8.3 | A6-A7 |
| 8.4, 8.5 | V3, V4 |
| 9.1-9.3 | B1-B3 |
| 9.4, 9.5 | V3, V4 |
| 10.1, 10.2 | B4-B5 |
| 10.3, 10.4 | V3, V4 |
| 11.1-11.3 | C1-C2 |
| 11.4 | V3 |
| 12.1-12.4 | C3-C6 |
| 12.5-12.7 | V2, V3, V4 |
| 13.1-13.3 | D1-D2 |
| 13.4 | V3 |
| 14.1-14.3 | D3-D4 |
| 14.4, 14.5 | Cut |
