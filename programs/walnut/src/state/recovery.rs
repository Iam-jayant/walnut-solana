//! `recover_empty_pool`: the separately reviewed recovery operation that must
//! restore the canonical empty state before supply reopens (Task 8.1; design
//! "Supply"; Requirement 8.2 and 8.3).
//!
//! # What it is for
//!
//! The first-supplier rule ([`super::first_supplier`]) refuses a Pool that has
//! no Supply Shares but still has residue. The residue the owner of the code
//! can reach without a person deciding is rounding dust: after the last
//! supplier withdraws, `floor` leaves `cash - reserves` of a few base units
//! that belong to nobody. This operation moves exactly that residue into
//! Reserves; the treasury then withdraws Reserves with `reserve_withdrawal`
//! and the Pool is canonically empty again.
//!
//! # Authority and effect (review caveat)
//!
//! * The signer must be the Market admin (`Role::Admin`). Design is silent on
//!   the authority; admin-only is the narrowest existing role that is not the
//!   treasury (the treasury is the beneficiary of the reclassified Reserves,
//!   so it must not also decide when residue exists).
//! * The Pool is Locked for the whole operation ([`begin_pool_mutation`]), so
//!   no supply, withdrawal, or index accrual interleaves.
//! * The circuit changes ONE encrypted field: `reserves' = cash`, and only
//!   when, after the interest checkpoint, Supply Shares and Debt Shares are
//!   zero, the Market's `unsettled_recovery_claim_count` is zero, and the
//!   residue `cash - reserves` is nonzero and at most
//!   [`RECOVERY_DUST_LIMIT`](crate::math::supply::RECOVERY_DUST_LIMIT)
//!   (0.01 USDC, bound in the Intent). Anything else is Zero. No token moves,
//!   Cash and shares are untouched, and no supplier equity changes (it is zero
//!   by definition: no shares exist).
//! * It cannot clear Debt Shares, Supply Shares, or unsettled Recovery Claims.
//!   Those need repay, Absorb, and `settle_recovery_claim`.
//!
//! A reviewer should check: (1) the admin cannot use it to take value from
//! suppliers (no shares exist and the residue is capped); (2) the cap, not the
//! signer, bounds the amount; (3) `reserve_withdrawal` is the only way the
//! residue leaves the vault.
//!
//! # Flow
//!
//! Identical to `reserve_withdrawal`: queue ([`queue_recover_empty_pool`]),
//! callback ([`settle_recover_empty_pool`]), timeout recovery
//! ([`abandon_recover_empty_pool`]). `Zero` stores nothing.
//!
//! # Intent binding
//!
//! `accounts_commitment`: signer, Market, Asset State, Pool (captured
//! generation). `inputs_commitment`: the Pool payload, both indexes, the
//! reserve factor, the claims-clear flag, and the dust limit.

use anchor_lang::prelude::Pubkey;

use super::accounts::{EncryptedPoolState, PoolState};
use super::band::{INPUT_ACCOUNTING_INDEX, INPUT_POOL_CIPHERTEXT_FIRST, INPUT_POOL_NONCE};
use super::coordination::CoordinationSlot;
use super::pool_mutation::{
    abandon_pool_mutation, begin_pool_mutation, pool_dependencies, settle_pool_decision,
    IntentCommitments, PoolCapture, PoolMutationRequest,
};
use super::reserve_withdrawal::{INPUT_ACCOUNTED_INDEX, INPUT_RESERVE_FACTOR_BPS};
use super::supply::INPUT_CLAIMS_CLEAR;
use super::Slot;
use crate::errors::ErrorCode;
use crate::market::{authorize, AssetState, Market, Role};
use crate::math::supply::RECOVERY_DUST_LIMIT;
use crate::operation::callback::{
    classify_callback, CallbackContext, CallbackDecision, CallbackReport,
};
use crate::operation::commitment::{
    account_commitment, input_commitment, AccountBinding, AccountRole, InputField, InputValue,
};
use crate::operation::{IntentStatus, OperationCounter, OperationIntent, OperationKind};

/// Input field id: the dust limit (`U128`).
pub const INPUT_DUST_LIMIT: u8 = 30;
/// Decision Expiry distance, in slots.
pub const RECOVERY_DECISION_SLOTS: u64 = 300;
/// Slots between Decision Expiry and Lock expiry.
pub const RECOVERY_LOCK_GRACE_SLOTS: u64 = 150;

/// The ordered accounts bound by a recovery Intent.
pub fn recovery_bindings(
    authority: Pubkey,
    market_key: Pubkey,
    asset_state: Pubkey,
    pool: Pubkey,
    pool_generation: Option<u64>,
) -> [AccountBinding; 4] {
    let program = crate::ID;
    let plain = |key, role, expected_owner, writable, generation| AccountBinding {
        key,
        role,
        expected_owner,
        mint: None,
        authority: None,
        writable,
        generation,
    };
    [
        plain(
            authority,
            AccountRole::Owner,
            anchor_lang::system_program::ID,
            true,
            None,
        ),
        plain(market_key, AccountRole::Market, program, false, None),
        plain(asset_state, AccountRole::AssetState, program, false, None),
        plain(pool, AccountRole::Pool, program, true, pool_generation),
    ]
}

/// The ordered inputs bound by a recovery Intent.
pub fn recovery_inputs(
    pool: &PoolState,
    accounted_index: u128,
    accounting_index: u128,
    reserve_factor_bps: u16,
    claims_clear: bool,
) -> [InputField<'static>; 10] {
    let c = &pool.encrypted.ciphertexts;
    let first = INPUT_POOL_CIPHERTEXT_FIRST;
    [
        InputField::new(INPUT_POOL_NONCE, InputValue::U128(pool.encrypted.nonce)),
        InputField::new(first, InputValue::Bytes32(c[0])),
        InputField::new(first + 1, InputValue::Bytes32(c[1])),
        InputField::new(first + 2, InputValue::Bytes32(c[2])),
        InputField::new(first + 3, InputValue::Bytes32(c[3])),
        InputField::new(INPUT_ACCOUNTING_INDEX, InputValue::U128(accounting_index)),
        InputField::new(INPUT_ACCOUNTED_INDEX, InputValue::U128(accounted_index)),
        InputField::new(
            INPUT_RESERVE_FACTOR_BPS,
            InputValue::U64(reserve_factor_bps as u64),
        ),
        InputField::new(INPUT_CLAIMS_CLEAR, InputValue::Bool(claims_clear)),
        InputField::new(INPUT_DUST_LIMIT, InputValue::U128(RECOVERY_DUST_LIMIT)),
    ]
}

/// One `recover_empty_pool` queue request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoverEmptyPoolRequest {
    /// The Operation Intent PDA address.
    pub intent_key: Pubkey,
    /// The Pool counter's next Intent sequence.
    pub sequence: u64,
    /// The Arcium computation offset.
    pub computation_offset: u64,
    /// The current slot.
    pub queued_slot: Slot,
    /// The current Unix time.
    pub now_ts: i64,
}

/// Queues a recovery: admin only, then the shared Pool mutation queue. Atomic.
#[allow(clippy::too_many_arguments)]
pub fn queue_recover_empty_pool(
    market: &Market,
    market_key: Pubkey,
    asset_state_key: Pubkey,
    asset: &mut AssetState,
    pool: &mut PoolState,
    pool_key: Pubkey,
    counter: &mut OperationCounter,
    signer: &Pubkey,
    request: &RecoverEmptyPoolRequest,
) -> Result<OperationIntent, ErrorCode> {
    authorize(market, Role::Admin, signer)?;
    let accounted = pool.accounted_borrow_index;
    let reserve_factor_bps = asset.risk.reserve_factor_bps;
    let claims_clear = super::first_supplier::claims_clear(market);
    let signer = *signer;
    let pool_snapshot = pool.clone();
    let mutation = PoolMutationRequest {
        intent_key: request.intent_key,
        owner: pool_key,
        payer: signer,
        kind: OperationKind::RecoverEmptyPool,
        sequence: request.sequence,
        computation_offset: request.computation_offset,
        queued_slot: request.queued_slot,
        now_ts: request.now_ts,
        decision_slots: RECOVERY_DECISION_SLOTS,
        lock_grace_slots: RECOVERY_LOCK_GRACE_SLOTS,
    };
    begin_pool_mutation(
        market.status,
        asset,
        pool,
        pool_key,
        &mut [],
        counter,
        &mutation,
        |capture: &PoolCapture| {
            let bindings = recovery_bindings(
                signer,
                market_key,
                asset_state_key,
                pool_key,
                Some(capture.pool_generation),
            );
            let inputs = recovery_inputs(
                &pool_snapshot,
                accounted,
                capture.accounting_borrow_index,
                reserve_factor_bps,
                claims_clear,
            );
            IntentCommitments {
                accounts: account_commitment(&bindings),
                inputs: input_commitment(OperationKind::RecoverEmptyPool, &inputs),
            }
        },
    )
}

/// What the `recover_empty_pool` callback observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoverEmptyPoolReport {
    /// Whether Arcium `verify_output` succeeded.
    pub output_authenticated: bool,
    /// The computation offset the callback's computation account belongs to.
    pub computation_offset: u64,
    /// The Operation Intent account the callback received.
    pub intent_key: Pubkey,
    /// The output Pool payload.
    pub output: EncryptedPoolState,
    /// The revealed applied flag.
    pub applied: bool,
    /// The current slot.
    pub current_slot: Slot,
}

/// What the callback did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoverEmptyPoolSettlement {
    /// `Applied`: the Pool payload (`reserves := cash`) is stored.
    Recovered,
    /// The circuit refused: nothing stored.
    Zero,
    /// `Expired`, `Stale`, or `Failed`: nothing stored.
    Failed(IntentStatus),
    /// The Intent was already terminal; nothing changed.
    Ignored,
}

/// Verifies and applies a `recover_empty_pool` callback. Unauthenticated
/// output and binding mismatches are errors (the Intent stays `Queued`).
/// Atomic on error.
pub fn settle_recover_empty_pool(
    market: &Market,
    market_key: Pubkey,
    asset: &AssetState,
    pool: &mut PoolState,
    pool_key: Pubkey,
    intent: &mut OperationIntent,
    report: &RecoverEmptyPoolReport,
) -> Result<RecoverEmptyPoolSettlement, ErrorCode> {
    let bindings = recovery_bindings(
        intent.payer,
        market_key,
        market.asset_state,
        pool_key,
        intent.pool_generation,
    );
    let inputs = recovery_inputs(
        pool,
        pool.accounted_borrow_index,
        intent.accounting_borrow_index,
        asset.risk.reserve_factor_bps,
        super::first_supplier::claims_clear(market),
    );
    let context = CallbackContext {
        output_authenticated: report.output_authenticated,
        computation_offset: report.computation_offset,
        intent_key: report.intent_key,
        kind: OperationKind::RecoverEmptyPool,
        accounts: &bindings,
        input_fields: &inputs,
        current: pool_dependencies(pool, asset),
        current_slot: report.current_slot,
        report: CallbackReport::Success {
            nonzero: report.applied,
        },
    };
    let decision = classify_callback(intent, &context)?;
    match decision {
        CallbackDecision::LeaveQueued => return Err(ErrorCode::CallbackOutputNotAuthenticated),
        CallbackDecision::Ignore => return Ok(RecoverEmptyPoolSettlement::Ignored),
        _ => {}
    }
    let result = settle_pool_decision(
        pool,
        pool_key,
        &mut [],
        intent,
        decision,
        &report.output,
        report.current_slot,
    )?;
    Ok(match result.status {
        IntentStatus::Applied => RecoverEmptyPoolSettlement::Recovered,
        IntentStatus::Zero => RecoverEmptyPoolSettlement::Zero,
        status => RecoverEmptyPoolSettlement::Failed(status),
    })
}

/// `abandon_recover_empty_pool`: permissionless timeout recovery after Lock
/// expiry. Nothing moved, so nothing to refund. Returns the Locks cleared.
pub fn abandon_recover_empty_pool(
    pool: &mut PoolState,
    pool_key: Pubkey,
    intent: &mut OperationIntent,
    current_slot: Slot,
) -> Result<usize, ErrorCode> {
    let others: &mut [CoordinationSlot<'_>] = &mut [];
    abandon_pool_mutation(
        pool,
        pool_key,
        others,
        intent,
        OperationKind::RecoverEmptyPool,
        current_slot,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::test_fixtures::{asset, key, market, pool, ADMIN, MARKET, POOL};

    const SIGNER: u8 = ADMIN;

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

    fn request() -> RecoverEmptyPoolRequest {
        RecoverEmptyPoolRequest {
            intent_key: key(0x71),
            sequence: 0,
            computation_offset: 777,
            queued_slot: 100,
            now_ts: 2_000,
        }
    }

    fn queue(w: &mut World, signer: u8) -> Result<OperationIntent, ErrorCode> {
        queue_recover_empty_pool(
            &w.market,
            key(MARKET),
            w.market.asset_state,
            &mut w.asset,
            &mut w.pool,
            key(POOL),
            &mut w.counter,
            &key(signer),
            &request(),
        )
    }

    fn out_state() -> EncryptedPoolState {
        EncryptedPoolState {
            nonce: 4_242,
            ciphertexts: [[9; 32]; 4],
        }
    }

    fn report(intent: &OperationIntent, applied: bool) -> RecoverEmptyPoolReport {
        RecoverEmptyPoolReport {
            output_authenticated: true,
            computation_offset: intent.computation_offset,
            intent_key: intent.key,
            output: out_state(),
            applied,
            current_slot: intent.queued_slot + 10,
        }
    }

    fn settle(
        w: &mut World,
        intent: &mut OperationIntent,
        r: &RecoverEmptyPoolReport,
    ) -> Result<RecoverEmptyPoolSettlement, ErrorCode> {
        settle_recover_empty_pool(
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
    fn only_the_admin_may_queue_and_the_pool_is_locked() {
        for other in [3u8, 4, 5, 6, 0x99] {
            let mut w = world();
            let before = (w.pool.coordination, w.counter.next_sequence);
            assert_eq!(queue(&mut w, other).unwrap_err(), ErrorCode::NotMarketAdmin);
            assert_eq!((w.pool.coordination, w.counter.next_sequence), before);
        }
        let mut w = world();
        let intent = queue(&mut w, SIGNER).unwrap();
        assert_eq!(intent.kind, OperationKind::RecoverEmptyPool);
        assert!(intent.lock_owner().holds(&w.pool.coordination.lock));
        assert_eq!(w.counter.next_sequence, 1);
        // A second recovery (or any Pool mutation) is refused while Locked.
        assert_eq!(
            queue(&mut w, SIGNER).unwrap_err(),
            ErrorCode::PoolLockedForIndexMutation
        );
    }

    #[test]
    fn recovered_stores_the_payload_and_zero_stores_nothing() {
        let mut w = world();
        let mut intent = queue(&mut w, SIGNER).unwrap();
        let generation = w.pool.coordination.generation;
        let stored = w.pool.encrypted;
        let r = report(&intent, true);
        assert_eq!(
            settle(&mut w, &mut intent, &r),
            Ok(RecoverEmptyPoolSettlement::Recovered)
        );
        assert_eq!(w.pool.encrypted, out_state());
        assert_eq!(w.pool.coordination.generation, generation + 1);
        assert!(!w.pool.coordination.lock.is_active());
        assert_eq!(intent.status, IntentStatus::Applied);
        assert_ne!(stored, w.pool.encrypted);

        let mut w = world();
        let mut intent = queue(&mut w, SIGNER).unwrap();
        let stored = w.pool.encrypted;
        let r = report(&intent, false);
        assert_eq!(
            settle(&mut w, &mut intent, &r),
            Ok(RecoverEmptyPoolSettlement::Zero)
        );
        assert_eq!(w.pool.encrypted, stored);
        assert!(!w.pool.coordination.lock.is_active());
    }

    #[test]
    fn a_claim_count_change_after_queueing_reverts_the_callback() {
        let mut w = world();
        let mut intent = queue(&mut w, SIGNER).unwrap();
        w.market.unsettled_recovery_claim_count = 1; // the bound flag no longer matches
        let r = report(&intent, true);
        let stored = w.pool.encrypted;
        assert!(settle(&mut w, &mut intent, &r).is_err());
        assert_eq!(intent.status, IntentStatus::Queued);
        assert_eq!(w.pool.encrypted, stored);
        assert!(w.pool.coordination.lock.is_active());
    }

    #[test]
    fn unauthenticated_output_reverts_and_leaves_the_intent_queued() {
        let mut w = world();
        let mut intent = queue(&mut w, SIGNER).unwrap();
        let mut r = report(&intent, true);
        r.output_authenticated = false;
        assert_eq!(
            settle(&mut w, &mut intent, &r),
            Err(ErrorCode::CallbackOutputNotAuthenticated)
        );
        assert_eq!(intent.status, IntentStatus::Queued);
        assert!(w.pool.coordination.lock.is_active());
    }

    #[test]
    fn abandonment_after_lock_expiry_unlocks_and_blocks_late_callbacks() {
        let mut w = world();
        let mut intent = queue(&mut w, SIGNER).unwrap();
        let expiry = intent.lock_expiry_slot;
        assert!(
            abandon_recover_empty_pool(&mut w.pool, key(POOL), &mut intent, expiry - 1).is_err()
        );
        assert_eq!(
            abandon_recover_empty_pool(&mut w.pool, key(POOL), &mut intent, expiry + 1),
            Ok(1)
        );
        assert_eq!(intent.status, IntentStatus::Abandoned);
        let r = report(&intent, true);
        assert_eq!(
            settle(&mut w, &mut intent, &r),
            Ok(RecoverEmptyPoolSettlement::Ignored)
        );
    }
}
