# Walnut Protocol: Mainnet Deployment Cost (Solana L1)

**Estimated total: ~8–10 SOL (~$1,200–1,500) + $10 USDC treasury minimum**

| Item | Cost (SOL) | Notes |
|---|---|---|
| Program deployment | ~1.5–2.0 | Rent-exempt storage for the compiled program (~6,960 lamports/byte) |
| Arcium MPC circuits (7 active) | ~5.0–6.0 | `lend_op`, `disclose_position`, `rotate_position_key`, `pool_op`, `position_private`, `liquidate_pool`, `pool_band` — stored on-chain as rent-exempt accounts |
| Account creation | ~0.5–1.0 | Market (668 B), LendingPool (253 B), treasury and supporting accounts |
| Tx fees + operating reserve | ~1.0–1.5 | Deploy transactions, keeper operations, 1 SOL kept free |
| **Total** | **~8–10 SOL** | |

## Treasury
- Minimum **$10 USDC** in the treasury USDC account (mainnet mint `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v`).
- Revenue already exists via the reserve factor on borrower interest; no extra 1% fee is needed.

## Why it costs more than a basic Solana program
A plain Solana program costs roughly $5–10 to deploy. About 60% of Walnut's cost is Arcium circuit storage, not the program itself.

## Cost-saving notes
- Reclaim the 6 legacy circuits (`add_together`, `init_position`, `apply_position_delta`, `liquidation_receipts`, `oversized_output`, `rotate_viewing_key`) to recover rent. Do not upload them on mainnet.
- Deployment rent for the program buffer is recoverable when closing buffers, so only the final program size is locked up.
- Figures are estimates: confirm with `solana rent <bytes>` and the actual compiled sizes before funding.
