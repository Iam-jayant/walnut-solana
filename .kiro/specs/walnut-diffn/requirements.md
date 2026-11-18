# Requirements Document

## Introduction

Walnut-Diffn is a hackathon demo of private lending on Solana with Arcium MPC. It reuses the deployed Walnut program, Market, vaults, Position, shield and unshield from the main `walnut-solana` spec, and adds private collateral, borrow and repay. Balances live encrypted in each user's Position; only shield and unshield move public token amounts.

This spec is for testing only. It deliberately skips hardening: no interest, no shares or NAV, no liquidation, no oracle validation, no caps beyond what the circuit checks, and the SOL price is a public instruction argument. The main spec stays the source of truth for the production design.

## Glossary

- **Position**: the existing per-owner account holding `Enc<Shared, PositionSecret>` (free SOL, free USDC, collateral SOL, a spare field, debt USDC), sealed to the owner's Viewing Key.
- **Shield / Unshield**: the existing Task 5 instructions that move public tokens into and out of the vaults and credit or debit the encrypted free balances.
- **Position operation**: one of add collateral, withdraw collateral, borrow, repay, applied privately to the Position by one circuit.
- **Demo price**: the public SOL/USD price (USDC base units per SOL) passed by the caller.

## Requirements

### Requirement 1: Private position operations

**User Story:** As a borrower, I want to move funds between my free balance, collateral and debt privately, so that nobody can see my collateral, debt or borrowed amount.

#### Acceptance Criteria

1. WHEN the owner submits add collateral with an amount THEN the system SHALL move that amount from free SOL to collateral SOL inside the encrypted Position, or change nothing if free SOL is insufficient.
2. WHEN the owner submits borrow with an amount and the demo price THEN the system SHALL add the amount to both debt USDC and free USDC only if the new debt stays within the loan-to-value limit of the collateral, and SHALL change nothing otherwise.
3. WHEN the owner submits repay with an amount THEN the system SHALL subtract the amount from both free USDC and debt USDC only if both cover it, and SHALL change nothing otherwise.
4. WHEN the owner submits withdraw collateral with an amount and the demo price THEN the system SHALL move the amount from collateral SOL back to free SOL only if the remaining collateral still covers the debt within the loan-to-value limit, and SHALL change nothing otherwise.
5. THE system SHALL reveal only whether the operation applied; amounts, balances, collateral and debt SHALL stay encrypted.
6. THE system SHALL perform every position operation with one circuit whose cost is in the range of the existing shield and unshield circuits, with no division by a secret value.

### Requirement 2: Reuse of deployed components

**User Story:** As the demo operator, I want to reuse the existing devnet deployment, so that the demo runs within the remaining time and funds.

#### Acceptance Criteria

1. THE system SHALL reuse the deployed program id `5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea`, the existing Market, test USDC mint, vaults, Position initialization, shield, unshield and unwrap.
2. THE system SHALL keep all existing instructions, tests and specs unchanged in behavior.
3. WHEN a borrower unshields borrowed USDC THEN the system SHALL pay it from the USDC vault, which lenders fund by shielding USDC.

### Requirement 3: Demo and test

**User Story:** As a hackathon judge, I want one script that shows the full private loan flow on devnet, so that I can see it work end to end.

#### Acceptance Criteria

1. WHEN the demo runs THEN it SHALL shield SOL, add collateral, borrow USDC, unshield the borrowed USDC, repay, withdraw collateral and unshield SOL, decrypting the Position locally after each step.
2. WHEN a demo step's decrypted balances differ from the expected values THEN the demo SHALL fail with a clear message.
3. THE demo SHALL show one rejected over-limit borrow that leaves the Position unchanged.
4. THE system SHALL pass the existing build, lint and unit test checks after the change.