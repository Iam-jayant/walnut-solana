# ADR-0002: Deferred features

- Status: Accepted
- Date: 2026-11-15
- Related: Requirement 18; [ADR-0001](0001-stack-and-scope.md); [SPEC.md](../SPEC.md)

## Context

Walnut v1 is deliberately narrow (ADR-0001). Several features from earlier designs, or commonly expected of lending protocols, each add a class of risk that v1 is not built to handle: cross-asset debt, writes to a Position initiated by someone other than its owner, farmable scoring, or auctions that bidders cannot price.

## Decision

The following features are out of scope for v1. Each requires a new specification and security review before implementation.

| Feature                                                                  | Main reason for deferral                                                                                         |
| ------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------- |
| User-to-user encrypted transfers (with inbox-based receipt claiming)     | Externally initiated Position writes; concurrent-writer and key/nonce risks; needs a claim-inbox design.         |
| Repayment activity tiers or identity-backed credit                       | Scores can be farmed with cheap repay cycles; identity backing needs a separate privacy and legal model.         |
| Borrowing SOL                                                            | Adds a second debt asset and cross-asset debt accounting.                                                        |
| USDC or mixed collateral; additional assets                              | Requires isolated markets and additional oracle and liquidation paths.                                           |
| Sealed-bid whole-position liquidation auctions                           | Bidders cannot rationally price an encrypted, unknown-size lot.                                                  |
| P2P lending                                                              | Different matching, pricing, and default model.                                                                  |
| Under-collateralized lending                                             | Requires credit and enforcement mechanisms outside v1.                                                           |
| Multi-wallet identity aggregation                                        | Links wallets, weakening the privacy model; needs its own threat model.                                          |
| Token-2022 Confidential Transfers and other confidential token standards | Separate encryption system and composability review.                                                             |
| Cross-protocol synchronous composability                                 | Arcium computations are asynchronous; synchronous calls into encrypted state are not supported by the v1 design. |
| Wallet-loss recovery                                                     | Ordinary key rotation is not recovery. Any recovery scheme needs a separate threat model and mainnet review.     |

## Rules

- No deferred feature may be introduced through an undocumented production flag, mock branch, or hidden instruction.
- Production code contains no mock settlement, fake transaction, or simulated success path for a deferred feature.
- Public copy must not describe deferred features as available.
- Reintroducing a feature requires a new ADR that supersedes the relevant row of this table, an updated spec, and the reviews listed in [OPERATIONS.md](../OPERATIONS.md#mainnet-gates).

## Consequences

- v1 users cannot transfer encrypted balances to each other; value leaves a Position only through unshield (public amount) or liquidation.
- v1 liquidations are formulaic and partial, which keeps liquidator pricing simple at the cost of more transactions for large Positions.
- Lenders and borrowers are limited to the SOL/USDC market until isolated markets are specified.

- Task 8: supply and withdraw-supply requests (USDC units or shares) are public computation arguments in v1, not encrypted inputs. The Intent callback rebuilds its input commitment from the echoed request, as unshield does. Encrypted request inputs are deferred. INITIAL_SHARE_SCALE is 1,000. The circuit's domain is u64 values and 2^63 total shares.
