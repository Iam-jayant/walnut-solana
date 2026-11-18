//! `Disclosure` (walnut-lend): a field-masked copy of a Position sealed to an
//! auditor's x25519 key.
//!
//! PDA `["disclosure", position, auditor]`. The owner creates (or reuses) the
//! account at queue time and the `disclose_position` callback writes the
//! sealed fields, so a failed callback leaves the previous Disclosure in
//! place. A new Disclosure to the same auditor overwrites the old one.
//!
//! The Position is neither changed nor Locked. The Intent captures the
//! Position generation; the Disclosure records it.
//!
//! `accounts_commitment` covers the owner, the Position (captured generation)
//! and the Disclosure. `inputs_commitment` covers the Viewing key, epoch,
//! stored nonce, auditor key, mask, the five stored ciphertexts and the
//! output-nonce input. The mask is stored in the Intent's `asset_generation`.

use anchor_lang::prelude::*;
use core::result::Result;

use super::accounts::{Position, PositionStatus};
use super::nonce::{derive_output_nonce_input, INPUT_OUTPUT_NONCE};
use super::position::{INPUT_NONCE, INPUT_VIEWING_KEY_EPOCH, INPUT_VIEWING_PUBKEY};
use super::shield::{INPUT_AMOUNT, INPUT_STATE_CIPHERTEXT_FIRST};
use super::Slot;
use crate::errors::ErrorCode;
use crate::operation::callback::{
    apply_decision_to_slots, classify_callback, CallbackContext, CallbackDecision, CallbackReport,
    CurrentDependencies,
};
use crate::operation::commitment::{
    account_commitment, input_commitment, AccountBinding, AccountRole, InputField, InputValue,
};
use crate::operation::seeds::reserve_intent;
use crate::operation::{IntentStatus, OperationCounter, OperationIntent, OperationKind};

/// PDA seed prefix.
pub const DISCLOSURE_SEED: &[u8] = b"disclosure";
/// Decision Expiry distance, in slots.
pub const DISCLOSE_DECISION_SLOTS: u64 = 1_500;
/// Slots between Decision Expiry and abandonment.
pub const DISCLOSE_LOCK_GRACE_SLOTS: u64 = 300;
/// Mask bits: free_sol, free_usdc, collateral, supply_shares, debt_shares.
pub const DISCLOSURE_MASK_ALL: u64 = 0b1_1111;
/// Input id: auditor key.
pub const INPUT_AUDITOR: u8 = 30;

/// The Disclosure account.
#[account]
#[derive(InitSpace)]
pub struct Disclosure {
    /// The disclosed Position.
    pub position: Pubkey,
    /// The Position owner.
    pub owner: Pubkey,
    /// The auditor's x25519 public key.
    pub auditor: Pubkey,
    /// Selected fields, one bit per field (unselected fields are zero).
    pub mask: u8,
    /// Output nonce of the sealed fields.
    pub nonce: u128,
    /// free_sol, free_usdc, collateral, supply_shares, debt_shares.
    pub ciphertexts: [[u8; 32]; 5],
    /// Position generation the Disclosure was taken at.
    pub position_generation: u64,
    /// Slot the Disclosure was stored.
    pub slot: u64,
    /// PDA bump.
    pub bump: u8,
}

impl Disclosure {
    /// Account data length including Anchor's 8-byte discriminator.
    pub const ACCOUNT_LEN: usize = 8 + Self::INIT_SPACE;

    /// An empty Disclosure for `position` and `auditor`.
    pub fn blank(position: Pubkey, owner: Pubkey, auditor: [u8; 32], bump: u8) -> Self {
        Self {
            position,
            owner,
            auditor: Pubkey::new_from_array(auditor),
            mask: 0,
            nonce: 0,
            ciphertexts: [[0; 32]; 5],
            position_generation: 0,
            slot: 0,
            bump,
        }
    }
}

/// The ordered accounts bound by a Disclose Intent.
pub fn disclose_bindings(
    owner: Pubkey,
    position: Pubkey,
    position_generation: Option<u64>,
    disclosure: Pubkey,
) -> [AccountBinding; 3] {
    let program = crate::ID;
    [
        AccountBinding {
            key: owner,
            role: AccountRole::Owner,
            expected_owner: anchor_lang::system_program::ID,
            mint: None,
            authority: None,
            writable: true,
            generation: None,
        },
        AccountBinding {
            key: position,
            role: AccountRole::Position,
            expected_owner: program,
            mint: None,
            authority: Some(owner),
            writable: false,
            generation: position_generation,
        },
        AccountBinding {
            key: disclosure,
            role: AccountRole::Disclosure,
            expected_owner: program,
            mint: None,
            authority: Some(owner),
            writable: true,
            generation: None,
        },
    ]
}

/// The ordered inputs bound by a Disclose Intent.
pub fn disclose_inputs(
    position: &Position,
    viewing_pubkey: [u8; 32],
    auditor: [u8; 32],
    mask: u64,
    output_nonce_input: u128,
) -> [InputField<'static>; 11] {
    let c = &position.encrypted_state.ciphertexts;
    let first = INPUT_STATE_CIPHERTEXT_FIRST;
    [
        InputField::new(INPUT_VIEWING_PUBKEY, InputValue::Bytes32(viewing_pubkey)),
        InputField::new(
            INPUT_VIEWING_KEY_EPOCH,
            InputValue::U64(position.viewing_key_epoch),
        ),
        InputField::new(
            INPUT_NONCE,
            InputValue::U128(position.encrypted_state.nonce),
        ),
        InputField::new(INPUT_AUDITOR, InputValue::Bytes32(auditor)),
        InputField::new(INPUT_AMOUNT, InputValue::U64(mask)),
        InputField::new(first, InputValue::Bytes32(c[0])),
        InputField::new(first + 1, InputValue::Bytes32(c[1])),
        InputField::new(first + 2, InputValue::Bytes32(c[2])),
        InputField::new(first + 3, InputValue::Bytes32(c[3])),
        InputField::new(first + 4, InputValue::Bytes32(c[4])),
        InputField::new(INPUT_OUTPUT_NONCE, InputValue::U128(output_nonce_input)),
    ]
}

/// One `disclose_position` queue request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiscloseRequest {
    /// The Operation Intent PDA address.
    pub intent_key: Pubkey,
    /// The signer's next Intent sequence.
    pub sequence: u64,
    /// The Arcium computation offset.
    pub computation_offset: u64,
    /// The current slot.
    pub queued_slot: Slot,
    /// The Disclosure PDA address.
    pub disclosure: Pubkey,
    /// The auditor's x25519 public key.
    pub auditor: [u8; 32],
    /// The field mask.
    pub mask: u64,
}

/// Checks the mask, the signer and the Position; builds and reserves the
/// Intent. The Position is not Locked.
pub fn queue_disclosure(
    position: &Position,
    position_key: Pubkey,
    counter: &mut OperationCounter,
    signer: &Pubkey,
    request: &DiscloseRequest,
) -> Result<OperationIntent, ErrorCode> {
    if request.mask == 0 || request.mask & !DISCLOSURE_MASK_ALL != 0 {
        return Err(ErrorCode::DisclosureMaskInvalid);
    }
    if *signer != position.owner {
        return Err(ErrorCode::PositionOpNotOwner);
    }
    if position.status != PositionStatus::Active {
        return Err(ErrorCode::PositionOpPositionNotActive);
    }
    let decision_expiry_slot = request
        .queued_slot
        .checked_add(DISCLOSE_DECISION_SLOTS)
        .ok_or(ErrorCode::InvalidIntentTiming)?;
    let lock_expiry_slot = decision_expiry_slot
        .checked_add(DISCLOSE_LOCK_GRACE_SLOTS)
        .ok_or(ErrorCode::InvalidIntentTiming)?;
    let generation = Some(position.coordination.generation);
    let bindings = disclose_bindings(position.owner, position_key, generation, request.disclosure);
    let inputs = disclose_inputs(
        position,
        position.viewing_pubkey,
        request.auditor,
        request.mask,
        derive_output_nonce_input(&position_key, request.sequence),
    );
    let intent = OperationIntent {
        key: request.intent_key,
        owner: position.owner,
        payer: position.owner,
        reclaim_authority: position.owner,
        kind: OperationKind::Disclose,
        status: IntentStatus::Queued,
        sequence: request.sequence,
        computation_offset: request.computation_offset,
        accounts_commitment: account_commitment(&bindings),
        inputs_commitment: input_commitment(OperationKind::Disclose, &inputs),
        queued_slot: request.queued_slot,
        decision_expiry_slot,
        lock_expiry_slot,
        position: Some(position_key),
        position_generation: generation,
        asset_generation: request.mask,
        ..OperationIntent::default()
    };
    let mut next_counter = *counter;
    reserve_intent(&mut next_counter, &intent)?;
    *counter = next_counter;
    Ok(intent)
}

/// What the `disclose_position` callback observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiscloseReport {
    /// Whether Arcium `verify_output` succeeded.
    pub output_authenticated: bool,
    /// The computation offset the callback's computation account belongs to.
    pub computation_offset: u64,
    /// The Operation Intent account the callback received.
    pub intent_key: Pubkey,
    /// The output's encryption key (the auditor key).
    pub encryption_key: [u8; 32],
    /// The output nonce (output-nonce input + 1).
    pub output_nonce: u128,
    /// The masked output ciphertexts.
    pub ciphertexts: [[u8; 32]; 5],
    /// The current slot.
    pub current_slot: Slot,
}

/// What the callback did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscloseSettlement {
    /// The Disclosure was written.
    Stored,
    /// The attempt ended with this status; the Disclosure is untouched.
    Failed(IntentStatus),
    /// The Intent was already terminal.
    Ignored,
}

/// Verifies and applies a `disclose_position` callback, writing the
/// Disclosure only on `Applied`. Errors revert (the Intent stays `Queued`).
pub fn settle_disclosure(
    position: &Position,
    position_key: Pubkey,
    disclosure: &mut Disclosure,
    disclosure_key: Pubkey,
    intent: &mut OperationIntent,
    report: &DiscloseReport,
) -> Result<DiscloseSettlement, ErrorCode> {
    let bindings = disclose_bindings(
        position.owner,
        position_key,
        intent.position_generation,
        disclosure_key,
    );
    let inputs = disclose_inputs(
        position,
        position.viewing_pubkey,
        report.encryption_key,
        intent.asset_generation,
        report.output_nonce.wrapping_sub(1),
    );
    let context = CallbackContext {
        output_authenticated: report.output_authenticated,
        computation_offset: report.computation_offset,
        intent_key: report.intent_key,
        kind: OperationKind::Disclose,
        accounts: &bindings,
        input_fields: &inputs,
        current: CurrentDependencies {
            position_generation: Some(position.coordination.generation),
            ..CurrentDependencies::captured_by(intent)
        },
        current_slot: report.current_slot,
        report: CallbackReport::Success { nonzero: true },
    };
    let decision = classify_callback(intent, &context)?;
    match decision {
        CallbackDecision::LeaveQueued => return Err(ErrorCode::CallbackOutputNotAuthenticated),
        CallbackDecision::Ignore => return Ok(DiscloseSettlement::Ignored),
        _ => {}
    }
    if disclosure.position != position_key || disclosure.auditor.to_bytes() != report.encryption_key
    {
        return Err(ErrorCode::DisclosureAuditorMismatch);
    }
    let mut next_intent = *intent;
    let result = apply_decision_to_slots(&mut next_intent, &mut [], decision, report.current_slot)?;
    *intent = next_intent;
    if result.status != IntentStatus::Applied {
        return Ok(DiscloseSettlement::Failed(result.status));
    }
    disclosure.mask = intent.asset_generation as u8;
    disclosure.nonce = report.output_nonce;
    disclosure.ciphertexts = report.ciphertexts;
    disclosure.position_generation = position.coordination.generation;
    disclosure.slot = report.current_slot;
    Ok(DiscloseSettlement::Stored)
}

/// `abandon_disclosure`: permissionless recovery after the grace window. No
/// Lock exists, so only the Intent is closed.
pub fn abandon_disclosure(
    position_key: Pubkey,
    intent: &mut OperationIntent,
    current_slot: Slot,
) -> Result<(), ErrorCode> {
    if intent.kind != OperationKind::Disclose || intent.position != Some(position_key) {
        return Err(ErrorCode::DisclosureIntentMismatch);
    }
    if intent.status.is_terminal() {
        return Err(ErrorCode::IntentNotQueued);
    }
    crate::operation::abandon_intent(intent, current_slot, &mut [])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::test_fixtures::{key, position};

    fn request(mask: u64) -> DiscloseRequest {
        DiscloseRequest {
            intent_key: key(0x80),
            sequence: 0,
            computation_offset: 9,
            queued_slot: 100,
            disclosure: key(0x81),
            auditor: [0x77; 32],
            mask,
        }
    }

    fn report(intent: &OperationIntent, auditor: [u8; 32]) -> DiscloseReport {
        DiscloseReport {
            output_authenticated: true,
            computation_offset: intent.computation_offset,
            intent_key: intent.key,
            encryption_key: auditor,
            output_nonce: derive_output_nonce_input(&key(0x23), 0) + 1,
            ciphertexts: [[0x44; 32]; 5],
            current_slot: 130,
        }
    }

    #[test]
    fn account_len_is_fixed() {
        // 8 + 32*3 + 1 + 16 + 160 + 8 + 8 + 1
        assert_eq!(Disclosure::ACCOUNT_LEN, 298);
    }

    #[test]
    fn mask_must_select_known_fields() {
        let pos = position();
        let mut counter = OperationCounter::new(pos.owner, 1);
        for bad in [0u64, 0b10_0000, u64::MAX] {
            assert_eq!(
                queue_disclosure(&pos, key(0x23), &mut counter, &pos.owner, &request(bad)),
                Err(ErrorCode::DisclosureMaskInvalid)
            );
        }
        assert!(
            queue_disclosure(&pos, key(0x23), &mut counter, &pos.owner, &request(0b101)).is_ok()
        );
        assert_eq!(counter.next_sequence, 1);
        assert_eq!(
            queue_disclosure(&pos, key(0x23), &mut counter, &key(0x55), &request(1)),
            Err(ErrorCode::PositionOpNotOwner)
        );
    }

    #[test]
    fn callback_writes_the_disclosure_and_leaves_the_position_unlocked() {
        let pos = position();
        let mut counter = OperationCounter::new(pos.owner, 1);
        let mut intent =
            queue_disclosure(&pos, key(0x23), &mut counter, &pos.owner, &request(0b11)).unwrap();
        let mut d = Disclosure::blank(key(0x23), pos.owner, [0x77; 32], 1);
        let r = report(&intent, [0x77; 32]);
        let s = settle_disclosure(&pos, key(0x23), &mut d, key(0x81), &mut intent, &r).unwrap();
        assert_eq!(s, DiscloseSettlement::Stored);
        assert_eq!(d.mask, 0b11);
        assert_eq!(d.ciphertexts, [[0x44; 32]; 5]);
        assert_eq!(d.position_generation, pos.coordination.generation);
        assert_eq!(intent.status, IntentStatus::Applied);
    }

    #[test]
    fn output_sealed_to_another_key_reverts_and_a_second_disclosure_overwrites() {
        let pos = position();
        let mut counter = OperationCounter::new(pos.owner, 1);
        let mut intent =
            queue_disclosure(&pos, key(0x23), &mut counter, &pos.owner, &request(1)).unwrap();
        let mut d = Disclosure::blank(key(0x23), pos.owner, [0x77; 32], 1);
        // a different encryption key breaks the inputs commitment
        let wrong = report(&intent, [0x78; 32]);
        assert!(
            settle_disclosure(&pos, key(0x23), &mut d, key(0x81), &mut intent, &wrong).is_err()
        );
        assert_eq!(intent.status, IntentStatus::Queued);
        assert_eq!(d.mask, 0);
        // overwrite: a later Disclosure replaces the earlier fields
        d.mask = 0b11111;
        d.ciphertexts = [[0x99; 32]; 5];
        let mut counter2 = OperationCounter::new(pos.owner, 1);
        let mut later =
            queue_disclosure(&pos, key(0x23), &mut counter2, &pos.owner, &request(0b100)).unwrap();
        let r = report(&later, [0x77; 32]);
        settle_disclosure(&pos, key(0x23), &mut d, key(0x81), &mut later, &r).unwrap();
        assert_eq!(d.mask, 0b100);
        assert_eq!(d.ciphertexts, [[0x44; 32]; 5]);
    }

    #[test]
    fn abandon_closes_a_stuck_intent_after_expiry() {
        let pos = position();
        let mut counter = OperationCounter::new(pos.owner, 1);
        let mut intent =
            queue_disclosure(&pos, key(0x23), &mut counter, &pos.owner, &request(1)).unwrap();
        assert!(abandon_disclosure(key(0x23), &mut intent, 200).is_err());
        let expiry = intent.lock_expiry_slot + 1;
        abandon_disclosure(key(0x23), &mut intent, expiry).unwrap();
        assert_eq!(intent.status, IntentStatus::Abandoned);
    }
}
