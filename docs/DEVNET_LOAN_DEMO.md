# Devnet private loan demo (walnut-diffn)

Private collateral, borrow and repay on Solana devnet with Arcium MPC. Balances, collateral and debt stay encrypted in the owner's Position; only shield and unshield move public token amounts. Every value below was decrypted locally by the owner with their Viewing Key after each step.

- Program: `5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea`
- Market: `Du5ZWegZJj1RvpZ2GHVNgsy2MXAhiThpP3AHWRkJEEGr`
- Position: `3wAtFPYc3A22uyEaEd6AqutegWMXJChPnWoHEndytfyq`
- Demo SOL price: 150 USDC per SOL; loan-to-value: 50%
- Run: 2026-11-18T05:01:59.588Z

## Run it

```bash
cd /mnt/d/solana/walnut-solana
bash scripts/diffn-devnet.sh 2>&1 | tee ~/diffn.log
```

The script reclaims unused circuit rent, upgrades the program if the build changed, and runs `pnpm --filter @walnut/sdk demo:loan`. To run only the demo:

```bash
export ANCHOR_PROVIDER_URL="$(solana config get | awk '/RPC URL/ {print $3}')"
export ANCHOR_WALLET="$HOME/.config/solana/walnut-deployer.json"
export WALNUT_BENCH_CLUSTER_OFFSET=456
pnpm --filter @walnut/sdk demo:loan
```

The artifact is written to `artifacts/devnet-loan-demo.json` (gitignored).

## Result

Columns are the owner's decrypted free SOL, free USDC, collateral SOL and debt USDC after the step.

| Step                | Outcome                  | Free SOL    | Free USDC | Collateral SOL | Debt USDC | ms   | Tx                                                                                                                                           |
| ------------------- | ------------------------ | ----------- | --------- | -------------- | --------- | ---- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| shield_sol          | sol shield               | 0.234321000 | 0.234567  | 0.000000000    | 0.000000  | 7114 | [tx](https://explorer.solana.com/tx/3Vw5v7KqmcQKj833ozpEzcpB7vRUUUyu67Bow8cujwGHpwfZaW6mPidbzqZADRXphmJJbjP5K8P9E5owBUkJq9jw?cluster=devnet) |
| add_collateral      | applied                  | 0.034321000 | 0.234567  | 0.200000000    | 0.000000  | 6780 | [tx](https://explorer.solana.com/tx/5nVcCZwB4QW8QKfeKQSp29SJ1WsgwRJ9MLvJt2m3osAYvaE9AiuJ283MYHf8dX3F7gkUkSWcDEjoWY1RXFhLFjri?cluster=devnet) |
| borrow              | applied                  | 0.034321000 | 10.234567 | 0.200000000    | 10.000000 | 6153 | [tx](https://explorer.solana.com/tx/3XEDh5AnfpooPMfBUfwPFJTKRGo5EYMmH5mAwwvTRg1VjgziWEnqCu4ScBTwuiZ4ZusjUNUkbJAS5E7VVAB5prcQ?cluster=devnet) |
| borrow_over_limit   | refused, state unchanged | 0.034321000 | 10.234567 | 0.200000000    | 10.000000 | 6235 | [tx](https://explorer.solana.com/tx/2qkGoWVnM1VUYejPtuQQTLctBoMYRGvZGscGYduRuwsWP2az4E3yYayiYjx5FtfHqJ1wNzpEEiWLZCnQXTz9jCUg?cluster=devnet) |
| unshield_usdc       | usdc unshield            | 0.034321000 | 0.234567  | 0.200000000    | 10.000000 | 6708 | [tx](https://explorer.solana.com/tx/4MxYWqSSrbbRH7DJYzQrd29HFZiYDDQQiExVyeiJDcc3o6QW7XoenvXVrSHaBwz44KUSYDAhx1jbuwAt5xAVBXKH?cluster=devnet) |
| shield_usdc         | usdc shield              | 0.034321000 | 10.234567 | 0.200000000    | 10.000000 | 5266 | [tx](https://explorer.solana.com/tx/wpN7cAJAinzQqrWTddNpoLwJaK7G5jeYGdXmWfKEBmE46vF4REnZXzVd7EFrckTHAxv3rCxwLxuZHv6NeToMDL9?cluster=devnet)  |
| repay               | applied                  | 0.034321000 | 0.234567  | 0.200000000    | 0.000000  | 4880 | [tx](https://explorer.solana.com/tx/3pUrUC7CyhPyy2LRYKJGLv4kDUWgabBefNx6gd7gopaQbB6r8Rsc1XeyrqQ4ucFwNNERSzctWNNwLqNctBigdRP4?cluster=devnet) |
| withdraw_collateral | applied                  | 0.234321000 | 0.234567  | 0.000000000    | 0.000000  | 4984 | [tx](https://explorer.solana.com/tx/4oTDWbN3UmRmacKpb8HovU86Seo4eLYdcuDE2vCrDdwT2fWwhafHo5B3AN4Wc6dmecSAwFPtkqKjhHhgtjPaQf6j?cluster=devnet) |
| unshield_wsol       | sol unshield             | 0.034321000 | 0.234567  | 0.000000000    | 0.000000  | 6220 | [tx](https://explorer.solana.com/tx/66aui4cYjgQAtqtC1skbhpxjdNm1q62Zqy7RKXFRdj7eRwMazUG34MR79QJs3Go5Z1m6TcwE1ZHpAuEaRFV5ss9Y?cluster=devnet) |
| unwrap_wsol         | wSOL -> SOL              | 0.034321000 | 0.234567  | 0.000000000    | 0.000000  | 1471 | [tx](https://explorer.solana.com/tx/2ynVx7Hh33KcJMTUaMoKrYjm8cjt7JJM2Bkxp1gigdTXAaSo5t5ZRAScq6paxJMn5Xqaa7rarjp6wiPYXkk2JD14?cluster=devnet) |

The borrow limit is 0.2 SOL x 150 USDC x 50% = 15 USDC. The 10 USDC borrow applied; the over-limit borrow was refused by the circuit and left all balances unchanged. After repay, debt is 0 and the full collateral was released.

## Cost

The run started at about 12.2 SOL and ended at 25.9 SOL. The difference is the 16.6 SOL recovered from the unused `update_utilization_band` circuit, minus the program extend, the `position_op` circuit upload and fees. The 16 SOL upgrade buffer is refunded on deploy.

## Demo shortcuts (not production)

- The SOL price is a caller-supplied instruction argument, not an oracle price.
- No interest, no liquidation, no Pool accounting.
- Lender liquidity is test USDC minted by the deployer straight into the market vault.
- The operation type and amount are public circuit arguments; balances, collateral and debt are private.

See `.kiro/specs/walnut-diffn/` for the spec. Production rules remain in `.kiro/specs/walnut-solana/`.
