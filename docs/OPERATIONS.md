# Walnut-Solana operations

Status: baseline (Task 1.4). Deployment scripts, keeper configuration, and runbooks are written in later tasks (14, 16, 17). This document fixes the operating model they must follow. A separate `MAINNET_CHECKLIST.md` is added before mainnet preparation.

## Environments

| Network  | Purpose                       | Arcium                 | Pyth               | USDC               |
| -------- | ----------------------------- | ---------------------- | ------------------ | ------------------ |
| Localnet | Development and CI            | Local Arcium cluster   | Test-only fixtures | Test mint          |
| Devnet   | First public deployment, soak | Arcium devnet          | Real Pyth updates  | Circle devnet USDC |
| Mainnet  | Capped beta, after all gates  | Arcium mainnet cluster | Real Pyth updates  | Circle USDC        |

Each network has a Network Profile in `config/networks/` containing only public configuration: program ID, Arcium cluster, mints, vaults, Pyth feed IDs, RPC endpoint, commitment, and mainnet confirmation policy. Secrets are never stored in profiles.

Fixtures are test-only. Production code contains no mock settlement, fake transaction, fixture import, or simulated success path; CI enforces this with `pnpm check:fixtures`.

## Infrastructure without paid services

| Need           | Default                                     | Optional                    |
| -------------- | ------------------------------------------- | --------------------------- |
| Solana RPC     | Public endpoint or self-hosted node         | Any paid JSON-RPC provider  |
| Prices         | Public Pyth Hermes                          | Self-hosted Hermes          |
| Keeper         | Self-hosted Docker container                | Any container host          |
| Web hosting    | Static export or free-tier/self-hosted host | Vercel or other paid host   |
| Arcium tooling | Local Arcium CLI and Arx node image         | none                        |
| Monitoring     | Prometheus-compatible metrics, JSON logs    | Any compatible hosted stack |

Provider adapters are configuration only. Protocol correctness never depends on a specific provider.

## Costs

Open-source software removes licence costs. It does not remove network and operating costs. The following are unavoidable on mainnet:

| Cost                    | Who pays                      | Notes                                                                                                                                                      |
| ----------------------- | ----------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Program deployment      | Project                       | SOL for program account rent and upgrade buffers.                                                                                                          |
| Account rent            | Creator of each account       | Market, vaults, Pool, Positions, Intents, Pending Shields, Records, Claims, Disclosures. Reclaimable rent is returned on close where the design allows it. |
| Solana transaction fees | Signer of each transaction    | Includes optional priority fees.                                                                                                                           |
| Arcium computation fees | Payer recorded in each Intent | Users pay for their own actions; the keeper pays for maintenance computations.                                                                             |
| Keeper fee-payer funds  | Project or keeper operator    | Pays for rate epochs, accrual, health checks, Liquidation Records, and abandonment. Monitored wallet.                                                      |
| Independent reviews     | Project                       | Solana/Anchor program, Arcis circuit, and economic reviews are mainnet gates.                                                                              |
| Market liquidity        | Suppliers and liquidators     | Lending only works with USDC supplied; liquidations need liquidators holding shielded USDC.                                                                |
| Legal review            | Project                       | Mainnet gate.                                                                                                                                              |

Exact per-action fees are not stated here because they are measured in Task 2 (ADR-0003) and re-measured on devnet. The Web App estimates user-paid fees before each signature.

## Authorities

| Authority | Form on mainnet                  | Powers                                                         |
| --------- | -------------------------------- | -------------------------------------------------------------- |
| Upgrade   | Squads or equivalent multisig    | Program upgrades, behind an enforceable timelock               |
| Risk      | Squads or equivalent multisig    | Risk parameters and caps; adverse changes timelocked           |
| Treasury  | Configured treasury USDC account | Receives approved reserve withdrawals                          |
| Guardian  | Separate key or multisig         | Immediate granular pauses that expire unless governance renews |

Admin transfer is two-step (propose, accept). Cap increases are timelocked and require a published operational review. The keeper holds none of these authorities.

## Keeper

The keeper is a Docker image with local durable storage (or an optional open-source database). It is permissionless; anyone may run additional instances, and expected races do not corrupt state.

Jobs:

- advance rate epochs and request utilization-band updates;
- accrue the public Borrow Index when the Pool is unlocked;
- discover debt Positions (`has_debt == true`) and schedule non-locking health checks;
- call `open_liquidation` after valid freezes and advertise eligible Liquidation Records;
- abandon expired Intents and release expired Locks;
- reclaim computation rent and fees the keeper paid;
- monitor Recovery Claims;
- emit alerts.

The keeper never finances liquidations, never spends another user's balance, and never holds admin authority, user Viewing Keys, or authority over user funds.

The scan rate is derived from the on-chain debt-Position cap and the documented maximum detection-to-freeze time. The cap is set so that the measured keeper fleet can scan every debt Position within that window.

### Monitoring

The keeper exposes `/healthz`, Prometheus-compatible metrics, and JSON logs. Required signals:

- freshness of each job;
- RPC failures and latency;
- queue-to-callback latency and failure rate by operation kind;
- count of `Queued` Intents past Decision Expiry or Lock expiry;
- fee-payer SOL balance, with an alert below a configured threshold;
- oracle anomaly status and price age;
- utilization-band observation age.

Logs and metrics never contain secrets, Viewing Keys, or ciphertexts.

## Emergency procedures

Pause controls are granular: new supply, supply withdrawal, new borrow, repay, collateral addition, collateral withdrawal, shield, unshield, liquidation start, and liquidation completion.

The following remain available during an emergency unless executing them would directly violate custody:

- repay and collateral addition;
- callback completion;
- Pending Shield retry and refund;
- safe Lock and Intent recovery (abandonment).

Runbooks for oracle anomaly, Arcium cluster outage, RPC failover, keeper restart, stuck Intents, and governance timelock cancellation are written and rehearsed on devnet before mainnet.

## Deployment

Deployment scripts must be idempotent and verify the program binary, program ID, Arcium cluster, computation definitions, mints, vaults, Pyth feeds, authorities, caps, and Network Profile. Scripts refuse mainnet unless given an explicit confirmation containing the expected program ID and release hash. Mainnet deploys the identical reviewed artifact used for the final release candidate.

## Mainnet gates

Mainnet deployment is forbidden until every item passes:

- [ ] Task 2 Arcium feasibility ADR (ADR-0003) passed.
- [ ] All CI and devnet end-to-end tests passed.
- [ ] Accounting reference model and differential tests passed.
- [ ] Adversarial callback, oracle, concurrency, and liquidation tests passed.
- [ ] Measured throughput supports the enforced caps and keeper SLA.
- [ ] Independent Solana/Anchor program and Arcis circuit security review completed.
- [ ] Economic review of interest, rounding, caps, and liquidation completed.
- [ ] Upgrade, risk, and treasury multisigs configured.
- [ ] Timelocks and emergency runbooks tested.
- [ ] Arcium Recovery Peers and cluster migration tested.
- [ ] RPC failover and keeper restart tested.
- [ ] Legal review completed for intended jurisdictions and frontend operation.
- [ ] Public risk and privacy documentation published.
- [ ] Bug bounty or equivalent coordinated disclosure process active.

Mainnet starts with conservative caps set from measured operational capacity, not marketing targets.
