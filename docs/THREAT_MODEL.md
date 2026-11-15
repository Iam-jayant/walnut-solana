# Walnut-Solana threat model

Status: baseline (Task 1.4). This document will be revised after the Task 2 feasibility gate (ADR-0003) and before the independent reviews required for mainnet. It lists threats the design addresses. It does not claim losses are impossible.

## Assets to protect

1. **Custody:** wSOL and USDC held in the program vaults.
2. **Accounting integrity:** encrypted Position, Pool, and Recovery Claim state; the public Borrow Index; public counts.
3. **Confidentiality:** the encrypted fields listed in [PRIVACY_MODEL.md](PRIVACY_MODEL.md).
4. **Liveness of rescue:** a borrower's ability to repay or add collateral, and a depositor's ability to recover a Pending Shield.
5. **Viewing Keys:** client-side x25519 secrets derived from wallet signatures.
6. **Governance authority:** upgrade, risk, treasury, and guardian keys.

## Trust assumptions

| Party                      | Trusted for                                                    | Not trusted for                                     |
| -------------------------- | -------------------------------------------------------------- | --------------------------------------------------- |
| Solana validators          | Ordering and finality                                          | Confidentiality (all transaction data is public)    |
| Arcium cluster             | Privacy if at least one member is honest; authenticated output | Liveness: a member can delay or abort computations  |
| Pyth                       | Signed price data                                              | Freshness, confidence, sanity (Walnut checks these) |
| Upgrade and risk multisigs | Code and parameter changes, behind timelocks                   | Instant adverse changes (timelocked, cancellable)   |
| Guardian                   | Immediate, expiring pauses                                     | Long pauses or parameter changes                    |
| Keeper                     | Nothing. Every keeper action is permissionless and checked     | Funds, admin actions, Viewing Keys                  |
| Frontend host and browser  | Delivering unmodified code; protecting in-memory keys          | Everything else; see frontend section               |
| RPC provider               | Nothing for correctness                                        | Honest reads (may lie or withhold)                  |

## Threats and mitigations

### Callback and computation integrity

| Threat                                                     | Mitigation                                                                                                                         |
| ---------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| Forged or unauthenticated callback output                  | Callback verifies Arcium-signed output. Invalid output changes nothing; the Intent stays `Queued` until abandoned.                 |
| Duplicate or late callback                                 | Intent status transitions exactly once. Callbacks on terminal Intents return without writing.                                      |
| Stale callback after state changed                         | Callback compares every captured generation, accounting index, and rate epoch; marks `Stale` on mismatch.                          |
| Callback with substituted accounts or inputs               | `accounts_commitment` and `inputs_commitment` are recomputed and compared before any write.                                        |
| Old computation still running after Lock expiry            | Abandonment increments every still-matching generation, so the old callback can never match.                                       |
| Price-sensitive result applied after prices moved          | Decision Expiry is bound in the Intent; late results become `Expired`.                                                             |
| Key/nonce reuse in encrypted state                         | One encrypted Position structure; Task 2 records every key identity and nonce under randomized updates and fails on any repeat.    |
| Transaction or compute limits make an action unrecoverable | Task 2 measures every queue and callback envelope; over-ceiling features are redesigned or split. Callbacks never create accounts. |
| Circuit reveals more than intended                         | Each circuit reveals only the documented outputs; independent Arcis circuit review is a mainnet gate.                              |

### Accounting and economic

| Threat                                               | Mitigation                                                                                                                          |
| ---------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| New supplier captures earlier interest               | Interest is checkpointed before every Pool mutation; shares are minted from current encrypted supplier equity.                      |
| First-depositor or empty-pool share manipulation     | First supply requires canonical empty Pool and zero unsettled Recovery Claims; zero-share deposits move nothing.                    |
| Rounding drain                                       | Rounding always favours the protocol (debt `ceil`, payouts `floor`); property tests cover all rounding boundaries.                  |
| Bad debt counted twice or hidden                     | Absorb removes Debt Shares and uses Reserves first; supplier equity formula has no second loss term.                                |
| Index misuse                                         | Accounting index only for shares and Cash; projected risk index only for health and caps; accounting index never decreases.         |
| Liquidator overcharge or over-seizure                | Seizure is limited to payment plus bonus with borrower-protecting floor rounding; post-liquidation health must weakly improve.      |
| Liquidation configuration that cannot restore health | Governance enforces `L × B < 1` and `T > L × B` with a rounding margin.                                                             |
| Arithmetic overflow                                  | Checked arithmetic against a committed range table for amounts, prices, exponents, indexes, rates, time deltas, and shares.         |
| Vault value leaked through failed operations         | Zero, failed, stale, expired, duplicate, and abandoned operations move no value; vault conservation is a property-tested invariant. |
| Treasury drains supplier funds                       | Reserve withdrawal is all-or-zero against both Cash and Reserves and reduces them equally, leaving supplier equity unchanged.       |
| Exposure larger than operations can monitor          | On-chain supply, debt, per-Position debt, Position-count, and debt-Position-count caps sized from measured keeper capacity.         |

### Oracle

| Threat                                  | Mitigation                                                                                                         |
| --------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| Spoofed price account                   | Verify Pyth receiver ownership and configured SOL/USD and USDC/USD feed IDs.                                       |
| Stale or wide prices                    | Age (default 60 s) and confidence (default 2%) limits with governed bounds.                                        |
| Manipulated or anomalous spot price     | Collateral at `price - confidence`, debt at `price + confidence`; spot-versus-EMA and last-good-price breakers.    |
| Liquidation during oracle anomaly       | Anomaly mode blocks freezes, liquidations, Absorb, borrows, and collateral withdrawal; pending results go `Stale`. |
| Liquidation using the freeze-time price | Liquidation always rechecks health with a fresh observation.                                                       |

### Liveness and griefing

| Threat                                            | Mitigation                                                                                                            |
| ------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| Attacker locks a Position to block repayment      | Health checks never lock Positions. Writer Locks expire and can be abandoned permissionlessly.                        |
| Health-check spam                                 | Per-caller and per-Position rate limits or bonds.                                                                     |
| Funds stuck after a failed shield                 | Pending Shield retry or refund to the recorded source, mutually exclusive with credit.                                |
| Buyer funds stuck in a failed recovery settlement | Exactly-once refund of escrowed USDC on any non-applied outcome.                                                      |
| Emergency pause blocks rescue                     | Repay, collateral addition, callback completion, and recovery stay available in emergency mode.                       |
| Guardian holds the market paused indefinitely     | Guardian pauses expire unless renewed by governance.                                                                  |
| Arcium cluster unavailable                        | No silent loss; Intents are abandoned and retried. Arcium Recovery Peers and cluster migration tested before mainnet. |
| Keeper offline or competing keepers               | Keeper actions are permissionless and idempotent; anyone may run one.                                                 |

### Governance and upgrade

| Threat                                 | Mitigation                                                                                                         |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| Compromised single admin key           | Separate upgrade, risk, treasury, and guardian authorities; upgrade and risk are Squads (or equivalent) multisigs. |
| Malicious upgrade or adverse parameter | Enforceable timelock with cancellation; parameter changes emit old/new values and invalidate pending Intents.      |
| Admin takeover by transfer             | Two-step propose/accept admin transfer.                                                                            |
| Unreviewed deployment                  | Mainnet uses the identical reviewed artifact; scripts require explicit program ID and release hash confirmation.   |

### Frontend and key handling

| Threat                                            | Mitigation                                                                                                                           |
| ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| Malicious npm dependency                          | Pinned lockfile, `pnpm audit` in CI with expiring exceptions, minimal dependencies in key-handling code.                             |
| Compromised hosting or CDN                        | Static, self-hostable build so users can run their own copy; strict CSP. Residual risk remains for users of a hosted instance.       |
| Malicious browser extension                       | Cannot be fully mitigated. Documented risk; keys held only in memory with an explicit lock action.                                   |
| Wallet signature not deterministic                | Wallet compatibility tests; unsupported wallets get a clear recovery warning before Position creation.                               |
| RPC manipulation                                  | RPC is configurable; the SDK checks account owners and PDAs; correctness of on-chain state does not depend on the RPC.               |
| Phishing site requesting the derivation signature | The signed message is domain-separated by program ID, network, Position, and epoch; UI and docs explain which message is legitimate. |
| Telemetry leaking secrets                         | No third-party analytics or scripts while a key is unlocked; error reports strip transaction data and ciphertexts.                   |
| Lost wallet                                       | Not recoverable in v1. Ordinary key rotation is not wallet-loss recovery. Any recovery scheme needs its own threat model and review. |

## Residual risks

- Arcium collusion of every cluster member would break confidentiality.
- Smart-contract or circuit bugs not found in review could lose funds.
- Fast SOL price moves beyond the liquidation bonus can create bad debt, which Reserves and then suppliers absorb.
- Public metadata permits the inference described in [PRIVACY_MODEL.md](PRIVACY_MODEL.md#known-inference-channels).
- Ciphertext is permanent; a future key compromise exposes past data.
- Regulatory treatment is unresolved; legal review is a mainnet gate.
