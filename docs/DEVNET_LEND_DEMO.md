# Devnet lending demo (walnut-lend)

Walnut-Lend extends the private loan product into a lending market on Solana devnet with Arcium MPC. Each Position keeps its free balances, collateral, supply shares and debt shares encrypted. Operation amounts, pool totals and the interest indexes are public.

- Program: `5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea` (unchanged, one program upgrade)
- Cluster offset: 456, branch `v2`
- Spec: `.kiro/specs/walnut-lend/`

## What is new

- **On-chain Pyth price.** The caller no longer passes a price. Queue instructions read the Pyth `PriceUpdateV2` account stored in the `LendingPool` (`oracle`, `feed_id`, `max_price_age_seconds`) and use the low bound (`price - confidence`). A wrong, stale or wide-confidence price rejects the instruction before the Position is locked.
- **Supply and withdraw supply with interest.** Lenders supply USDC for supply shares. `LendingPool` holds the public `supply_index_1e9` and `borrow_index_1e9`, which accrue per second from utilization (kinked rate model, reserve factor to reserves). A share is worth more USDC as the supply index rises. Borrowers owe `debt_shares * borrow_index`. A full repay passes `amount = full_repay_amount(debt_shares, borrow_index)`.
- **Health check and liquidation.** Anyone can call `check_health` on a Position. The circuit reveals one bit, whether the Position is unhealthy, and never changes it. `liquidate` is permissionless: the liquidator pays USDC into the vault, the program computes the debt burn and the wSOL seize (with the liquidation bonus) from the public index and price, and the circuit re-checks health. If the Position is healthy (or the close factor or collateral bound fails) the USDC is refunded.
- **Viewing Key rotation.** `rotate_position_key` re-seals the Position under a new Viewing Key (next epoch). The new key decrypts the same balances and the old key no longer does.
- **Auditor disclosure.** `disclose_position(auditor_pubkey, mask)` re-seals a field-masked copy to an auditor x25519 key and writes the `Disclosure` PDA `["disclosure", position, auditor]`. Unselected fields are zero. The auditor decrypts locally (web: `/app/audit`).
- **Web.** The landing page is at `/` and the app moved to `/app`.

## Deploy

One command from WSL, run by the deployer (agents cannot use the deployer keypair). It is resumable: it records progress in `artifacts/lend-deploy-state.json`, checks on-chain state before each step and stops with a clear message when the balance is short.

```bash
cd /mnt/d/solana/walnut-solana
bash scripts/lend-devnet.sh 2>&1 | tee ~/lend.log
```

Steps: build, reclaim rent from unused comp defs, one program upgrade, `init_lending_pool`, register and upload `lend_op`, `disclose_position` and `rotate_position_key`, summary. Re-run the same command after any stop.

## Run the end-to-end demo

After the deploy finished:

```bash
export ANCHOR_PROVIDER_URL="$(solana config get | awk '/RPC URL/ {print $3}')"
export ANCHOR_WALLET="$HOME/.config/solana/walnut-deployer.json"
export WALNUT_BENCH_CLUSTER_OFFSET=456
pnpm --filter @walnut/sdk demo:lend
```

The demo needs the market and the demo owner Position from `demo:balance`. It builds instructions with the final IDL (`target/idl/walnut.json`), prints no RPC URL or key, writes `artifacts/devnet-lend-e2e.json` (gitignored) and exits 0 on success. State lives in `~/.walnut-demo/devnet` (the demo owner `owner.json`, and `lend-target.json` for the third Position).

| #   | Step                       | Assertion                                                                                                                                                                                                            |
| --- | -------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `setup`                    | The SDK `decodeLendingPool` matches the IDL decode (indexes).                                                                                                                                                        |
| 2   | `lender_supply`            | The deployer mints test USDC, shields it, runs `lend_op` supply. Vault delta equals the amount. Decrypted Position equals `applyLendOp` on the previous state with the shares from the `LendOpQueued` event.         |
| 3   | `borrower_collateral`      | Demo owner shields SOL (wrap + SyncNative) and adds collateral. Decrypted state matches.                                                                                                                             |
| 4   | `borrower_borrow`          | Borrow up to 80% of the LTV capacity at the current Pyth price (max 10 USDC). Decrypted state matches.                                                                                                               |
| 5   | `accrual_and_health_check` | Wait 45 s, then `check_health` (which accrues the pool). Borrow index strictly rose; supply index rose when the projection says it must, never fell. Borrower is healthy (`unhealthy = false`), state unchanged.     |
| 6   | `borrower_repay_full`      | Shield 1 test USDC for interest, repay with `amount = full_repay_amount`. Debt shares are 0 afterwards. A refusal because the index moved is retried (up to 3 attempts).                                             |
| 7   | `lender_withdraw_supply`   | Withdraw the lender's shares at the pool's supply index. Received USDC is at least the supplied amount minus 1 base unit (rounding favours the pool); the demo reports the interest gained.                          |
| 8   | `target_position`          | A third keypair opens a Position, adds 0.05 SOL collateral and borrows 90% of the LTV capacity.                                                                                                                      |
| 9   | `health_check_target`      | `check_health` on the third Position, state unchanged.                                                                                                                                                               |
| 10  | `liquidation_refused`      | Liquidation of a healthy Position (0.5 USDC): the Intent ends refused, the Position is unchanged, the liquidator USDC and the vault balance return to their previous values.                                         |
| 11  | `liquidation_applied`      | Runs only when step 9 reported unhealthy: wSOL seized equals the event amount, USDC paid equals the repay amount, state matches the mirror. Otherwise recorded as `skipped: no unhealthy position at current price`. |
| 12  | `key_rotation`             | Epoch increments, the new key decrypts the same five balances, the old key fails.                                                                                                                                    |
| 13  | `disclosure`               | Disclose free USDC, collateral and debt to a fresh auditor key. The auditor decrypts the selected fields (equal to the Position), the unselected fields stay hidden, another key cannot read it.                     |

A mismatch fails the run with `step "<name>" failed: <check>: expected X, got Y`. The price cannot be forced on devnet, so step 11 depends on the market at run time. The third Position stays open after the run and is reused by the next run.

## SOL budget

| Item                                                          | Estimate (SOL)                            |
| ------------------------------------------------------------- | ----------------------------------------- |
| Deployer start (design)                                       | about 23.9                                |
| Reclaim old comp defs                                         | positive, measured at run time            |
| Program extend                                                | -1 to -3                                  |
| Upgrade buffer                                                | about 16 held, refunded on deploy         |
| `lend_op`, `disclose_position`, `rotate_position_key` uploads | about 3.5 + 2 + 2                         |
| `LendingPool` account and fees                                | about 0.1                                 |
| Demo: top-up of the demo owner and third keypair              | up to 0.4 each, plus fees and Intent rent |
| Demo: test USDC                                               | free (deployer is the mint authority)     |

The peak is the buffer plus the extend, so the deploy script reclaims first. The demo spends roughly 1 SOL in total, most of it as top-ups that stay on the demo owner and the third keypair.

## Privacy notes

- Private: each Position's free SOL, free USDC, collateral, supply shares and debt shares, stored as ciphertexts under the owner's Viewing Key.
- Public: the operation type and amount of each queued operation, the share amount derived from it, the pool totals (`total_supply_shares`, `total_debt_shares`), the indexes, reserves, and the token movements of shield, unshield and liquidation.
- Totals change only by operation amounts that are already visible as token transfers or instruction arguments, so they reveal nothing about per-Position balances.
- The extra bit: a health check or liquidation reveals whether the target Position was unhealthy at that moment, only for Positions someone chose to check.
- A Disclosure reveals only the selected fields, only to the holder of the auditor secret. The mask itself is public.

## Results

Fill in after the run.

- Run date:
- Deploy: final deployer balance:
- `artifacts/devnet-lend-e2e.json` summary:

| Step | Result | Detail | Tx  |
| ---- | ------ | ------ | --- |
|      |        |        |     |

Index movement during the run (supply and borrow index before and after): _to fill_.

Liquidation: _applied or "skipped: no unhealthy position at current price"_.

Circuit weights (`lend_op`, `disclose_position`): _from task 7.1_.
