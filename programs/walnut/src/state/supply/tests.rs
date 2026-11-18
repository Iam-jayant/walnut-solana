//! Unit tests of the supply and withdraw-supply rules (Task 8.2, 8.3).

use super::*;
use crate::market::pause_flags;
use crate::state::nonce::output_nonce;
use crate::state::test_fixtures::{
    asset, key, market, pool, position, MARKET, OWNER, POOL, POSITION,
};
use crate::state::EncryptedPositionPayload;

struct World {
    market: Market,
    asset: AssetState,
    pool: PoolState,
    position: Position,
    counter: OperationCounter,
}

fn world() -> World {
    World {
        market: market(),
        asset: asset(),
        pool: pool(),
        position: position(),
        counter: OperationCounter::new(key(OWNER), 255),
    }
}

fn request(is_withdraw: bool, requested: u128) -> SupplyFlowRequest {
    SupplyFlowRequest {
        intent_key: key(0x70),
        sequence: 0,
        computation_offset: 4_242,
        queued_slot: 100,
        now_ts: 2_000,
        is_withdraw,
        requested,
    }
}

fn queue_as(
    w: &mut World,
    signer: &Pubkey,
    is_withdraw: bool,
    requested: u128,
) -> Result<OperationIntent, ErrorCode> {
    queue_supply_flow(
        &w.market,
        key(MARKET),
        w.market.asset_state,
        &mut w.asset,
        &mut w.pool,
        key(POOL),
        &mut w.position,
        key(POSITION),
        &mut w.counter,
        signer,
        &request(is_withdraw, requested),
    )
}

fn queue(w: &mut World, is_withdraw: bool, requested: u128) -> OperationIntent {
    queue_as(w, &key(OWNER), is_withdraw, requested).unwrap()
}

type Snapshot = (
    AssetState,
    crate::state::Coordination,
    crate::state::Coordination,
    u128,
    u64,
);

fn snap(w: &World) -> Snapshot {
    (
        w.asset.clone(),
        w.pool.coordination,
        w.position.coordination,
        w.pool.accounted_borrow_index,
        w.counter.next_sequence,
    )
}

fn report(w: &World, intent: &OperationIntent, applied: bool, requested: u128) -> SupplyFlowReport {
    SupplyFlowReport {
        output_authenticated: true,
        computation_offset: intent.computation_offset,
        intent_key: intent.key,
        pool_output: EncryptedPoolState {
            nonce: 999,
            ciphertexts: [[9; 32]; 4],
        },
        encryption_key: w.position.viewing_pubkey,
        position_nonce: output_nonce(&key(POSITION), intent.sequence),
        position_ciphertexts: [[8; 32]; 5],
        applied,
        requested,
        current_slot: intent.queued_slot + 10,
    }
}

fn settle(
    w: &mut World,
    intent: &mut OperationIntent,
    r: &SupplyFlowReport,
) -> Result<SupplyFlowSettlement, ErrorCode> {
    settle_supply_flow(
        &w.market,
        key(MARKET),
        &w.asset,
        &mut w.pool,
        key(POOL),
        &mut w.position,
        key(POSITION),
        intent,
        r,
    )
}

#[test]
fn queue_locks_pool_and_position_accrues_and_binds_the_request() {
    let mut w = world();
    let intent = queue(&mut w, false, 500);
    assert_eq!(intent.kind, OperationKind::Supply);
    assert_eq!(intent.position, Some(key(POSITION)));
    assert!(intent.lock_owner().holds(&w.pool.coordination.lock));
    assert!(intent.lock_owner().holds(&w.position.coordination.lock));
    assert!(
        w.asset.accounting_borrow_index > crate::math::WAD,
        "accrued"
    );
    assert_eq!(w.counter.next_sequence, 1);
    intent.validate_new().unwrap();
    let mut w = world();
    assert_eq!(queue(&mut w, true, 5).kind, OperationKind::WithdrawSupply);
}

#[test]
fn queue_rejects_non_owner_zero_inactive_and_contention_changing_nothing() {
    let mut w = world();
    let before = snap(&w);
    assert_eq!(
        queue_as(&mut w, &key(0x99), false, 5).unwrap_err(),
        ErrorCode::SupplyNotOwner
    );
    assert_eq!(
        queue_as(&mut w, &key(OWNER), false, 0).unwrap_err(),
        ErrorCode::SupplyZeroRequest
    );
    w.position.status = PositionStatus::Uninitialized;
    assert_eq!(
        queue_as(&mut w, &key(OWNER), false, 5).unwrap_err(),
        ErrorCode::SupplyPositionNotActive
    );
    assert_eq!(snap(&w), before);
    // Contention: a second request while the first holds the Locks.
    let mut w = world();
    queue(&mut w, false, 5);
    let before = snap(&w);
    assert_eq!(
        queue_as(&mut w, &key(OWNER), true, 5).unwrap_err(),
        ErrorCode::PoolLockedForIndexMutation
    );
    assert_eq!(snap(&w), before);
}

#[test]
fn pauses_gate_each_direction_and_a_stale_band_blocks_only_supply() {
    let mut w = world();
    w.asset.pauses.flags = pause_flags::SUPPLY;
    assert!(queue_as(&mut w, &key(OWNER), false, 5).is_err());
    assert!(queue_as(&mut w, &key(OWNER), true, 5).is_ok());
    let mut w = world();
    w.asset.pauses.flags = pause_flags::WITHDRAW_SUPPLY;
    assert!(queue_as(&mut w, &key(OWNER), true, 5).is_err());
    assert!(queue_as(&mut w, &key(OWNER), false, 5).is_ok());

    // Band observed at slot 90; the maximum age is 9,000 slots.
    let mut w = world();
    let mut late = request(false, 5);
    late.queued_slot = 90 + w.asset.rates.band_max_age_slots + 1;
    let before = snap(&w);
    let attempt = |w: &mut World, r: &SupplyFlowRequest| {
        queue_supply_flow(
            &w.market,
            key(MARKET),
            w.market.asset_state,
            &mut w.asset,
            &mut w.pool,
            key(POOL),
            &mut w.position,
            key(POSITION),
            &mut w.counter,
            &key(OWNER),
            r,
        )
    };
    assert_eq!(
        attempt(&mut w, &late).unwrap_err(),
        ErrorCode::UtilizationBandStale
    );
    assert_eq!(snap(&w), before);
    // Withdrawal needs no fresh band.
    late.is_withdraw = true;
    assert!(attempt(&mut w, &late).is_ok());
    // Never observed.
    let mut w = world();
    w.asset.utilization_observation_slot = 0;
    assert_eq!(
        queue_as(&mut w, &key(OWNER), false, 5).unwrap_err(),
        ErrorCode::UtilizationBandNeverObserved
    );
}

#[test]
fn applied_stores_both_payloads_bumps_both_generations_and_releases_both_locks() {
    let mut w = world();
    let mut intent = queue(&mut w, false, 500);
    let (pg, sg) = (
        w.pool.coordination.generation,
        w.position.coordination.generation,
    );
    let r = report(&w, &intent, true, 500);
    assert_eq!(
        settle(&mut w, &mut intent, &r),
        Ok(SupplyFlowSettlement::Applied { is_withdraw: false })
    );
    assert_eq!(w.pool.encrypted, r.pool_output);
    assert_eq!(
        w.position.encrypted_state,
        EncryptedPositionPayload {
            nonce: r.position_nonce,
            ciphertexts: r.position_ciphertexts,
        }
    );
    assert_eq!(w.pool.coordination.generation, pg + 1);
    assert_eq!(w.position.coordination.generation, sg + 1);
    assert!(!w.pool.coordination.lock.is_active());
    assert!(!w.position.coordination.lock.is_active());
    assert_eq!(
        w.pool.accounted_borrow_index,
        intent.accounting_borrow_index
    );
    assert_eq!(intent.status, IntentStatus::Applied);
}

#[test]
fn zero_stores_nothing_and_does_not_advance_the_accounted_index() {
    let mut w = world();
    let mut intent = queue(&mut w, true, 5);
    let (pool_before, pos_before) = (w.pool.encrypted, w.position.encrypted_state);
    let r = report(&w, &intent, false, 5);
    assert_eq!(
        settle(&mut w, &mut intent, &r),
        Ok(SupplyFlowSettlement::Zero)
    );
    assert_eq!(
        (w.pool.encrypted, w.position.encrypted_state),
        (pool_before, pos_before)
    );
    assert_eq!(w.pool.accounted_borrow_index, crate::math::WAD);
    assert!(!w.pool.coordination.lock.is_active());
    assert!(!w.position.coordination.lock.is_active());
}

#[test]
fn every_binding_mismatch_reverts_and_leaves_the_intent_queued() {
    type Tamper = fn(&mut SupplyFlowReport);
    let tampers: [(&str, Tamper); 3] = [
        ("echoed request", |r| r.requested += 1),
        ("output nonce", |r| r.position_nonce += 1),
        ("encryption key", |r| r.encryption_key = [0x77; 32]),
    ];
    for (name, tamper) in tampers {
        let mut w = world();
        let mut intent = queue(&mut w, false, 500);
        let before = (w.pool.clone(), w.position.clone());
        let mut r = report(&w, &intent, true, 500);
        tamper(&mut r);
        assert!(settle(&mut w, &mut intent, &r).is_err(), "{name}");
        assert_eq!(intent.status, IntentStatus::Queued, "{name}");
        assert_eq!(w.pool.encrypted, before.0.encrypted, "{name}");
        assert_eq!(
            w.position.encrypted_state, before.1.encrypted_state,
            "{name}"
        );
        assert!(w.pool.coordination.lock.is_active(), "{name}");
    }
    // Unauthenticated output.
    let mut w = world();
    let mut intent = queue(&mut w, false, 500);
    let mut r = report(&w, &intent, true, 500);
    r.output_authenticated = false;
    assert_eq!(
        settle(&mut w, &mut intent, &r),
        Err(ErrorCode::CallbackOutputNotAuthenticated)
    );
    // A different Position's key, a changed claim flag, or a changed cap.
    let mut w = world();
    let mut intent = queue(&mut w, false, 500);
    let r = report(&w, &intent, true, 500);
    w.market.unsettled_recovery_claim_count = 1;
    assert!(settle(&mut w, &mut intent, &r).is_err());
    w.market.unsettled_recovery_claim_count = 0;
    w.asset.caps.supply_cap_usdc = 7;
    assert!(settle(&mut w, &mut intent, &r).is_err());
}

#[test]
fn expired_and_abandoned_attempts_store_nothing_and_late_callbacks_are_ignored() {
    let mut w = world();
    let mut intent = queue(&mut w, false, 500);
    let stored = (w.pool.encrypted, w.position.encrypted_state);
    let mut r = report(&w, &intent, true, 500);
    r.current_slot = intent.decision_expiry_slot + 1;
    assert_eq!(
        settle(&mut w, &mut intent, &r),
        Ok(SupplyFlowSettlement::Failed(IntentStatus::Expired))
    );
    assert_eq!((w.pool.encrypted, w.position.encrypted_state), stored);
    assert!(!w.pool.coordination.lock.is_active());
    assert!(!w.position.coordination.lock.is_active());

    let mut w = world();
    let mut intent = queue(&mut w, false, 500);
    let expiry = intent.lock_expiry_slot;
    assert!(abandon_supply_flow(
        &mut w.pool,
        key(POOL),
        &mut w.position,
        key(POSITION),
        &mut intent,
        expiry - 1
    )
    .is_err());
    assert_eq!(
        abandon_supply_flow(
            &mut w.pool,
            key(POOL),
            &mut w.position,
            key(POSITION),
            &mut intent,
            expiry + 1
        ),
        Ok(2)
    );
    assert_eq!(intent.status, IntentStatus::Abandoned);
    let r = report(&w, &intent, true, 500);
    assert_eq!(
        settle(&mut w, &mut intent, &r),
        Ok(SupplyFlowSettlement::Ignored)
    );
}

#[test]
fn output_nonces_are_unique_per_attempt_and_never_the_stored_one() {
    let a = output_nonce(&key(POSITION), 0);
    let b = output_nonce(&key(POSITION), 1);
    assert_ne!(a, b);
    let mut w = world();
    let mut intent = queue(&mut w, false, 500);
    let mut r = report(&w, &intent, true, 500);
    r.pool_output.nonce = w.pool.encrypted.nonce;
    assert_eq!(
        settle(&mut w, &mut intent, &r),
        Err(ErrorCode::PoolOutputNonceReused)
    );
}
