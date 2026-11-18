# Requirements Document

## Introduction

Walnut-Lend turns the working devnet private-lending product (`walnut-diffn`) into a lending market on Solana with Arcium MPC. It adds an on-chain Pyth price, enforced pause and caps, USDC supply and withdraw for lenders, interest through public indexes, health check and liquidation, Viewing Key rotation, selective disclosure to an auditor, and a landing page at "/" with the existing app moved to "/app". Per-position balances, collateral and debt stay encrypted. Operation amounts stay public. Baseline: program `5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea`, Arcium cluster 456, branch `v2`, `docs/DEVNET_LOAN_DEMO.md`.

## Glossary

- **Program**: the deployed Walnut Anchor program.
- **Market**: the existing public account holding configuration, pause flag, caps, indexes and pool totals.
- **Position**: the per-owner account holding `Enc<Shared, PositionSecret>`, sealed to the owner's Viewing Key.
- **Circuit**: an Arcium MPC computation over secret values.
- **Oracle**: the Pyth SOL/USD price account read on chain.
- **validate_price**: the existing price validation function in the Program.
- **Borrow_Index / Supply_Index**: public Market indexes (1.0 initial) that grow with accrued interest.
- **Debt_Shares**: the encrypted debt field of a Position, interpreted as shares; debt in USDC = shares × Borrow_Index.
- **Pool_Totals**: public Market totals of supplied USDC, borrowed USDC and reserves.
- **Utilization**: total borrowed divided by total supplied, computed publicly from Pool_Totals.
- **Reserve_Factor**: the share of borrower interest retained by the Market.
- **Unhealthy_Flag**: the single revealed bit that a Position's debt exceeds its liquidation threshold.
- **Liquidator**: any account that repays a public USDC amount of another Position's debt.
- **Liquidation_Bonus**: public percentage of extra SOL collateral paid to the Liquidator.
- **Viewing_Key**: the user key that seals a Position and is rotated by the owner.
- **Auditor_Key**: a public key chosen by the owner to receive disclosed fields.
- **Disclosure**: an encrypted copy of chosen Position fields sealed to the Auditor_Key.
- **App**: the Next.js front end. **Landing_Page**: the page at "/".

## Requirements

### Requirement 1: On-chain Pyth price

**User Story:** As a borrower, I want the SOL price read from Pyth on chain, so that nobody can supply a favourable price.

#### Acceptance Criteria

1. WHEN a borrow or withdraw-collateral instruction is submitted, THE Program SHALL read the SOL/USD price from the Oracle account and pass it to the Circuit instead of a caller-supplied price.
2. THE Program SHALL validate the Oracle price with validate_price, using the low bound (price minus confidence) for collateral valuation.
3. THE Program SHALL apply a maximum price age that is configurable in the Market and set to a devnet-tolerant value.
4. IF the Oracle account is not the one configured in the Market, THEN THE Program SHALL reject the instruction.
5. IF validate_price rejects the price, THEN THE Program SHALL reject the instruction and leave the Position unchanged.
6. THE Program SHALL remove the caller-supplied price argument from the instructions.

### Requirement 2: Enforced pause and caps

**User Story:** As a protocol operator, I want the existing pause and caps enforced, so that I can stop or limit activity.

#### Acceptance Criteria

1. WHILE the Market is paused, THE Program SHALL reject supply, borrow, add collateral and liquidation instructions.
2. WHILE the Market is paused, THE Program SHALL accept repay, withdraw supply, withdraw collateral and unshield instructions.
3. IF a supply would raise total supplied above the supply cap, THEN THE Program SHALL reject the instruction.
4. IF a borrow would raise total borrowed above the borrow cap, THEN THE Program SHALL reject the instruction.
5. IF a shield or add-collateral amount would exceed the per-asset cap in the Market, THEN THE Program SHALL reject the instruction.

### Requirement 3: Lender supply and withdraw of USDC

**User Story:** As a lender, I want to supply and withdraw USDC, so that I fund borrowers and earn interest.

#### Acceptance Criteria

1. WHEN a lender supplies a public USDC amount, THE Program SHALL transfer the USDC to the vault and credit encrypted supply shares equal to amount ÷ Supply_Index to the lender's Position.
2. WHEN a lender withdraws a public USDC amount, THE Program SHALL debit supply shares equal to amount ÷ Supply_Index and transfer the USDC from the vault to the lender.
3. IF the lender's supply shares do not cover the withdrawal, THEN THE Program SHALL leave the Position unchanged and transfer no USDC.
4. IF vault liquidity is below the withdrawal amount, THEN THE Program SHALL reject the instruction.
5. THE Program SHALL update the public supplied total in Pool_Totals on every supply and withdraw.

### Requirement 4: Interest accrual

**User Story:** As a lender, I want interest paid by borrowers, so that supplying USDC earns a return while positions stay private.

#### Acceptance Criteria

1. WHEN any lending instruction executes, THE Program SHALL first accrue interest, updating Borrow_Index, Supply_Index and Pool_Totals from the elapsed time since the last accrual.
2. THE Program SHALL compute the borrow rate from Utilization using the existing kinked rate model.
3. THE Program SHALL increase Pool_Totals borrowed by the accrued borrower interest and credit that interest to suppliers through Supply_Index, minus the Reserve_Factor share credited to reserves.
4. THE Program SHALL keep per-position supply shares, collateral, Debt_Shares and free balances encrypted.
5. THE Program SHALL treat the debt field of an existing Position holding plain USDC debt as Debt_Shares at Borrow_Index 1.0.
6. WHEN a borrower borrows or repays a public USDC amount, THE Program SHALL add or subtract amount ÷ Borrow_Index in Debt_Shares and update Pool_Totals borrowed.

### Requirement 5: Health check and liquidation

**User Story:** As a liquidator, I want to repay an unhealthy position's debt and receive discounted collateral, so that the market stays solvent.

#### Acceptance Criteria

1. WHEN a health check is requested for a Position, THE Circuit SHALL reveal only the Unhealthy_Flag, using the Oracle low-bound price and the current Borrow_Index.
2. WHEN a Liquidator submits a liquidation with a public USDC repay amount and the Unhealthy_Flag is set, THE Program SHALL transfer the USDC from the Liquidator to the vault and reduce the target Position's Debt_Shares by amount ÷ Borrow_Index.
3. WHEN a liquidation applies, THE Program SHALL compute the SOL collateral payout as repay amount × (1 + Liquidation_Bonus) ÷ price using public values and transfer it from the SOL vault to the Liquidator.
4. THE Circuit SHALL reduce the target Position's collateral SOL by the payout amount.
5. IF the Position is healthy, THEN THE Program SHALL refuse the liquidation, transfer nothing and leave the Position unchanged.
6. IF the payout exceeds the Position's collateral, THEN THE Circuit SHALL leave the Position unchanged and THE Program SHALL return the Liquidator's USDC.

### Requirement 6: Viewing Key rotation

**User Story:** As a user, I want to rotate my Viewing Key, so that a leaked key stops exposing my Position.

#### Acceptance Criteria

1. WHEN the owner requests rotation in the App, THE App SHALL generate a new Viewing Key and submit the existing re-seal instruction so the Position is sealed to the new key.
2. WHEN rotation completes, THE App SHALL decrypt the Position with the new Viewing Key and display the same balances as before.
3. WHEN rotation completes, THE App SHALL replace the stored Viewing Key with the new one.
4. IF rotation fails, THEN THE App SHALL keep the previous Viewing Key and display an error.
5. THE App SHALL keep the existing Viewing Key relock behaviour unchanged.

### Requirement 7: Selective disclosure to an auditor

**User Story:** As a user, I want to share chosen Position fields with an auditor, so that I can prove my state without revealing everything.

#### Acceptance Criteria

1. WHEN the owner selects fields and an Auditor_Key in the App, THE Program SHALL store a Disclosure containing only the selected Position fields sealed to the Auditor_Key.
2. THE Circuit SHALL produce the Disclosure without changing the Position.
3. WHEN the owner submits a new Disclosure for the same Auditor_Key, THE Program SHALL replace the previous Disclosure.
4. WHEN an auditor supplies the Auditor_Key secret in the local auditor view, THE App SHALL decrypt and display the disclosed fields.
5. IF a field was not selected, THEN THE Disclosure SHALL contain no value for that field.

### Requirement 8: Landing page and app routing

**User Story:** As a visitor, I want a landing page describing Walnut on Solana, so that I understand the product before opening the app.

#### Acceptance Criteria

1. THE Landing_Page SHALL be served at "/" and ported from the EVM reference `d:\solana\walnut` (`components/landing/*`, `app/page.tsx`).
2. THE Landing_Page SHALL describe Solana and Arcium MPC and SHALL NOT mention FHE.
3. THE App SHALL serve the existing application at "/app", with all internal links, navigation and redirects updated.
4. THE Landing_Page SHALL link to "/app" for launching the application.
5. THE App SHALL show no text describing reduced scope, demo limitations or test-only status.

### Requirement 9: Circuit constraints and reuse

**User Story:** As a developer, I want the changes to fit Arcium limits and keep existing flows, so that the deployment stays feasible.

#### Acceptance Criteria

1. THE Circuit SHALL use only multiplication and comparison on secret values, with no division by a secret value.
2. THE Circuit SHALL stay within Arcium's CU limit, using position_op at 5.3e8 CU as the reference.
3. THE Program SHALL add as few new circuits as the scope allows, reusing existing circuits where possible.
4. THE Program SHALL keep the existing shield, unshield, deposit, withdraw, collateral, borrow and repay flows working.
5. THE Program SHALL keep the program id, cluster 456 and branch `v2`, and ship as a single devnet deployment at the end.
6. THE Program SHALL leave amounts of supply, withdraw, borrow, repay and liquidation operations public.

### Requirement 10: Final verification

**User Story:** As the project owner, I want one verification pass, so that I know the product works end to end.

#### Acceptance Criteria

1. THE project SHALL pass the build, lint and unit tests.
2. WHEN the devnet end-to-end run executes, THE run SHALL perform supply, borrow, interest accrual, repay, withdraw supply, health check, liquidation, Viewing Key rotation and disclosure.
3. IF a step's decrypted or public values differ from the expected values, THEN THE run SHALL fail with a message naming the step.
4. THE run SHALL show interest accrued to the lender as a Supply_Index increase and a larger USDC withdrawal than the supplied amount.
5. THE run SHALL show a liquidation refused on a healthy Position.
