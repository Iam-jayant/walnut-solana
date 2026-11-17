//! `update_utilization_band`: queue and callback rules (Task 7.2; design:
//! "Interest and pool accounting", Requirements 7.x).
//!
//! The computation reads the Pool's encrypted Cash and Debt Shares at the
//! public accounting Borrow Index and reveals exactly one value: the band
//! index (0..=19, 5% each). It is read-only. It neither runs the interest
//! checkpoint nor takes a Lock, and it writes no encrypted state, so there is
//! no output nonce to derive (the Pool is `Enc<Mxe>` and no Viewing Key is
//! involved).
//!
//! # Flow
//!
//! 1. `update_utilization_band` (any payer): [`queue_band_update`] requires an
//!    `Active` Market and Pool, builds a [`OperationKind::UtilizationBand`]
//!    Intent that captures the Pool generation, Asset State generation, rate
//!    epoch, and accounting index and timestamp, reserves its sequence from the
//!    Pool-owned counter, and the handler queues the circuit. The Pool is not
//!    Locked and its generation does not change.
//! 2. Callback ([`settle_band_update`]): runs `classify_callback` with the
//!    current Pool generation, Asset State generation, rate epoch, and
//!    accounting index. `Applied` stores the band and the observation slot on
//!    the Asset State; `Stale` (any of those moved), `Expired`, and `Failed`
//!    store nothing. An unauthenticated output or a binding mismatch reverts
//!    and leaves the Intent `Queued`. A terminal Intent is a no-op.
//!
//! # Observation slot
//!
//! The recorded observation slot is the Intent's **queue slot**: the Pool was
//! unchanged between the queue and the callback (otherwise the Intent is
//! `Stale`), so the value describes the queue slot, and using it makes the
//! observation age conservative. An observation older than the stored one is
//! not written (the Intent still resolves `Applied`).
//!
//! # Commitments
//!
//! `accounts_commitment`: payer, Market, Asset State, and Pool (captured
//! generation), all read for the computation. `inputs_commitment`: the Pool
//! payload nonce, its four ciphertexts, and the accounting index.

use anchor_lang::prelude::Pubkey;

use super::accounts::{PoolState, PoolStatus};
use super::Slot;
use crate::errors::ErrorCode;
use crate::market::{AssetState, MarketStatus};
use crate::math::rate::{apply_band_observation, BAND_COUNT};
use crate::operation::callback::{
    apply_decision_to_slots, classify_callback, CallbackContext, CallbackDecision, CallbackReport,
    CurrentDependencies, StaleReason,
};
use crate::operation::commitment::{
    account_commitment, input_commitment, AccountBinding, AccountRole, InputField, InputValue,
};
use crate::operation::seeds::reserve_intent;
use crate::operation::{IntentStatus, OperationCounter, OperationIntent, OperationKind};

/// Input field id: the Pool payload nonce (`U128`).
pub const INPUT_POOL_NONCE: u8 = 16;
/// Input field id of the first Pool ciphertext; the four ciphertexts take
/// ids 17 to 20.
pub const INPUT_POOL_CIPHERTEXT_FIRST: u8 = 17;
/// Input field id: the accounting Borrow Index passed to the circuit (`U128`).
pub const INPUT_ACCOUNTING_INDEX: u8 = 21;

/// Decision Expiry distance of a band update, in slots.
pub const BAND_DECISION_SLOTS: u64 = 300;
/// Slots between Decision Expiry and Lock expiry.
pub const BAND_LOCK_GRACE_SLOTS: u64 = 150;

/// The accounts a band Intent binds, in order: payer, Market, Asset State,
/// Pool (with its captured generation). All are read-only for the
/// computation; the Asset State is written only by the callback.
pub fn band_bindings(
    payer: Pubkey,
    market_key: Pubkey,
    asset_state: Pubkey,
    pool: Pubkey,
    pool_generation: Option<u64>,
) -> [AccountBinding; 4] {
    let program = crate::ID;
    let binding = |key, role, expected_owner, generation| AccountBinding {
        key,
        role,
        expected_owner,
        mint: None,
        authority: None,
        writable: false,
        generation,
    };
    [
        binding(
            payer,
            AccountRole::Payer,
            anchor_lang::system_program::ID,
            None,
        ),
        binding(market_key, AccountRole::Market, program, None),
        binding(asset_state, AccountRole::AssetState, program, None),
        binding(pool, AccountRole::Pool, program, pool_generation),
    ]
}

/// The ordered inputs a band Intent binds.
pub fn band_inputs(pool: &PoolState, accounting_index: u128) -> [InputField<'static>; 6] {
    let c = &pool.encrypted.ciphertexts;
    let first = INPUT_POOL_CIPHERTEXT_FIRST;
    [
        InputField::new(INPUT_POOL_NONCE, InputValue::U128(pool.encrypted.nonce)),
        InputField::new(first, InputValue::Bytes32(c[0])),
        InputField::new(first + 1, InputValue::Bytes32(c[1])),
        InputField::new(first + 2, InputValue::Bytes32(c[2])),
        InputField::new(first + 3, InputValue::Bytes32(c[3])),
        InputField::new(INPUT_ACCOUNTING_INDEX, InputValue::U128(accounting_index)),
    ]
}

/// One band update queue request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BandRequest {
    /// The Operation Intent PDA address.
    pub intent_key: Pubkey,
    /// The Pool counter's next Intent sequence.
    pub sequence: u64,
    /// The Arcium computation offset.
    pub computation_offset: u64,
    /// The current slot.
    pub queued_slot: Slot,
}

/// The accounts a band update touches, as keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BandKeys {
    /// The payer (signer).
    pub payer: Pubkey,
    /// The Market.
    pub market: Pubkey,
    /// The Asset State.
    pub asset_state: Pubkey,
    /// The Pool State.
    pub pool: Pubkey,
}

/// Builds the band Intent.
pub fn build_band_intent(
    keys: &BandKeys,
    asset: &AssetState,
    pool: &PoolState,
    request: &BandRequest,
) -> Result<OperationIntent, ErrorCode> {
    let decision_expiry_slot = request
        .queued_slot
        .checked_add(BAND_DECISION_SLOTS)
        .ok_or(ErrorCode::InvalidIntentTiming)?;
    let lock_expiry_slot = decision_expiry_slot
        .checked_add(BAND_LOCK_GRACE_SLOTS)
        .ok_or(ErrorCode::InvalidIntentTiming)?;
    let generation = Some(pool.coordination.generation);
    let bindings = band_bindings(
        keys.payer,
        keys.market,
        keys.asset_state,
        keys.pool,
        generation,
    );
    let inputs = band_inputs(pool, asset.accounting_borrow_index);
    Ok(OperationIntent {
        key: request.intent_key,
        owner: keys.pool,
        payer: keys.payer,
        reclaim_authority: keys.payer,
        kind: OperationKind::UtilizationBand,
        status: IntentStatus::Queued,
        sequence: request.sequence,
        computation_offset: request.computation_offset,
        asset_generation: asset.generation,
        oracle_generation: asset.oracle_generation,
        rate_epoch_generation: asset.rate_epoch_generation,
        accounting_borrow_index: asset.accounting_borrow_index,
        accounting_index_timestamp: asset.accounting_index_timestamp,
        risk_borrow_index_at_expiry: asset.accounting_borrow_index,
        accounts_commitment: account_commitment(&bindings),
        inputs_commitment: input_commitment(OperationKind::UtilizationBand, &inputs),
        queued_slot: request.queued_slot,
        decision_expiry_slot,
        lock_expiry_slot,
        pool_generation: generation,
        ..OperationIntent::default()
    })
}

/// `update_utilization_band`: requires an `Active` Market and Pool, builds
/// and reserves the Intent. Takes no Lock and changes no generation.
/// Atomic: on error the counter is unchanged.
pub fn queue_band_update(
    market_status: MarketStatus,
    keys: &BandKeys,
    asset: &AssetState,
    pool: &PoolState,
    counter: &mut OperationCounter,
    request: &BandRequest,
) -> Result<OperationIntent, ErrorCode> {
    if market_status != MarketStatus::Active {
        return Err(ErrorCode::MarketNotActive);
    }
    if pool.status != PoolStatus::Active {
        return Err(ErrorCode::BandPoolNotActive);
    }
    let intent = build_band_intent(keys, asset, pool, request)?;
    let mut next_counter = *counter;
    reserve_intent(&mut next_counter, &intent)?;
    *counter = next_counter;
    Ok(intent)
}

/// What the callback observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BandReport {
    /// Whether Arcium `verify_output` succeeded.
    pub output_authenticated: bool,
    /// The computation offset the callback's computation account belongs to.
    pub computation_offset: u64,
    /// The Operation Intent account the callback received.
    pub intent_key: Pubkey,
    /// The revealed band index.
    pub band: u8,
    /// The current slot.
    pub current_slot: Slot,
}

/// What the callback did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandSettlement {
    /// `Applied`: the band was current and is recorded (unless a newer
    /// observation was already stored, `written == false`).
    Applied {
        /// The revealed band.
        band: u8,
        /// The observation slot (the queue slot).
        observation_slot: Slot,
        /// Whether the Asset State was written.
        written: bool,
    },
    /// A captured dependency moved: nothing stored.
    Stale(StaleReason),
    /// `Expired` or `Failed`: nothing stored.
    Failed(IntentStatus),
    /// The Intent was already terminal; nothing changed.
    Ignored,
}

/// Verifies and applies an `update_utilization_band` callback.
///
/// Unauthenticated output and commitment, offset, or binding mismatches are
/// errors (the Intent stays `Queued`). Atomic on error.
pub fn settle_band_update(
    keys: &BandKeys,
    asset: &mut AssetState,
    pool: &PoolState,
    intent: &mut OperationIntent,
    report: &BandReport,
) -> Result<BandSettlement, ErrorCode> {
    // The payer is part of the Intent and cannot be substituted: rebuild the
    // bindings from the Intent's own payer and the accounts received.
    let bindings = band_bindings(
        intent.payer,
        keys.market,
        keys.asset_state,
        keys.pool,
        intent.pool_generation,
    );
    let inputs = band_inputs(pool, intent.accounting_borrow_index);
    let context = CallbackContext {
        output_authenticated: report.output_authenticated,
        computation_offset: report.computation_offset,
        intent_key: report.intent_key,
        kind: OperationKind::UtilizationBand,
        accounts: &bindings,
        input_fields: &inputs,
        current: CurrentDependencies {
            pool_generation: Some(pool.coordination.generation),
            asset_generation: asset.generation,
            rate_epoch_generation: asset.rate_epoch_generation,
            accounting_borrow_index: asset.accounting_borrow_index,
            accounting_index_timestamp: asset.accounting_index_timestamp,
            // The band does not depend on the oracle.
            ..CurrentDependencies::captured_by(intent)
        },
        current_slot: report.current_slot,
        report: CallbackReport::Success { nonzero: true },
    };
    let decision = classify_callback(intent, &context)?;
    match decision {
        CallbackDecision::LeaveQueued => return Err(ErrorCode::CallbackOutputNotAuthenticated),
        CallbackDecision::Ignore => return Ok(BandSettlement::Ignored),
        CallbackDecision::Applied if report.band >= BAND_COUNT => {
            return Err(ErrorCode::UtilizationBandInvalid)
        }
        _ => {}
    }

    let mut next_intent = *intent;
    let result = apply_decision_to_slots(&mut next_intent, &mut [], decision, report.current_slot)?;
    let settlement = match decision {
        CallbackDecision::Applied => {
            let observation_slot = intent.queued_slot;
            let written = observation_slot >= asset.utilization_observation_slot;
            if written {
                apply_band_observation(asset, report.band, observation_slot)?;
            }
            BandSettlement::Applied {
                band: report.band,
                observation_slot,
                written,
            }
        }
        CallbackDecision::Stale(reason) => BandSettlement::Stale(reason),
        _ => BandSettlement::Failed(result.status),
    };
    *intent = next_intent;
    Ok(settlement)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::{Caps, PauseState, RateParams, RiskParams, TimelockState};
    use crate::math::WAD;
    use crate::state::accounts::EncryptedPoolState;
    use crate::state::coordination::Coordination;

    fn key(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    fn keys() -> BandKeys {
        BandKeys {
            payer: key(1),
            market: key(2),
            asset_state: key(3),
            pool: key(4),
        }
    }

    fn asset() -> AssetState {
        AssetState {
            layout_version: 1,
            bump: 1,
            market: key(2),
            generation: 5,
            oracle_generation: 2,
            rate_epoch_generation: 3,
            accounting_borrow_index: WAD + 7,
            accounting_index_timestamp: 1_000,
            utilization_band: 4,
            utilization_observation_slot: 10,
            risk: RiskParams::LAUNCH_DEFAULT,
            rates: RateParams::LAUNCH_DEFAULT,
            caps: Caps::default(),
            pauses: PauseState::default(),
            timelock: TimelockState::default(),
            reserved: [0; 128],
        }
    }

    fn pool() -> PoolState {
        PoolState {
            coordination: Coordination {
                generation: 9,
                ..Coordination::default()
            },
            bump: 1,
            layout_version: 1,
            status: PoolStatus::Active,
            market: key(2),
            accounted_borrow_index: WAD,
            encrypted: EncryptedPoolState {
                nonce: 77,
                ciphertexts: [[1; 32], [2; 32], [3; 32], [4; 32]],
            },
            unsettled_recovery_claim_count: 0,
        }
    }

    fn request(queued_slot: u64) -> BandRequest {
        BandRequest {
            intent_key: key(9),
            sequence: 0,
            computation_offset: 1_234,
            queued_slot,
        }
    }

    fn queued() -> (OperationIntent, OperationCounter) {
        let mut counter = OperationCounter::new(key(4), 255);
        let intent = queue_band_update(
            MarketStatus::Active,
            &keys(),
            &asset(),
            &pool(),
            &mut counter,
            &request(100),
        )
        .unwrap();
        (intent, counter)
    }

    /// The program error code of a failed result.
    fn code<T>(result: Result<T, ErrorCode>) -> u32 {
        match result {
            Ok(_) => panic!("expected an error"),
            Err(error) => u32::from(error),
        }
    }

    fn report(intent: &OperationIntent, band: u8, slot: u64) -> BandReport {
        BandReport {
            output_authenticated: true,
            computation_offset: intent.computation_offset,
            intent_key: intent.key,
            band,
            current_slot: slot,
        }
    }

    #[test]
    fn queue_takes_no_lock_and_captures_dependencies() {
        let (intent, counter) = queued();
        assert_eq!(counter.next_sequence, 1);
        assert_eq!(intent.kind, OperationKind::UtilizationBand);
        assert_eq!(intent.pool_generation, Some(9));
        assert_eq!(intent.rate_epoch_generation, 3);
        assert_eq!(intent.accounting_borrow_index, WAD + 7);
        assert!(intent.position.is_none());
        assert_eq!(intent.owner, key(4));
        intent.validate_new().unwrap();
    }

    #[test]
    fn queue_rejects_inactive_market_and_pool_without_touching_the_counter() {
        let mut counter = OperationCounter::new(key(4), 255);
        let mut inactive = pool();
        inactive.status = PoolStatus::Initializing;
        assert_eq!(
            code(queue_band_update(
                MarketStatus::Active,
                &keys(),
                &asset(),
                &inactive,
                &mut counter,
                &request(100)
            )),
            u32::from(ErrorCode::BandPoolNotActive)
        );
        assert_eq!(
            code(queue_band_update(
                MarketStatus::Uninitialized,
                &keys(),
                &asset(),
                &pool(),
                &mut counter,
                &request(100)
            )),
            u32::from(ErrorCode::MarketNotActive)
        );
        assert_eq!(counter.next_sequence, 0);
    }

    #[test]
    fn a_current_callback_writes_the_band_at_the_queue_slot() {
        let (mut intent, _) = queued();
        let mut a = asset();
        let r = report(&intent, 12, 120);
        let result = settle_band_update(&keys(), &mut a, &pool(), &mut intent, &r).unwrap();
        assert_eq!(
            result,
            BandSettlement::Applied {
                band: 12,
                observation_slot: 100,
                written: true
            }
        );
        assert_eq!(
            (a.utilization_band, a.utilization_observation_slot),
            (12, 100)
        );
        assert_eq!(intent.status, IntentStatus::Applied);
        // The Pool generation is untouched by a read-only update.
        assert_eq!(pool().coordination.generation, 9);
        // A duplicate callback is a no-op.
        let r = report(&intent, 3, 121);
        let again = settle_band_update(&keys(), &mut a, &pool(), &mut intent, &r).unwrap();
        assert_eq!(again, BandSettlement::Ignored);
        assert_eq!(a.utilization_band, 12);
    }

    #[test]
    fn every_moved_dependency_makes_the_callback_stale_and_writes_nothing() {
        type Change = fn(&mut AssetState, &mut PoolState);
        let cases: [(Change, StaleReason); 4] = [
            (
                |_, p| p.coordination.generation += 1,
                StaleReason::PoolGeneration,
            ),
            (|a, _| a.rate_epoch_generation += 1, StaleReason::RateEpoch),
            (
                |a, _| a.accounting_borrow_index += 1,
                StaleReason::AccountingIndex,
            ),
            (
                |a, _| a.accounting_index_timestamp += 1,
                StaleReason::AccountingIndexTimestamp,
            ),
        ];
        for (change, reason) in cases {
            let (mut intent, _) = queued();
            let (mut a, mut p) = (asset(), pool());
            change(&mut a, &mut p);
            let before = a.clone_for_test();
            let r = report(&intent, 12, 120);
            let result = settle_band_update(&keys(), &mut a, &p, &mut intent, &r).unwrap();
            assert_eq!(result, BandSettlement::Stale(reason));
            assert_eq!(intent.status, IntentStatus::Stale);
            assert_eq!(a.clone_for_test(), before);
        }
    }

    #[test]
    fn expired_failed_unauthenticated_and_invalid_callbacks_store_nothing() {
        // Expired.
        let (mut intent, _) = queued();
        let mut a = asset();
        let late = intent.decision_expiry_slot + 1;
        let r = report(&intent, 12, late);
        let result = settle_band_update(&keys(), &mut a, &pool(), &mut intent, &r).unwrap();
        assert_eq!(result, BandSettlement::Failed(IntentStatus::Expired));
        assert_eq!(a.utilization_band, 4);
        // Unauthenticated: reverts, Intent stays Queued.
        let (mut intent, _) = queued();
        let mut bad = report(&intent, 12, 120);
        bad.output_authenticated = false;
        assert_eq!(
            code(settle_band_update(
                &keys(),
                &mut a,
                &pool(),
                &mut intent,
                &bad
            )),
            u32::from(ErrorCode::CallbackOutputNotAuthenticated)
        );
        assert_eq!(intent.status, IntentStatus::Queued);
        // Invalid band index.
        let invalid = report(&intent, 20, 120);
        assert_eq!(
            code(settle_band_update(
                &keys(),
                &mut a,
                &pool(),
                &mut intent,
                &invalid
            )),
            u32::from(ErrorCode::UtilizationBandInvalid)
        );
        assert_eq!(intent.status, IntentStatus::Queued);
        // A substituted Pool payload fails the inputs commitment.
        let good = report(&intent, 12, 120);
        let mut other = pool();
        other.encrypted.nonce += 1;
        assert!(settle_band_update(&keys(), &mut a, &other, &mut intent, &good).is_err());
        // A substituted Asset State key fails the accounts commitment.
        let mut wrong = keys();
        wrong.asset_state = key(99);
        assert!(settle_band_update(&wrong, &mut a, &pool(), &mut intent, &good).is_err());
        assert_eq!(intent.status, IntentStatus::Queued);
    }

    #[test]
    fn an_older_observation_does_not_overwrite_a_newer_one() {
        let (mut intent, _) = queued(); // queue slot 100
        let mut a = asset();
        a.utilization_observation_slot = 150;
        let r = report(&intent, 12, 120);
        let result = settle_band_update(&keys(), &mut a, &pool(), &mut intent, &r).unwrap();
        assert_eq!(
            result,
            BandSettlement::Applied {
                band: 12,
                observation_slot: 100,
                written: false
            }
        );
        assert_eq!(
            (a.utilization_band, a.utilization_observation_slot),
            (4, 150)
        );
    }
    impl AssetState {
        fn clone_for_test(&self) -> (u8, u64, u128, i64, u64) {
            (
                self.utilization_band,
                self.utilization_observation_slot,
                self.accounting_borrow_index,
                self.accounting_index_timestamp,
                self.generation,
            )
        }
    }
}
