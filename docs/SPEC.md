# Walnut-Solana specification

Status: baseline (Task 1.4). The authoritative requirements and design live in `.kiro/specs/walnut-solana/requirements.md` and `design.md`. This document summarizes them for readers of the repository. If the two disagree, the spec files win and this document must be corrected.

## What Walnut is

Walnut-Solana is a confidential, over-collateralized lending market on Solana. Encrypted state transitions run on **Arcium multi-party computation (MPC)**. Walnut does **not** use fully homomorphic encryption (FHE) and must not be described as an FHE protocol.

Individual balances, collateral, supply shares, debt shares, and liquidation amounts are encrypted. Wallet addresses, Position addresses, action timing, shield and unshield amounts, protocol token-account balances, and a small set of lifecycle fields are public. The exact split is in [PRIVACY_MODEL.md](PRIVACY_MODEL.md).

## v1 scope

The v1 market has one risk direction:

| Item                 | v1 choice                                                                   |
| -------------------- | --------------------------------------------------------------------------- |
| Collateral           | SOL, held as wSOL in a program vault                                        |
| Supplied asset       | Circle USDC                                                                 |
| Borrowed asset       | Circle USDC                                                                 |
| Settlement           | Internal encrypted accounting; public SPL transfers only at shield/unshield |
| Liquidation          | Formulaic encrypted partial liquidation, with Absorb as a last resort       |
| Disclosure           | Owner-authorized re-encryption of chosen fields to an auditor key           |
| Position reads       | Local decryption in the user's client; no Walnut server is contacted        |
| Encryption mechanism | Arcium MPC with `Enc<Shared>` / `Enc<Mxe>` ciphertexts                      |

User actions in v1:

- create a Position and derive its Viewing Key;
- shield and unshield SOL and USDC;
- supply and withdraw USDC;
- post and withdraw SOL collateral;
- borrow and repay USDC;
- liquidate an unhealthy Position from the liquidator's own shielded USDC;
- create a selective disclosure for an auditor;
- rotate the Viewing Key.

Deferred features are listed in [ADR-0002](adr/0002-deferred-features.md). None of them may ship behind an undocumented flag or mock branch.

## Launch policy

1. Devnet is the first public deployment.
2. Mainnet is forbidden until every gate in [OPERATIONS.md](OPERATIONS.md#mainnet-gates) passes.
3. Mainnet starts as a capped beta. Supply, borrow, Position-count, and debt-Position-count caps are enforced on-chain.
4. Public copy may not say "production-grade", "audited", "first on Solana", "fully private", "anonymous", "FHE", or "compliance-ready" unless a dated evidence record supports the exact claim.
5. Walnut describes itself as using Arcium MPC and encrypted Position amounts.
6. No specification, test count, or audit is presented as a guarantee that losses are impossible.

## Pending decision: Arcium state model

The encrypted-state layout is not final. Task 2 is a blocking feasibility gate that must prove, against the pinned Arcium release, that one `Enc<Shared>` Position structure can be stored, reread, updated, and decrypted locally without key/nonce reuse, and that every queue and callback transaction fits under 1,232 bytes (target 1,150).

- **Preferred layout:** one `Enc<Shared>` `PositionSecret` per Position, sealed to the owner's Viewing Key; Pool and Recovery Claim state as `Enc<Mxe>`.
- **Fallback:** `Enc<Mxe>` Position state plus an owner-authorized view computation that re-encrypts to the Viewing Key on request.

ADR-0003 will record the result. No financial instruction is implemented before ADR-0003 passes. Statements in these documents about `Enc<Shared>` Position state hold only if ADR-0003 adopts the preferred layout.

## Core concepts

- **Market:** global configuration, authorities, vaults, public counts and count caps.
- **Asset State:** public per-market risk configuration, accounting Borrow Index, rate epoch, utilization band, caps, and pauses.
- **Pool State:** encrypted USDC Cash, total Supply Shares, total Debt Shares, and Reserves, plus public revision metadata.
- **Position:** owner-bound PDA with one encrypted state (free SOL, free USDC, SOL collateral, USDC Supply Shares, USDC Debt Shares) and public lifecycle fields.
- **Operation Intent:** a PDA created for every queued computation. It records captured generations, indexes, oracle observations, commitments to accounts and inputs, Decision Expiry, and a terminal status.
- **Generation:** a monotonically increasing revision counter. A callback may write only the generations captured by its Operation Intent.
- **Lock:** a writer lock on an account, held by one Operation Intent until callback or abandonment.
- **Pending Shield:** a PDA recording a deposited amount until its encrypted credit completes or it is refunded.
- **Liquidation Record, Recovery Position, Recovery Claim, Pending Recovery Settlement, Disclosure:** see [ARCHITECTURE.md](ARCHITECTURE.md#accounts).

## Operation outcomes

Every Operation Intent moves exactly once from `Queued` to one terminal status:

| Status      | Meaning                                                                          | Financial effect   |
| ----------- | -------------------------------------------------------------------------------- | ------------------ |
| `Applied`   | Authenticated success, dependencies current, nonzero approved transition         | Applied atomically |
| `Zero`      | Authenticated success, dependencies current, nothing approved                    | None               |
| `Failed`    | Authenticated static failure callback                                            | None               |
| `Stale`     | Authenticated success, but a captured generation or commitment no longer matches | None               |
| `Expired`   | Authenticated success arriving after Decision Expiry                             | None               |
| `Abandoned` | Permissionless timeout recovery after Lock expiry, with no terminal callback     | None               |

The SDK adds a client-side `TimedOut` outcome for computations it stopped waiting on. Invalid or unauthenticated callback output changes nothing; the Intent stays `Queued` until abandoned.

## Accounting rules (summary)

- The public accounting Borrow Index starts at WAD (10^18) and never decreases.
- The accounting index is used only for share minting/burning, Cash, and interest checkpoints. A risk index projected to Decision Expiry is used only for health and cap checks.
- Every Pool-mutating computation first checkpoints interest and credits the reserve-factor share to encrypted Reserves.
- `supplier_equity = cash + current_debt - reserves`. Bad debt is expressed once, by removing Debt Shares and using Reserves first.
- Supply mints `floor(amount × total_supply_shares ÷ supplier_equity_before)` shares. Withdrawal pays `floor(shares × supplier_equity_before ÷ total_supply_shares)`, all-or-zero against Cash.
- Borrow books `ceil` Debt Shares against the borrower; rounding surplus goes to Reserves.
- Collateral is valued at `price - confidence`; debt at `price + confidence`.
- Liquidation repays toward a target Health Factor, seizes at most payment plus bonus with borrower-protecting rounding, and must weakly improve health.

Full formulas are in `design.md` (Interest and pool accounting, Health model, Liquidation design).

## Cost and infrastructure

Walnut has no mandatory paid service. It works with a public or self-hosted Solana RPC, public Pyth Hermes, a self-hosted Docker keeper, local Arcium tooling, and static or free-tier web hosting. Paid RPC or hosting can be plugged in through adapters, but protocol correctness never depends on them.

Open-source software does not make mainnet free. These costs remain and are described in [OPERATIONS.md](OPERATIONS.md#costs):

- Solana transaction fees and account rent;
- Arcium computation fees;
- program deployment;
- keeper fee-payer funds;
- independent security and economic reviews;
- market liquidity.

Users pay their own action and computation fees. Protocol maintenance transactions are paid from a monitored keeper wallet.

## Related documents

- [ARCHITECTURE.md](ARCHITECTURE.md): components, accounts, and computation lifecycle.
- [PRIVACY_MODEL.md](PRIVACY_MODEL.md): public and encrypted fields per account and action.
- [THREAT_MODEL.md](THREAT_MODEL.md): trust assumptions, threats, and mitigations.
- [OPERATIONS.md](OPERATIONS.md): environments, keeper, monitoring, costs, and launch gates.
- [ADR-0001](adr/0001-stack-and-scope.md): stack and scope.
- [ADR-0002](adr/0002-deferred-features.md): deferred features.
- [CI.md](CI.md) and [WSL2_SETUP.md](WSL2_SETUP.md): build and toolchain.
