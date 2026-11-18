//! Pool and Borrow Index serialization (Task 7.4; design: "Interest and pool
//! accounting", "Generations and Locks"; Requirements 7.3, 7.4, 7.9, 7.10).
//!
//! Every computation that mutates Pool Cash, Supply Shares, Debt Shares, or
//! Reserves (supply, withdrawal, borrow, repay, reserve withdrawal, partial
//! liquidation, Absorb) goes through the three functions here, so the rule is
//! implemented once:
//!
//! 1. [`begin_pool_mutation`] at **queue time**:
//!    * requires an `Active` Market and Pool;
//!    * refuses while the Pool is Locked by another operation
//!      ([`ensure_pool_unlocked_for_index_mutation`]), so the public
//!      accounting Borrow Index cannot move under an in-flight computation;
//!    * accrues the public accounting Borrow Index and timestamp
//!      ([`accrue_asset_index`]) **before** the Intent captures it;
//!    * captures, in one [`PoolCapture`], the accounting index and timestamp
//!      (the only index share, Cash, and checkpoint math may use), the
//!      projected risk index through Decision Expiry (the only index health
//!      and cap checks may use; always `>=` the accounting index and a
//!      distinct field), the Pool generation, and the asset, oracle, and
//!      rate-epoch generations;
//!    * acquires the Pool Lock and the Locks of any other protected accounts
//!      (Positions) atomically, all or none.
//!
//!    Atomic: on any error the Asset State, Pool, other slots, and the
//!    counter are unchanged.
//! 2. [`settle_pool_decision`] in the **callback**: applies a decision that
//!    `classify_callback` returned. `Applied` stores the new
//!    [`EncryptedPoolState`], sets `accounted_borrow_index` to the Intent's
//!    accounting index, bumps every matching generation, and releases the
//!    Locks. `Zero`, `Stale`, `Expired`, and `Failed` release the Locks and
//!    store nothing (the next operation checkpoints from the old accounted
//!    index, so no interest is lost). Atomic.
//! 3. [`abandon_pool_mutation`]: permissionless timeout recovery. It bumps the
//!    Pool generation and clears the Lock before the Intent becomes
//!    `Abandoned`, so any old output can never apply.
//!
//! # Two indexes
//!
//! `accounting_borrow_index` is public state that only grows, and only at
//! queue time of a Pool mutation (or a governed `Rates` change, which also
//! refuses while the Pool is Locked). `risk_borrow_index_at_expiry` is
//! never written back to any account.
//!
//! # Read-only band update
//!
//! `update_utilization_band` takes no Lock and does not accrue. It captures the
//! Pool generation, the accounting index and timestamp, and the rate epoch, so
//! it resolves `Stale` after any Pool mutation was queued (the index moved) or
//! applied (the generation moved) and after a rate change.
//!
//! # Output nonces of `Enc<Mxe>`
//!
//! The Pool is `Enc<Mxe, PoolSecret>`: the MXE cluster picks the output nonce,
//! there is no client key and no input-nonce-plus-one rule to defend (the rule
//! of `state::nonce` is for `Enc<Shared>` outputs). The callback only rejects
//! an output whose nonce equals the stored one
//! ([`ErrorCode::PoolOutputNonceReused`]).

use anchor_lang::prelude::Pubkey;

use super::accounts::{EncryptedPoolState, PoolState, PoolStatus};
use super::coordination::{acquire_locks, CoordinationSlot};
use super::Slot;
use crate::errors::ErrorCode;
use crate::market::{AssetState, MarketStatus};
use crate::math::rate::{accrue_asset_index, project_risk_index};
use crate::operation::callback::{
    apply_decision_to_slots, CallbackDecision, CallbackResult, CurrentDependencies,
};
use crate::operation::seeds::reserve_intent;
use crate::operation::{
    abandon_intent, IntentStatus, OperationCounter, OperationIntent, OperationKind,
};

/// Everything a Pool-mutating Intent captures at queue time, after the
/// accounting index was accrued.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolCapture {
    /// The Pool generation captured by the Intent.
    pub pool_generation: u64,
    /// The Asset State generation.
    pub asset_generation: u64,
    /// The oracle generation.
    pub oracle_generation: u64,
    /// The rate-epoch generation.
    pub rate_epoch_generation: u64,
    /// The accounting Borrow Index after accrual (share, Cash, checkpoint).
    pub accounting_borrow_index: u128,
    /// The timestamp the accounting index was accrued to.
    pub accounting_index_timestamp: i64,
    /// The index projected through Decision Expiry (health and caps only).
    pub risk_borrow_index_at_expiry: u128,
}

/// The two commitments an Intent stores. Built by the caller from the
/// [`PoolCapture`] because the input commitment binds the accounting index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntentCommitments {
    /// `accounts_commitment`.
    pub accounts: [u8; 32],
    /// `inputs_commitment`.
    pub inputs: [u8; 32],
}

/// One Pool mutation queue request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolMutationRequest {
    /// The Operation Intent PDA address.
    pub intent_key: Pubkey,
    /// The Intent owner (the PDA seed owner: the Pool for Pool-owned counters,
    /// the Position owner for user operations).
    pub owner: Pubkey,
    /// The payer, who is also the reclaim authority.
    pub payer: Pubkey,
    /// The operation kind.
    pub kind: OperationKind,
    /// The owner's next Intent sequence.
    pub sequence: u64,
    /// The Arcium computation offset.
    pub computation_offset: u64,
    /// The current slot.
    pub queued_slot: Slot,
    /// The current Unix time (accrual target).
    pub now_ts: i64,
    /// Slots from the queue slot to Decision Expiry.
    pub decision_slots: u64,
    /// Slots from Decision Expiry to Lock expiry.
    pub lock_grace_slots: u64,
}

/// Fails with [`ErrorCode::PoolLockedForIndexMutation`] while any operation
/// holds the Pool Lock (even an expired one: it must be abandoned first).
/// Every independent writer of the accounting index (a governed `Rates`
/// execution, a standalone accrual) calls this first.
pub fn ensure_pool_unlocked_for_index_mutation(pool: &PoolState) -> Result<(), ErrorCode> {
    if pool.coordination.lock.is_active() {
        return Err(ErrorCode::PoolLockedForIndexMutation);
    }
    Ok(())
}

/// An independent accrual of the accounting index (no Intent): refused while
/// the Pool is Locked. Atomic.
pub fn accrue_independently(
    asset: &mut AssetState,
    pool: &PoolState,
    now_ts: i64,
) -> Result<(), ErrorCode> {
    ensure_pool_unlocked_for_index_mutation(pool)?;
    accrue_asset_index(asset, now_ts)?;
    Ok(())
}

/// The current dependency values of a Pool mutation callback.
pub fn pool_dependencies(pool: &PoolState, asset: &AssetState) -> CurrentDependencies {
    CurrentDependencies {
        position_generation: None,
        second_position_generation: None,
        pool_generation: Some(pool.coordination.generation),
        asset_generation: asset.generation,
        oracle_generation: asset.oracle_generation,
        rate_epoch_generation: asset.rate_epoch_generation,
        accounting_borrow_index: asset.accounting_borrow_index,
        accounting_index_timestamp: asset.accounting_index_timestamp,
    }
}

/// Queue-time orchestration of a Pool-financial-state mutation (see the module
/// docs). `others` are the Locks of the operation's other protected accounts
/// (Positions); the caller sets the Intent's `position*` fields on the returned
/// Intent. `commit` builds the Intent commitments from the captured values.
#[allow(clippy::too_many_arguments)]
pub fn begin_pool_mutation(
    market_status: MarketStatus,
    asset: &mut AssetState,
    pool: &mut PoolState,
    pool_key: Pubkey,
    others: &mut [CoordinationSlot<'_>],
    counter: &mut OperationCounter,
    request: &PoolMutationRequest,
    commit: impl FnOnce(&PoolCapture) -> IntentCommitments,
) -> Result<OperationIntent, ErrorCode> {
    if market_status != MarketStatus::Active {
        return Err(ErrorCode::MarketNotActive);
    }
    if pool.status != PoolStatus::Active {
        return Err(ErrorCode::PoolMutationPoolNotActive);
    }
    ensure_pool_unlocked_for_index_mutation(pool)?;

    let decision_expiry_slot = request
        .queued_slot
        .checked_add(request.decision_slots)
        .ok_or(ErrorCode::InvalidIntentTiming)?;
    let lock_expiry_slot = decision_expiry_slot
        .checked_add(request.lock_grace_slots)
        .ok_or(ErrorCode::InvalidIntentTiming)?;

    // 1. Accrue on a copy: the accounting index is captured only after it.
    let mut next_asset = asset.clone();
    accrue_asset_index(&mut next_asset, request.now_ts)?;
    let risk_index = project_risk_index(&next_asset, request.now_ts, request.decision_slots)?;
    let capture = PoolCapture {
        pool_generation: pool.coordination.generation,
        asset_generation: next_asset.generation,
        oracle_generation: next_asset.oracle_generation,
        rate_epoch_generation: next_asset.rate_epoch_generation,
        accounting_borrow_index: next_asset.accounting_borrow_index,
        accounting_index_timestamp: next_asset.accounting_index_timestamp,
        risk_borrow_index_at_expiry: risk_index,
    };
    let commitments = commit(&capture);
    let intent = OperationIntent {
        key: request.intent_key,
        owner: request.owner,
        payer: request.payer,
        reclaim_authority: request.payer,
        kind: request.kind,
        status: IntentStatus::Queued,
        sequence: request.sequence,
        computation_offset: request.computation_offset,
        asset_generation: capture.asset_generation,
        oracle_generation: capture.oracle_generation,
        rate_epoch_generation: capture.rate_epoch_generation,
        accounting_borrow_index: capture.accounting_borrow_index,
        accounting_index_timestamp: capture.accounting_index_timestamp,
        risk_borrow_index_at_expiry: capture.risk_borrow_index_at_expiry,
        accounts_commitment: commitments.accounts,
        inputs_commitment: commitments.inputs,
        queued_slot: request.queued_slot,
        decision_expiry_slot,
        lock_expiry_slot,
        pool_generation: Some(capture.pool_generation),
        ..OperationIntent::default()
    };

    // 2. Reserve the sequence and take every Lock, all or none.
    let mut next_counter = *counter;
    reserve_intent(&mut next_counter, &intent)?;
    let mut next_pool = pool.coordination;
    {
        let mut slots: Vec<CoordinationSlot<'_>> = Vec::with_capacity(others.len() + 1);
        slots.push(CoordinationSlot::new(pool_key, &mut next_pool));
        for slot in others.iter_mut() {
            slots.push(CoordinationSlot {
                key: slot.key,
                state: &mut *slot.state,
            });
        }
        acquire_locks(intent.lock_owner(), intent.lock_expiry_slot, &mut slots)?;
    }

    // 3. Commit.
    *asset = next_asset;
    pool.coordination = next_pool;
    *counter = next_counter;
    Ok(intent)
}

/// Applies a decision from `classify_callback` to the Pool and the other
/// protected accounts (see the module docs). The caller must already have
/// returned for `Ignore` and `LeaveQueued`. On `Applied`, `output` is stored.
/// Atomic.
pub fn settle_pool_decision(
    pool: &mut PoolState,
    pool_key: Pubkey,
    others: &mut [CoordinationSlot<'_>],
    intent: &mut OperationIntent,
    decision: CallbackDecision,
    output: &EncryptedPoolState,
    current_slot: Slot,
) -> Result<CallbackResult, ErrorCode> {
    if pool.status != PoolStatus::Active {
        return Err(ErrorCode::PoolMutationPoolNotActive);
    }
    if !intent.lock_owner().holds(&pool.coordination.lock) {
        return Err(ErrorCode::PoolMutationIntentMismatch);
    }
    if decision == CallbackDecision::Applied && output.nonce == pool.encrypted.nonce {
        return Err(ErrorCode::PoolOutputNonceReused);
    }
    let mut next_intent = *intent;
    let mut next_pool = pool.coordination;
    let result = {
        let mut slots: Vec<CoordinationSlot<'_>> = Vec::with_capacity(others.len() + 1);
        slots.push(CoordinationSlot::new(pool_key, &mut next_pool));
        for slot in others.iter_mut() {
            slots.push(CoordinationSlot {
                key: slot.key,
                state: &mut *slot.state,
            });
        }
        apply_decision_to_slots(&mut next_intent, &mut slots, decision, current_slot)?
    };
    *intent = next_intent;
    pool.coordination = next_pool;
    if result.status == IntentStatus::Applied {
        pool.encrypted = *output;
        pool.accounted_borrow_index = intent.accounting_borrow_index;
    }
    Ok(result)
}

/// Permissionless timeout recovery of a queued Pool mutation of `kind`. The
/// Pool generation is bumped before the Lock clears, so a late computation
/// can never apply. Returns the number of Locks cleared. Atomic.
pub fn abandon_pool_mutation(
    pool: &mut PoolState,
    pool_key: Pubkey,
    others: &mut [CoordinationSlot<'_>],
    intent: &mut OperationIntent,
    kind: OperationKind,
    current_slot: Slot,
) -> Result<usize, ErrorCode> {
    if intent.kind != kind || intent.pool_generation.is_none() {
        return Err(ErrorCode::PoolMutationIntentMismatch);
    }
    if intent.status.is_terminal() {
        return Err(ErrorCode::IntentNotQueued);
    }
    if !intent.lock_owner().holds(&pool.coordination.lock) {
        return Err(ErrorCode::PoolMutationIntentMismatch);
    }
    let mut next_intent = *intent;
    let mut next_pool = pool.coordination;
    let cleared = {
        let mut slots: Vec<CoordinationSlot<'_>> = Vec::with_capacity(others.len() + 1);
        slots.push(CoordinationSlot::new(pool_key, &mut next_pool));
        for slot in others.iter_mut() {
            slots.push(CoordinationSlot {
                key: slot.key,
                state: &mut *slot.state,
            });
        }
        abandon_intent(&mut next_intent, current_slot, &mut slots)?
    };
    *intent = next_intent;
    pool.coordination = next_pool;
    Ok(cleared)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::market::{Caps, PauseState, RateParams, RiskParams, TimelockState};
    use crate::math::WAD;
    use crate::state::coordination::Coordination;
    use crate::state::Generation;

    fn key(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    pub(crate) fn asset() -> AssetState {
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

    pub(crate) fn pool() -> PoolState {
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
        }
    }

    fn request(sequence: u64, offset: u64, now_ts: i64) -> PoolMutationRequest {
        PoolMutationRequest {
            intent_key: key(10 + sequence as u8),
            owner: key(4),
            payer: key(1),
            kind: OperationKind::ReserveWithdrawal,
            sequence,
            computation_offset: offset,
            queued_slot: 100,
            now_ts,
            decision_slots: 300,
            lock_grace_slots: 150,
        }
    }

    fn snap(p: &PoolState) -> (Coordination, u128, EncryptedPoolState) {
        (p.coordination, p.accounted_borrow_index, p.encrypted)
    }

    fn commit(_: &PoolCapture) -> IntentCommitments {
        IntentCommitments {
            accounts: [1; 32],
            inputs: [2; 32],
        }
    }

    fn begin(
        asset: &mut AssetState,
        pool: &mut PoolState,
        counter: &mut OperationCounter,
        sequence: u64,
        now_ts: i64,
    ) -> Result<OperationIntent, ErrorCode> {
        begin_pool_mutation(
            MarketStatus::Active,
            asset,
            pool,
            key(4),
            &mut [],
            counter,
            &request(sequence, 500 + sequence, now_ts),
            commit,
        )
    }

    fn code<T>(result: Result<T, ErrorCode>) -> u32 {
        match result {
            Ok(_) => panic!("expected an error"),
            Err(error) => u32::from(error),
        }
    }

    #[test]
    fn queue_accrues_first_then_captures_two_distinct_indexes_and_locks() {
        let (mut a, mut p) = (asset(), pool());
        let mut counter = OperationCounter::new(key(4), 255);
        let intent = begin(&mut a, &mut p, &mut counter, 0, 1_000 + 3_600).unwrap();
        assert!(a.accounting_borrow_index > WAD + 7, "index accrued");
        assert_eq!(a.accounting_index_timestamp, 4_600);
        assert_eq!(intent.accounting_borrow_index, a.accounting_borrow_index);
        assert_eq!(intent.accounting_index_timestamp, 4_600);
        assert!(intent.risk_borrow_index_at_expiry > intent.accounting_borrow_index);
        assert_eq!(intent.pool_generation, Some(9));
        assert_eq!(intent.asset_generation, 5);
        assert_eq!(intent.rate_epoch_generation, 3);
        assert!(intent.lock_owner().holds(&p.coordination.lock));
        assert_eq!(p.coordination.generation, 9, "queue never bumps");
        assert_eq!(counter.next_sequence, 1);
        intent.validate_new().unwrap();
    }

    #[test]
    fn queue_on_a_locked_pool_fails_and_changes_nothing() {
        let (mut a, mut p) = (asset(), pool());
        let mut counter = OperationCounter::new(key(4), 255);
        begin(&mut a, &mut p, &mut counter, 0, 2_000).unwrap();
        let (a_before, p_before, c_before) = (a.clone(), snap(&p), counter);
        assert_eq!(
            code(begin(&mut a, &mut p, &mut counter, 1, 9_000)),
            u32::from(ErrorCode::PoolLockedForIndexMutation)
        );
        assert_eq!(a, a_before);
        assert_eq!(snap(&p), p_before);
        assert_eq!(counter, c_before);
    }

    #[test]
    fn independent_accrual_is_refused_while_locked_and_allowed_after_release() {
        let (mut a, mut p) = (asset(), pool());
        let mut counter = OperationCounter::new(key(4), 255);
        let mut intent = begin(&mut a, &mut p, &mut counter, 0, 2_000).unwrap();
        let before = a.clone();
        assert_eq!(
            code(accrue_independently(&mut a, &p, 9_000)),
            u32::from(ErrorCode::PoolLockedForIndexMutation)
        );
        assert_eq!(a, before);
        let new_state = EncryptedPoolState {
            nonce: 78,
            ciphertexts: [[9; 32]; 4],
        };
        settle_pool_decision(
            &mut p,
            key(4),
            &mut [],
            &mut intent,
            CallbackDecision::Zero,
            &new_state,
            120,
        )
        .unwrap();
        accrue_independently(&mut a, &p, 9_000).unwrap();
        assert!(a.accounting_borrow_index > before.accounting_borrow_index);
    }

    #[test]
    fn a_locked_other_account_blocks_the_queue_atomically() {
        let (mut a, mut p) = (asset(), pool());
        let mut counter = OperationCounter::new(key(4), 255);
        let mut position = Coordination::default();
        position.lock.active = true;
        let (a_before, p_before, c_before) = (a.clone(), snap(&p), counter);
        let result = begin_pool_mutation(
            MarketStatus::Active,
            &mut a,
            &mut p,
            key(4),
            &mut [CoordinationSlot::new(key(5), &mut position)],
            &mut counter,
            &request(0, 500, 5_000),
            commit,
        );
        assert_eq!(code(result), u32::from(ErrorCode::LockHeld));
        assert_eq!((&a, snap(&p), counter), (&a_before, p_before, c_before));
    }

    #[test]
    fn inactive_market_or_pool_is_rejected() {
        let (mut a, mut p) = (asset(), pool());
        let mut counter = OperationCounter::new(key(4), 255);
        p.status = PoolStatus::Initializing;
        assert_eq!(
            code(begin(&mut a, &mut p, &mut counter, 0, 2_000)),
            u32::from(ErrorCode::PoolMutationPoolNotActive)
        );
        p.status = PoolStatus::Active;
        let result = begin_pool_mutation(
            MarketStatus::Uninitialized,
            &mut a,
            &mut p,
            key(4),
            &mut [],
            &mut counter,
            &request(0, 500, 2_000),
            commit,
        );
        assert_eq!(code(result), u32::from(ErrorCode::MarketNotActive));
    }

    #[test]
    fn applied_stores_state_bumps_generation_and_sets_the_accounted_index() {
        let (mut a, mut p) = (asset(), pool());
        let mut counter = OperationCounter::new(key(4), 255);
        let mut intent = begin(&mut a, &mut p, &mut counter, 0, 4_000).unwrap();
        let out = EncryptedPoolState {
            nonce: 5,
            ciphertexts: [[7; 32]; 4],
        };
        let result = settle_pool_decision(
            &mut p,
            key(4),
            &mut [],
            &mut intent,
            CallbackDecision::Applied,
            &out,
            150,
        )
        .unwrap();
        assert_eq!(result.status, IntentStatus::Applied);
        assert_eq!(p.coordination.generation, 10);
        assert!(!p.coordination.lock.is_active());
        assert_eq!(p.encrypted, out);
        assert_eq!(p.accounted_borrow_index, intent.accounting_borrow_index);
    }

    #[test]
    fn non_applied_decisions_release_and_store_nothing() {
        for (decision, status) in [
            (CallbackDecision::Zero, IntentStatus::Zero),
            (
                CallbackDecision::Stale(crate::operation::StaleReason::RateEpoch),
                IntentStatus::Stale,
            ),
            (CallbackDecision::Failed, IntentStatus::Failed),
        ] {
            let (mut a, mut p) = (asset(), pool());
            let mut counter = OperationCounter::new(key(4), 255);
            let mut intent = begin(&mut a, &mut p, &mut counter, 0, 4_000).unwrap();
            let stored = p.encrypted;
            let out = EncryptedPoolState {
                nonce: 5,
                ciphertexts: [[7; 32]; 4],
            };
            settle_pool_decision(&mut p, key(4), &mut [], &mut intent, decision, &out, 150)
                .unwrap();
            assert_eq!(intent.status, status);
            assert_eq!(p.encrypted, stored);
            assert_eq!(p.accounted_borrow_index, WAD);
            assert_eq!(p.coordination.generation, 9);
            assert!(!p.coordination.lock.is_active());
        }
        // Expired is applied only after Decision Expiry.
        let (mut a, mut p) = (asset(), pool());
        let mut counter = OperationCounter::new(key(4), 255);
        let mut intent = begin(&mut a, &mut p, &mut counter, 0, 4_000).unwrap();
        let out = p.encrypted;
        let decision_expiry = intent.decision_expiry_slot;
        settle_pool_decision(
            &mut p,
            key(4),
            &mut [],
            &mut intent,
            CallbackDecision::Expired,
            &out,
            decision_expiry + 1,
        )
        .unwrap();
        assert_eq!(intent.status, IntentStatus::Expired);
        assert_eq!(p.coordination.generation, 9);
    }

    #[test]
    fn a_repeated_output_nonce_is_rejected_atomically() {
        let (mut a, mut p) = (asset(), pool());
        let mut counter = OperationCounter::new(key(4), 255);
        let mut intent = begin(&mut a, &mut p, &mut counter, 0, 4_000).unwrap();
        let (p_before, i_before) = (snap(&p), intent);
        let reused = EncryptedPoolState {
            nonce: p.encrypted.nonce,
            ciphertexts: [[7; 32]; 4],
        };
        let result = settle_pool_decision(
            &mut p,
            key(4),
            &mut [],
            &mut intent,
            CallbackDecision::Applied,
            &reused,
            150,
        );
        assert_eq!(code(result), u32::from(ErrorCode::PoolOutputNonceReused));
        assert_eq!((snap(&p), intent), (p_before, i_before));
    }

    #[test]
    fn abandonment_bumps_the_generation_first_so_old_output_cannot_apply() {
        let (mut a, mut p) = (asset(), pool());
        let mut counter = OperationCounter::new(key(4), 255);
        let mut intent = begin(&mut a, &mut p, &mut counter, 0, 4_000).unwrap();
        let kind = OperationKind::ReserveWithdrawal;
        let lock_expiry = intent.lock_expiry_slot;
        // Too early: nothing changes.
        let (p_before, i_before) = (snap(&p), intent);
        assert!(
            abandon_pool_mutation(&mut p, key(4), &mut [], &mut intent, kind, lock_expiry).is_err()
        );
        assert_eq!((snap(&p), intent), (p_before, i_before));
        // Wrong kind is rejected.
        assert_eq!(
            code(abandon_pool_mutation(
                &mut p,
                key(4),
                &mut [],
                &mut intent,
                OperationKind::Supply,
                lock_expiry + 1
            )),
            u32::from(ErrorCode::PoolMutationIntentMismatch)
        );
        let cleared =
            abandon_pool_mutation(&mut p, key(4), &mut [], &mut intent, kind, lock_expiry + 1)
                .unwrap();
        assert_eq!(cleared, 1);
        assert_eq!(intent.status, IntentStatus::Abandoned);
        assert_eq!(p.coordination.generation, 10);
        assert!(!p.coordination.lock.is_active());
        // The old computation's generation is no longer current.
        let owner = intent.lock_owner();
        assert!(!p.coordination.is_current_writer(&owner, 9));
        let _ = Generation::new(0);
        // A second abandonment and a late callback decision both fail.
        assert!(
            abandon_pool_mutation(&mut p, key(4), &mut [], &mut intent, kind, lock_expiry + 2)
                .is_err()
        );
    }

    mod band_staleness {
        //! The read-only band update takes no Lock and goes `Stale` when a
        //! Pool mutation is queued or applied, or the rate epoch moves.
        use super::*;
        use crate::operation::callback::StaleReason;
        use crate::state::band::{
            queue_band_update, settle_band_update, BandKeys, BandReport, BandRequest,
            BandSettlement,
        };

        fn band_keys() -> BandKeys {
            BandKeys {
                payer: key(1),
                market: key(2),
                asset_state: key(3),
                pool: key(4),
            }
        }

        fn queue_band(a: &AssetState, p: &PoolState) -> OperationIntent {
            let mut counter = OperationCounter::new(key(4), 255);
            queue_band_update(
                MarketStatus::Active,
                &band_keys(),
                a,
                p,
                &mut counter,
                &BandRequest {
                    intent_key: key(40),
                    sequence: 0,
                    computation_offset: 4_321,
                    queued_slot: 100,
                },
            )
            .unwrap()
        }

        fn settle(
            a: &mut AssetState,
            p: &PoolState,
            intent: &mut OperationIntent,
        ) -> BandSettlement {
            settle_band_update(
                &band_keys(),
                a,
                p,
                intent,
                &BandReport {
                    output_authenticated: true,
                    computation_offset: 4_321,
                    intent_key: key(40),
                    band: 3,
                    current_slot: 120,
                },
            )
            .unwrap()
        }

        #[test]
        fn band_queue_is_lock_free_even_while_the_pool_is_locked() {
            let (mut a, mut p) = (asset(), pool());
            let mut counter = OperationCounter::new(key(4), 255);
            begin(&mut a, &mut p, &mut counter, 0, 4_000).unwrap();
            let before = p.coordination;
            let band = queue_band(&a, &p);
            assert_eq!(p.coordination, before);
            assert_eq!(band.kind, OperationKind::UtilizationBand);
        }

        #[test]
        fn band_is_current_when_nothing_moved() {
            let (mut a, p) = (asset(), pool());
            let mut intent = queue_band(&a, &p);
            assert!(matches!(
                settle(&mut a, &p, &mut intent),
                BandSettlement::Applied { band: 3, .. }
            ));
        }

        #[test]
        fn band_goes_stale_when_the_index_is_accrued_by_a_queued_mutation() {
            let (mut a, mut p) = (asset(), pool());
            let mut intent = queue_band(&a, &p);
            let mut counter = OperationCounter::new(key(4), 255);
            begin(&mut a, &mut p, &mut counter, 0, 4_000).unwrap();
            let before = a.utilization_band;
            assert_eq!(
                settle(&mut a, &p, &mut intent),
                BandSettlement::Stale(StaleReason::AccountingIndex)
            );
            assert_eq!(a.utilization_band, before);
            assert_eq!(intent.status, IntentStatus::Stale);
        }

        #[test]
        fn band_goes_stale_on_pool_generation_and_rate_epoch_changes() {
            let (mut a, mut p) = (asset(), pool());
            let mut intent = queue_band(&a, &p);
            p.coordination.generation += 1;
            assert_eq!(
                settle(&mut a, &p, &mut intent),
                BandSettlement::Stale(StaleReason::PoolGeneration)
            );
            let (mut a, p) = (asset(), pool());
            let mut intent = queue_band(&a, &p);
            a.rate_epoch_generation += 1;
            assert_eq!(
                settle(&mut a, &p, &mut intent),
                BandSettlement::Stale(StaleReason::RateEpoch)
            );
        }
    }
}
