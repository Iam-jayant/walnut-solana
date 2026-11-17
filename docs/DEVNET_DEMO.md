# Devnet balance demo (Task 5.7)

`packages/sdk/demo/balance-demo.ts` runs the Position balance path against the real Arcium devnet MPC cluster (offset 456). It shields SOL and USDC, decrypts the Position locally, unshields USDC, sends one oversized unshield that resolves Zero, unshields wSOL, and unwraps the wSOL to native SOL in a second transaction.

## Run it

Prerequisites: the program is deployed to devnet at the size of the current build (see "Deploying the program"), the MXE keygen is finalized, and the wallet is the program upgrade authority with at least 5 devnet SOL for the first run (circuit uploads and Market creation). Read the RPC URL from the Solana CLI config; never print it or paste it into a file, because a keyed RPC URL contains an `api-key`.

```bash
export ANCHOR_PROVIDER_URL="$(solana config get | awk '/RPC URL/ {print $3}')"
export ANCHOR_WALLET="$HOME/.config/solana/walnut-deployer.json"
export WALNUT_BENCH_CLUSTER_OFFSET=456
pnpm --filter @walnut/sdk demo:balance
```

The demo needs `target/idl/walnut.json` (from `pnpm run build`) and `build/*.arcis`. All output passes through a redactor that replaces `api-key=...` and the literal RPC URL.

## What it does

| Step                                                                                | What is checked                                                                                                                                                                                                                                                                                                                                                                                                      |
| ----------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| (a) Comp defs `init_pool_state`, `init_position_state`, `shield_credit`, `unshield` | Registered and circuit uploads finalized (reuses `bench/program.ts` `ensureCompDef`, which resumes interrupted uploads). `rotate_position_key` is not needed.                                                                                                                                                                                                                                                        |
| (b) Market and Pool                                                                 | Creates a 6-decimal test USDC mint, `initialize_market`, `queue_pool_state_init`, waits for the Pool to become `Active`. Skipped when the Market exists.                                                                                                                                                                                                                                                             |
| (c) Demo owner and Position                                                         | Fresh keypair funded with 0.5 SOL by the deployer, `initialize_position`, waits for `Active`, decrypts a zero Position.                                                                                                                                                                                                                                                                                              |
| (d) Shield                                                                          | Mints 5 test USDC to the owner. Shields 1.234567 USDC, then 0.054321 SOL (System transfer, `SyncNative`, and `shield` in one transaction). Waits for the callback and the Pending Shield to reach `credited`. The vault grows by exactly the amount. Derives the Viewing Key (the owner signs `viewingKeyMessage`, `deriveViewingKey`), decrypts, and asserts `free_usdc` and `free_sol` equal the shielded amounts. |
| (e) Unshield                                                                        | Unshields 1 USDC (status `applied`; token balance and vault change by exactly 1 USDC; decrypted balance falls). An oversized request (free balance plus one base unit) ends `zero`, with no transfer and an unchanged balance. Unshields 0.02 wSOL, then a separate `CloseAccount` transaction (`buildUnwrapWsolTransaction`) returns native SOL; the demo asserts the exact lamport gain.                           |
| (f) Output                                                                          | Prints a summary table with explorer links, latencies, fees, and compute units, and writes `artifacts/devnet-balance-demo.json` (gitignored). Returns the owner's leftover SOL to the deployer.                                                                                                                                                                                                                      |

Every queue transaction sets a 600,000 compute-unit limit. The default 200,000 is not enough: `initialize_position` consumed about 208,000 CU on devnet and the other queue instructions about 340,000 to 360,000 CU. Clients (the SDK action builders and the web app) must set a limit of at least 400,000.

## Resuming and state

The demo is idempotent. Progress lives outside the repository in `~/.walnut-demo/devnet/` (override with `WALNUT_DEMO_DIR`):

- `state.json`: finished steps, the test USDC mint, and any queued-but-unfinished operation (computation offset and sequence), so a rerun resumes the wait instead of sending a second operation.
- `owner.json`, `risk.json`, `treasury.json`, `guardian.json`: devnet keypairs (mode 0600). They are never committed. Delete the directory to start a new demo owner against the same Market.

Reads and sends retry on transient RPC errors (429, timeouts, blockhash expiry).

## Addresses used

| Item                  | Address                                        |
| --------------------- | ---------------------------------------------- |
| Program               | `5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea`  |
| Market                | `Du5ZWegZJj1RvpZ2GHVNgsy2MXAhiThpP3AHWRkJEEGr` |
| wSOL mint (native)    | `So11111111111111111111111111111111111111112`  |
| Test USDC mint        | `GHLMq3gCUHkb9sGPouXNo4Lt9QpGCmz5UMrtF7EwVDLT` |
| wSOL vault            | `8wLfwZZbLUqkqpJ4gu2jdEqKtrqT9D17d1LwPX7EyBvP` |
| USDC vault            | `4Nk1Lvbs5cHZ4V1oVBtG5JRbimEKrPVMxg6M1fL6h9JZ` |
| Demo owner (this run) | `HemKYVjEf4rfqCG9fwDjfyJff4a3sVuBEVKDogRnCnwN` |
| Demo Position         | `3wAtFPYc3A22uyEaEd6AqutegWMXJChPnWoHEndytfyq` |

The test USDC mint is named `WALNUT-DEVNET-TEST-USDC` in the demo output. It is a throwaway 6-decimal mint whose mint authority is the deployer; it is not real USDC. `config/networks/devnet.json` lists the mints and vaults. The Market launch values are: supply cap 1,000,000 USDC, total debt cap 500,000 USDC, per-Position debt cap 10,000 USDC, Position count cap 1,000, debt-Position count cap 250. The risk, treasury, and guardian roles are three distinct generated keys kept in the state directory; the admin is the deployer.

## Results of the recorded run

All callbacks finalized on the first attempt. Latencies are milliseconds: queue is send to confirmed; callback is confirmed to the Intent reaching a terminal status (polled every 1.5 s, so values carry that granularity).

| Step                    | Queue | Callback | Compute units | Transaction                                                                                                                                           |
| ----------------------- | ----- | -------- | ------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| initialize_position     | 1826  | 2108     | 207,679       | [4Ha1bhyW...](https://explorer.solana.com/tx/4Ha1bhyWve1dcDFngsRRWn5ttvHdKXRQtvjaxwu77xfFCiCnVWQ2e4276JCQV6KmQW3G1FprnfQoeX8DRoGVkpcg?cluster=devnet) |
| shield_usdc             | 1243  | 3924     | 360,088       | [GHXncy8F...](https://explorer.solana.com/tx/GHXncy8FdNFB44k5PhPfSCejsifoLYJbHk6LyBqbrWteouuN3yJZMuCpsT2WmnKBEi9UJTkDe2BDkb5MEUGLrhH?cluster=devnet)  |
| shield_sol              | 1359  | 3935     | 340,788       | [2qkivt2S...](https://explorer.solana.com/tx/2qkivt2Se8HRqCQJv7NaeX7gQkziV8z4Js58rbQK4rCDSCCmwWwzSdUrcR41yr1PFUrGM2R7qqS7p7zji1J84xZ1?cluster=devnet) |
| unshield_usdc           | 1642  | 4022     | 343,180       | [SEVtsaZY...](https://explorer.solana.com/tx/SEVtsaZYv1hjYozTBKJ31F3Cb6jaAWFvM2kCdrQraYscTVf6gYqJB1aqVbnSxvueDpt714ipeqbcxNNkpgAG1Vo?cluster=devnet)  |
| unshield_usdc_oversized | 1440  | 2154     | 341,696       | [27vr1aAL...](https://explorer.solana.com/tx/27vr1aALguj1Pk9CBDUAy5miua5sU2QKF1fDMaVEMpSnRVkpQKtuDM8nYpWPh9G17hPSBMFRFGb1Q9S71gdLFVVR?cluster=devnet) |
| unshield_wsol           | 3390  | 2159     | 352,021       | [3myhSYw7...](https://explorer.solana.com/tx/3myhSYw79CdeLPqvwn5KasUXMVY4JNRbQprRnJndYDrQY74spUhDWyQZ6q9M2VihMeSyEuXjKFz8t9hFS9adWqnh?cluster=devnet) |
| unwrap_wsol             | 1808  | n/a      | n/a           | [drVAUwGy...](https://explorer.solana.com/tx/drVAUwGyJ2qBcgoWvRbBDzugfJpSaeYYRVNb44gdKnW9GUYJnktR9uiQQRfrfhGXHagLP6XV5YbCsWtTwqU8A34?cluster=devnet)  |

Decrypted balances matched the expected values at every check: after the two shields `free_usdc = 1,234,567` and `free_sol = 54,321,000`; after the USDC unshield `free_usdc = 234,567`; after the oversized request (234,568) the balance stayed `234,567` and no tokens moved; after the wSOL unshield `free_sol = 34,321,000`. The unwrap returned 20,000,000 lamports of wrapped SOL plus 1,488,440 lamports of account rent, less the 5,000 lamport fee, to the owner.

## Cost

Each transaction pays the 5,000 lamport base fee (no priority fee). The Arcium computation fees and the account rent for the Position, Operation Intents, Pending Shields, computation accounts, and token accounts come out of the owner's balance. The owner spent about 0.0886 SOL in total (0.5 SOL funded, 0.411 SOL returned), most of it recoverable rent.

| Item                                                                                                | SOL (approx.) |
| --------------------------------------------------------------------------------------------------- | ------------- |
| Program extend from 749,200 to 1,842,592 bytes (rent)                                               | 5.55          |
| Upgrade buffer for 1,777,056 bytes (refunded to the deployer on deploy)                             | 9.03 (0)      |
| Circuit uploads, test USDC mint, Market, Pool, Position funding, and test accounts on the first run | 3.96          |

The deployer went from 20.37 SOL before the deployment to 10.84 SOL after the demo (9.53 SOL net, most of it rent that stays locked in the program and circuit accounts).

## Deploying the program

The first devnet deployment was the spike-only build (749,200 bytes). The current build is 1,777,056 bytes, so the program account had to be extended before the upgrade:

```bash
pnpm run build
solana program extend 5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea 1093392   # so_size - current + 64 KiB
solana program write-buffer target/deploy/walnut.so --buffer ~/.config/solana/walnut-buffer.json \
  --use-rpc --with-compute-unit-price 50000 --max-sign-attempts 500        # rerun the same command to resume
solana program deploy --program-id target/deploy/walnut-keypair.json \
  --buffer ~/.config/solana/walnut-buffer.json \
  --upgrade-authority ~/.config/solana/walnut-deployer.json --use-rpc
solana program show --buffers                                              # must list no stray buffers
```

The public devnet RPC cannot carry the buffer write; use a keyed RPC set in the Solana CLI config and keep that URL out of logs and files. The MXE is never redeployed, closed, or recreated.

## Known limits

- The run proves the shield, decrypt, unshield, oversized-unshield, and unwrap path on the shared devnet cluster once. It does not exercise rotation, Pending Shield refunds, or concurrent operations on devnet.
- Latencies come from one run on a shared cluster and are not a benchmark. Use `pnpm --filter @walnut/sdk bench:latency` for distributions.
- The demo exposes the owner key in the state directory so it can be resumed. That is acceptable for throwaway devnet keys only.
- The test USDC mint is controlled by the deployer, so anyone with that key can mint unlimited test USDC. Do not treat it as a value-bearing asset.
- Fees shown in the table are base fees only; the computation fee and rent appear in the owner's balance change.
