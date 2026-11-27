# Mainnet Deployment Plan

Status: the build is implemented; nothing is deployed. Covers the new mainnet
program id and hosting the Arcium circuits off-chain on GitHub Releases. Read with
[`MAINNET_COST.md`](MAINNET_COST.md) and [`DEVNET_DEPLOY.md`](DEVNET_DEPLOY.md).

## 1. Decisions

| Decision            | Choice                                                                                        |
| ------------------- | --------------------------------------------------------------------------------------------- |
| Program id          | New keypair for mainnet. Devnet keeps `5Eunnix…jTcjea`.                                       |
| Circuit storage     | Off-chain: `CircuitSource::OffChain`, with the `.arcis` files hosted as GitHub Release assets |
| Arcium cluster      | Mainnet cluster offset `2026` (see `Arcium.toml`)                                             |
| Circuits registered | The 11 circuits of the confidential path (see [`MAINNET_COST.md`](MAINNET_COST.md))           |
| Program build       | `scripts/build-mainnet.sh`: no `legacy` feature, size-optimized                               |

## 2. Can a comp def use only a hash? (Arcium docs, checked 2026-11-26)

- **A hash alone is not enough.** `init_computation_def` takes an optional
  `CircuitSource`. `None` means the circuit bytes are uploaded to on-chain
  `ComputationDefinitionRaw` accounts. That is the default build. The
  `offchain-circuits` feature (section 3) switches the 11 live ones.
- **`CircuitSource::OffChain` is the supported way to avoid on-chain bytes.** It
  takes a **public URL and the SHA-256 hash**. Arx nodes download the file and
  check it against the hash. Source:
  [computation-def-accs](https://docs.arcium.com/developers/program/computation-def-accs)
  and [deployment](https://docs.arcium.com/developers/deployment).
- **On-chain upload is not required** for off-chain sources. The docs do not
  describe an on-chain upload or finalize step for them. Only on-chain circuits
  need `arcium close-computation-definition-buffers` afterwards.
- **The hash is the SHA-256 of the raw `.arcis` file.** `arcium build` writes it
  to `build/<circuit>.hash`. We checked this locally: `sha256(build/lend_op.arcis)`
  equals `build/lend_op.hash`. The docs say to use `circuit_hash!("<name>")` and
  never a placeholder such as `[0u8; 32]`.
- **Hosting requirement:** the files must be "publicly readable without
  authentication". The docs do not cover content-type, HTTP redirects, size
  limits or approved hosts.
- **Open questions for Arcium (ask on Discord before mainnet):**
  1. Do mainnet Arx nodes accept `OffChain` sources? The mainnet section of the
     docs says nothing either way.
  2. Do the nodes follow HTTP redirects? GitHub Release asset URLs return a 302
     to `objects.githubusercontent.com`.
  3. Is there a maximum circuit file size, or a timeout for fetching one?

## 3. The slim mainnet build

```bash
scripts/build-mainnet.sh                  # Linux or WSL2; builds, deploys nothing
```

It writes `artifacts/mainnet/`: `walnut.so`, `walnut.json` (IDL) and
`circuits/` with `SHA256SUMS`. It also overwrites `target/deploy/walnut.so`,
so run `arcium build` again before devnet or localnet work.

Cargo features in `programs/walnut/Cargo.toml`:

- `legacy` (default): the spike circuits, `position_op`, the public-amount
  `lend_op` and `liquidate`, the v1 supply flows, reserve withdrawal and empty
  pool recovery. The mainnet build leaves it out (41 of 103 instructions).
- `offchain-circuits`: each `init_*_comp_def` registers a release URL and
  `circuit_hash!` instead of expecting an on-chain upload. The URL is set in the
  `circuit_source!` macro in `programs/walnut/src/lib.rs`.

Kept: governance and pause controls, the balance flows (shield and unshield), the
private pool, collateral and liquidation flows, health check, disclosure, key
rotation, and every retry, refund and abandon path.

### What the slim build changes for users

- **Private mode only.** The public-amount instructions are gone. The web app
  still has that fallback UI and uses it when private mode is not detected, so
  finish setup (all comp defs, Active pool, band rate source) before opening it.
- **No protocol revenue withdrawal.** `reserve_withdrawal` reads the old
  `reserves` field, which stays zero on the private path (ADR-0004). Reserves
  accumulate inside the encrypted pool. Withdrawing them needs a new circuit and
  a program upgrade. The devnet build has the same limit in private mode.

## 4. GitHub Release rules

- Use a dedicated tag per circuit build, for example `circuits-v1`, then
  `circuits-v2`. **Never replace an asset under an existing tag.** The on-chain
  hash is fixed, so a replaced file breaks every computation that uses it.
- Upload only the active circuit files, named exactly `<circuit>.arcis`.
- Attach `SHA256SUMS` (from `sha256sum build/*.arcis`) so anyone can check the
  files.
- The repository must be **public**. Private repositories need authentication
  for release assets, and the nodes cannot download them.
- Keep a second copy elsewhere (an IPFS pin or a public bucket). The comp def URL
  cannot be changed after registration.
- GitHub Releases allow up to 2 GiB per asset. The largest active circuit is
  about 1.2 MB.

```bash
scripts/build-mainnet.sh
cd artifacts/mainnet/circuits
gh release create circuits-v1 --title "Walnut circuits v1" --notes "Arcium circuits for mainnet program <PROGRAM_ID>" ./*.arcis SHA256SUMS
# Check that the files download without authentication and match the hashes
curl -sL https://github.com/Iam-jayant/walnut-solana/releases/download/circuits-v1/lend_op.arcis | sha256sum
```

## 5. New mainnet program id

1. Generate the key offline: `solana-keygen new -o ~/walnut-mainnet/walnut-keypair.json`.
   Store it in your secrets vault and never commit it (`*keypair*.json` is
   gitignored).
2. Before the mainnet build, copy it to `target/deploy/walnut-keypair.json`.
   `arcium build` then re-syncs `declare_id!`. Keep the devnet keypair backed up
   separately, because the same path is used for both.
3. Add `[programs.mainnet]` to `Anchor.toml`. Uncomment `[clusters.mainnet]` in
   `Arcium.toml`.
4. Add `config/networks/mainnet.json` with the new id and cluster offset `2026`.
   Update `web` and `@walnut/sdk` network profiles and the `NEXT_PUBLIC_*` env.
5. Regenerate the IDL (`web/lib/idl/`) from the mainnet build.
6. Deploy with a hardware wallet or multisig as upgrade authority. Afterwards,
   move the upgrade authority to a Squads multisig.

## 6. Order of operations

1. Dry run on devnet with a throwaway program id: build, publish a test release,
   deploy, register the comp defs and run
   `pnpm --filter @walnut/sdk demo:private`. This is the first runtime test of
   the size-optimized binary. Check that every queue and callback stays within
   its compute limit, and that Arx nodes can fetch the circuits.
2. `scripts/build-mainnet.sh` with the mainnet keypair in place.
3. Create the `circuits-v1` GitHub Release and verify every hash over plain HTTP.
4. `arcium deploy --cluster-offset 2026 …` on mainnet, then check that the
   deployed program is the one in `artifacts/mainnet/`.
5. Register the 11 comp defs, with the mainnet RPC and wallet configured:
   `WALNUT_ALL_CIRCUITS=1 node packages/sdk/scripts/lend-deploy.ts comp-defs run`
   (it reads the IDL from `target/idl/walnut.json`).
6. Initialize the market, pool state and lending pool, then
   `lend-deploy.ts private-setup`. Market initialization has no mainnet script
   yet; today it lives in the devnet demo `balance-demo.ts`.
7. Point the web app at mainnet (`web/lib/config.ts`, IDL, oracle, RPC), then
   open behind caps.

## 7. Verified so far (2026-11-26)

- The slim binary (1,684,552 bytes) deploys to a local `solana-test-validator`.
- All 62 slim instructions match the web app's IDL (discriminators, arguments,
  account names).
- The default build is byte-for-byte the same size as before the change. Rust
  format, clippy and tests, TypeScript type checks and unit tests, and the web
  production build pass.
- **Not verified:** any MPC computation against the slim binary, off-chain
  circuit fetching, and compute usage under `opt-level = "z"`.
