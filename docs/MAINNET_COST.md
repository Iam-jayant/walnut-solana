# Mainnet Deployment Cost (Solana L1)

Rent figures use about 6,960 lamports per byte, the rent-exempt minimum. Sizes
are from the current local build (`target/deploy/walnut.so`, `build/*.arcis`).
Before funding, check each figure with `solana rent <bytes>`. The deployment plan
is in [`MAINNET_DEPLOY.md`](MAINNET_DEPLOY.md).

## Summary

| Item                                          | On-chain circuits | Off-chain circuits (planned) |
| --------------------------------------------- | ----------------- | ---------------------------- |
| Program data (`walnut.so`, about 3.46 MB)     | about 24 SOL      | about 24 SOL                 |
| Deploy buffer (temporary, recoverable)        | about 24 SOL      | about 24 SOL                 |
| 7 active circuits (about 4.6 MB)              | about 32 SOL      | about 0 SOL                  |
| Comp def, MXE, market, pool and other accounts | about 0.5–1 SOL   | about 0.5–1 SOL              |
| Fees and operating reserve                    | about 1–2 SOL     | about 1–2 SOL                |
| **SOL locked after deploy**                   | **about 58 SOL**  | **about 26 SOL**             |
| **Peak SOL needed during deploy**             | **about 82 SOL**  | **about 50 SOL**             |

The earlier estimate of 8–10 SOL was too low. It assumed circuit storage of
about 5–6 SOL and a program of about 1.5–2 SOL. Neither matches the real sizes.

## Active circuits

| Circuit               | Size (bytes) | On-chain rent (SOL) |
| --------------------- | ------------ | ------------------- |
| `liquidate_pool`      | 1,199,157    | about 8.3           |
| `pool_op`             | 1,019,852    | about 7.1           |
| `pool_band`           | 697,195      | about 4.9           |
| `lend_op`             | 601,238      | about 4.2           |
| `position_private`    | 500,625      | about 3.5           |
| `rotate_position_key` | 288,935      | about 2.0           |
| `disclose_position`   | 288,681      | about 2.0           |
| **Total**             | **4,595,683** | **about 32**       |

Do not register the legacy, spike or devnet circuits on mainnet:
`supply_flow` (39 MB), `recover_empty_pool`, `reserve_withdrawal`,
`update_utilization_band`, `add_together`, `init_position`,
`apply_position_delta`, `liquidation_receipts`, `oversized_output` and
`rotate_viewing_key`. Check that list against the instructions the mainnet web
app and keeper actually call.

## Ways to reduce cost

1. **Off-chain circuits** on GitHub Releases save about 32 SOL. Hosting is free.
2. **A smaller program binary.** Leaving the spike and legacy instructions out of
   the mainnet build lowers both program rent and the deploy buffer. Each MB
   removed saves about 7 SOL of locked rent.
3. **Close the deploy buffer** after deployment. `solana program deploy` does this
   on success. If a deploy fails, close it with `solana program close --buffers`.
4. **Do not over-allocate.** Deploy without a large `--max-len`. Extend later with
   `solana program extend` if an upgrade needs more space.

## Treasury

- Keep at least **$10 USDC** in the treasury USDC account. The mainnet mint is
  `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v`.
- Revenue comes from the reserve factor on borrower interest. No extra protocol
  fee is needed.
