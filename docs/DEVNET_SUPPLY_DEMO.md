# Devnet supply demo (Task 8.5)

`packages/sdk/demo/supply-demo.ts` runs the USDC supply path against the real Arcium devnet MPC cluster (offset 456) on the existing devnet Market and test USDC mint. It shields test USDC, refreshes the utilization band, supplies, checks equity neutrality, sends one over-withdrawal that resolves Zero, withdraws all supply shares, and unshields the USDC back to the owner's token account.

## Interest scenario (read this first)

Devnet has no borrowers until Task 9, so real interest is zero and the demo does not fake any. What it shows is that the claim stays exactly equal to the supplied amount: the over-withdrawal forces a queue-time index accrual and the circuit's interest checkpoint, the accounting Borrow Index may move, and yet with zero Debt Shares no interest reaches suppliers, so `claim = floor(shares x equity / total shares)` is still the supplied amount. The final withdrawal then pays exactly that claim and the demo checks the decrypted Free USDC and the token balance to the base unit.

Nonzero accrual is shown elsewhere, with the real code and no mocks:

- `programs/walnut/src/math/supply/tests.rs`: `a_supplier_just_after_an_accrual_gets_no_earlier_interest` (equity 1,950,000 after a doubled index; a 195,000 supply mints exactly 10% and can exit with exactly 195,000; after a later accrual its claim is 195,000 + 40,909), `a_supplier_just_before_an_accrual_shares_the_later_interest`, and `prop_a_new_supplier_never_captures_earlier_interest`.
- `packages/reference-model/test/supply.test.ts` (the same numbers on the TypeScript model) and `state/supply_checkpoint.rs` (`NOHIST`, `NAV`, `LEDGER` after every step of 110,000+ sequences).
- Task 9's devnet demo will show nonzero accrual on-chain once borrowing exists.

## Who can decrypt the claim

A Position is decryptable only by its owner. The Pool is `Enc<Mxe>`: neither the owner nor anyone else can read supplier equity or total supply shares. The owner therefore decrypts their supply shares, and computes a claim only when the NAV is known. The demo owner is the only supplier, so equity is the supplied Cash and total shares are its own shares; `claimForShares` (`packages/sdk/src/supply.ts`) is the circuit's exact formula and the withdrawal then proves it (an unexpected supplier would make the equality check fail loudly, not silently).

## Run it

Prerequisites: the program is deployed to devnet at the size of the current build with the Task 8 circuits (see "Deploying the Task 8 build"), the MXE keygen is finalized, and the wallet is the program upgrade authority with at least 3 devnet SOL (two circuit uploads and the owner's rent). The demo reuses the Market, the test USDC mint, and the demo owner of the balance demo from `~/.walnut-demo/devnet/state.json` when they exist, and keeps its own progress in `supply-state.json` beside it. Read the RPC URL from the Solana CLI config; never print it or paste it into a file.

```bash
export ANCHOR_PROVIDER_URL="$(solana config get | awk '/RPC URL/ {print $3}')"
export ANCHOR_WALLET="$HOME/.config/solana/walnut-deployer.json"
export WALNUT_BENCH_CLUSTER_OFFSET=456
pnpm --filter @walnut/sdk demo:supply
```

All output passes through the redactor (`api-key=...` and the literal RPC URL are replaced). Every queue transaction sets an 800,000 compute-unit limit (shield used 340,000 to 360,000 CU on devnet; supply is heavier, and clients must set at least 600,000).

## What it does

| Step                      | What is checked                                                                                                                                                                                                                                    |
| ------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| (a) Comp defs             | Registers and uploads `update_utilization_band` and `supply_flow` (and re-checks the four earlier circuits) with `ensureCompDef`, which resumes interrupted uploads. Market and Pool are reused; they are never recreated.                         |
| (b) Position              | Reuses the demo owner's Active Position, or creates one.                                                                                                                                                                                           |
| (c) Shield                | Mints 3 test USDC, shields 2 USDC. The vault grows by exactly 2 USDC and the decrypted Free USDC grows by exactly 2 USDC.                                                                                                                          |
| (d) Band                  | Supply is risk-increasing, so a fresh band is required. If the stored band is older than the maximum age, runs `update_utilization_band` (Pool-owned counter) and waits for `applied`.                                                             |
| (e) Supply                | Supplies 1.5 USDC. Intent `applied`; Free USDC falls by exactly 1.5 USDC; supply shares minted (1,500,000 x 1,000 on a canonically empty Pool).                                                                                                    |
| (f) Equity neutrality     | An over-withdrawal (shares + 1) resolves `zero`: shares unchanged, nothing stored. Prints the accounting index before and after (it accrues; no Debt Shares, so no interest). The claim computed with `claimForShares` equals the supplied amount. |
| (g) Withdraw and unshield | Withdraws all shares (`applied`); Free USDC rises by exactly the claim; shares are zero. Unshields 1.5 USDC: the token account and the vault change by exactly 1.5 USDC.                                                                           |
| (h) Output                | Summary table with explorer links, latencies, fees, and compute units, and `artifacts/devnet-supply-demo.json` (gitignored). Returns the owner's leftover SOL to the deployer.                                                                     |

If the Pool holds a non-canonical residue, supply ends `zero` and the demo stops with an explanatory error. If another supplier shares the Pool, the exact-claim checks fail loudly rather than pass on a guess. After the demo the Pool is empty again (Cash 0, shares 0), so the first-supplier rule is satisfied on the next run.

## Deploying the Task 8 build

The Task 8 build is 2,218,400 bytes; the deployed program account holds 1,842,592 bytes, so it must be extended by at least 375,853 bytes (the 45-byte ProgramData header counts). The MXE and Market already exist and are not touched.

```bash
pnpm run build
solana rent 2218400                                   # buffer rent (about 11.27 SOL, refunded on deploy)
solana program extend 5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea 400000   # about 2.03 SOL of rent stays locked
solana program write-buffer target/deploy/walnut.so --buffer ~/.config/solana/walnut-buffer.json \
  --use-rpc --with-compute-unit-price 50000 --max-sign-attempts 500        # rerun the same command to resume
solana program deploy --program-id target/deploy/walnut-keypair.json \
  --buffer ~/.config/solana/walnut-buffer.json \
  --upgrade-authority ~/.config/solana/walnut-deployer.json --use-rpc
solana program show --buffers                                              # must list no stray buffers
```

Funds needed before starting: the buffer rent (11.27 SOL) plus the extend rent (about 2.03 SOL) plus a 1 SOL margin, about 14.3 SOL; the buffer rent returns on deploy. Keep the deployer above 1 SOL at all times. The public devnet RPC cannot carry the buffer write; use a keyed RPC set in the Solana CLI config.

## Status

The code is complete and type-checked, and every rule it relies on is covered by the Rust and TypeScript tests. The devnet upgrade and the recorded run are not part of this change: the upgrade spends real devnet SOL and was not executed by the automated session that wrote this (the deployer held 20.84 SOL, enough for the 14.3 SOL needed). Run the commands above, then the demo, and record the step table here.

## Known limits

- One run on a shared cluster proves the path once; latencies are not a benchmark.
- The demo exposes the owner key in the state directory so it can be resumed. That is acceptable for throwaway devnet keys only.
- The test USDC mint is controlled by the deployer; do not treat it as a value-bearing asset.
- The demo does not exercise `recover_empty_pool`; it needs a Pool with rounding dust and is covered by the Rust tests (`state/recovery.rs`, `prop_recovery_only_reclassifies_dust_as_reserves`, and the `supply_checkpoint` dust world).
