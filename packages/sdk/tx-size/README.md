# Transaction-envelope size prototypes (Task 2.5)

This suite is the transaction-size half of the Arcium feasibility gate
(Requirement 2.7). For every v1 operation it builds the realistic **queue** and
**callback** transactions, compiles them as versioned (v0) messages with and
without an address lookup table (ALT), and measures the exact serialized byte
length. It asserts every transaction stays under the **1,232-byte hard limit**
and flags any over the **1,150-byte target**.

Run it with `pnpm test:tx-size` from the repository root (or `pnpm test:tx-size`
inside this package). It is wired into the existing suite runner
(`scripts/run-suite.mjs test:tx-size`) and the `rust-arcium` CI job.

## Why this is CI-runnable without a localnet

The transactions are built and serialized entirely in memory with
`@solana/web3.js`. No RPC, no Arcium cluster, and no on-chain execution are
needed, so the whole suite runs in CI even though `arcium test` hangs locally.
Serialized size depends only on:

- the number of accounts and the signer/writable split,
- the instruction data length,
- address-lookup-table membership.

It does **not** depend on the actual 32-byte value of any key, so the prototypes
use deterministic placeholder pubkeys with the real account **counts and layout**
from `design.md`. web3's own `MessageV0.serialize()` encodes into fixed
1,232-byte buffers and throws on an oversized envelope, so the size is computed
analytically from the authoritative compiled message (`v0ByteLength`) using the
Solana wire format; this lets the gate _report_ an over-limit transaction rather
than crash on it.

## What each operation's envelope contains

`accounts.ts` lists, per operation, the full ordered account inventory and the
instruction-data size for both phases:

- **Standard Arcium accounts** are modeled on the `@arcium-hq/client` 0.16.0
  layout proven in `tests/walnut.ts`/`tests/failure-matrix.ts`: the queue path
  carries payer, MXE, mempool, executing pool, computation account, comp-def,
  cluster, fee pool, clock, system program, and the Arcium program; callbacks
  carry the computation/cluster/fee-pool/executing-pool accounts, the
  instructions sysvar, the Arcium signer PDA, and the Arcium program.
- **Custom accounts** come from `design.md`: Market, Asset State, Pool, Position,
  Operation Intent, Pending Shield, Liquidation Record, Recovery Claim, Pending
  Settlement, Disclosure, vaults and vault authority, user/treasury/buyer token
  accounts, the SPL token program, and the Pyth SOL/USD and USDC/USD price
  accounts where the operation reads them.
- Every transaction also carries a `setComputeUnitLimit` and a
  `setComputeUnitPrice` compute-budget instruction (priority fee), which is the
  realistic worst case for the size gate.

Envelopes built from an existing/known instruction are tagged `real` (Position
initialization, shield, unshield, the two-owner liquidation receipts, key rotation); the
rest are tagged `modeled` and constructed from the design account list because
the financial instructions are not implemented until later tasks.

## Compute-unit simulation assumptions (callback)

The callback `setComputeUnitLimit` value is simulated per operation to reflect
the heaviest realistic output-verification plus state writes. These change only
the 4-byte compute-budget argument, not the account layout, but they are
recorded here as an ADR-0003 input:

| Class       | CU limit  | Operations                                              |
| ----------- | --------- | ------------------------------------------------------- |
| light       | 200,000   | utilization-band update, health (reveal-only)           |
| position    | 400,000   | position init, shield, disclosure, key rotation         |
| pool        | 600,000   | supply, withdraw supply, borrow, repay                  |
| cpi         | 700,000   | unshield, reserve withdrawal, recovery settlement (CPI) |
| absorb      | 1,000,000 | absorb + Recovery Claim populate                        |
| liquidation | 1,200,000 | two-Position partial liquidation with two Receipts      |

## ALT findings

All **queue** transactions fit comfortably under the target without an ALT. On
the **callback** side, the operations with the largest encrypted output
(liquidation, absorb) exceed the 1,150-byte target without an ALT; moving the
non-signer, non-program accounts into a lookup table (Arcium already provisions
a per-MXE ALT) brings them back under the hard limit. The two-Position
liquidation callback is the tightest envelope because `design.md` requires the
size test to include both fixed-size Receipt ciphertexts; if additional headroom
is needed, those ciphertexts can be emitted in the callback event (as the design
describes) instead of carried as instruction data, which removes roughly 290
bytes. The authoritative per-operation table is printed by `pnpm test:tx-size`.
