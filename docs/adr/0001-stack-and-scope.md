# ADR-0001: Stack and v1 scope

- Status: Accepted
- Date: 2026-11-15
- Related: Requirements 1, 2, 14, 15, 18; [SPEC.md](../SPEC.md); [ADR-0002](0002-deferred-features.md); ADR-0003 (pending, Task 2)

## Context

Walnut-Solana is a confidential lending market. An earlier design combined dual-asset borrowing, credit tiers, private user-to-user transfers, and sealed whole-position auctions. That design multiplied cross-asset debt cases, externally initiated Position writes, and callback-size risk, and included an auction whose bidders could not price an unknown-size lot.

Walnut needs encrypted computation on balances that the chain can verify and that only the owner can read, with no Walnut-operated decryption server and no mandatory paid service.

## Decision

### Encryption: Arcium MPC, not FHE

Walnut uses **Arcium multi-party computation**. Encrypted state transitions are Arcis circuits executed by an Arcium MPC cluster and applied through authenticated callbacks to the Walnut Solana program. Walnut does not use fully homomorphic encryption and must not be described as FHE.

Reasons:

- Arcium integrates with Anchor programs through queue and callback instructions on Solana.
- Owner-readable `Enc<Shared>` ciphertext lets users decrypt their Position locally.
- `Enc<Mxe>` ciphertext lets the protocol hold aggregate Pool state that no individual can read.

Accepted consequences:

- Privacy assumes at least one honest Arcium cluster member. Cluster members can affect liveness.
- Every encrypted operation is asynchronous (queue, compute, callback), so the program needs Operation Intents, Locks, generations, and abandonment.
- Arcium computation fees are an unavoidable per-operation cost.

The Position ciphertext layout (one `Enc<Shared>` structure, or `Enc<Mxe>` plus an owner view computation) is **not** decided here. Task 2 is a blocking gate and ADR-0003 records the result. No financial instruction is implemented before ADR-0003 passes.

### Stack

| Layer              | Choice                                                                                               |
| ------------------ | ---------------------------------------------------------------------------------------------------- |
| On-chain program   | Rust, Anchor, Arcium program integration (`programs/walnut`)                                         |
| Encrypted circuits | Arcis (`encrypted-ixs`)                                                                              |
| Oracle             | Pyth `PriceUpdateV2` via the Pyth receiver; public Hermes for updates                                |
| SDK                | TypeScript, browser and Node (`packages/sdk`)                                                        |
| Reference model    | TypeScript plaintext model for previews and differential tests (`packages/reference-model`)          |
| Keeper             | TypeScript service in Docker (`keeper`)                                                              |
| Web App            | Next.js App Router, TypeScript, Tailwind CSS v4, Solana wallet-adapter, local Telegraf fonts (`web`) |
| Package management | pnpm workspace                                                                                       |
| Build environment  | Linux or WSL2 for all Rust, Solana, Anchor, and Arcium commands                                      |
| CI                 | GitHub Actions; see [CI.md](../CI.md)                                                                |

Exact versions are pinned in `toolchain.versions` and checked by `scripts/check-toolchain.sh`. At the date of this ADR the coordinated set is Arcium 0.16.0 (CLI, crates, `@arcium-hq/client`), Anchor 1.0.2, Solana CLI 3.1.10, Rust 1.99.1, Node 26.13.1, and pnpm 9.15.9. All Arcium components move together as one release. `toolchain.versions` is authoritative if this table falls out of date. See [WSL2_SETUP.md](../WSL2_SETUP.md#compatibility-matrix).

### No mandatory paid services

Every required component is open-source or freely usable: public or self-hosted RPC, public Pyth Hermes, self-hosted keeper, local Arcium tooling, static or free-tier hosting. Paid providers may be added as optional adapters. The SDK must not depend on Helius, Vercel, or any other paid provider.

This does not make mainnet free. Rent, transaction fees, Arcium computation fees, deployment, keeper funds, reviews, and liquidity remain. See [OPERATIONS.md](../OPERATIONS.md#costs).

### v1 scope

- One market: SOL collateral (as wSOL), USDC supply and borrow.
- Shield and unshield SOL and USDC.
- Encrypted free balances, collateral, Supply Shares, and Debt Shares.
- Local decryption of the user's own Position.
- Formulaic encrypted partial liquidation, with Absorb and Recovery Claims for residual bad debt.
- Owner-authorized selective disclosure to an auditor key.
- Viewing Key rotation.
- On-chain caps; devnet first; mainnet only as a capped beta after every gate.

Everything else is deferred; see [ADR-0002](0002-deferred-features.md).

## Consequences

- One risk direction removes cross-asset debt and simplifies oracle and liquidation logic.
- No external party writes to another user's Position except liquidation, which is fully specified and bounded.
- Public metadata (addresses, timing, shield/unshield amounts, `has_debt`, counts, freeze state) is accepted and documented in [PRIVACY_MODEL.md](../PRIVACY_MODEL.md).
- If Task 2 fails a gate, the design and requirements are updated before Task 3. The gate is not bypassed for schedule.
