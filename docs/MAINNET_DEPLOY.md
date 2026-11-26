# Mainnet Deployment Plan

Status: plan, not yet executed. Covers the new mainnet program id and hosting the
Arcium circuits off-chain on GitHub Releases. Read with
[`MAINNET_COST.md`](MAINNET_COST.md) and [`DEVNET_DEPLOY.md`](DEVNET_DEPLOY.md).

## 1. Decisions

| Decision          | Choice                                                                                       |
| ----------------- | -------------------------------------------------------------------------------------------- |
| Program id        | New keypair for mainnet. Devnet keeps `5Eunnix…jTcjea`.                                      |
| Circuit storage   | Off-chain: `CircuitSource::OffChain`, with the `.arcis` files hosted as GitHub Release assets |
| Arcium cluster    | Mainnet cluster offset `2026` (see `Arcium.toml`)                                            |
| Circuits deployed | Only the 7 active circuits. The legacy and spike circuits are not registered on mainnet.     |

## 2. Can a comp def use only a hash? (Arcium docs, checked 2026-11-26)

- **A hash alone is not enough.** `init_computation_def` takes an optional
  `CircuitSource`. `None` means the circuit bytes are uploaded to on-chain
  `ComputationDefinitionRaw` accounts. That is what every `init_*_comp_def` in
  `programs/walnut/src/lib.rs` does today.
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

## 3. Program change (not yet applied)

Each active `init_*_comp_def` changes from `None` to an off-chain source:

```rust
use arcium_client::idl::arcium::types::{CircuitSource, OffChainCircuitSource};
use arcium_macros::circuit_hash;

const CIRCUITS_BASE_URL: &str =
    "https://github.com/Iam-jayant/walnut-solana/releases/download/circuits-v1";

pub fn init_lend_op_comp_def(ctx: Context<InitLendOpCompDef>) -> Result<()> {
    init_computation_def(
        ctx.accounts,
        Some(CircuitSource::OffChain(OffChainCircuitSource {
            source: format!("{CIRCUITS_BASE_URL}/lend_op.arcis"),
            hash: circuit_hash!("lend_op"),
        })),
    )?;
    Ok(())
}
```

Put this behind a Cargo feature, for example `offchain-circuits`. Devnet, localnet
and CI then keep the `None` path, which `arcium test` relies on.

`circuit_hash!` reads `build/<name>.hash` at compile time. The program and the
uploaded files must therefore come from **the same `arcium build`**. If you rebuild
the circuits, you must also rebuild the program and upload to a new release tag.

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
arcium build
sha256sum build/{lend_op,disclose_position,rotate_position_key,pool_op,position_private,liquidate_pool,pool_band}.arcis > SHA256SUMS
gh release create circuits-v1 --title "Walnut circuits v1" --notes "Arcium circuits for mainnet program <PROGRAM_ID>" \
  build/{lend_op,disclose_position,rotate_position_key,pool_op,position_private,liquidate_pool,pool_band}.arcis SHA256SUMS
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

1. `arcium build` with `--features offchain-circuits` and the mainnet keypair.
2. Create the `circuits-v1` GitHub Release and verify every hash over plain
   HTTP.
3. Run a dry run on devnet: deploy a throwaway program id with the same feature,
   register one off-chain comp def, and run one computation end to end. This
   answers the redirect question in practice.
4. `arcium deploy --cluster-offset 2026 …` on mainnet.
5. Call the 7 active `init_*_comp_def` instructions. Do not call the legacy ones.
6. Initialize the market and pool, check MXE keygen, then open behind caps.

## 7. Further savings

- **Program size:** `walnut.so` is about 3.46 MB, so program rent is about 24
  SOL. A deploy needs about the same again in a temporary, recoverable buffer.
  Removing the spike and legacy instructions (`add_together`, `init_position`,
  `apply_position_delta`, `liquidation_receipts`, `oversized_output`,
  `rotate_viewing_key` and their accounts) from the mainnet build, behind a
  feature flag, should shrink the program a lot.
- Build with `overflow-checks` kept on and `opt-level = "z"` or `"s"` for the
  program crate. Measure size against compute units before choosing.
