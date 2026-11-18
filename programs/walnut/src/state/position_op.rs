//! Demo `position_op` (walnut-diffn): add or withdraw collateral, borrow, and
//! repay on the encrypted Position, as pure encrypted state updates (no token
//! transfer). Shaped like `state::unshield`: queue, callback, abandon.
//!
//! `position_op_rules` is the plain-Rust mirror of the `position_op` circuit.
//! `debt_shares_usdc` holds plain debt in USDC base units.
//!
//! The Intent's `inputs_commitment` covers the Viewing key, epoch, stored
//! nonce, `op`, `amount`, the five stored ciphertexts and the output-nonce
//! input. `price` and `ltv_bps` are circuit arguments only: the callback has
//! no echo of them (demo shortcut).

use anchor_lang::prelude::Pubkey;

use super::accounts::{EncryptedPositionPayload, Position, PositionStatus};
use super::coordination::{acquire_locks, CoordinationSlot};
use super::nonce::{derive_output_nonce_input, INPUT_OUTPUT_NONCE};
use super::position::{INPUT_NONCE, INPUT_VIEWING_KEY_EPOCH, INPUT_VIEWING_PUBKEY};
use super::shield::{INPUT_AMOUNT, INPUT_ASSET, INPUT_STATE_CIPHERTEXT_FIRST};
use super::Slot;
use crate::errors::ErrorCode;
use crate::market::{Market, MarketStatus};
use crate::operation::callback::{
    apply_decision_to_slots, classify_callback, CallbackContext, CallbackDecision, CallbackReport,
    CurrentDependencies,
};
use crate::operation::commitment::{
    account_commitment, input_commitment, AccountBinding, AccountRole, InputField, InputValue,
};
use crate::operation::seeds::reserve_intent;
use crate::operation::{IntentStatus, OperationCounter, OperationIntent, OperationKind};

/// Decision Expiry distance, in slots.
pub const POSITION_OP_DECISION_SLOTS: u64 = 1_500;
/// Slots between Decision Expiry and Lock expiry.
pub const POSITION_OP_LOCK_GRACE_SLOTS: u64 = 300;

/// Operation tags.
pub const OP_ADD_COLLATERAL: u8 = 0;
/// Withdraw collateral.
pub const OP_WITHDRAW_COLLATERAL: u8 = 1;
/// Borrow.
pub const OP_BORROW: u8 = 2;
/// Repay.
pub const OP_REPAY: u8 = 3;

/// The decrypted Position balances (`PositionSecret` without the unused
/// supply field). `debt` is plain USDC base units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PositionBalances {
    /// Free SOL (lamports).
    pub free_sol: u64,
    /// Free USDC (base units).
    pub free_usdc: u64,
    /// Collateral SOL (lamports).
    pub collateral_sol: u64,
    /// Debt (USDC base units).
    pub debt: u128,
}

/// Plain-Rust mirror of the `position_op` circuit. Returns the new balances
/// and whether the op applied; a refusal returns the input unchanged.
pub fn position_op_rules(
    state: PositionBalances,
    op: u8,
    amount: u64,
    price: u64,
    ltv_bps: u64,
) -> (PositionBalances, bool) {
    const SCALE: u128 = 1_000_000_000 * 10_000;
    let a = amount as u128;
    let limit = (price as u128) * (ltv_bps as u128);
    let mut s = state;
    let applied = match op {
        OP_ADD_COLLATERAL => a <= state.free_sol as u128,
        OP_WITHDRAW_COLLATERAL => {
            a <= state.collateral_sol as u128
                && state.debt * SCALE <= (state.collateral_sol as u128 - a) * limit
        }
        OP_BORROW => {
            (state.debt + a) * SCALE <= (state.collateral_sol as u128) * limit
                && state.free_usdc as u128 + a <= u64::MAX as u128
        }
        OP_REPAY => a <= state.free_usdc as u128 && a <= state.debt,
        _ => false,
    };
    if applied {
        match op {
            OP_ADD_COLLATERAL => {
                s.free_sol -= amount;
                s.collateral_sol += amount;
            }
            OP_WITHDRAW_COLLATERAL => {
                s.collateral_sol -= amount;
                s.free_sol += amount;
            }
            OP_BORROW => {
                s.debt += a;
                s.free_usdc += amount;
            }
            _ => {
                s.free_usdc -= amount;
                s.debt -= a;
            }
        }
    }
    (s, applied)
}

/// The ordered accounts bound by a PositionOp Intent.
pub fn position_op_bindings(
    owner: Pubkey,
    market_key: Pubkey,
    asset_state: Pubkey,
    position: Pubkey,
    position_generation: Option<u64>,
) -> [AccountBinding; 4] {
    let program = crate::ID;
    let plain = |key, role, expected_owner, writable, authority, generation| AccountBinding {
        key,
        role,
        expected_owner,
        mint: None,
        authority,
        writable,
        generation,
    };
    [
        plain(
            owner,
            AccountRole::Owner,
            anchor_lang::system_program::ID,
            true,
            None,
            None,
        ),
        plain(market_key, AccountRole::Market, program, false, None, None),
        plain(
            asset_state,
            AccountRole::AssetState,
            program,
            false,
            None,
            None,
        ),
        plain(
            position,
            AccountRole::Position,
            program,
            true,
            Some(owner),
            position_generation,
        ),
    ]
}

/// The ordered inputs bound by a PositionOp Intent.
pub fn position_op_inputs(
    viewing_pubkey: [u8; 32],
    viewing_key_epoch: u64,
    state: &EncryptedPositionPayload,
    op: u8,
    amount: u64,
    output_nonce_input: u128,
) -> [InputField<'static>; 11] {
    let c = &state.ciphertexts;
    let first = INPUT_STATE_CIPHERTEXT_FIRST;
    [
        InputField::new(INPUT_VIEWING_PUBKEY, InputValue::Bytes32(viewing_pubkey)),
        InputField::new(INPUT_VIEWING_KEY_EPOCH, InputValue::U64(viewing_key_epoch)),
        InputField::new(INPUT_NONCE, InputValue::U128(state.nonce)),
        InputField::new(INPUT_ASSET, InputValue::U8(op)),
        InputField::new(INPUT_AMOUNT, InputValue::U64(amount)),
        InputField::new(first, InputValue::Bytes32(c[0])),
        InputField::new(first + 1, InputValue::Bytes32(c[1])),
        InputField::new(first + 2, InputValue::Bytes32(c[2])),
        InputField::new(first + 3, InputValue::Bytes32(c[3])),
        InputField::new(first + 4, InputValue::Bytes32(c[4])),
        InputField::new(INPUT_OUTPUT_NONCE, InputValue::U128(output_nonce_input)),
    ]
}

/// One `position_op` queue request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PositionOpRequest {
    /// The Operation Intent PDA address.
    pub intent_key: Pubkey,
    /// The owner's next Intent sequence.
    pub sequence: u64,
    /// The Arcium computation offset.
    pub computation_offset: u64,
    /// The current slot.
    pub queued_slot: Slot,
    /// The operation tag.
    pub op: u8,
    /// The public amount.
    pub amount: u64,
}

/// `position_op`: checks the Market and the Position, builds and reserves the
/// Intent, and Locks the Position. Atomic. No pause check (demo).
pub fn queue_position_op(
    market: &Market,
    market_key: Pubkey,
    position: &mut Position,
    position_key: Pubkey,
    counter: &mut OperationCounter,
    signer: &Pubkey,
    request: &PositionOpRequest,
) -> Result<OperationIntent, ErrorCode> {
    if market.status != MarketStatus::Active {
        return Err(ErrorCode::MarketNotActive);
    }
    if *signer != position.owner {
        return Err(ErrorCode::PositionOpNotOwner);
    }
    if position.status != PositionStatus::Active {
        return Err(ErrorCode::PositionOpPositionNotActive);
    }
    let decision_expiry_slot = request
        .queued_slot
        .checked_add(POSITION_OP_DECISION_SLOTS)
        .ok_or(ErrorCode::InvalidIntentTiming)?;
    let lock_expiry_slot = decision_expiry_slot
        .checked_add(POSITION_OP_LOCK_GRACE_SLOTS)
        .ok_or(ErrorCode::InvalidIntentTiming)?;
    let generation = Some(position.coordination.generation);
    let bindings = position_op_bindings(
        position.owner,
        market_key,
        market.asset_state,
        position_key,
        generation,
    );
    let inputs = position_op_inputs(
        position.viewing_pubkey,
        position.viewing_key_epoch,
        &position.encrypted_state,
        request.op,
        request.amount,
        derive_output_nonce_input(&position_key, request.sequence),
    );
    let intent = OperationIntent {
        key: request.intent_key,
        owner: position.owner,
        payer: position.owner,
        reclaim_authority: position.owner,
        kind: OperationKind::PositionOp,
        status: IntentStatus::Queued,
        sequence: request.sequence,
        computation_offset: request.computation_offset,
        accounts_commitment: account_commitment(&bindings),
        inputs_commitment: input_commitment(OperationKind::PositionOp, &inputs),
        queued_slot: request.queued_slot,
        decision_expiry_slot,
        lock_expiry_slot,
        position: Some(position_key),
        position_generation: generation,
        ..OperationIntent::default()
    };
    let mut next_counter = *counter;
    reserve_intent(&mut next_counter, &intent)?;
    acquire_locks(
        intent.lock_owner(),
        intent.lock_expiry_slot,
        &mut [CoordinationSlot::new(
            position_key,
            &mut position.coordination,
        )],
    )?;
    *counter = next_counter;
    Ok(intent)
}

/// What the `position_op` callback observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PositionOpReport {
    /// Whether Arcium `verify_output` succeeded.
    pub output_authenticated: bool,
    /// The computation offset the callback's computation account belongs to.
    pub computation_offset: u64,
    /// The Operation Intent account the callback received.
    pub intent_key: Pubkey,
    /// The output's encryption key.
    pub encryption_key: [u8; 32],
    /// The output nonce (output-nonce input + 1).
    pub output_nonce: u128,
    /// The output ciphertexts in `PositionSecret` order.
    pub ciphertexts: [[u8; 32]; 5],
    /// The revealed applied flag.
    pub applied: bool,
    /// The revealed op echo.
    pub op: u8,
    /// The revealed amount echo.
    pub amount: u64,
    /// The current slot.
    pub current_slot: Slot,
}

/// What the callback did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionOpSettlement {
    /// The new state is stored.
    Applied,
    /// The circuit refused the op: nothing stored, Lock released.
    Rejected,
    /// The attempt ended with this status; nothing stored, Lock released.
    Failed(IntentStatus),
    /// The Intent was already terminal.
    Ignored,
}

/// Verifies and applies a `position_op` callback. Errors revert (the Intent
/// stays `Queued`). Atomic on error.
pub fn settle_position_op(
    market: &Market,
    market_key: Pubkey,
    position: &mut Position,
    position_key: Pubkey,
    intent: &mut OperationIntent,
    report: &PositionOpReport,
) -> Result<PositionOpSettlement, ErrorCode> {
    let bindings = position_op_bindings(
        position.owner,
        market_key,
        market.asset_state,
        position_key,
        intent.position_generation,
    );
    let inputs = position_op_inputs(
        report.encryption_key,
        position.viewing_key_epoch,
        &position.encrypted_state,
        report.op,
        report.amount,
        report.output_nonce.wrapping_sub(1),
    );
    let context = CallbackContext {
        output_authenticated: report.output_authenticated,
        computation_offset: report.computation_offset,
        intent_key: report.intent_key,
        kind: OperationKind::PositionOp,
        accounts: &bindings,
        input_fields: &inputs,
        current: CurrentDependencies {
            position_generation: Some(position.coordination.generation),
            ..CurrentDependencies::captured_by(intent)
        },
        current_slot: report.current_slot,
        report: CallbackReport::Success {
            nonzero: report.applied,
        },
    };
    let decision = classify_callback(intent, &context)?;
    match decision {
        CallbackDecision::LeaveQueued => return Err(ErrorCode::CallbackOutputNotAuthenticated),
        CallbackDecision::Ignore => return Ok(PositionOpSettlement::Ignored),
        _ => {}
    }
    if position.status != PositionStatus::Active {
        return Err(ErrorCode::PositionOpPositionNotActive);
    }
    if !intent.lock_owner().holds(&position.coordination.lock) {
        return Err(ErrorCode::PositionOpIntentMismatch);
    }
    let mut next_intent = *intent;
    let mut coordination = position.coordination;
    let result = apply_decision_to_slots(
        &mut next_intent,
        &mut [CoordinationSlot::new(position_key, &mut coordination)],
        decision,
        report.current_slot,
    )?;
    let settlement = match result.status {
        IntentStatus::Applied => PositionOpSettlement::Applied,
        IntentStatus::Zero => PositionOpSettlement::Rejected,
        status => PositionOpSettlement::Failed(status),
    };
    *intent = next_intent;
    position.coordination = coordination;
    if settlement == PositionOpSettlement::Applied {
        position.encrypted_state = EncryptedPositionPayload {
            nonce: report.output_nonce,
            ciphertexts: report.ciphertexts,
        };
    }
    Ok(settlement)
}

/// `abandon_position_op`: permissionless timeout recovery after Lock expiry.
pub fn abandon_position_op(
    position: &mut Position,
    position_key: Pubkey,
    intent: &mut OperationIntent,
    current_slot: Slot,
) -> Result<usize, ErrorCode> {
    if intent.kind != OperationKind::PositionOp || intent.position != Some(position_key) {
        return Err(ErrorCode::PositionOpIntentMismatch);
    }
    if intent.status.is_terminal() {
        return Err(ErrorCode::IntentNotQueued);
    }
    if !intent.lock_owner().holds(&position.coordination.lock) {
        return Err(ErrorCode::PositionOpIntentMismatch);
    }
    let cleared = crate::operation::abandon_intent(
        intent,
        current_slot,
        &mut [CoordinationSlot::new(
            position_key,
            &mut position.coordination,
        )],
    )?;
    Ok(cleared)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRICE: u64 = 150_000_000; // 150 USDC per SOL
    const LTV: u64 = 5_000;

    fn st(free_sol: u64, free_usdc: u64, collateral_sol: u64, debt: u128) -> PositionBalances {
        PositionBalances {
            free_sol,
            free_usdc,
            collateral_sol,
            debt,
        }
    }

    #[test]
    fn add_collateral_applies_and_refuses() {
        let (s, ok) = position_op_rules(st(5, 0, 1, 0), OP_ADD_COLLATERAL, 5, PRICE, LTV);
        assert!(ok);
        assert_eq!(s, st(0, 0, 6, 0));
        let before = st(5, 0, 1, 0);
        assert_eq!(
            position_op_rules(before, OP_ADD_COLLATERAL, 6, PRICE, LTV),
            (before, false)
        );
    }

    #[test]
    fn borrow_hits_the_exact_ltv_boundary() {
        // 2 SOL collateral * 150 USDC * 50% = 150 USDC max debt.
        let s = st(0, 0, 2_000_000_000, 0);
        let (s2, ok) = position_op_rules(s, OP_BORROW, 150_000_000, PRICE, LTV);
        assert!(ok);
        assert_eq!(s2, st(0, 150_000_000, 2_000_000_000, 150_000_000));
        assert_eq!(
            position_op_rules(s, OP_BORROW, 150_000_001, PRICE, LTV),
            (s, false)
        );
    }

    #[test]
    fn withdraw_collateral_respects_ltv() {
        // 2 SOL, 75 USDC debt: 1 SOL left supports exactly 75.
        let s = st(0, 75_000_000, 2_000_000_000, 75_000_000);
        let (s2, ok) = position_op_rules(s, OP_WITHDRAW_COLLATERAL, 1_000_000_000, PRICE, LTV);
        assert!(ok);
        assert_eq!(s2.collateral_sol, 1_000_000_000);
        assert_eq!(s2.free_sol, 1_000_000_000);
        assert!(!position_op_rules(s, OP_WITHDRAW_COLLATERAL, 1_000_000_001, PRICE, LTV).1);
        assert!(!position_op_rules(s, OP_WITHDRAW_COLLATERAL, 2_000_000_001, PRICE, LTV).1);
    }

    #[test]
    fn repay_applies_and_refuses() {
        let s = st(0, 40, 1, 100);
        let (s2, ok) = position_op_rules(s, OP_REPAY, 40, PRICE, LTV);
        assert!(ok);
        assert_eq!(s2, st(0, 0, 1, 60));
        assert!(!position_op_rules(s, OP_REPAY, 41, PRICE, LTV).1);
        assert!(!position_op_rules(st(0, 200, 1, 100), OP_REPAY, 101, PRICE, LTV).1);
    }

    #[test]
    fn borrow_refuses_free_usdc_overflow_and_unknown_op() {
        let s = st(0, u64::MAX, u64::MAX, 0);
        assert_eq!(position_op_rules(s, OP_BORROW, 1, PRICE, LTV), (s, false));
        assert_eq!(position_op_rules(s, 4, 1, PRICE, LTV), (s, false));
    }
}
