# Mainnet Deployment Cost (Solana L1)

Rent is from `solana rent -um` on 2026-11-26. Program sizes are measured from
builds of this repository (Solana CLI 3.1.10). The deployment plan is in
[`MAINNET_DEPLOY.md`](MAINNET_DEPLOY.md).

Almost all of this SOL is a **refundable rent deposit**. Only transaction fees,
about 0.05–0.1 SOL, are actually spent.

## Summary (slim build, off-chain circuits)

| Item                                                 | SOL            |
| ---------------------------------------------------- | -------------- |
| Program data (`walnut.so`, 1,684,552 bytes)          | 8.56           |
| Deploy buffer (needed during deploy, returned after) | 8.56           |
| 11 circuits, hosted on GitHub Releases               | 0              |
| Comp def, MXE, market, pool and treasury accounts    | about 0.1      |
| Fees and operating reserve                           | about 1        |
| **Locked after deploy**                              | **about 9.7**  |
| **Peak needed during deploy**                        | **about 18.2** |

Arcium's per-computation fees on mainnet are not included. Confirm them with
Arcium.

## Program size

| Build                                                 | Bytes     | Rent (SOL) |
| ----------------------------------------------------- | --------- | ---------- |
| Full program, default settings (devnet build)         | 3,069,488 | 15.59      |
| Slim, default settings                                | 2,138,248 | 10.86      |
| Slim, `opt-level = "s"`, no instruction-name logs     | 1,806,024 | 9.17       |
| Slim, `opt-level = "z"`, no instruction-name logs     | 1,743,776 | 8.86       |
| **Mainnet build** (above plus `location-detail=none`) | 1,684,552 | **8.56**   |

About 0.9 MB of the slim binary is Anchor's generated account validation. Going
much lower means removing validation or changing the account layouts that
clients use, so the build stops here.

The slim build has no walnut stack warnings. With `opt-level = "z"` the compiler
reports one oversized frame in `arcium_client`
(`arcium_staking::MxeRecoveryStakeState` deserialization). Walnut does not use
that type.

## Circuits

Hosting the circuits off-chain avoids about 27 SOL of rent for 5,351,977 bytes.

| Circuit               | Bytes     | Used for                    |
| --------------------- | --------- | --------------------------- |
| `liquidate_pool`      | 1,199,157 | Private liquidation         |
| `pool_op`             | 1,019,852 | Private supply and borrow   |
| `pool_band`           | 697,195   | Utilization band (interest) |
| `lend_op`             | 601,238   | Health check                |
| `position_private`    | 500,625   | Private collateral changes  |
| `shield_credit`       | 334,870   | Deposit                     |
| `unshield`            | 304,778   | Withdrawal                  |
| `rotate_position_key` | 288,935   | Viewing Key rotation        |
| `disclose_position`   | 288,681   | Selective disclosure        |
| `init_pool_state`     | 63,475    | Encrypted pool setup        |
| `init_position_state` | 53,171    | Encrypted position setup    |

## Loader constraint

`arcium-client 0.16.0` checks the program's upgrade authority through the
upgradeable loader's `ProgramData` account. Loader v4, which deploys without a
separate buffer, is therefore not an option. The deploy needs about twice the
program rent for the few minutes of the deploy.

## Treasury

- Keep at least **$10 USDC** in the treasury USDC account. The mainnet mint is
  `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v`.
