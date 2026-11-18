# Implementation Plan: Walnut-Diffn

## Overview

Build the private loan flow in three code tasks and one run task. No intermediate full test runs; each code task only compile-checks. The last task runs the full checks and the devnet demo.

## Task Dependency Graph

```json
{
  "waves": [
    { "id": 0, "tasks": ["1"] },
    { "id": 1, "tasks": ["2"] },
    { "id": 2, "tasks": ["3"] },
    { "id": 3, "tasks": ["4"] },
    { "id": 4, "tasks": ["5"] },
    { "id": 5, "tasks": ["6"] }
  ]
}
```
## Tasks

- [x] 1. Add the `position_op` circuit and program instructions
  - Add `position_op` to `encrypted-ixs/src/lib.rs` following `unshield`, with the four ops from the design.
  - Add `init_position_op_comp_def`, `position_op`, `position_op_callback`, `abandon_position_op` and their account structs to `programs/walnut/src/lib.rs`, following the unshield queue, callback and abandon pattern.
  - Add `OperationKind::PositionOp`, events, and append-only errors; add a plain-Rust mirror of the rules with unit tests.
  - Compile-check only with `cargo check`.
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 2.2_

- [x] 2. Add SDK helpers and the devnet loan demo
  - Add `packages/sdk/src/position-op.ts` with the op enum, the client-side rule mirror and the instruction builder, exported from `index.ts`, with a unit test.
  - Add `packages/sdk/demo/loan-demo.ts` and the `demo:loan` script, reusing the balance demo state and helpers.
  - Typecheck the SDK only.
  - _Requirements: 2.1, 2.3, 3.1, 3.2, 3.3_

- [x] 3. Deploy and run the demo on devnet
  - Reclaim devnet rent held by the unused `update_utilization_band` computation definition if funds are short.
  - Build, extend and upgrade the program, register the `position_op` computation definition, and run `demo:loan`.
  - Record the step table in `docs/DEVNET_LOAN_DEMO.md`.
  - _Requirements: 2.1, 3.1, 3.2, 3.3_

- [x] 4. Run the final checks
  - Run build, fmt, clippy, Rust unit tests, typecheck, lint, format check and JS unit tests, and fix failures.
  - _Requirements: 3.4_

- [x] 5. Build the Web App on devnet
  - Build the Next.js app in `web/` with wallet connect (Phantom, Solflare, Backpack via Wallet Standard), Viewing Key unlock by wallet signature, and local decryption of the Position.
  - Add Position setup, deposit and withdraw (shield and unshield for SOL and USDC, with the wSOL unwrap), add and withdraw collateral, borrow and repay, each with live MPC status.
  - Add `scripts/diffn-liquidity.sh` that mints test USDC into the market vault for borrowers.
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5, 2.1, 2.3_

- [x] 6. Check and push the Web App
  - Run the web build, typecheck, lint and format check plus the existing JS checks, and fix failures.
  - Commit to the `v2` branch and push.
  - _Requirements: 3.4_
## Notes

- Testing-only demo spec; production rules stay in the walnut-solana spec.
- Code tasks compile-check only; the full verification runs once in task 4.
