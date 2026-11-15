# Walnut-Solana privacy model

Status: baseline (Task 1.4). Source of truth: `.kiro/specs/walnut-solana/requirements.md` (Requirements 5, 7, 12, 13, 17) and `design.md`. The encrypted Position format is pending ADR-0003; see [Pending decision](#pending-decision-position-ciphertext-mode).

## Summary

Walnut uses **Arcium multi-party computation (MPC)**, not fully homomorphic encryption (FHE). Encrypted amounts are processed by an Arcium MPC cluster. No single cluster node sees plaintext, and Walnut runs no server that decrypts user data.

Walnut hides **amounts inside a Position and inside the Pool**. It does not hide **who** uses the protocol, **when**, or **which action** they take. Walnut is not anonymous and does not provide "full privacy". Anyone reading the chain can see wallet addresses, Position addresses, instruction types, timing, every SPL transfer into or out of the vaults, and the public lifecycle fields listed below.

## Assumptions

- **Arcium honesty.** Privacy holds if at least one member of the Arcium cluster is honest, under Arcium's documented dishonest-majority model. Malicious or unavailable members can delay or block computations (a liveness failure) even when they cannot read data or forge results.
- **Client security.** The Viewing Key is derived and held in the user's browser or SDK process. A compromised browser, extension, or frontend host can read it. See [THREAT_MODEL.md](THREAT_MODEL.md#frontend-and-key-handling).
- **Circuit correctness.** A circuit that reveals more than intended breaks the model. Circuits are in scope for independent review before mainnet.
- **No revocation.** Ciphertext written on-chain is permanent. If a key that can decrypt it later leaks, the data is exposed.

## Ciphertext modes

| Mode          | Who can decrypt                                    | Used for                                                     |
| ------------- | -------------------------------------------------- | ------------------------------------------------------------ |
| `Enc<Shared>` | Holder of the matching x25519 secret, plus the MXE | Position state (preferred layout), Receipts, Disclosures     |
| `Enc<Mxe>`    | Only the Arcium MXE cluster, jointly               | Pool State, Recovery Claim, Position state (fallback layout) |

### Pending decision: Position ciphertext mode

Task 2 must prove that one `Enc<Shared>` Position structure can be persisted, reread, updated, and decrypted locally without any encryption key/nonce pair repeating. If it cannot, ADR-0003 adopts `Enc<Mxe>` Position state plus an owner-authorized view computation that re-encrypts the state to the owner's Viewing Key on request. In both layouts the same fields are encrypted and the same fields are public. Only the mechanism by which the owner reads their own state changes.

## Account field tables

"Public" means readable by anyone from account data, transaction data, or events. "Encrypted" means stored only as ciphertext. Nonces and key metadata that accompany ciphertext are public.

### Market

| Field                                                                  | Visibility |
| ---------------------------------------------------------------------- | ---------- |
| Upgrade, admin/risk, treasury, and guardian authorities; pending admin | Public     |
| wSOL vault, USDC vault, treasury USDC account, Recovery Position       | Public     |
| Active Asset State                                                     | Public     |
| `position_count`, `debt_position_count`                                | Public     |
| `unsettled_recovery_claim_count`                                       | Public     |
| Position-count cap, debt-Position-count cap                            | Public     |
| Encrypted fields                                                       | None       |

### Asset State

| Field                                                                                                  | Visibility |
| ------------------------------------------------------------------------------------------------------ | ---------- |
| Configuration, oracle, and rate-epoch generations                                                      | Public     |
| Accounting Borrow Index and its timestamp                                                              | Public     |
| Current utilization band index and observation slot                                                    | Public     |
| Current borrow rate (derived from band)                                                                | Public     |
| LTV, Liquidation Threshold, target Health Factor, bonus, close factor, reserve factor, rate parameters | Public     |
| `supply_cap_usdc`, `total_debt_cap_usdc`, `position_debt_cap_usdc`                                     | Public     |
| Pyth feed IDs, max price age, max confidence, deviation bounds, anomaly status, last-good observations | Public     |
| Granular pause flags, pause expiries, timelock queue                                                   | Public     |
| Encrypted fields                                                                                       | None       |

### Pool State

| Field                             | Visibility             |
| --------------------------------- | ---------------------- |
| `generation`, `lock`              | Public                 |
| `accounted_borrow_index`          | Public                 |
| Ciphertext nonce and key metadata | Public                 |
| `cash_usdc`                       | Encrypted (`Enc<Mxe>`) |
| `total_supply_shares`             | Encrypted (`Enc<Mxe>`) |
| `total_debt_shares`               | Encrypted (`Enc<Mxe>`) |
| `reserves_usdc`                   | Encrypted (`Enc<Mxe>`) |

Exact Pool Cash, total Supply Shares, total Debt Shares, Reserves, and therefore exact TVL and exact utilization, are encrypted in v1. Only a coarse utilization band is revealed.

### Position (user)

| Field                                                             | Visibility                               |
| ----------------------------------------------------------------- | ---------------------------------------- |
| `owner` (wallet), Position PDA address                            | Public                                   |
| `viewing_pubkey`, `viewing_key_epoch`                             | Public                                   |
| `status` (for example `Initializing`, active)                     | Public                                   |
| `has_debt` (boolean only)                                         | Public                                   |
| `frozen`, `freeze_epoch`, `frozen_since_slot`                     | Public                                   |
| `generation`, `lock`, `next_operation_seq`, `next_disclosure_seq` | Public                                   |
| Ciphertext nonce and key metadata                                 | Public                                   |
| `free_sol`                                                        | Encrypted                                |
| `free_usdc`                                                       | Encrypted                                |
| `collateral_sol`                                                  | Encrypted                                |
| `supply_shares_usdc`                                              | Encrypted                                |
| `debt_shares_usdc`                                                | Encrypted                                |
| Health Factor, debt amount, collateral value                      | Not stored; derivable only by key holder |

### Recovery Position (protocol-owned)

Same layout as a user Position. Its address and public lifecycle fields are public. Its free SOL and collateral SOL are encrypted. The ciphertext mode is chosen in ADR-0003.

### Pending Shield

| Field                                                | Visibility |
| ---------------------------------------------------- | ---------- |
| `owner`, `position`, `asset`, `source_token_account` | Public     |
| `amount`                                             | Public     |
| `sequence`, `attempt_sequence`                       | Public     |
| `active_intent`, `active_computation_offset`         | Public     |
| `captured_position_generation`, `status`             | Public     |
| Encrypted fields                                     | None       |

The shield amount is public because the SPL transfer into the vault is public.

### Operation Intent

| Field                                                                             | Visibility                                                           |
| --------------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| `owner`, `payer`, `kind` (operation type)                                         | Public                                                               |
| `computation_offset`                                                              | Public                                                               |
| Position(s) and captured generations; Pool, Asset, oracle, rate-epoch generations | Public                                                               |
| Accounting Borrow Index and timestamp; risk Borrow Index at expiry                | Public                                                               |
| Oracle publish times                                                              | Public                                                               |
| `accounts_commitment`, `inputs_commitment` (hashes)                               | Public                                                               |
| `queued_slot`, `decision_expiry_slot`                                             | Public                                                               |
| `status` (`Queued`, `Applied`, `Zero`, `Failed`, `Stale`, `Expired`, `Abandoned`) | Public                                                               |
| Encrypted amount inputs (for example a borrow or supply amount)                   | Encrypted, in transaction data; only committed by hash in the Intent |

### Liquidation Record

| Field                                                             | Visibility                                |
| ----------------------------------------------------------------- | ----------------------------------------- |
| `position`, `freeze_epoch`                                        | Public                                    |
| `position_generation_at_open`, `frozen_since_slot`, `opened_slot` | Public                                    |
| `status` (`Open`, `Resolved`, `Absorbed`)                         | Public                                    |
| `active_intent`                                                   | Public                                    |
| Encrypted fields                                                  | None. The record holds no private amount. |

### Recovery Claim

| Field                                   | Visibility             |
| --------------------------------------- | ---------------------- |
| `borrower_position`, `freeze_epoch`     | Public                 |
| `absorb_intent`, `status`, `generation` | Public                 |
| `collateral_sol_remaining`              | Encrypted (`Enc<Mxe>`) |
| `supplier_loss_usdc_remaining`          | Encrypted (`Enc<Mxe>`) |
| `reserve_replenishment_usdc_remaining`  | Encrypted (`Enc<Mxe>`) |
| `recovered_usdc`                        | Encrypted (`Enc<Mxe>`) |

### Pending Recovery Settlement

| Field                                                       | Visibility |
| ----------------------------------------------------------- | ---------- |
| Buyer, buyer wSOL destination, Recovery Claim               | Public     |
| USDC offer amount (escrowed into the vault)                 | Public     |
| Requested wSOL amount                                       | Public     |
| Attempt sequence, active Intent, computation offset, status | Public     |
| Encrypted fields                                            | None       |

### Disclosure

| Field                                         | Visibility                                   |
| --------------------------------------------- | -------------------------------------------- |
| Owner, Position, auditor x25519 public key    | Public                                       |
| Program ID, program version, network          | Public                                       |
| Position generation, slot, timestamp          | Public                                       |
| Pyth feed IDs and publish times, Borrow Index | Public                                       |
| Selected-field mask and superseded marker     | Public                                       |
| Owner-selected Position fields                | Encrypted to the auditor key (`Enc<Shared>`) |

A Disclosure cannot be revoked. A later record can mark it superseded, but data already decrypted by the auditor cannot be taken back. Disclosure is a user-authorized disclosure primitive. It is not automatic regulatory compliance.

### Vault token accounts

The wSOL and USDC vaults are ordinary SPL token accounts. Their balances and every transfer in or out are public.

### Events and Receipts

| Item                                                              | Visibility                                          |
| ----------------------------------------------------------------- | --------------------------------------------------- |
| Intent outcome events (kind, status, accounts)                    | Public                                              |
| Shield, unshield, reserve-withdrawal, recovery-settlement amounts | Public                                              |
| Parameter-change events (old and new values)                      | Public                                              |
| Borrower and liquidator Receipts                                  | Encrypted to each party's current Viewing Key epoch |

Program logs and events contain no other plaintext amount.

## What each action reveals

| Action                     | Public                                                                                                        | Encrypted                                   |
| -------------------------- | ------------------------------------------------------------------------------------------------------------- | ------------------------------------------- |
| `init_position`            | Owner, Position, Viewing public key, epoch 0, `position_count` increment                                      | Initial zero state                          |
| Shield                     | Owner, asset, amount, source account, time                                                                    | Resulting free balance                      |
| Unshield                   | Owner, asset, approved amount, destination, time; zero approval reveals insufficient balance for that request | Remaining balance                           |
| Supply / withdraw supply   | Owner, action type, time, `Applied` vs `Zero`                                                                 | Amount, shares, remaining balances          |
| Post / withdraw collateral | Owner, action type, time, `Applied` vs `Zero`                                                                 | Amount, remaining collateral                |
| Borrow                     | Owner, time, `Applied` vs `Zero`, `has_debt` change from false to true, `debt_position_count`                 | Amount, Debt Shares, health                 |
| Repay                      | Owner, time, `Applied` vs `Zero`, `has_debt` change to false on full repayment, unfreeze                      | Amount, remaining debt                      |
| Health check               | Caller, target Position, one `unhealthy` bit (as a freeze), `freeze_epoch`, `frozen_since_slot`               | Collateral, debt, Health Factor             |
| Partial liquidation        | Liquidator, borrower Position, time, Record status, whether debt reached zero or target health was restored   | Repayment, seized SOL, remaining balances   |
| Absorb                     | That Absorb happened, Record `Absorbed`, `unsettled_recovery_claim_count`                                     | Residual debt, collateral moved, loss split |
| Recovery settlement        | Buyer, USDC offer, wSOL delivered, claim completion                                                           | Remaining claim amounts                     |
| Reserve withdrawal         | Requested amount, approval, treasury destination                                                              | Remaining Cash and Reserves                 |
| Utilization band update    | Band index                                                                                                    | Cash, debt, exact utilization               |
| Disclosure                 | Owner, auditor key, metadata, field mask                                                                      | Selected field values                       |
| Key rotation               | Old and new epochs, new Viewing public key                                                                    | Re-encrypted state                          |

## Known inference channels

Encrypting amounts does not stop all inference. Users and reviewers should assume the following are possible:

- **Flow tracing.** Shield and unshield amounts are public. If a user shields 1,000 USDC and later unshields 1,000 USDC with no activity in between, an observer can link the two.
- **Vault balances.** Public vault balances equal the sum of all free balances, Pool Cash, pending deposits, and segregated recovery funds. They bound aggregate activity even though Pool Cash itself is encrypted.
- **Outcome bits.** Whether an Intent ends `Applied` or `Zero` is public. A `Zero` borrow tells an observer that some condition (health, caps, liquidity, minimum size) failed for that request.
- **Lifecycle bits.** `has_debt`, freeze state, `debt_position_count`, and Liquidation Record status are public by design so that keepers can find debt Positions and liquidations can be opened.
- **Small anonymity set.** During the capped beta the number of Positions is small. Timing and count changes are easier to correlate.
- **Utilization band and rate.** The band and rate change when aggregate supply or debt moves. A large single action may visibly shift the band.
- **Metadata permanence.** Account addresses, Intent records, and events stay on-chain. Closing an account later does not erase history.

The Web App shows an action-specific privacy label listing the public fields, encrypted fields, and permanent metadata before every signature.
