# Oracle checkpoint (Task 6.5)

Status of the Pyth oracle core after Tasks 6.1 to 6.5: what is tested, how each
requirement maps to a test, what the localnet fixture does, the real devnet
read, and what remains for Task 9.

Authority: `programs/walnut/src/oracle/mod.rs` (`validate_price`). The TypeScript
mirror in `packages/sdk/src/oracle.ts` ports the same rules with `bigint` for
the SDK, the demo, and the fixture; golden vectors appear in both test suites.

## Requirement to test map

Rust tests live in `programs/walnut/src/oracle/mod.rs` (`oracle::tests::`).
TypeScript tests are in `packages/sdk/test/oracle.test.ts` (SDK) and
`tests/oracle-fixture.ts` (localnet fixture, mocha).

| Requirement                           | Rust tests                                                                                                                                                                                                                                                                                                                                                                                                                   | TypeScript                                                                                                                                                                                                                                                                                                                                    |
| ------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 6.1 owner and feed id                 | `wrong_owner_is_rejected_by_the_pipeline`, `wrong_owner_is_reported_before_a_wrong_feed`, `wrong_feed_id_single_bit_is_rejected_by_the_pipeline`, `rejects_a_foreign_owner`, `rejects_a_wrong_feed_id`                                                                                                                                                                                                                       | `wrong owner and wrong feed id are rejected`                                                                                                                                                                                                                                                                                                  |
| 6.2 age 5 to 300 s                    | `age_exactly_max_passes_and_one_more_second_is_stale` (5, 60, 300), `publish_time_equal_to_now_passes_and_future_time_fails`, `rejects_future_and_negative_times`, `param_bounds_are_enforced_on_every_field`                                                                                                                                                                                                                | `age boundary: max passes, max+1 and future fail`                                                                                                                                                                                                                                                                                             |
| 6.3 confidence 0.1% to 10%            | `confidence_exactly_at_the_bps_bound_passes_and_one_unit_more_fails` (10, 200, 1000 bps), `confidence_check_does_not_overflow_at_u64_extremes`, `collateral_with_confidence_equal_to_price_is_rejected`                                                                                                                                                                                                                      | `confidence boundary is exact at 200 bps`                                                                                                                                                                                                                                                                                                     |
| 6.4 positive, exponent, overflow      | `zero_price_is_rejected`, `negative_price_is_rejected_including_i64_min`, `zero_or_negative_ema_is_rejected`, `exponent_below_min_and_above_max_is_rejected_by_the_pipeline`, `exponent_window_edges_are_accepted_by_normalization`, `pow10_covers_the_whole_exponent_window`, `normalization_overflow_is_an_error_not_a_wrap`, `price_plus_confidence_overflow_is_an_error`, `normalized_price_domain_boundaries_are_exact` | `zero and negative prices are rejected`, `exponent outside -12..12 is rejected`, `normalization overflow and domain edges`, `price plus confidence beyond u64 never reaches normalization`                                                                                                                                                    |
| 6.5 conservative direction            | `collateral_rounds_down_and_debt_rounds_up_through_the_pipeline`, `collateral_uses_price_minus_conf_and_debt_uses_price_plus_conf`, `validates_sol_as_collateral_and_usdc_as_debt`                                                                                                                                                                                                                                           | `golden: SOL collateral 149.995 and USDC debt 1.0001`, `golden: collateral rounds down (99) and debt rounds up (101)`                                                                                                                                                                                                                         |
| 6.6 EMA and last-good breakers        | `spot_versus_ema_deviation_boundary_is_exact_in_both_directions`, `spot_versus_ema_extreme_deviation_is_rejected`, `spot_versus_last_good_deviation_boundary_is_exact`, `deviation_check_handles_zero_reference_and_overflow`                                                                                                                                                                                                | `spot-versus-EMA deviation boundary is exact in both directions`                                                                                                                                                                                                                                                                              |
| 6.7 anomaly bumps generation, blocks  | `entering_anomaly_is_a_normal_to_anomaly_transition`, `anomaly_blocks_price_sensitive_operations_only`, `price_sensitive_classification_matches_the_design`                                                                                                                                                                                                                                                                  |                                                                                                                                                                                                                                                                                                                                               |
| 6.8 pending callbacks go `Stale`      | `an_oracle_generation_change_stales_the_callback_before_expiry`, `stale_oracle_generation_takes_precedence_over_expiry` (real `classify_callback`)                                                                                                                                                                                                                                                                           |                                                                                                                                                                                                                                                                                                                                               |
| 6.9 resume only after both feeds pass | `prop_anomaly_bumps_generation_stales_pending_and_resume_needs_both_feeds`                                                                                                                                                                                                                                                                                                                                                   |                                                                                                                                                                                                                                                                                                                                               |
| 6.10 update flows                     | Task 6.4: `packages/sdk/test/pyth.test.ts`                                                                                                                                                                                                                                                                                                                                                                                   | `pyth.test.ts`                                                                                                                                                                                                                                                                                                                                |
| 6.11 Decision Expiry                  | `decision_expiry_slot_is_inclusive_for_a_price_sensitive_callback` (slot 100 applies, 101 is `Expired`), `a_price_sensitive_intent_captures_publish_times_and_oracle_generation`                                                                                                                                                                                                                                             |                                                                                                                                                                                                                                                                                                                                               |
| Layout and fixture                    |                                                                                                                                                                                                                                                                                                                                                                                                                              | `PriceUpdateV2 encodes to the golden bytes and round-trips`, `a partial verification level round-trips...`, `parsing rejects a short account and a bad discriminator`, `Hermes accumulator decode extracts the price message`, `feed ids are 32 bytes and a parsed account validates end to end`; mocha `oracle fixture (Task 6.5)` (3 tests) |

Property tests (deterministic LCG, no new crates; run by
`pnpm run test:rust:property`):

| Test                                                                       | Property                                                                                                                                                                                        |
| -------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `prop_pipeline_is_the_conjunction_of_its_checks` (6.1)                     | Task 6.1 property: age, confidence, positivity conjunction.                                                                                                                                     |
| `prop_validate_price_accepts_iff_every_rule_holds`                         | 60,000 random valid and invalid views and parameters (extreme mantissas, exponents, times): `validate_price` accepts exactly when an independently written rule list does, with the same value. |
| `prop_low_never_exceeds_spot_and_spot_never_exceeds_high`                  | For every accepted view, `low <= floor(spot) <= high`.                                                                                                                                          |
| `prop_normalization_is_monotone_in_the_mantissa`                           | Monotone in the mantissa for floor and ceil; `ceil - floor <= 1`.                                                                                                                               |
| `prop_no_panics_on_the_full_input_domain`                                  | No panic or wrap for any `i64`, `u64`, `i32`, `u16`, `u32` input.                                                                                                                               |
| `prop_anomaly_bumps_generation_stales_pending_and_resume_needs_both_feeds` | Entering anomaly bumps the generation and stales a pending callback; a callback at the current generation applies; resume only when both feeds pass in one observation.                         |

Bugs found: none. The review of `validate_price`, `normalize_price`,
`conservative_mantissa`, `check_age`, and `deviation_within` against the new
boundary and extreme-domain tests produced no failure.

## Localnet signed-update fixture

`tests/oracle-fixture.ts` (mocha, CI only, no validator or provider needed).

No program change is needed. The program pins the receiver id as the constant
`PYTH_RECEIVER_PROGRAM_ID`, and `solana-test-validator --account <addr> <file>`
can preload an account owned by ANY program id, including a program that is
absent from the ledger. The fixture therefore writes accounts with the genuine
`PriceUpdateV2` layout (discriminator, `Full` verification level, feed id, price
message) owned by `rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ`:

- `target/oracle-fixture/sol-usd.json` (150.00, conf 0.005, exponent -8)
- `target/oracle-fixture/usdc-usd.json` (1.00, conf 0.0001, exponent -8)

Properties: deterministic addresses (from labelled seeds), a file is rewritten
only when its bytes change, publish time defaults to "now" or
`ORACLE_FIXTURE_PUBLISH_TIME`, and the encoder is pinned to a golden byte vector
shared with `packages/sdk/test/oracle.test.ts`. "Signed" here means the account
shape the receiver writes after Wormhole verification; the receiver's signature
check is not exercised on localnet because the receiver program is absent.

To use it in a localnet run (needs Task 9's price-sensitive instruction):

```sh
ORACLE_FIXTURE_PUBLISH_TIME=$(date +%s) pnpm exec ts-mocha -p ./tsconfig.json tests/oracle-fixture.ts
solana-test-validator \
  --account <sol-usd address> target/oracle-fixture/sol-usd.json \
  --account <usdc-usd address> target/oracle-fixture/usdc-usd.json
```

The addresses are printed by the test. `Anchor.toml` is unchanged; wiring the
accounts into `[[test.validator.account]]` is a Task 9 decision because it needs
a stable publish time (the age bound is at most 300 s).

## Real devnet update

Script: `pnpm --filter @walnut/sdk demo:oracle` (`packages/sdk/demo/oracle-devnet.ts`).

Finding: Hermes (`hermes.pyth.network`, including `hermes-beta`) now answers
HTTP 401 without a Pyth API key, so a keyless signed-update fetch is not
possible. The script instead reads the REAL `PriceUpdateV2` accounts Pyth's push
oracle keeps on devnet (shard 0 sponsored feeds, PDAs of
`pythWSnswVUd12oZpeFP8e9CVaEqJg25g1Vtc2biRsT`), which are owned by the receiver
program. If `HERMES_API_KEY` is set it also decodes and validates a signed
Hermes accumulator update.

Feed ids (same on devnet and mainnet, recorded in `packages/sdk/src/oracle.ts`):

- SOL/USD `ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d`
- USDC/USD `eaa020c61cc479712813461ce153894a96a6c00b21ed0cfc2798d1f9a9e9c94a`

Run of 2026-11-17 against public devnet (slot 523318815, cluster time
1794930463):

| Feed     | Account                                        | Price       | Confidence           | Exponent | Age   | Strict (60 s) | Age-neutral                      |
| -------- | ---------------------------------------------- | ----------- | -------------------- | -------- | ----- | ------------- | -------------------------------- |
| SOL/USD  | `7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE` | 72.96000122 | 0.02818575 (0.0386%) | -8       | 855 s | StalePrice    | PASS, collateral low 72.93181547 |
| USDC/USD | `Dpw1EAVrSB1ibxiDQyTAW6Zip3J4Btk2x4SgApQCeFbX` | 0.99975969  | 0.0000812 (0.0081%)  | -8       | 855 s | StalePrice    | PASS, debt high 0.99984089       |

Both accounts are owned by `rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ` with
verification level Full and carry the configured feed ids. The strict run
correctly rejects them: devnet sponsored feeds are pushed only every several
minutes, so the 60 s default age bound fails closed, which is the intended
behavior. The age-neutral run evaluates every other rule (owner, feed,
confidence, positivity, exponent, normalization, spot-versus-EMA) as of the
publish time and passes. The SOL price is whatever Pyth's devnet pusher
publishes; it is not a market reference.

Posting a fresh update to devnet is not done: it needs
`@pythnetwork/pyth-solana-receiver` (a new dependency, not approved) and a
Hermes API key. Once both exist, post with the deployer wallet
(`~/.config/solana/walnut-deployer.json`) using the receiver SDK's
`postPriceUpdate` flow, then rerun `demo:oracle` against the posted account.

## Open items for Task 9

- Build `PythPriceView` from the loaded `PriceUpdateV2` account on chain
  (Anchor deserialization using the layout above, owner from `AccountInfo`).
- Persist feed ids, `OracleParams`, `OracleStatus`, and `LastGoodObservation` in
  market state, and bump `AssetState::oracle_generation` when
  `enters_anomaly` is true.
- Wire the spot-versus-last-good breaker (`deviation_within` against
  `LastGoodObservation`) into the instruction; today only the helper and its
  boundary test exist.
- Capture publish times, oracle generation, and Decision Expiry in each
  price-sensitive Intent (fields exist on `OperationIntent`).
- Decide how localnet loads the fixture accounts (`[[test.validator.account]]`
  with a fresh publish time) and add on-chain tests against them.
- Obtain a Hermes API key and approve the receiver SDK dependency to post live
  updates on devnet.
