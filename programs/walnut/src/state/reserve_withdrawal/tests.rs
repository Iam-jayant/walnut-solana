//! Unit tests of the reserve withdrawal rules (Task 7.5).

use super::*;
use crate::errors::ErrorCode;
use crate::market::{
    pause_flags, Caps, MarketStatus, PauseState, RateParams, RiskParams, TimelockState,
};
use crate::math::WAD;
use crate::operation::StaleReason;
use crate::state::accounts::PoolStatus;
use crate::state::coordination::Coordination;

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}

const TREASURY: u8 = 5;
const TREASURY_TOKEN: u8 = 0x60;
const POOL: u8 = 0x42;
const MARKET: u8 = 0x30;

pub(super) fn market() -> Market {
    Market {
        layout_version: 1,
        status: MarketStatus::Active,
        bump: 0xFE,
        vault_authority_bump: 0xFD,
        admin: key(1),
        pending_admin: Pubkey::default(),
        upgrade_authority: key(3),
        risk_authority: key(4),
        treasury_authority: key(TREASURY),
        guardian_authority: key(6),
        asset_state: key(0x41),
        pool_state: key(POOL),
        recovery_position: key(0x43),
        vault_authority: key(0x44),
        wsol_mint: key(0x45),
        usdc_mint: key(0x46),
        wsol_vault: key(0x47),
        usdc_vault: key(0x48),
        treasury_usdc_account: key(TREASURY_TOKEN),
        position_count: 0,
        debt_position_count: 0,
        unsettled_recovery_claim_count: 0,
        position_count_cap: 10,
        debt_position_count_cap: 5,
        initialized_slot: 1,
        debt_position_reservations: 0,
        reserved: [0; 120],
    }
}

pub(super) fn asset() -> AssetState {
    AssetState {
        layout_version: 1,
        bump: 1,
        market: key(MARKET),
        generation: 5,
        oracle_generation: 2,
        rate_epoch_generation: 3,
        accounting_borrow_index: WAD,
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

pub(super) fn pool() -> PoolState {
    PoolState {
        coordination: Coordination {
            generation: 9,
            ..Coordination::default()
        },
        bump: 1,
        layout_version: 1,
        status: PoolStatus::Active,
        market: key(MARKET),
        accounted_borrow_index: WAD,
        encrypted: EncryptedPoolState {
            nonce: 77,
            ciphertexts: [[1; 32], [2; 32], [3; 32], [4; 32]],
        },
        unsettled_recovery_claim_count: 0,
    }
}

fn destination() -> TokenAccountView {
    TokenAccountView {
        key: key(TREASURY_TOKEN),
        mint: key(0x46),
        owner: key(TREASURY),
    }
}

fn request(amount: u64) -> ReserveWithdrawalRequest {
    ReserveWithdrawalRequest {
        intent_key: key(0x70),
        sequence: 0,
        computation_offset: 4_242,
        queued_slot: 100,
        now_ts: 2_000,
        amount,
    }
}

struct World {
    market: Market,
    asset: AssetState,
    pool: PoolState,
    counter: OperationCounter,
}

fn world() -> World {
    World {
        market: market(),
        asset: asset(),
        pool: pool(),
        counter: OperationCounter::new(key(POOL), 255),
    }
}

fn queue_as(
    w: &mut World,
    signer: &Pubkey,
    dest: &TokenAccountView,
    amount: u64,
) -> Result<OperationIntent, ErrorCode> {
    queue_reserve_withdrawal(
        &w.market,
        key(MARKET),
        w.market.asset_state,
        &mut w.asset,
        &mut w.pool,
        key(POOL),
        &mut w.counter,
        signer,
        dest,
        &request(amount),
    )
}

fn queue(w: &mut World, amount: u64) -> OperationIntent {
    queue_as(w, &key(TREASURY), &destination(), amount).unwrap()
}

fn code<T>(result: Result<T, ErrorCode>) -> u32 {
    match result {
        Ok(_) => panic!("expected an error"),
        Err(error) => u32::from(error),
    }
}

fn snap(w: &World) -> (AssetState, Coordination, u128, EncryptedPoolState, u64) {
    (
        w.asset.clone(),
        w.pool.coordination,
        w.pool.accounted_borrow_index,
        w.pool.encrypted,
        w.counter.next_sequence,
    )
}

fn out_state() -> EncryptedPoolState {
    EncryptedPoolState {
        nonce: 999,
        ciphertexts: [[9; 32]; 4],
    }
}

fn report(intent: &OperationIntent, approved: u64, requested: u64) -> ReserveWithdrawalReport {
    ReserveWithdrawalReport {
        output_authenticated: true,
        computation_offset: intent.computation_offset,
        intent_key: intent.key,
        output: out_state(),
        approved_amount: approved,
        requested_amount: requested,
        destination: destination(),
        mint: key(0x46),
        vault: key(0x48),
        vault_authority: key(0x44),
        token_program: anchor_spl::token::ID,
        current_slot: intent.queued_slot + 10,
    }
}

fn settle(
    w: &mut World,
    intent: &mut OperationIntent,
    r: &ReserveWithdrawalReport,
) -> Result<ReserveWithdrawalSettlement, ErrorCode> {
    settle_reserve_withdrawal(
        &w.market,
        key(MARKET),
        &w.asset,
        &mut w.pool,
        key(POOL),
        intent,
        r,
    )
}

#[test]
fn queue_locks_the_pool_accrues_and_binds_the_request() {
    let mut w = world();
    let intent = queue(&mut w, 500);
    assert_eq!(intent.kind, OperationKind::ReserveWithdrawal);
    assert_eq!(intent.payer, key(TREASURY));
    assert_eq!(intent.owner, key(POOL));
    assert!(intent.lock_owner().holds(&w.pool.coordination.lock));
    assert!(w.asset.accounting_borrow_index > WAD, "accrued at queue");
    assert_eq!(
        intent.accounting_borrow_index,
        w.asset.accounting_borrow_index
    );
    assert!(intent.risk_borrow_index_at_expiry > intent.accounting_borrow_index);
    assert_eq!(w.counter.next_sequence, 1);
    intent.validate_new().unwrap();
}

#[test]
fn only_the_treasury_authority_may_queue() {
    for signer in [key(1), key(3), key(4), key(6), key(0x99)] {
        let mut w = world();
        let before = snap(&w);
        assert_eq!(
            code(queue_as(&mut w, &signer, &destination(), 500)),
            u32::from(ErrorCode::NotTreasuryAuthority)
        );
        assert_eq!(snap(&w), before);
    }
}

#[test]
fn queue_rejects_zero_unset_and_wrong_destinations_changing_nothing() {
    let mut w = world();
    let before = snap(&w);
    assert_eq!(
        code(queue_as(&mut w, &key(TREASURY), &destination(), 0)),
        u32::from(ErrorCode::ReserveWithdrawalZeroAmount)
    );
    let wrong = [
        TokenAccountView {
            key: key(0x61),
            ..destination()
        },
        TokenAccountView {
            mint: key(0x45),
            ..destination()
        },
        TokenAccountView {
            owner: key(0x99),
            ..destination()
        },
    ];
    for dest in wrong {
        assert_eq!(
            code(queue_as(&mut w, &key(TREASURY), &dest, 5)),
            u32::from(ErrorCode::TreasuryAccountInvalid)
        );
    }
    w.market.treasury_usdc_account = Pubkey::default();
    assert_eq!(
        code(queue_as(&mut w, &key(TREASURY), &destination(), 5)),
        u32::from(ErrorCode::TreasuryAccountNotSet)
    );
    assert_eq!(snap(&w), before);
}

#[test]
fn queue_obeys_the_withdraw_supply_pause_and_a_locked_pool() {
    let mut w = world();
    w.asset.pauses.flags = pause_flags::WITHDRAW_SUPPLY;
    let before = snap(&w);
    assert_eq!(
        code(queue_as(&mut w, &key(TREASURY), &destination(), 5)),
        u32::from(ErrorCode::ActionPaused)
    );
    assert_eq!(snap(&w), before);
    w.asset.pauses.flags = 0;
    queue(&mut w, 5);
    let locked = snap(&w);
    let mut again = request(5);
    again.sequence = 1;
    again.intent_key = key(0x71);
    let result = queue_reserve_withdrawal(
        &w.market,
        key(MARKET),
        w.market.asset_state,
        &mut w.asset,
        &mut w.pool,
        key(POOL),
        &mut w.counter,
        &key(TREASURY),
        &destination(),
        &again,
    );
    assert_eq!(
        code(result),
        u32::from(ErrorCode::PoolLockedForIndexMutation)
    );
    assert_eq!(snap(&w), locked);
}

#[test]
fn applied_stores_state_and_returns_the_exact_transfer() {
    let mut w = world();
    let mut intent = queue(&mut w, 500);
    let accounting = intent.accounting_borrow_index;
    let r = report(&intent, 500, 500);
    let s = settle(&mut w, &mut intent, &r).unwrap();
    assert_eq!(
        s,
        ReserveWithdrawalSettlement::Approved {
            amount: 500,
            destination: key(TREASURY_TOKEN)
        }
    );
    assert_eq!(intent.status, IntentStatus::Applied);
    assert_eq!(w.pool.encrypted, out_state());
    assert_eq!(w.pool.coordination.generation, 10);
    assert!(!w.pool.coordination.lock.is_active());
    assert_eq!(w.pool.accounted_borrow_index, accounting);
}

#[test]
fn zero_stale_expired_and_unauthenticated_store_nothing() {
    // Zero.
    let mut w = world();
    let mut intent = queue(&mut w, 500);
    let stored = w.pool.encrypted;
    let r = report(&intent, 0, 500);
    assert_eq!(
        settle(&mut w, &mut intent, &r).unwrap(),
        ReserveWithdrawalSettlement::Zero
    );
    assert_eq!(intent.status, IntentStatus::Zero);
    assert_eq!(w.pool.encrypted, stored);
    assert_eq!(w.pool.accounted_borrow_index, WAD);
    assert_eq!(w.pool.coordination.generation, 9);
    assert!(!w.pool.coordination.lock.is_active());

    // Stale: the Asset State generation moved (a governed change).
    let mut w = world();
    let mut intent = queue(&mut w, 500);
    w.asset.generation += 1;
    let r = report(&intent, 500, 500);
    assert_eq!(
        settle(&mut w, &mut intent, &r).unwrap(),
        ReserveWithdrawalSettlement::Failed(IntentStatus::Stale)
    );
    assert_eq!(w.pool.encrypted, pool().encrypted);
    assert!(!w.pool.coordination.lock.is_active());

    // Stale: the rate epoch moved.
    let mut w = world();
    let mut intent = queue(&mut w, 500);
    w.asset.rate_epoch_generation += 1;
    let r = report(&intent, 500, 500);
    settle(&mut w, &mut intent, &r).unwrap();
    assert_eq!(intent.status, IntentStatus::Stale);
    let _ = StaleReason::RateEpoch;

    // Expired.
    let mut w = world();
    let mut intent = queue(&mut w, 500);
    let mut r = report(&intent, 500, 500);
    r.current_slot = intent.decision_expiry_slot + 1;
    assert_eq!(
        settle(&mut w, &mut intent, &r).unwrap(),
        ReserveWithdrawalSettlement::Failed(IntentStatus::Expired)
    );
    assert_eq!(w.pool.encrypted, pool().encrypted);

    // Unauthenticated: reverts and leaves everything.
    let mut w = world();
    let mut intent = queue(&mut w, 500);
    let before = snap(&w);
    let mut r = report(&intent, 500, 500);
    r.output_authenticated = false;
    assert_eq!(
        code(settle(&mut w, &mut intent, &r)),
        u32::from(ErrorCode::CallbackOutputNotAuthenticated)
    );
    assert_eq!(intent.status, IntentStatus::Queued);
    assert_eq!(snap(&w), before);
    assert!(w.pool.coordination.lock.is_active());
}

#[test]
fn every_binding_is_checked_before_anything_is_applied() {
    type Tamper = fn(&mut ReserveWithdrawalReport);
    let tampers: [(&str, Tamper); 8] = [
        ("destination key", |r| r.destination.key = key(0x61)),
        ("destination owner", |r| r.destination.owner = key(0x99)),
        ("destination mint", |r| r.destination.mint = key(0x45)),
        ("vault", |r| r.vault = key(0x47)),
        ("mint", |r| r.mint = key(0x45)),
        ("vault authority", |r| r.vault_authority = key(0x99)),
        ("token program", |r| r.token_program = key(0x98)),
        ("requested echo", |r| r.requested_amount = 501),
    ];
    for (name, tamper) in tampers {
        let mut w = world();
        let mut intent = queue(&mut w, 500);
        let before = snap(&w);
        let mut r = report(&intent, 500, 500);
        tamper(&mut r);
        assert!(settle(&mut w, &mut intent, &r).is_err(), "{name}");
        assert_eq!(intent.status, IntentStatus::Queued, "{name}");
        assert_eq!(snap(&w), before, "{name}");
        assert!(w.pool.coordination.lock.is_active(), "{name}");
    }
}

#[test]
fn a_partial_or_mismatched_approval_is_rejected() {
    for approved in [1u64, 499, 501] {
        let mut w = world();
        let mut intent = queue(&mut w, 500);
        let before = snap(&w);
        let r = report(&intent, approved, 500);
        assert_eq!(
            code(settle(&mut w, &mut intent, &r)),
            u32::from(ErrorCode::ReserveWithdrawalApprovalInvalid)
        );
        assert_eq!(snap(&w), before);
    }
}

#[test]
fn a_treasury_authority_rotated_after_queue_cannot_receive_the_transfer() {
    let mut w = world();
    let mut intent = queue(&mut w, 500);
    // The token owner still matches the commitment, but the Market now names
    // another treasury authority.
    w.market.treasury_authority = key(0x77);
    let before = snap(&w);
    let r = report(&intent, 500, 500);
    assert_eq!(
        code(settle(&mut w, &mut intent, &r)),
        u32::from(ErrorCode::TreasuryAccountInvalid)
    );
    assert_eq!(snap(&w), before);
}

#[test]
fn duplicate_and_late_callbacks_after_a_terminal_intent_change_nothing() {
    let mut w = world();
    let mut intent = queue(&mut w, 500);
    let r = report(&intent, 500, 500);
    settle(&mut w, &mut intent, &r).unwrap();
    let before = snap(&w);
    assert_eq!(
        settle(&mut w, &mut intent, &r).unwrap(),
        ReserveWithdrawalSettlement::Ignored
    );
    assert_eq!(snap(&w), before);

    // Late callback after abandonment.
    let mut w = world();
    let mut intent = queue(&mut w, 500);
    let lock_expiry = intent.lock_expiry_slot;
    abandon_reserve_withdrawal(&mut w.pool, key(POOL), &mut intent, lock_expiry + 1).unwrap();
    let before = snap(&w);
    let mut r = report(&intent, 500, 500);
    r.current_slot = intent.lock_expiry_slot + 2;
    assert_eq!(
        settle(&mut w, &mut intent, &r).unwrap(),
        ReserveWithdrawalSettlement::Ignored
    );
    assert_eq!(snap(&w), before);
    assert_eq!(w.pool.coordination.generation, 10);
}

#[test]
fn abandonment_waits_for_lock_expiry_and_bumps_the_generation() {
    let mut w = world();
    let mut intent = queue(&mut w, 500);
    let lock_expiry = intent.lock_expiry_slot;
    assert!(abandon_reserve_withdrawal(&mut w.pool, key(POOL), &mut intent, lock_expiry).is_err());
    assert_eq!(w.pool.coordination.generation, 9);
    assert_eq!(
        abandon_reserve_withdrawal(&mut w.pool, key(POOL), &mut intent, lock_expiry + 1).unwrap(),
        1
    );
    assert_eq!(intent.status, IntentStatus::Abandoned);
    assert_eq!(w.pool.coordination.generation, 10);
    assert!(!w.pool.coordination.lock.is_active());
}

#[test]
fn the_admin_sets_the_treasury_account_with_the_right_mint_and_owner() {
    let mut m = market();
    m.treasury_usdc_account = Pubkey::default();
    let good = destination();
    // Only the admin.
    for signer in [key(TREASURY), key(4), key(0x99)] {
        assert!(set_treasury_usdc_account(&mut m, &signer, &good).is_err());
    }
    for bad in [
        TokenAccountView {
            mint: key(0x45),
            ..good
        },
        TokenAccountView {
            owner: key(0x99),
            ..good
        },
        TokenAccountView {
            key: key(0x48),
            ..good
        },
        TokenAccountView {
            key: Pubkey::default(),
            ..good
        },
    ] {
        assert_eq!(
            code(set_treasury_usdc_account(&mut m, &key(1), &bad)),
            u32::from(ErrorCode::TreasuryAccountInvalid)
        );
        assert_eq!(m.treasury_usdc_account, Pubkey::default());
    }
    assert_eq!(
        set_treasury_usdc_account(&mut m, &key(1), &good).unwrap(),
        Pubkey::default()
    );
    assert_eq!(m.treasury_usdc_account, good.key);
}
