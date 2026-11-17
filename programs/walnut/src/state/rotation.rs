//! Viewing Key rotation, its callback, and abandonment (Task 5.5;
//! Requirements 5.10, 5.11, 5.12, 3.1-3.10).
//!
//! Pure Rust rules behind `rotate_position_key`, the `rotate_position_key`
//! callback, and `abandon_rotation`. The Anchor handlers in `lib.rs` and
//! `instructions/rotation.rs` load accounts and queue the computation; every
//! state change goes through these functions.
//!
//! # Flow
//!
//! 1. `rotate_position_key(expected_old_epoch, expected_new_epoch,
//!    expected_old_pubkey, new_viewing_pubkey)` (owner-signed):
//!    [`queue_rotation`] checks that the signer is the Position owner, the
//!    Position is `Active` with a free Lock, the new key is nonzero and
//!    different from the old one, and that the owner's expectations match the
//!    Position (old epoch and old public key) with `new epoch = old epoch + 1`
//!    (no overflow). It creates the Intent, Locks the Position, and the
//!    handler queues the `rotate_position_key` circuit. Nothing changes yet.
//! 2. The circuit reads the stored payload under the OLD key and re-seals the
//!    same five values to the NEW key under the program-derived output nonce
//!    (`state::nonce`). It reveals nothing.
//! 3. Callback ([`settle_rotation`]): `Applied` writes the re-sealed payload,
//!    the new public key, and the incremented epoch together, bumps the
//!    generation, and releases the Lock. `Stale`, `Expired`, and failed
//!    outcomes only release the Lock: the old key, epoch, and payload stay
//!    valid. A terminal Intent is a no-op. Unauthenticated output and every
//!    binding mismatch revert, so the Intent stays `Queued` until
//!    [`abandon_rotation`].
//! 4. `abandon_rotation` (any signer, after Lock expiry): releases the Lock
//!    and changes nothing else.
//!
//! # Atomicity
//!
//! The payload, `viewing_pubkey`, and `viewing_key_epoch` change in exactly
//! one place, the last statements of the `Applied` arm of [`settle_rotation`],
//! after every fallible step (classification, decision application, epoch
//! increment) has succeeded on copies. No other function writes any of the
//! three fields. A Position therefore always holds a payload sealed to the
//! key and epoch it stores. The rotation is not pausable (`Action::
//! ViewingKeyRotation`); it moves no value.
//!
//! # Intent binding
//!
//! `accounts_commitment` covers the owner (signer and payer) and the Position
//! (captured generation). `inputs_commitment` covers: the old Viewing public
//! key and epoch, the stored state nonce and five ciphertexts, the output
//! nonce input (`state::nonce`, which binds the Position and the Intent
//! sequence), and the new Viewing public key and epoch. The callback rebuilds
//! the commitment from the Position it holds and the key the output is sealed
//! to, so an output sealed to any other key, or computed from other state,
//! fails the commitment check. The Intent also carries the sequence and the
//! Decision Expiry. The program ID is bound by the Position/Intent PDAs, and
//! the network is bound in the client-signed key-derivation message (the
//! program has no network concept).

use anchor_lang::prelude::Pubkey;

use super::accounts::{EncryptedPositionPayload, Position, PositionStatus};
use super::coordination::{acquire_locks, CoordinationSlot};
use super::nonce::{derive_output_nonce_input, INPUT_OUTPUT_NONCE};
use super::position::{
    check_viewing_key, INPUT_NONCE, INPUT_VIEWING_KEY_EPOCH, INPUT_VIEWING_PUBKEY,
};
use super::shield::INPUT_STATE_CIPHERTEXT_FIRST;
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

/// Input field id: the new Viewing public key (`Bytes32`). Append-only; ids
/// 1-13 are taken by the Position, shield, and output-nonce fields.
pub const INPUT_NEW_VIEWING_PUBKEY: u8 = 14;
/// Input field id: the new Viewing Key epoch (`U64`). Append-only.
pub const INPUT_NEW_VIEWING_KEY_EPOCH: u8 = 15;

/// Decision Expiry distance of a rotation, in slots. The circuit reads no
/// price or index, so a fixed window is used, as for unshield.
pub const ROTATION_DECISION_SLOTS: u64 = 1_500;

/// Slots between Decision Expiry and Lock expiry.
pub const ROTATION_LOCK_GRACE_SLOTS: u64 = 300;

/// Rotation failures. Mapped onto program errors in `errors.rs`. A zero new
/// key is [`super::position::PositionError::InvalidViewingKey`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RotationError {
    /// The signer is not the Position owner.
    NotOwner,
    /// The Position is not `Active`.
    PositionNotActive,
    /// The new Viewing public key equals the current one.
    KeyUnchanged,
    /// The expected old or new epoch is not the Position's.
    EpochMismatch,
    /// The expected old public key is not the Position's.
    OldKeyMismatch,
    /// The epoch cannot be incremented.
    EpochOverflow,
    /// The Intent is not this Position's queued rotation.
    IntentMismatch,
}

/// The ordered accounts bound by a Rotation Intent.
pub fn rotation_bindings(
    owner: Pubkey,
    position: Pubkey,
    position_generation: Option<u64>,
) -> [AccountBinding; 2] {
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
            expected_owner: crate::ID,
            mint: None,
            authority: Some(owner),
            writable: true,
            generation: position_generation,
        },
    ]
}

/// The keys and epochs a rotation moves between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RotationKeys {
    /// The Viewing public key the Position has now.
    pub old_pubkey: [u8; 32],
    /// The epoch the Position has now.
    pub old_epoch: u64,
    /// The Viewing public key to rotate to.
    pub new_pubkey: [u8; 32],
    /// The epoch after rotation (`old_epoch + 1`).
    pub new_epoch: u64,
}

/// The ordered inputs bound by a Rotation Intent.
pub fn rotation_inputs(
    keys: &RotationKeys,
    state: &EncryptedPositionPayload,
    output_nonce_input: u128,
) -> [InputField<'static>; 11] {
    let c = &state.ciphertexts;
    let first = INPUT_STATE_CIPHERTEXT_FIRST;
    [
        InputField::new(INPUT_VIEWING_PUBKEY, InputValue::Bytes32(keys.old_pubkey)),
        InputField::new(INPUT_VIEWING_KEY_EPOCH, InputValue::U64(keys.old_epoch)),
        InputField::new(INPUT_NONCE, InputValue::U128(state.nonce)),
        InputField::new(first, InputValue::Bytes32(c[0])),
        InputField::new(first + 1, InputValue::Bytes32(c[1])),
        InputField::new(first + 2, InputValue::Bytes32(c[2])),
        InputField::new(first + 3, InputValue::Bytes32(c[3])),
        InputField::new(first + 4, InputValue::Bytes32(c[4])),
        InputField::new(INPUT_OUTPUT_NONCE, InputValue::U128(output_nonce_input)),
        InputField::new(
            INPUT_NEW_VIEWING_PUBKEY,
            InputValue::Bytes32(keys.new_pubkey),
        ),
        InputField::new(INPUT_NEW_VIEWING_KEY_EPOCH, InputValue::U64(keys.new_epoch)),
    ]
}

/// One `rotate_position_key` queue request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RotationRequest {
    /// The Operation Intent PDA address.
    pub intent_key: Pubkey,
    /// The owner's next Intent sequence.
    pub sequence: u64,
    /// The Arcium computation offset.
    pub computation_offset: u64,
    /// The current slot.
    pub queued_slot: Slot,
    /// The epoch the owner expects the Position to have.
    pub expected_old_epoch: u64,
    /// The epoch the owner wants (`expected_old_epoch + 1`).
    pub expected_new_epoch: u64,
    /// The Viewing public key the owner expects the Position to have.
    pub expected_old_pubkey: [u8; 32],
    /// The Viewing public key to rotate to.
    pub new_viewing_pubkey: [u8; 32],
}

/// Builds the Rotation Intent for `request` against `position`.
pub fn build_rotation_intent(
    position: &Position,
    position_key: Pubkey,
    request: &RotationRequest,
) -> Result<OperationIntent, ErrorCode> {
    let decision_expiry_slot = request
        .queued_slot
        .checked_add(ROTATION_DECISION_SLOTS)
        .ok_or(ErrorCode::InvalidIntentTiming)?;
    let lock_expiry_slot = decision_expiry_slot
        .checked_add(ROTATION_LOCK_GRACE_SLOTS)
        .ok_or(ErrorCode::InvalidIntentTiming)?;
    let generation = Some(position.coordination.generation);
    let bindings = rotation_bindings(position.owner, position_key, generation);
    let keys = RotationKeys {
        old_pubkey: position.viewing_pubkey,
        old_epoch: position.viewing_key_epoch,
        new_pubkey: request.new_viewing_pubkey,
        new_epoch: request.expected_new_epoch,
    };
    let inputs = rotation_inputs(
        &keys,
        &position.encrypted_state,
        derive_output_nonce_input(&position_key, request.sequence),
    );
    Ok(OperationIntent {
        key: request.intent_key,
        owner: position.owner,
        payer: position.owner,
        reclaim_authority: position.owner,
        kind: OperationKind::ViewingKeyRotation,
        status: IntentStatus::Queued,
        sequence: request.sequence,
        computation_offset: request.computation_offset,
        accounts_commitment: account_commitment(&bindings),
        inputs_commitment: input_commitment(OperationKind::ViewingKeyRotation, &inputs),
        queued_slot: request.queued_slot,
        decision_expiry_slot,
        lock_expiry_slot,
        position: Some(position_key),
        position_generation: generation,
        ..OperationIntent::default()
    })
}

/// The epoch after `epoch`, or [`RotationError::EpochOverflow`].
pub fn next_epoch(epoch: u64) -> Result<u64, RotationError> {
    epoch.checked_add(1).ok_or(RotationError::EpochOverflow)
}

/// `rotate_position_key`: checks the signer, the Position, the keys, and the
/// owner's epoch and key expectations; builds and reserves the Intent; and
/// Locks the Position. Not pausable, and it reads no Market. Atomic: on error
/// the Position and the counter are unchanged.
pub fn queue_rotation(
    position: &mut Position,
    position_key: Pubkey,
    counter: &mut OperationCounter,
    signer: &Pubkey,
    request: &RotationRequest,
) -> Result<OperationIntent, ErrorCode> {
    if *signer != position.owner {
        return Err(RotationError::NotOwner.into());
    }
    if position.status != PositionStatus::Active {
        return Err(RotationError::PositionNotActive.into());
    }
    check_viewing_key(&request.new_viewing_pubkey)?;
    if request.expected_old_pubkey != position.viewing_pubkey {
        return Err(RotationError::OldKeyMismatch.into());
    }
    if request.expected_old_epoch != position.viewing_key_epoch {
        return Err(RotationError::EpochMismatch.into());
    }
    let new_epoch = next_epoch(position.viewing_key_epoch)?;
    if request.expected_new_epoch != new_epoch {
        return Err(RotationError::EpochMismatch.into());
    }
    if request.new_viewing_pubkey == position.viewing_pubkey {
        return Err(RotationError::KeyUnchanged.into());
    }
    let intent = build_rotation_intent(position, position_key, request)?;
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

/// What the `rotate_position_key` callback observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RotationReport {
    /// Whether Arcium `verify_output` succeeded.
    pub output_authenticated: bool,
    /// The computation offset the callback's computation account belongs to.
    pub computation_offset: u64,
    /// The Operation Intent account the callback received.
    pub intent_key: Pubkey,
    /// The output's encryption key: the NEW Viewing public key.
    pub encryption_key: [u8; 32],
    /// The output nonce (output-nonce input + 1, see `state::nonce`).
    pub output_nonce: u128,
    /// The output ciphertexts in `PositionSecret` order.
    pub ciphertexts: [[u8; 32]; 5],
    /// The current slot.
    pub current_slot: Slot,
}

/// What the callback did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RotationSettlement {
    /// `Applied`: the payload, key, and epoch changed together.
    Rotated {
        /// The epoch before.
        old_epoch: u64,
        /// The epoch after.
        new_epoch: u64,
        /// The key before.
        old_pubkey: [u8; 32],
        /// The key after.
        new_pubkey: [u8; 32],
    },
    /// The attempt ended with this status (`Expired`, `Stale`, `Failed`,
    /// `Zero`); nothing stored, Lock released, old key still valid.
    Failed(IntentStatus),
    /// The Intent was already terminal; nothing changed.
    Ignored,
}

/// Verifies and applies a `rotate_position_key` callback (Task 3.3 helpers).
///
/// Unauthenticated output and commitment, offset, or binding mismatches are
/// errors, so the transaction reverts and the Intent stays `Queued`. A
/// terminal Intent is `Ignored`. Otherwise the decision releases the Lock;
/// only `Applied` stores the payload, the new key, and the new epoch, together
/// and last (bumping the generation). Atomic on error.
pub fn settle_rotation(
    position: &mut Position,
    position_key: Pubkey,
    intent: &mut OperationIntent,
    report: &RotationReport,
) -> Result<RotationSettlement, ErrorCode> {
    let old_epoch = position.viewing_key_epoch;
    let old_pubkey = position.viewing_pubkey;
    let new_epoch = next_epoch(old_epoch)?;
    let bindings = rotation_bindings(position.owner, position_key, intent.position_generation);
    // The output-nonce input is output nonce - 1; the commitment binds it to
    // the value derived at queue time from the Intent sequence.
    let inputs = rotation_inputs(
        &RotationKeys {
            old_pubkey,
            old_epoch,
            new_pubkey: report.encryption_key,
            new_epoch,
        },
        &position.encrypted_state,
        report.output_nonce.wrapping_sub(1),
    );
    let context = CallbackContext {
        output_authenticated: report.output_authenticated,
        computation_offset: report.computation_offset,
        intent_key: report.intent_key,
        kind: OperationKind::ViewingKeyRotation,
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
        CallbackDecision::Ignore => return Ok(RotationSettlement::Ignored),
        _ => {}
    }
    if position.status != PositionStatus::Active {
        return Err(RotationError::PositionNotActive.into());
    }
    if !intent.lock_owner().holds(&position.coordination.lock) {
        return Err(RotationError::IntentMismatch.into());
    }
    if decision == CallbackDecision::Applied {
        // Defense in depth: the commitment already binds these.
        check_viewing_key(&report.encryption_key)?;
        if report.encryption_key == old_pubkey {
            return Err(RotationError::KeyUnchanged.into());
        }
    }

    let mut next_intent = *intent;
    let mut coordination = position.coordination;
    let result = apply_decision_to_slots(
        &mut next_intent,
        &mut [CoordinationSlot::new(position_key, &mut coordination)],
        decision,
        report.current_slot,
    )?;

    // Everything fallible is done. Commit.
    *intent = next_intent;
    position.coordination = coordination;
    if result.status != IntentStatus::Applied {
        return Ok(RotationSettlement::Failed(result.status));
    }
    position.encrypted_state = EncryptedPositionPayload {
        nonce: report.output_nonce,
        ciphertexts: report.ciphertexts,
    };
    position.viewing_pubkey = report.encryption_key;
    position.viewing_key_epoch = new_epoch;
    Ok(RotationSettlement::Rotated {
        old_epoch,
        new_epoch,
        old_pubkey,
        new_pubkey: report.encryption_key,
    })
}

/// `abandon_rotation`: permissionless timeout recovery of a queued rotation
/// after Lock expiry. Bumps the Position generation, releases the Lock, and
/// marks the Intent `Abandoned`. The key, epoch, and payload are untouched.
/// Atomic. Returns the number of Locks cleared.
pub fn abandon_rotation(
    position: &mut Position,
    position_key: Pubkey,
    intent: &mut OperationIntent,
    current_slot: Slot,
) -> Result<usize, ErrorCode> {
    if intent.kind != OperationKind::ViewingKeyRotation || intent.position != Some(position_key) {
        return Err(RotationError::IntentMismatch.into());
    }
    if intent.status.is_terminal() {
        return Err(ErrorCode::IntentNotQueued);
    }
    if !intent.lock_owner().holds(&position.coordination.lock) {
        return Err(RotationError::IntentMismatch.into());
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
mod tests;
