# ADR-0004: Private operation amounts without secret division

Status: accepted (feasibility passed). Supersedes the public-amount arguments of ADR-0002 for supply, withdraw supply, borrow, repay and liquidation.

## Context

`supply_flow` (encrypted NAV, secret divisions) measured 20,736,187,008 CU and Arcium rejected it. The walnut-diffn and walnut-lend work then shipped working flows with public amounts and public pool totals (`lend_op`, `LendingPool`). Public totals leak every amount, because one operation moves the total by exactly its amount.

## Decision

1. **Verify, never divide.** The client knows the public indexes. It computes the share count off chain and encrypts `(amount, shares)` to the MXE. The circuit checks the conversion with multiplications and comparisons only:
   - supply: `shares*sidx <= amount*1e9 < (shares+1)*sidx`
   - withdraw supply and borrow: `shares*idx >= amount*1e9 > (shares-1)*idx`
   - repay: `shares*bidx <= amount*1e9 < (shares+1)*bidx`
   - liquidation seize: `seize*price*1e4 <= amount*1e9*(1e4+bonus) < (seize+1)*price*1e4`
2. **Encrypted Pool, public indexes.** The Pool keeps `cash`, `total_supply_shares` and `total_debt_shares` encrypted (`Enc<Mxe, PoolSecret>`, existing layout; `reserves` stays unused at zero). Reserves are implicit: `cash + debt_value - supplier_equity`, so no interest checkpoint circuit runs on the Pool. Public `supply_index` and `borrow_index` advance from the revealed utilization band, using the band's upper edge for borrowers and its lower edge for suppliers. The surplus stays in the Pool as reserves, so suppliers are never over-credited.
3. **Private liquidation.** The liquidator pays from its own encrypted free USDC and receives the seized lamports into its own encrypted free SOL. No SPL transfer, no escrow, no refund path. Borrower, liquidator and Pool change in one circuit.
4. **What stays public:** wallet addresses, timing, the operation kind, shield and unshield amounts (SPL transfers), the one `applied` bit of each computation, the health-check bit and the utilization band.
5. **Events and intents carry no amounts.** The Intent input commitment hashes ciphertexts and public parameters only.

## Feasibility measurement (task P.1)

| Circuit                                                                               | Weight (CU)   | Gates | Reference                                                      |
| ------------------------------------------------------------------------------------- | ------------- | ----- | -------------------------------------------------------------- |
| `pool_op` (supply, withdraw supply, borrow, repay; Pool + Position + encrypted input) | 798,522,416   | 5,086 | `supply_flow` 20,736,187,008; `lend_op` 558,589,580 (deployed) |
| `liquidate_pool` (Pool + borrower + liquidator + encrypted input)                     | 1,037,952,272 | 4,923 |                                                                |

| `position_private` (add and withdraw collateral, encrypted lamports) | 644,454,740 | 4,392 | |

All three are about 20 times below the rejected `supply_flow` and in the range of the deployed circuits.

## Program (tasks P.3 and P.4)

Instructions: `pool_op(op, accrue_to_ts, input_pubkey, input_nonce, input_ciphertexts[2])`, `position_private(op, accrue_to_ts, input_*[2])`, `liquidate_pool(accrue_to_ts, input_*[3])`, their callbacks, `init_*_comp_def`, and `abandon_private_op`. Rules: `programs/walnut/src/state/private_op.rs`.

- **Index agreement.** The client picks `accrue_to_ts` (not in the future, at most 120 s old, not before the last accrual) and projects the LendingPool indexes to it. The queue accrues to exactly that time, so both sides use the same indexes. A mismatch only makes the circuit refuse.
- **Locks.** `pool_op` locks the Pool and the Position, `position_private` the Position, `liquidate_pool` the Pool, the target and the liquidator's Position, all or none. `Applied` stores every payload and bumps every generation.
- **Binding.** The accounts commitment covers payer, Market, Asset State, LendingPool, Pool, and both Positions with generations. The inputs commitment covers the stored payloads, Viewing keys, epochs, output-nonce inputs and the public parameters. The client ciphertext is immutable inside the queued computation and is not bound separately.
- **Events.** `PrivateOpQueued`, `PrivateOpSettled` (one `applied` bit) and `PrivateOpAbandoned` carry the kind and op code only.
- **Not enforced on the private path:** the total-debt cap (the debt total is encrypted). The per-Position debt cap and the supply cap are enforced inside the circuits.

## Interest from the band (task P.5)

- `LendingPool.rate_source` (byte 189, formerly reserved and zero, so existing accounts read as `0`): `0` accrues from the public totals (walnut-lend path), `1` from the revealed utilization band. The admin switches it with `set_lending_rate_source`, which first accrues to now with the old source.
- Band mode (`LendingPool::accrue_banded`): borrow index at the kinked rate of the band's upper edge, rounded up; supply index at `rate(lower edge) * lower edge * (1 - reserve factor)`, rounded down. A property test checks that suppliers are never credited more than borrowers pay for any utilization inside the band. Reserves are the implicit surplus inside the encrypted Pool.
- The existing `update_utilization_band` circuit is reused unchanged. When an optional `lending_pool` account in band mode is passed, the band is measured at `borrow_index_1e9 * 1e9` (WAD), which is exact for debt shares booked at the 1e9 index. The circuit index is bound through `risk_borrow_index_at_expiry`.
- A band-mode borrow requires a fresh band (`band_max_age_slots`), so borrowers never accrue at a stale band. Repay, withdrawals and collateral never wait on the band.
- Every accrual (`lend_op`, `check_health`, `liquidate` and the private path) goes through `accrue_auto`, so both paths move the same indexes the same way.

## SDK (task P.6)

`packages/sdk/src/private-op.ts`: conversions and mirrors of the circuit checks, `encryptPrivateInput` (fresh ephemeral x25519 key and nonce per input), `chooseAccrualTs`, and builders for `pool_op`, `position_private`, `liquidate_pool`, `abandon_private_op` and `set_lending_rate_source`. `lending-pool.ts` decodes `rateSource` and mirrors band-mode accrual (`projectIndexesBanded`, `projectIndexesAuto`), with golden vectors shared with the Rust tests.

## Consequences

- `lend_op`, `LendingPool` public totals and `liquidate` stay in the program until the private path is cut over; they are not deleted.
- The UI market card switches from exact totals to the utilization band.
- A liquidator needs its own Position.
- Rounding stays against the user: debt up, supply shares down, seize down.
