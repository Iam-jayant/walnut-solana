# Requirements Document

## Introduction

### Purpose and launch scope

Walnut-Solana is a confidential, over-collateralized lending market on Solana. It uses Arcium MPC for encrypted state transitions. It does not use FHE and SHALL NOT be marketed as an FHE protocol.

The production v1 market is deliberately narrow:

- users may shield and unshield SOL and USDC;
- lenders may supply USDC;
- borrowers may post SOL collateral and borrow USDC;
- individual balances, collateral, supply shares, debt shares, and liquidation amounts remain encrypted;
- users decrypt their own Position locally;
- health checks and partial liquidations execute through Arcium;
- users may selectively disclose a signed snapshot to an auditor;
- wallet addresses, Position addresses, action timing, shield and unshield amounts, and protocol account balances remain public.

The following features are deferred until v1 has operated safely under capped limits:

- encrypted-amount transfers between users;
- repayment or credit tiers;
- borrowing SOL;
- USDC or mixed collateral;
- sealed-bid whole-position auctions;
- P2P lending;
- multi-wallet identity aggregation;
- Token-2022 Confidential Transfers;
- under-collateralized lending.

This reduced scope is a safety decision. It removes cross-asset debt, externally initiated Position writes, farmable credit scoring, and an auction in which bidders cannot rationally price an unknown-size lot.

### Non-negotiable launch policy

1. Devnet is the first public deployment.
2. Mainnet deployment is forbidden until every Mainnet Gate in Requirement 16 passes.
3. Mainnet starts as a capped beta. Supply, borrow, Position-count, and debt-Position-count caps SHALL be enforced on-chain.
4. “Production-grade”, “audited”, “first on Solana”, “fully private”, “anonymous”, “FHE”, and “compliance-ready” SHALL NOT appear in public copy unless a dated evidence record supports the exact claim.
5. The product SHALL describe itself as using Arcium MPC and encrypted Position amounts.
6. No specification, test count, or audit SHALL be presented as a guarantee that losses are impossible.

### Cost and infrastructure policy

Walnut SHALL use open-source or freely usable software wherever possible and SHALL have no mandatory paid SaaS dependency.

The repository SHALL support:

- a public or self-hosted Solana RPC selected by configuration;
- a self-hosted Docker keeper;
- static or free-tier web hosting;
- public Pyth Hermes access;
- local Arcium development tooling;
- optional paid RPC and hosting providers without coupling protocol correctness to them.

Mainnet is not zero-cost. The documentation SHALL state that Solana fees, account rent, Arcium computation fees, deployment fees, keeper fee-payer funds, security reviews, and market liquidity cannot be eliminated by architecture. Users SHALL pay their own action and computation fees. Protocol maintenance transactions SHALL be funded from an explicitly monitored keeper wallet.

## Glossary

- **Asset State:** Public per-market state containing configuration, Borrow Index, rate epoch, caps, and pause controls.
- **Borrow Index:** Public WAD-scaled index converting encrypted Debt Shares into USDC debt.
- **Callback:** The later Solana instruction that verifies and applies an Arcium computation result.
- **Decision Expiry:** The latest slot or timestamp at which a price- or health-sensitive computation may commit.
- **Encrypted Position State:** One encrypted structure containing free SOL, free USDC, SOL collateral, USDC Supply Shares, and USDC Debt Shares.
- **Generation:** A monotonically increasing account revision. A callback may write only the generations captured by its Operation Intent.
- **Health Factor:** Liquidation-threshold-weighted SOL collateral value divided by USDC debt value.
- **Liquidation Record:** Public lifecycle metadata for one frozen Position. It contains no private amount.
- **Operation Intent:** A pre-created PDA recording one queued computation, its dependency snapshots, account and input commitments, expiry, and terminal status.
- **Pending Shield:** A unique PDA recording tokens deposited before their encrypted credit has completed, including the currently active credit attempt.
- **Pool State:** The encrypted USDC pool accounting state: cash, total Supply Shares, total Debt Shares, and Reserves, plus public revision metadata.
- **Position:** The owner-bound PDA containing encrypted state, Viewing Key epoch and public key, generation, public `has_debt`, freeze epoch, and lifecycle flags.
- **Recovery Claim:** A PDA binding collateral recovered during Absorb to the supplier loss and Reserve replenishment that its sale proceeds must satisfy.
- **Recovery Position:** A protocol-owned encrypted Position receiving only collateral legitimately allocated during bad-debt recovery. Its proceeds remain claim-bound until the related Recovery Claim is satisfied.
- **Viewing Key:** An x25519 key derived client-side from a domain-separated wallet signature that includes a Position-specific key epoch.

## Requirements

### Requirement 1: Repository, toolchain, CI, and documentation

**User Story:** As a maintainer, I want a pinned, reproducible repository with CI and baseline documents, so that every contributor builds and verifies the same system.


#### Acceptance Criteria

1. THE Repository SHALL contain `programs/walnut`, `encrypted-ixs`, `packages/sdk`, `packages/reference-model`, `keeper`, `web`, `tests`, `config/networks`, `scripts`, and `docs`.
2. THE Repository SHALL pin exact compatible Rust, Solana CLI, Anchor, Arcium CLI/crates/TypeScript packages, Node, and pnpm versions.
3. All Arcium components SHALL use one coordinated release. The setup task SHALL verify the current compatibility matrix rather than trusting a stale version comment.
4. CI SHALL run build, clippy with warnings denied, rustfmt, TypeScript type-check, eslint, prettier, unit tests, property tests, Arcium integration tests, dependency audits, secret scanning, and fixture-import checks.
5. High or critical advisories SHALL fail CI unless a committed exception names the advisory, justification, owner, and unexpired expiry date.
6. Secrets, keypairs, `.env` files, build output, ledgers, and node modules SHALL be ignored.
7. The Repository SHALL contain `SPEC.md`, `ARCHITECTURE.md`, `PRIVACY_MODEL.md`, `THREAT_MODEL.md`, `OPERATIONS.md`, `MAINNET_CHECKLIST.md`, and architecture decision records.
8. Production code SHALL contain no mock settlement, fake transaction, fixture import, or simulated success path.

### Requirement 2: Mandatory Arcium feasibility gate

**User Story:** As a protocol engineer, I want a mandatory Arcium feasibility gate, so that no financial code is built on an unproven encrypted-state model.


No financial instruction beyond a test counter may be implemented until this requirement passes and an ADR records the evidence.

#### Acceptance Criteria

1. A compiled spike SHALL prove the exact account-backed encrypted-input API supported by the pinned Arcium release.
2. The spike SHALL store, reread, update, and locally decrypt the full Encrypted Position State.
3. The spike SHALL test two owners and cross-owner computation output even though user transfers are deferred, because liquidation writes two owners.
4. Every effective encryption-key identity and nonce SHALL be recorded during randomized updates. No key/nonce pair may repeat.
5. If one `Enc<Shared>` Position structure cannot safely and efficiently persist, the ADR SHALL adopt `Enc<Mxe>` state plus an owner-authorized view computation before financial implementation begins.
6. The spike SHALL exercise success, circuit failure, authenticated static failure callback, invalid output, callback failure, compute exhaustion, `OutputTooLarge`, queued expiry, late callback, duplicate callback, and abandonment.
7. The spike SHALL serialize the actual planned queue and callback transactions for supply, borrow, repay, unshield, liquidation, disclosure, and key rotation. Every transaction SHALL be below 1,232 bytes, with a target ceiling of 1,150 bytes.
8. The spike SHALL measure p50 and p95 queue-to-callback latency, callback compute units, fees, failure rate, and sustained operations per minute.
9. The spike SHALL prove finalized-rent reclamation and expired-computation-fee reclamation.
10. Failure of any gate SHALL cause the design and requirements to be updated before Task 3. It SHALL NOT be bypassed to preserve schedule.

### Requirement 3: Generations, Operation Intents, locks, and recovery

**User Story:** As a protocol engineer, I want generations, Operation Intents, locks, and recovery paths, so that stale or duplicate callbacks cannot change financial state.


#### Acceptance Criteria

1. Every queued computation SHALL create or initialize a unique Operation Intent PDA keyed by operation owner and sequence or computation offset.
2. An Operation Intent SHALL store operation type, computation offset, relevant account generations, Asset State generation, accounting Borrow Index and timestamp, risk Borrow Index projected to Decision Expiry, rate-epoch generation, oracle publish times where applicable, queue slot, Decision Expiry, payer, and status.
3. An Operation Intent SHALL bind canonical domain-separated `accounts_commitment` and `inputs_commitment` hashes. The first covers every ordered account, role, expected owner, mint, authority, writability flag, and generation. The second covers immutable public and encrypted inputs, ciphertext metadata, requested amounts, destinations, feed observations, indexes, key epochs, and operation-specific PDAs.
4. Status SHALL transition exactly once from `Queued` to one of:
   - `Applied`: authenticated success, dependencies match, and the approved transition is nonzero;
   - `Zero`: authenticated success, dependencies match, and no financial value is approved;
   - `Failed`: authenticated static failure callback;
   - `Stale`: authenticated success whose captured dependency or commitment no longer matches;
   - `Expired`: authenticated success arriving after Decision Expiry while still queued;
   - `Abandoned`: permissionless timeout recovery after Lock expiry without a terminal callback.
5. A callback SHALL verify the Arcium output, Operation Intent, computation offset, both commitments, account generations, index fields, rate epoch, and Decision Expiry before any financial write.
6. A stale, expired, duplicate, late, mismatched, or already-terminal callback SHALL change no financial state. Invalid or unauthenticated output SHALL leave the Intent queued for timeout recovery.
7. A computation that may update encrypted balances or custody state SHALL acquire every affected application Lock. A Health Intent SHALL not lock the Position and may update only public freeze metadata after verifying generation and expiry.
8. Lock expiry permits abandonment, not an assertion that the old computation stopped executing. Abandonment SHALL increment every still-matching protected generation before another writer can commit.
9. Authenticated static failure callbacks SHALL finalize known computation failures. Invalid callback output SHALL never authorize cleanup or a financial write.
10. Recovery instructions SHALL be permissionless where they cannot redirect value. Value refunds SHALL always go to the recorded owner or source account.
11. All terminal paths SHALL define computation-rent and fee-reclaim ownership.

### Requirement 4: Market, governance, and emergency controls

**User Story:** As a protocol operator, I want governed markets with timelocks, caps, and emergency controls, so that risk stays bounded and rescue actions stay available.


#### Acceptance Criteria

1. Initialization SHALL create one Market, one Asset State for the SOL/USDC market, one USDC Pool State, wSOL and USDC vault token accounts, and a Recovery Position.
2. A second initialization SHALL fail.
3. Admin transfer SHALL use propose and accept steps.
4. Mainnet SHALL separate upgrade, risk, treasury, and guardian authorities. Upgrade and risk authorities SHALL be Squads multisigs or equivalent on-chain multisigs.
5. Adverse risk changes and upgrades SHALL have an enforceable timelock and cancellation path. Emergency pauses may be immediate.
6. Parameter changes SHALL increment the Asset State generation, emit old and new values, and invalidate incompatible pending computations.
7. Pause controls SHALL distinguish new supply, supply withdrawal, new borrow, repay, collateral addition, collateral withdrawal, shield, unshield, liquidation start, and liquidation completion.
8. Repay, collateral addition, callback completion, Pending Shield recovery, and safe lock/intent recovery SHALL remain available in emergency mode unless executing them would directly violate custody.
9. Guardian pauses SHALL expire unless renewed by governance. Only governance may extend a long-duration pause.
10. On-chain caps SHALL include:
   - `supply_cap_usdc`, checked against encrypted supplier equity after the proposed supply;
   - `total_debt_cap_usdc`, checked against total debt at the projected risk index;
   - `position_debt_cap_usdc`, checked against Position debt at the projected risk index;
   - public Position and debt-Position count caps.
   Passive interest MAY move an existing market above a supply or debt cap, but new risk-increasing actions SHALL then return zero.
11. The Market SHALL maintain public `position_count`, `debt_position_count`, and `unsettled_recovery_claim_count`; each Position SHALL expose only a public `has_debt` boolean, never its amount. Counts SHALL update only on authenticated lifecycle transitions.
12. Risk bounds SHALL include LTV, Liquidation Threshold, target Health Factor, liquidation bonus, close factor, reserve factor, oracle age/confidence, rate parameters, and Decision Expiry.
13. For every allowed liquidation configuration, `liquidation_threshold × (1 + liquidation_bonus) < 1` and `target_health_factor > liquidation_threshold × (1 + liquidation_bonus)` SHALL hold with a documented rounding margin.

### Requirement 5: Positions, shield, unshield, and Viewing Keys

**User Story:** As a user, I want to create a Position, shield and unshield assets, and manage my Viewing Key, so that I can hold encrypted balances that only I can read.


#### Acceptance Criteria

1. `init_position` SHALL be owner-signed and create a Position in `Initializing` status before queueing encrypted zero state.
2. Failed or expired initialization SHALL be retryable or closable by the owner. A failed initialization SHALL not permanently consume the Position PDA.
3. A Position SHALL contain one Encrypted Position State or the fallback layout approved by the Requirement 2 ADR.
4. Each shield SHALL create a unique Pending Shield PDA containing owner, asset, source, amount, sequence, status, attempt sequence, active Intent, active computation offset, and captured Position generation.
5. Shield SHALL first transfer the exact public amount into the vault, then queue its encrypted credit.
6. A Pending Shield SHALL transition exactly once from `Deposited` to `Credited` or `Refunded`. A retry SHALL require `Deposited`, require the previous attempt to be terminal, increment the attempt sequence, and install a new active Intent and computation offset. Refund SHALL require no queued active attempt. A callback may credit only the matching active attempt.
7. Unshield SHALL reveal the approved amount because the outgoing SPL transfer is public. Insufficient encrypted balance SHALL approve zero.
8. SOL unshield SHALL first deliver wSOL. Native SOL unwrap SHALL be a separate owner-signed transaction unless the Requirement 2 spike proves a safe PDA-controlled alternative.
9. Routine Position reads SHALL decrypt locally and SHALL not contact a Walnut server.
10. A Position SHALL store `viewing_key_epoch`, starting at zero. Viewing Key derivation SHALL bind program ID, network, Position PDA, and epoch.
11. Key rotation SHALL require the Position owner’s signature and SHALL bind the Position, old and new key epochs, old and new public keys, program ID, network, sequence, and expiry. Successful callback SHALL re-encrypt state and increment the epoch atomically.
12. The wallet message signature and Viewing secret SHALL remain client-side and SHALL never be submitted on-chain.
13. Wallet-loss recovery SHALL not be described as ordinary key rotation. Any recovery scheme SHALL receive a separate threat model and mainnet review.
14. Viewing secrets SHALL remain in browser memory only. The Web App SHALL use a strict CSP and SHALL load no third-party analytics or scripts while a Viewing Key is unlocked.

### Requirement 6: Pyth oracle and degraded modes

**User Story:** As a risk manager, I want validated Pyth prices with degraded modes, so that price-sensitive operations fail closed on bad data.


#### Acceptance Criteria

1. Price-sensitive operations SHALL use Pyth `PriceUpdateV2` accounts with verified account owner and the configured SOL/USD and USDC/USD feed IDs.
2. Price age SHALL default to 60 seconds and be bounded from 5 to 300 seconds.
3. Confidence SHALL default to at most 2% of price and be bounded from 0.1% to 10%.
4. Prices SHALL be positive and SHALL fit documented mantissa, exponent, normalization, and u128 arithmetic domains.
5. Collateral SHALL use price minus confidence. Debt SHALL use price plus confidence.
6. The oracle module SHALL support configurable spot-versus-EMA deviation and last-good-price circuit breakers.
7. Entering oracle anomaly mode SHALL increment the oracle/configuration generation. New health freezes, liquidation openings, partial-liquidation requests, collateral withdrawals, borrows, and Absorb requests SHALL be blocked.
8. Pending price-sensitive callbacks bound to a prior oracle generation SHALL finalize `Stale` without financial writes. Repayment and collateral addition SHALL remain available.
9. Liquidation SHALL resume only after both feeds pass owner, feed, age, confidence, positivity, exponent, and spot-versus-EMA checks in a fresh observation.
10. The SDK SHALL support both same-transaction and post-then-use Pyth update flows. The supported production path SHALL be selected from measured transaction sizes.
11. Every price-sensitive Operation Intent SHALL bind oracle publish times, oracle generation, and a Decision Expiry. An expired result SHALL not commit.

### Requirement 7: Pool accounting and interest

**User Story:** As a supplier, I want encrypted Pool accounting and interest, so that my share of the Pool is priced correctly without exposing exact aggregates.


#### Acceptance Criteria

1. The USDC Pool State SHALL encrypt Pool Cash, total Supply Shares, total Debt Shares, and Reserves.
2. The Pool State SHALL store a public generation and the Borrow Index through which encrypted Reserves were last accounted.
3. The public accounting Borrow Index SHALL start at WAD and SHALL never decrease.
4. Operation Intents SHALL distinguish `accounting_borrow_index` and timestamp from `risk_borrow_index_at_expiry`. Share minting, burning, Cash, and interest checkpointing SHALL use only the accounting index. Health and cap checks SHALL use only the projected risk index.
5. The borrow rate SHALL use a bounded kinked model and the latest public utilization band, not an exact individual or pool amount.
6. Utilization SHALL be revealed only as a configured coarse band. Exact Pool Cash, total Supply Shares, and total Debt Shares SHALL remain encrypted in v1.
7. `update_utilization_band` SHALL compute utilization from encrypted Cash and current debt, reveal only the band index, and write it only if captured Pool generation, accounting index, and rate epoch still match. Zero denominator SHALL map to the lowest band.
8. A rate epoch SHALL have a maximum duration. Risk-increasing operations SHALL return zero when the utilization band is too old; low operation count SHALL not keep rates stale indefinitely.
9. Before any computation that mutates Pool Cash, Supply Shares, Debt Shares, or Reserves is queued, the program SHALL accrue the accounting Borrow Index and lock the Pool against another index or financial-state mutation until callback or abandonment. `update_utilization_band` is a read-only optimistic computation: it does not lock or checkpoint the Pool and may publish only if Pool generation and captured index/rate generations still match.
10. Every such computation, including supply, withdrawal, borrow, repay, reserve withdrawal, partial liquidation, and Absorb, SHALL first account borrower interest since the Pool’s last accounted Borrow Index and add the reserve-factor portion to encrypted Reserves.
11. Supplier equity SHALL equal `Pool Cash + current debt value - Reserves`, with checked arithmetic and documented rounding. No separate bad-debt-loss term SHALL be subtracted after debt shares are removed.
12. Supply shares SHALL be minted from current encrypted supplier equity, not a stale epoch Supply Index.
13. When total Supply Shares is zero, new supply SHALL move zero unless Cash, total Debt Shares, and Reserves are also zero and public `unsettled_recovery_claim_count` is zero.
14. Deposits or repayments that would mint or burn zero shares SHALL move zero value.
15. Reserve withdrawal SHALL use a public all-or-zero request. It SHALL checkpoint interest, approve only if the full request is no greater than encrypted Cash and Reserves, reduce both by that amount, and transfer exactly that public amount from the USDC vault to the configured treasury USDC account.
16. An informational Supply Index may be shown to users, but it SHALL NOT be the accounting authority unless equivalence to the encrypted NAV formula is proven.

### Requirement 8: USDC supply and withdrawal

**User Story:** As a lender, I want to supply and withdraw USDC against encrypted supplier equity, so that I earn interest without capturing earlier interest or losing value to rounding.


#### Acceptance Criteria

1. Supply SHALL accept an encrypted USDC amount and move value from encrypted Free USDC to encrypted Pool Cash.
2. When no Supply Shares exist, the circuit SHALL require Cash, Debt Shares, and Reserves to be zero and the program SHALL require public `unsettled_recovery_claim_count` to be zero, then mint at the documented initial scale. Otherwise `shares = floor(amount × total_supply_shares ÷ supplier_equity_before)`.
3. If balance is insufficient, a canonical-empty precondition fails, amount is below minimum, or calculated shares are zero, the operation SHALL move zero.
4. Withdrawal SHALL calculate `amount = floor(shares × supplier_equity_before ÷ total_supply_shares)`.
5. Withdrawal SHALL move zero unless the owner has the shares and Pool Cash covers the complete calculated amount.
6. Supply and withdrawal SHALL update Position and Pool generations atomically in one accepted callback.
7. No supplier may receive interest earned before their shares were minted.
8. Property tests SHALL cover first deposit, last withdrawal, reserve withdrawal, interest accrual, bad debt, dust, and all rounding boundaries.

### Requirement 9: SOL collateral and USDC borrowing

**User Story:** As a borrower, I want to post SOL collateral and borrow USDC, so that I can access liquidity while the Position stays healthy at conservative prices.


#### Acceptance Criteria

1. All risk math SHALL use one committed normalized USD scale. The shared functions SHALL include:
   - `sol_value_down(lamports) = floor(lamports × normalized_sol_price_low ÷ 10^9)`;
   - `usdc_value_up(units) = ceil(units × normalized_usdc_price_high ÷ 10^6)`;
   - `debt_units(shares, index) = ceil(shares × index ÷ WAD)`.
2. Posting collateral SHALL move an encrypted amount from Free SOL to SOL Collateral and SHALL move zero if balance is insufficient.
3. Borrow SHALL accept an encrypted USDC amount and use fresh conservative prices, accounting and projected risk Borrow Indexes, fixed market LTV, Pool Cash, caps, and encrypted Position state.
4. Borrow SHALL approve only when the amount meets the minimum, liquidity and caps cover it, and the resulting Position remains healthy at the projected risk index through Decision Expiry.
5. Debt Shares SHALL be booked at the accounting index and round against the borrower. Any booked-debt rounding surplus over USDC delivered SHALL be added to encrypted Reserves.
6. Borrow SHALL update encrypted Position and Pool state, public `has_debt`, `debt_position_count`, caps, and generations atomically. The circuit SHALL reveal only whether a zero-to-nonzero debt transition occurred, not the amount.
7. Frozen Positions and paused borrowing SHALL reject new borrow requests.
8. No public Debt Asset field is needed because v1 debt is always USDC.
9. Every approved borrow SHALL preserve the health invariant at the committed conservative prices and projected risk index.

### Requirement 10: Repay and collateral withdrawal

**User Story:** As a borrower, I want to repay debt and withdraw collateral, so that I can exit my Position and recover my assets safely.


#### Acceptance Criteria

1. Repay SHALL checkpoint Pool interest at the accounting Borrow Index before burning Debt Shares.
2. If the requested amount covers full debt, repay SHALL burn every Position Debt Share and charge exactly the rounded-up full debt.
3. For partial repayment, the computation SHALL choose the greatest share amount whose rounded-up USDC value at the accounting index does not exceed the request or Free USDC balance.
4. USDC transferred to Pool Cash SHALL equal the debt value removed at the accounting index, excluding documented protocol-favouring dust.
5. A zero-share repayment SHALL move zero value.
6. Full repayment SHALL set public `has_debt` false, decrement `debt_position_count`, unfreeze the Position, and mark any matching open Liquidation Record `Resolved` through a revealed nonzero-to-zero transition boolean.
7. Collateral withdrawal SHALL use fresh conservative prices and the risk Borrow Index projected through Decision Expiry.
8. Withdrawal SHALL move SOL to Free SOL only if the resulting Position remains healthy at the borrowing LTV.
9. Repayment and collateral addition SHALL be allowed while frozen only when no writer Lock is active. Expired liquidation writers SHALL be abandoned before rescue queues.
10. Collateral withdrawal SHALL not be allowed while frozen.
11. A debt-free Position SHALL be able to withdraw all collateral subject only to balance and pause rules.

### Requirement 11: Read-only health checks and freezing

**User Story:** As a keeper or user, I want non-locking health checks that can freeze unhealthy Positions, so that liquidation can start without blocking owner rescue.


#### Acceptance Criteria

1. Anyone MAY request a health check, but it SHALL create a Health Intent without locking the Position.
2. Health queueing SHALL fail while the Position has a writer Lock.
3. The Health Intent SHALL capture Position generation, prices, oracle publish time, and a Borrow Index projected through Decision Expiry.
4. The computation SHALL reveal only `unhealthy`.
5. The callback SHALL freeze the Position only if output verification succeeds, the Position has no active writer Lock, the Position generation is unchanged, and the Decision Expiry has not passed. A healthy-to-frozen transition SHALL increment public `freeze_epoch`, set `frozen_since_slot`, and increment Position generation.
6. A stale health result SHALL have no effect.
7. Health checks SHALL have per-caller and per-Position rate limits or bonds sufficient to prevent free spam.
8. Freezing SHALL create no account in the callback. A subsequent permissionless `open_liquidation` instruction SHALL create a Liquidation Record keyed by Position and `freeze_epoch`.
9. A frozen Position SHALL remain able to repay and add collateral after any active writer has completed or been abandoned.

### Requirement 12: Private partial liquidation and bad-debt recovery

**User Story:** As a liquidator, I want formulaic private partial liquidation with bad-debt recovery, so that unhealthy Positions are resolved fairly and losses are accounted once.


#### Acceptance Criteria

1. V1 SHALL use formulaic private partial liquidation. Whole-position sealed auctions are deferred.
2. A Liquidation Record SHALL be keyed by Position and `freeze_epoch` and store Position, freeze epoch, generation at open, frozen-since slot, opened slot, status (`Open`, `Resolved`, or `Absorbed`), and active Intent.
3. A liquidator SHALL submit an encrypted maximum USDC repayment from the liquidator’s own Free USDC.
4. Liquidation SHALL first checkpoint Pool interest at the accounting Borrow Index, then recheck borrower health with fresh Pyth prices and the projected risk Borrow Index. If healthy, it SHALL resolve the record, unfreeze, and apply zero liquidation.
5. Let normalized collateral value be `C`, projected debt value be `D`, liquidation-threshold ratio be `L`, bonus factor be `B`, and target Health Factor be `T`. Parameters SHALL satisfy `T > L × B`. The target repayment value SHALL be `max(0, ceil((T × D - L × C) ÷ (T - L × B)))` under the committed fixed-point scale.
6. Maximum collateral-supported repayment value SHALL be `floor(C ÷ B)` and SHALL be converted back to USDC units using the conservative USDC price before comparison with USDC request, balance, debt, or close-factor bounds.
7. The computation SHALL select the greatest Debt Share burn whose rounded payment at the accounting index satisfies requested repayment, liquidator balance, borrower debt, close factor, target repayment, and collateral-supported repayment bounds.
8. Seized SOL lamports SHALL be calculated from the accepted USDC payment, bonus, normalized prices, and SOL decimals with borrower-protecting floor rounding. If rounding cannot deliver the promised value without excess seizure, the operation SHALL return zero.
9. The circuit SHALL verify the post-operation health inequality rather than relying only on the algebraic bound.
10. The liquidator SHALL never pay more USDC than the SOL delivered is worth under the specified conservative prices and bonus formula.
11. Debt Shares removed and USDC added to Pool Cash SHALL use the accounting Borrow Index and the exact same rounded payment.
12. Collateral seizure SHALL be limited to the accepted payment plus bonus. Any excess collateral SHALL remain with the borrower.
13. If liquidation restores target health or clears debt, the Position SHALL unfreeze and the Liquidation Record SHALL become `Resolved`. Otherwise it SHALL remain frozen and `Open`.
14. Liquidation SHALL update public `has_debt` and `debt_position_count` only if debt reaches zero, using a revealed transition boolean.
15. The keeper SHALL advertise eligible Liquidation Records but SHALL NOT spend another user’s balance or finance liquidation.
16. Repay or collateral addition may queue after an expired liquidation Intent is abandoned. The late liquidation callback SHALL then be a no-op.
17. After `frozen_since_slot + recovery_delay`, `absorb` MAY remove residual Debt Shares. It SHALL require a matching open freeze epoch, checkpoint Pool interest, and recheck debt, health, and prices first.
18. Absorb SHALL allocate no more SOL collateral to the Recovery Position than residual debt plus a bounded recovery penalty. Excess SHALL remain with the borrower.
19. Absorb SHALL calculate `reserve_used = min(residual_debt, reserves)` and `supplier_loss = residual_debt - reserve_used`, then remove the Debt Shares and reduce Reserves only by `reserve_used`. No separate loss term SHALL be subtracted from supplier equity after debt removal.
20. The Absorb queue instruction SHALL pre-create a Recovery Claim with public borrower, freeze epoch, absorb Intent, and status metadata plus an encrypted payload containing claim-bound SOL, supplier-loss USDC remaining, Reserve-replenishment USDC remaining, and recovered USDC. The callback SHALL populate it only on accepted Absorb and increment `unsettled_recovery_claim_count`.
21. `settle_recovery_claim` SHALL use a pre-created Pending Recovery Settlement. Its queue transaction SHALL transfer a public USDC offer from the buyer into the USDC vault, record the public requested wSOL amount and destination, validate fresh Pyth prices and the bounded sale discount, and queue the computation.
22. On approval, the callback SHALL debit encrypted claim-bound SOL from the Recovery Position, transfer exactly the public wSOL amount to the recorded buyer destination, and update the encrypted Recovery Claim payload. It SHALL allocate escrowed USDC first to encrypted supplier loss by increasing Cash, then to encrypted Reserve replenishment by increasing Cash and Reserves equally. Only excess after both obligations reach zero may become treasury revenue. Fully settled claim transition SHALL decrement `unsettled_recovery_claim_count`.
23. On zero, failure, stale, expiry, or abandonment, the buyer SHALL be able to refund the escrowed USDC exactly once. All accounts, amounts, mints, authorities, attempt state, and destinations SHALL be bound by the Operation Intent.
24. Recovery settlement amounts are public, but remaining Recovery Claim amounts SHALL stay encrypted.
25. Liquidation Receipts SHALL be fixed-size owner-encrypted ciphertexts emitted in the authenticated callback event. No Receipt account is created in a callback.
26. Property tests SHALL prove health improvement, value conservation, exact debt/payment matching, no liquidator overcharge, no excess collateral seizure, no double-counted supplier loss, and Recovery Settlement credit/refund exclusivity.

### Requirement 13: Selective disclosure

**User Story:** As a Position owner, I want selective disclosure to an auditor, so that I can share a signed snapshot of chosen fields without exposing the rest.


#### Acceptance Criteria

1. Only the Position owner may create a Disclosure.
2. The Disclosure SHALL bind owner, Position, auditor public key, program ID, network, program version, Position generation, slot, timestamp, oracle feed IDs, oracle publish times, and Borrow Index.
3. The snapshot SHALL be re-encrypted to the auditor’s x25519 key and shall contain the owner-selected Position fields.
4. A third party without the auditor secret SHALL not decrypt the snapshot.
5. The UI SHALL state that on-chain ciphertext and metadata cannot be revoked and previously decrypted data cannot be taken back.
6. A later record MAY mark a Disclosure superseded, but SHALL not claim cryptographic revocation.
7. Documentation SHALL call this a user-authorized disclosure primitive, not automatic regulatory compliance.

### Requirement 14: SDK and browser security

**User Story:** As a developer, I want a TypeScript SDK with browser-safe key handling, so that every instruction can be built and tracked without leaking secrets.


#### Acceptance Criteria

1. One TypeScript SDK SHALL build every instruction and run in browsers and Node.
2. Network Profiles SHALL validate program ID, Arcium cluster, mints, vaults, Pyth feeds, RPC, commitment, and mainnet confirmation policy.
3. The SDK SHALL generate fresh encryption nonces and enforce the layout approved by the Requirement 2 ADR.
4. Viewing Key derivation SHALL use a fixed domain-separated message containing program ID, network, Position PDA, and `viewing_key_epoch`; epoch zero is used at initialization and rotation increments it.
5. The wallet message signature and derived secret SHALL remain local and SHALL never be placed in transaction data.
6. Wallet compatibility tests SHALL verify deterministic signatures for every supported wallet. Unsupported wallets SHALL receive a clear recovery warning.
7. The SDK SHALL track queue signature, computation account, callback result, Operation Intent status, rent claim, and fee reclaim.
8. Outcomes SHALL be `Applied`, `Zero`, `Failed`, `Stale`, `Expired`, `Abandoned`, or `TimedOut`.
9. Transaction builders SHALL check serialized size before sending and support versioned transactions, address lookup tables, compute budgets, and optional priority fees.
10. The SDK SHALL not depend on Helius, Vercel, or another paid provider. Provider adapters SHALL be configurable.
11. Program errors and privacy-relevant outcomes SHALL map to stable typed errors.

### Requirement 15: Keeper and Web App

**User Story:** As an operator and user, I want a self-hostable keeper and a transparent Web App, so that protocol maintenance runs reliably and users see what is public before signing.


#### Acceptance Criteria

1. The keeper SHALL run from a Docker image with no database dependency beyond local durable storage or an optional open-source database.
2. Keeper jobs SHALL include rate-epoch maintenance, Borrow Index accrual, health scheduling, Liquidation Record maintenance, expired Lock and Intent recovery, claims belonging to the keeper, and alerts.
3. The keeper SHALL never possess admin authority, user Viewing Keys, or authority over user funds.
4. Multiple keeper instances SHALL be safe. Expected races SHALL not corrupt state.
5. Scan rate SHALL be derived from the enforced debt-Position cap and the documented maximum detection-to-freeze time.
6. JSON logs, `/healthz`, and Prometheus-compatible metrics SHALL report job freshness, RPC failures, computation latency, failure rate, and fee-payer balance without secrets.
7. The Web App SHALL use Next.js App Router, TypeScript, Tailwind CSS v4, Solana wallet-adapter, and local Telegraf fonts.
8. The project owner has confirmed permission to use the Telegraf files and to replicate Zama's visual styling. The Web App SHALL reproduce zama.org's visual system (the palette #FFD209, #000000, #2D2D2D, #F4F4F4, #FFFFFF; type scale; spacing; layout grid; component shapes; and motion), recorded as measured tokens in `docs/design-tokens.md`. All text SHALL be Walnut's own: no headline, sentence, tagline, product claim, logo, illustration, or image SHALL be copied from Zama.
9. The Web App SHALL show each asynchronous state and every required recovery action.
10. Before signing, each form SHALL show fees and an action-specific privacy label listing public fields, encrypted fields, and permanent metadata.
11. The borrowing flow SHALL warn before signing that fallback recovery or future protocol versions may disclose additional data; v1 formulaic liquidation SHALL state exactly what it reveals.
12. The app SHALL pass type-check, lint, component tests, Playwright critical-flow tests, accessibility tests, reduced-motion checks, and mobile Lighthouse targets.
13. Hosting SHALL work on a free tier or static/self-hosted environment. Vercel is optional.
14. THE Web App SHALL include a landing page that follows zama.org's section structure and visual style, with Walnut's own copy, in this order: hero; a positioning statement; a live protocol section; use-case cards; a four-step how-it-works (shield, supply or borrow, private liquidation, selective disclosure); an Arcis code snippet from the Walnut repository; a call to action; and a footer.
15. THE landing page's live protocol section SHALL show only values that are public by protocol design and read from the chain: the current utilization band, the borrow rate, Position count, debt-Position count, and the configured caps. IF the RPC request fails or the market is not initialized, THEN it SHALL show an explicit unavailable state and never placeholder or invented numbers. It SHALL NOT display exact TVL, which v1 keeps encrypted.
16. THE landing page and docs SHALL follow the public-copy rules in the launch policy (no unsupported "production-grade", "audited", "first", "fully private", "anonymous", "FHE", or "compliance-ready" claims), SHALL describe the protocol as using Arcium MPC (not FHE), SHALL lay out correctly from 360 to 1,920 pixels wide, and SHALL meet the mobile Lighthouse targets.
17. THE Web App SHALL render docs pages from the Markdown files in the Repository's `docs` folder.

### Requirement 16: Devnet, mainnet, and operational gates

**User Story:** As a project owner, I want deployment scripts and gated launch criteria, so that mainnet launches only after every safety gate has passed.


#### Acceptance Criteria

1. Deployment scripts SHALL be idempotent and SHALL verify the program binary, program ID, Arcium cluster, computation definitions, mints, vaults, Pyth feeds, authorities, caps, and Network Profile.
2. Scripts SHALL refuse mainnet without an explicit confirmation containing the expected program ID and release hash.
3. Devnet tests SHALL execute every v1 action against the deployed program and web URL.
4. Mainnet SHALL use the identical reviewed code artifact as the final release candidate, with network-specific configuration and separately verified addresses.
5. Mainnet launch requires all of the following:
   - Requirement 2 ADR passed;
   - all CI and devnet end-to-end tests passed;
   - accounting reference model and differential tests passed;
   - adversarial callback, oracle, concurrency, and liquidation tests passed;
   - measured throughput supports the enforced caps and keeper SLA;
   - independent Solana/Anchor and Arcis circuit security review completed;
   - economic review of interest, rounding, caps, and liquidation completed;
   - upgrade, risk, and treasury multisigs configured;
   - timelocks and emergency runbooks tested;
   - Arcium Recovery Peers and cluster migration tested;
   - RPC failover and keeper restart tested;
   - legal review completed for intended jurisdictions and frontend operation;
   - public risk and privacy documentation published;
   - bug bounty or equivalent coordinated disclosure process active.
6. Mainnet SHALL begin with conservative caps set from measured operational capacity, not marketing targets.
7. Cap increases SHALL be timelocked and require a published operational review.

### Requirement 17: Security, privacy, and accounting invariants

**User Story:** As a security reviewer, I want explicit security, privacy, and accounting invariants, so that the system can be verified against concrete properties.


#### Acceptance Criteria

1. USDC vault conservation: `vault_usdc = Σ Position.free_usdc + Pool.cash_usdc + Σ deposited PendingShield USDC + explicitly segregated recovery USDC`.
2. wSOL vault conservation: `vault_wsol = Σ(Position.free_sol + Position.collateral_sol) + RecoveryPosition.free_sol + RecoveryPosition.collateral_sol + Σ deposited PendingShield SOL`.
3. Supply Shares, Debt Shares, and Reserves are claims or accounting values and SHALL NOT be counted as separate physical vault allocations. Recovery collateral SHALL be counted in the Recovery Position or a Recovery Claim, never both.
4. Supplier solvency: supplier equity SHALL equal Pool Cash plus current debt value minus Reserves. Removing bad Debt Shares and reducing Reserves SHALL express the loss exactly once.
5. Failed, zero, stale, expired, duplicate, and abandoned operations SHALL not create or redirect value.
6. A callback SHALL write only the generations and accounts bound by its Operation Intent commitments.
7. The accounting and projected risk Borrow Indexes SHALL be used only for their specified purposes; the accounting Borrow Index SHALL never decrease.
8. Supply shares SHALL not receive value earned before minting.
9. Debt value removed SHALL equal repayment value credited to Pool Cash at the accounting index.
10. Every permitted liquidation SHALL weakly improve health unless debt is cleared, and SHALL not overcharge or over-seize.
11. Admin reserve withdrawal SHALL transfer the exact approved public USDC amount and leave supplier equity unchanged.
12. No plaintext private amount SHALL appear in program logs or events except amounts explicitly public by protocol design, such as shield, unshield, reserve withdrawal, and Recovery Claim settlement.
13. Every account owner, PDA seed, signer, writability flag, token mint, vault, CPI program ID, Pyth feed, and callback account SHALL be validated.
14. All arithmetic SHALL be checked against a committed range table covering token amounts, prices, exponents, indexes, rates, time deltas, shares, and liquidation formulas.
15. The privacy model SHALL state that public `has_debt`, Position counts, debt-Position counts, freeze metadata, and recovery-settlement amounts are visible.
16. The privacy model SHALL state that MPC privacy assumes at least one honest Arcium cluster member under the documented model, while malicious or unavailable members can affect liveness.
17. The frontend threat model SHALL include malicious dependencies, compromised hosting, browser extensions, wallet incompatibility, RPC manipulation, and phishing.

### Requirement 18: Deferred work

**User Story:** As a product owner, I want deferred features recorded, so that nothing out of scope reaches production without a new specification and review.


The following require new specifications and security review before implementation:

#### Acceptance Criteria

- user-to-user encrypted transfers with inbox-based receipt claiming;
- repayment activity tiers or identity-backed credit;
- additional collateral and debt assets through isolated markets;
- sealed discount/capacity liquidation auctions;
- confidential token standards;
- P2P or under-collateralized loans;
- cross-protocol synchronous composability;
- wallet-loss recovery.

No deferred feature may be introduced through an undocumented production flag or mock branch.
