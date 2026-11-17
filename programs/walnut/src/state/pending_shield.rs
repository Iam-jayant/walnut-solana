//! Pending Shield PDAs (Task 5.2; Requirements 5.4, 5.6, 4.8).
//!
//! A Pending Shield records public tokens that are already in the vault but
//! not yet credited to the encrypted Position. This module holds the account,
//! its address, the per-Position shield sequence, and the pure state machine.
//! It moves no tokens and writes no ciphertext. The Task 5.3 shield, retry,
//! refund, and callback instructions perform the SPL transfers and the
//! encrypted credit and call these functions for every Pending Shield change.
//!
//! # Address and sequence
//!
//! ```text
//! PendingShield = PDA[ "pending_shield" | position | sequence u64 LE ]
//! ```
//!
//! `sequence` comes from `Position.next_shield_seq`. [`open_pending_shield`]
//! requires the exact next value and advances it by one with checked
//! arithmetic, so a sequence (and therefore an address) is used at most once
//! per Position. A replayed sequence fails with
//! [`PendingShieldError::SequenceMismatch`]; Anchor `init` on the PDA
//! additionally refuses an existing account, and [`open_pending_shield`]
//! refuses a non-`Uninitialized` target, so a Pending Shield is never
//! overwritten. A Pending Shield is never closed in v1, so its address is never
//! freed for reuse.
//!
//! # State machine
//!
//! | From        | Event                                                    | To          | Value          |
//! |-------------|----------------------------------------------------------|-------------|----------------|
//! | (none)      | [`open_pending_shield`] (attempt 1)                      | `Deposited` | tokens in vault |
//! | `Deposited` | [`settle_shield_attempt`], current attempt, `Applied`    | `Credited`  | encrypted credit |
//! | `Deposited` | [`settle_shield_attempt`], current attempt, other status | `Deposited` | none (retryable) |
//! | `Deposited` | [`retry_pending_shield`], prior attempt terminal, not `Applied` | `Deposited` (attempt + 1) | none |
//! | `Deposited` | [`refund_pending_shield`], active attempt terminal, not `Applied` | `Refunded` | tokens to source |
//! | any         | [`settle_shield_attempt`] for an old or foreign attempt  | unchanged   | none           |
//! | `Credited`, `Refunded` | anything                                      | unchanged   | none           |
//!
//! "Queued" means the active Operation Intent is still `Queued`. The active
//! Intent account is the authority: retry and refund must present it (key and
//! computation offset must match the stored attempt) and are refused while it
//! is `Queued`. Since an Intent becomes terminal exactly once, a queued Shield
//! Intent is always the current attempt of a `Deposited` Pending Shield, and a
//! callback for any earlier attempt finds a terminal Intent or a moved attempt
//! and changes nothing.
//!
//! An `Applied` active attempt with a `Deposited` Pending Shield cannot occur
//! when the callback calls [`settle_shield_attempt`] in the same instruction
//! that applies the credit. Retry and refund still refuse it
//! ([`PendingShieldError::AttemptApplied`]), so a credit can never be followed
//! by a refund or a second credit.
//!
//! # Fixed active-attempt fields
//!
//! The design lists `active_intent` and `active_computation_offset` as
//! `Option`s. Here they are plain fields: [`open_pending_shield`] installs
//! attempt 1, so a Pending Shield always has an attempt, and after
//! `Credited` or `Refunded` the fields keep the last attempt for audit. Plain
//! fields keep every offset fixed.

use anchor_lang::prelude::{
    account, AnchorDeserialize, AnchorSerialize, Discriminator, InitSpace, Pubkey, Space,
};

use super::accounts::{Position, PositionStatus};
use crate::operation::{IntentStatus, OperationIntent, OperationKind};

/// Seed of a Pending Shield PDA (followed by the Position and the sequence).
pub const PENDING_SHIELD_SEED: &[u8] = b"pending_shield";

/// The shielded asset. The tag is the Borsh variant index; append-only.
#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub enum ShieldAsset {
    /// SOL, held as wSOL in the Market wSOL vault.
    #[default]
    Sol,
    /// USDC, held in the Market USDC vault.
    Usdc,
}

/// Pending Shield lifecycle. Zeroed data reads as `Uninitialized`. The tag is
/// the Borsh variant index; append-only.
#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub enum PendingShieldStatus {
    /// Account space only; no shield has been recorded.
    #[default]
    Uninitialized,
    /// Tokens are in the vault; the encrypted credit is pending or retryable.
    Deposited,
    /// The encrypted credit was applied. Final.
    Credited,
    /// The tokens were returned to the source account. Final.
    Refunded,
}

impl PendingShieldStatus {
    /// Returns whether the status is final (`Credited` or `Refunded`).
    pub fn is_final(self) -> bool {
        matches!(
            self,
            PendingShieldStatus::Credited | PendingShieldStatus::Refunded
        )
    }
}

/// A Pending Shield PDA (design: "Shield").
///
/// Layout (offsets include the 8-byte Anchor discriminator; 211 bytes):
/// discriminator 0..8, `owner` 8..40, `position` 40..72,
/// `source_token_account` 72..104, `asset` 104, `status` 105, `bump` 106,
/// `amount` 107..115, `sequence` 115..123, `attempt_sequence` 123..131,
/// `active_intent` 131..163, `active_computation_offset` 163..171,
/// `captured_position_generation` 171..179, `reserved` 179..211. Integers are
/// little-endian. Later tasks may only take bytes from `reserved`.
#[account]
#[derive(InitSpace, Copy, Debug, Default, PartialEq, Eq)]
pub struct PendingShield {
    /// The Position owner who deposited.
    pub owner: Pubkey,
    /// The Position to credit. Part of the PDA seeds.
    pub position: Pubkey,
    /// The token account the deposit came from; the refund destination.
    pub source_token_account: Pubkey,
    /// The deposited asset.
    pub asset: ShieldAsset,
    /// Lifecycle status.
    pub status: PendingShieldStatus,
    /// The PDA bump.
    pub bump: u8,
    /// The public deposited amount in base units. Never zero.
    pub amount: u64,
    /// The per-Position shield sequence. Part of the PDA seeds.
    pub sequence: u64,
    /// The current credit attempt, starting at 1.
    pub attempt_sequence: u64,
    /// The Operation Intent of the current attempt.
    pub active_intent: Pubkey,
    /// The computation offset of the current attempt.
    pub active_computation_offset: u64,
    /// The Position generation the current attempt captured.
    pub captured_position_generation: u64,
    /// Reserved for later fields; always zero.
    pub reserved: [u8; 32],
}

impl PendingShield {
    /// Account data length including Anchor's 8-byte discriminator.
    pub const ACCOUNT_LEN: usize = 8 + Self::INIT_SPACE;

    /// Returns whether `(intent_key, computation_offset)` is the current
    /// attempt of a `Deposited` Pending Shield. Only such a callback may credit.
    pub fn is_current_attempt(&self, intent_key: &Pubkey, computation_offset: u64) -> bool {
        self.status == PendingShieldStatus::Deposited
            && self.active_intent == *intent_key
            && self.active_computation_offset == computation_offset
    }
}

/// Pending Shield failures. Mapped onto program errors in `errors.rs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingShieldError {
    /// The deposit amount is zero.
    ZeroAmount,
    /// The target account already holds a Pending Shield.
    AlreadyExists,
    /// The sequence is not the Position's next shield sequence.
    SequenceMismatch,
    /// The Position shield sequence would overflow.
    SequenceOverflow,
    /// The Position is not the Pending Shield's Position or owner.
    PositionMismatch,
    /// The Position is not `Active`.
    PositionNotActive,
    /// The signer is not the Pending Shield owner.
    NotOwner,
    /// The Pending Shield is not `Deposited`.
    NotDeposited,
    /// The active attempt is still queued.
    AttemptQueued,
    /// The active attempt was applied, so the deposit was credited.
    AttemptApplied,
    /// The presented Intent is not the active attempt.
    ActiveIntentMismatch,
    /// The attempt Intent's kind, status, owner, Position, or generation does
    /// not match this Pending Shield.
    AttemptBindingMismatch,
    /// The new attempt reuses the active attempt's Intent.
    AttemptReused,
    /// The attempt sequence would overflow.
    AttemptOverflow,
}

/// Derives the Pending Shield address and bump for `position` and `sequence`.
pub fn derive_pending_shield_address(
    position: &Pubkey,
    sequence: u64,
    program_id: &Pubkey,
) -> (Pubkey, u8) {
    let sequence = sequence.to_le_bytes();
    let (address, bump) = anchor_lang::derive_program_address(
        &[PENDING_SHIELD_SEED, position.as_ref(), &sequence],
        &program_id.to_bytes(),
    );
    (Pubkey::new_from_array(address), bump)
}

/// The identity of a Pending Shield being opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NewPendingShield {
    /// The Position owner (signer).
    pub owner: Pubkey,
    /// The Position PDA.
    pub position: Pubkey,
    /// The source token account the tokens were transferred from.
    pub source_token_account: Pubkey,
    /// The deposited asset.
    pub asset: ShieldAsset,
    /// The deposited amount.
    pub amount: u64,
    /// Must equal `Position.next_shield_seq`.
    pub sequence: u64,
    /// The Pending Shield PDA bump.
    pub bump: u8,
}

/// The value a successful refund releases. The instruction transfers exactly
/// `amount` of `asset` from the vault to `destination`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShieldRefund {
    /// The asset (selects the vault).
    pub asset: ShieldAsset,
    /// The amount to return.
    pub amount: u64,
    /// The recorded source token account.
    pub destination: Pubkey,
}

/// What [`settle_shield_attempt`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShieldAttemptOutcome {
    /// The current attempt was `Applied`; the Pending Shield is `Credited`.
    Credited {
        /// The credited asset.
        asset: ShieldAsset,
        /// The credited amount.
        amount: u64,
    },
    /// The current attempt ended with this non-`Applied` status. The Pending
    /// Shield stays `Deposited` and may be retried or refunded.
    Retryable(IntentStatus),
    /// Old attempt, foreign Intent, or final Pending Shield: nothing changed.
    Ignored,
}

/// Checks that `intent` is a fresh attempt for this owner and Position at the
/// Position's current generation.
fn check_new_attempt(
    intent: &OperationIntent,
    owner: &Pubkey,
    position_key: &Pubkey,
    position_generation: u64,
) -> Result<(), PendingShieldError> {
    let bound = intent.kind == OperationKind::Shield
        && intent.status == IntentStatus::Queued
        && intent.owner == *owner
        && intent.key != Pubkey::default()
        && intent.position == Some(*position_key)
        && intent.position_generation == Some(position_generation);
    if !bound {
        return Err(PendingShieldError::AttemptBindingMismatch);
    }
    Ok(())
}

/// Checks that the presented `active` Intent is this Pending Shield's active
/// attempt and has ended without a credit.
fn check_active_attempt_ended(
    shield: &PendingShield,
    active: &OperationIntent,
) -> Result<(), PendingShieldError> {
    if active.key != shield.active_intent
        || active.computation_offset != shield.active_computation_offset
    {
        return Err(PendingShieldError::ActiveIntentMismatch);
    }
    match active.status {
        IntentStatus::Queued => Err(PendingShieldError::AttemptQueued),
        IntentStatus::Applied => Err(PendingShieldError::AttemptApplied),
        _ => Ok(()),
    }
}

/// `shield` step 2: records the deposit in `Deposited` with attempt 1 bound to
/// `intent`, and advances `Position.next_shield_seq`.
///
/// Call after the exact `amount` was transferred into the vault and after
/// `intent` was built (kind `Shield`, `Queued`, owner, `position =
/// new.position`, `position_generation` = the Position's current generation).
/// Atomic: on error neither account changes.
pub fn open_pending_shield(
    shield: &mut PendingShield,
    position: &mut Position,
    new: &NewPendingShield,
    intent: &OperationIntent,
) -> Result<(), PendingShieldError> {
    if shield.status != PendingShieldStatus::Uninitialized {
        return Err(PendingShieldError::AlreadyExists);
    }
    if new.amount == 0 {
        return Err(PendingShieldError::ZeroAmount);
    }
    if position.owner != new.owner {
        return Err(PendingShieldError::PositionMismatch);
    }
    if position.status != PositionStatus::Active {
        return Err(PendingShieldError::PositionNotActive);
    }
    let generation = position.coordination.generation;
    check_new_attempt(intent, &new.owner, &new.position, generation)?;
    if new.sequence != position.next_shield_seq {
        return Err(PendingShieldError::SequenceMismatch);
    }
    let next_sequence = new
        .sequence
        .checked_add(1)
        .ok_or(PendingShieldError::SequenceOverflow)?;

    position.next_shield_seq = next_sequence;
    *shield = PendingShield {
        owner: new.owner,
        position: new.position,
        source_token_account: new.source_token_account,
        asset: new.asset,
        status: PendingShieldStatus::Deposited,
        bump: new.bump,
        amount: new.amount,
        sequence: new.sequence,
        attempt_sequence: 1,
        active_intent: intent.key,
        active_computation_offset: intent.computation_offset,
        captured_position_generation: generation,
        reserved: [0; 32],
    };
    Ok(())
}

/// Callback step: records the terminal outcome of an attempt.
///
/// Call in the callback instruction after the Intent's terminal status is
/// set, with that Intent. Only the current attempt of a `Deposited` Pending
/// Shield has effect: `Applied` marks `Credited` (the caller must apply the
/// encrypted credit in the same instruction, and only for this outcome); any
/// other terminal status leaves `Deposited`. An old attempt, a foreign Intent,
/// or a final Pending Shield returns `Ignored` and changes nothing. Atomic.
pub fn settle_shield_attempt(
    shield: &mut PendingShield,
    intent: &OperationIntent,
) -> Result<ShieldAttemptOutcome, PendingShieldError> {
    if !shield.is_current_attempt(&intent.key, intent.computation_offset) {
        return Ok(ShieldAttemptOutcome::Ignored);
    }
    let bound = intent.kind == OperationKind::Shield
        && intent.owner == shield.owner
        && intent.position == Some(shield.position)
        && intent.position_generation == Some(shield.captured_position_generation);
    if !bound {
        return Err(PendingShieldError::AttemptBindingMismatch);
    }
    match intent.status {
        IntentStatus::Queued => Err(PendingShieldError::AttemptQueued),
        IntentStatus::Applied => {
            shield.status = PendingShieldStatus::Credited;
            Ok(ShieldAttemptOutcome::Credited {
                asset: shield.asset,
                amount: shield.amount,
            })
        }
        status => Ok(ShieldAttemptOutcome::Retryable(status)),
    }
}

/// `retry_shield`: installs attempt `n + 1` bound to `next`.
///
/// Requires the owner, `Deposited`, the matching Position, the presented
/// `prior` Intent to be the active attempt and terminal without `Applied`,
/// and `next` to be a fresh Shield Intent at the Position's current
/// generation. Records the new Intent, offset, and generation. Returns the new
/// attempt sequence. Atomic. Not pausable (Requirement 4.8).
pub fn retry_pending_shield(
    shield: &mut PendingShield,
    signer: &Pubkey,
    position: &Position,
    position_key: &Pubkey,
    prior: &OperationIntent,
    next: &OperationIntent,
) -> Result<u64, PendingShieldError> {
    if *signer != shield.owner {
        return Err(PendingShieldError::NotOwner);
    }
    if shield.status != PendingShieldStatus::Deposited {
        return Err(PendingShieldError::NotDeposited);
    }
    if *position_key != shield.position || position.owner != shield.owner {
        return Err(PendingShieldError::PositionMismatch);
    }
    check_active_attempt_ended(shield, prior)?;
    if next.key == shield.active_intent {
        return Err(PendingShieldError::AttemptReused);
    }
    if position.status != PositionStatus::Active {
        return Err(PendingShieldError::PositionNotActive);
    }
    let generation = position.coordination.generation;
    check_new_attempt(next, &shield.owner, position_key, generation)?;
    let attempt = shield
        .attempt_sequence
        .checked_add(1)
        .ok_or(PendingShieldError::AttemptOverflow)?;

    shield.attempt_sequence = attempt;
    shield.active_intent = next.key;
    shield.active_computation_offset = next.computation_offset;
    shield.captured_position_generation = generation;
    Ok(attempt)
}

/// `refund_shield`: marks `Refunded` and returns the value to release.
///
/// Requires the owner, `Deposited`, and the presented `active` Intent to be
/// the active attempt and terminal without `Applied`. The instruction must
/// then transfer exactly [`ShieldRefund::amount`] to
/// [`ShieldRefund::destination`] in the same transaction. Because the status
/// becomes final, a second refund fails with
/// [`PendingShieldError::NotDeposited`]. Atomic. Not pausable
/// (Requirement 4.8).
pub fn refund_pending_shield(
    shield: &mut PendingShield,
    signer: &Pubkey,
    active: &OperationIntent,
) -> Result<ShieldRefund, PendingShieldError> {
    if *signer != shield.owner {
        return Err(PendingShieldError::NotOwner);
    }
    if shield.status != PendingShieldStatus::Deposited {
        return Err(PendingShieldError::NotDeposited);
    }
    check_active_attempt_ended(shield, active)?;
    shield.status = PendingShieldStatus::Refunded;
    Ok(ShieldRefund {
        asset: shield.asset,
        amount: shield.amount,
        destination: shield.source_token_account,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::derive_intent_address;
    use anchor_lang::{AccountDeserialize, AccountSerialize};

    fn tag<T: AnchorSerialize>(value: T) -> Vec<u8> {
        let mut bytes = Vec::new();
        value.serialize(&mut bytes).unwrap();
        bytes
    }

    const OWNER: Pubkey = Pubkey::new_from_array([0x11; 32]);
    const MARKET: Pubkey = Pubkey::new_from_array([0x22; 32]);
    const SOURCE: Pubkey = Pubkey::new_from_array([0x33; 32]);
    const STRANGER: Pubkey = Pubkey::new_from_array([0x44; 32]);

    fn position_key() -> Pubkey {
        crate::state::position::derive_position_address(&MARKET, &OWNER, &crate::ID).0
    }

    fn active_position() -> Position {
        let mut position = Position::blank(OWNER, MARKET, 1);
        position.status = PositionStatus::Active;
        position.coordination.generation = 3;
        position
    }

    fn intent(sequence: u64, offset: u64, generation: u64) -> OperationIntent {
        OperationIntent {
            key: derive_intent_address(&OWNER, sequence, &crate::ID).0,
            owner: OWNER,
            payer: OWNER,
            reclaim_authority: OWNER,
            kind: OperationKind::Shield,
            status: IntentStatus::Queued,
            sequence,
            computation_offset: offset,
            queued_slot: 10,
            decision_expiry_slot: 20,
            lock_expiry_slot: 30,
            position: Some(position_key()),
            position_generation: Some(generation),
            ..OperationIntent::default()
        }
    }

    fn new(sequence: u64, amount: u64) -> NewPendingShield {
        NewPendingShield {
            owner: OWNER,
            position: position_key(),
            source_token_account: SOURCE,
            asset: ShieldAsset::Usdc,
            amount,
            sequence,
            bump: 0xFC,
        }
    }

    fn opened() -> (PendingShield, Position, OperationIntent) {
        let mut position = active_position();
        let mut shield = PendingShield::default();
        let first = intent(0, 100, 3);
        open_pending_shield(&mut shield, &mut position, &new(0, 500), &first).unwrap();
        (shield, position, first)
    }

    fn ended(mut intent: OperationIntent, status: IntentStatus) -> OperationIntent {
        intent.status = status;
        intent
    }

    // ---- Layout ------------------------------------------------------------

    #[test]
    fn pending_shield_space_is_fixed() {
        assert_eq!(ShieldAsset::INIT_SPACE, 1);
        assert_eq!(PendingShieldStatus::INIT_SPACE, 1);
        assert_eq!(
            PendingShield::INIT_SPACE,
            32 + 32 + 32 + 1 + 1 + 1 + 8 + 8 + 8 + 32 + 8 + 8 + 32
        );
        assert_eq!(PendingShield::INIT_SPACE, 203);
        assert_eq!(PendingShield::ACCOUNT_LEN, 211);
    }

    #[test]
    fn pending_shield_byte_layout() {
        let shield = PendingShield {
            owner: Pubkey::new_from_array([0x01; 32]),
            position: Pubkey::new_from_array([0x02; 32]),
            source_token_account: Pubkey::new_from_array([0x03; 32]),
            asset: ShieldAsset::Usdc,
            status: PendingShieldStatus::Refunded,
            bump: 0xFE,
            amount: 0x1112_1314_1516_1718,
            sequence: 0x2122_2324_2526_2728,
            attempt_sequence: 0x3132_3334_3536_3738,
            active_intent: Pubkey::new_from_array([0x04; 32]),
            active_computation_offset: 0x4142_4344_4546_4748,
            captured_position_generation: 0x5152_5354_5556_5758,
            reserved: [0; 32],
        };
        let mut bytes = Vec::new();
        shield.try_serialize(&mut bytes).unwrap();
        let u64_at = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());

        assert_eq!(bytes.len(), PendingShield::ACCOUNT_LEN);
        assert_eq!(&bytes[..8], PendingShield::DISCRIMINATOR);
        assert_eq!(bytes[8..40], [0x01; 32]);
        assert_eq!(bytes[40..72], [0x02; 32]);
        assert_eq!(bytes[72..104], [0x03; 32]);
        assert_eq!(bytes[104], 1, "asset");
        assert_eq!(bytes[105], 3, "status");
        assert_eq!(bytes[106], 0xFE);
        assert_eq!(u64_at(107), 0x1112_1314_1516_1718);
        assert_eq!(u64_at(115), 0x2122_2324_2526_2728);
        assert_eq!(u64_at(123), 0x3132_3334_3536_3738);
        assert_eq!(bytes[131..163], [0x04; 32]);
        assert_eq!(u64_at(163), 0x4142_4344_4546_4748);
        assert_eq!(u64_at(171), 0x5152_5354_5556_5758);
        assert_eq!(bytes[179..211], [0u8; 32]);

        let back = PendingShield::try_deserialize(&mut bytes.as_slice()).unwrap();
        assert_eq!(back, shield);
    }

    #[test]
    fn tags_are_stable_and_zeroed_data_is_uninitialized() {
        assert_eq!(tag(ShieldAsset::Sol), [0]);
        assert_eq!(tag(ShieldAsset::Usdc), [1]);
        for (status, expected) in [
            (PendingShieldStatus::Uninitialized, 0u8),
            (PendingShieldStatus::Deposited, 1),
            (PendingShieldStatus::Credited, 2),
            (PendingShieldStatus::Refunded, 3),
        ] {
            assert_eq!(tag(status), [expected]);
        }
        let mut bytes = PendingShield::DISCRIMINATOR.to_vec();
        bytes.extend([0u8; PendingShield::INIT_SPACE]);
        let zeroed = PendingShield::try_deserialize(&mut bytes.as_slice()).unwrap();
        assert_eq!(zeroed.status, PendingShieldStatus::Uninitialized);
    }

    #[test]
    fn discriminator_is_distinct() {
        use crate::operation::OperationCounter;
        use crate::position::SpikePosition;
        use crate::state::PoolState;
        for other in [
            Position::DISCRIMINATOR,
            PoolState::DISCRIMINATOR,
            OperationIntent::DISCRIMINATOR,
            OperationCounter::DISCRIMINATOR,
            SpikePosition::DISCRIMINATOR,
        ] {
            assert_ne!(PendingShield::DISCRIMINATOR, other);
        }
    }

    #[test]
    fn addresses_are_unique_per_position_and_sequence() {
        let a = derive_pending_shield_address(&position_key(), 0, &crate::ID).0;
        let manual = Pubkey::find_program_address(
            &[
                b"pending_shield",
                position_key().as_ref(),
                &0u64.to_le_bytes(),
            ],
            &crate::ID,
        )
        .0;
        assert_eq!(a, manual);
        assert_ne!(
            a,
            derive_pending_shield_address(&position_key(), 1, &crate::ID).0
        );
        assert_ne!(a, derive_pending_shield_address(&STRANGER, 0, &crate::ID).0);
        assert_ne!(a, position_key());
        assert_ne!(a, derive_intent_address(&OWNER, 0, &crate::ID).0);
    }

    // ---- Open --------------------------------------------------------------

    #[test]
    fn open_records_every_field_and_advances_the_sequence() {
        let (shield, position, first) = opened();
        assert_eq!(shield.owner, OWNER);
        assert_eq!(shield.position, position_key());
        assert_eq!(shield.source_token_account, SOURCE);
        assert_eq!(shield.asset, ShieldAsset::Usdc);
        assert_eq!(shield.status, PendingShieldStatus::Deposited);
        assert_eq!(shield.bump, 0xFC);
        assert_eq!(shield.amount, 500);
        assert_eq!(shield.sequence, 0);
        assert_eq!(shield.attempt_sequence, 1);
        assert_eq!(shield.active_intent, first.key);
        assert_eq!(shield.active_computation_offset, 100);
        assert_eq!(shield.captured_position_generation, 3);
        assert_eq!(shield.reserved, [0; 32]);
        assert_eq!(position.next_shield_seq, 1);
        assert_eq!(position.coordination.generation, 3, "generation untouched");
        assert!(shield.is_current_attempt(&first.key, 100));
        assert!(!shield.is_current_attempt(&first.key, 101));
    }

    #[test]
    fn open_rejects_each_bad_input_without_change() {
        let base = active_position();
        let ok_intent = intent(0, 100, 3);
        let check = |shield: PendingShield,
                     position: Position,
                     new: NewPendingShield,
                     intent: OperationIntent,
                     expected: PendingShieldError| {
            let (mut s, mut p) = (shield, position.clone());
            assert_eq!(
                open_pending_shield(&mut s, &mut p, &new, &intent),
                Err(expected)
            );
            assert_eq!(s, shield);
            assert_eq!(p.next_shield_seq, position.next_shield_seq);
        };

        let (existing, _, _) = opened();
        check(
            existing,
            base.clone(),
            new(0, 500),
            ok_intent,
            PendingShieldError::AlreadyExists,
        );
        let empty = PendingShield::default();
        check(
            empty,
            base.clone(),
            new(0, 0),
            ok_intent,
            PendingShieldError::ZeroAmount,
        );
        let mut foreign = base.clone();
        foreign.owner = STRANGER;
        check(
            empty,
            foreign,
            new(0, 500),
            ok_intent,
            PendingShieldError::PositionMismatch,
        );
        for status in [
            PositionStatus::Uninitialized,
            PositionStatus::Initializing,
            PositionStatus::InitFailed,
        ] {
            let mut inactive = base.clone();
            inactive.status = status;
            check(
                empty,
                inactive,
                new(0, 500),
                ok_intent,
                PendingShieldError::PositionNotActive,
            );
        }
        let bad_intents = [
            OperationIntent {
                kind: OperationKind::Unshield,
                ..ok_intent
            },
            ended(ok_intent, IntentStatus::Failed),
            OperationIntent {
                owner: STRANGER,
                ..ok_intent
            },
            OperationIntent {
                key: Pubkey::default(),
                ..ok_intent
            },
            OperationIntent {
                position: Some(STRANGER),
                ..ok_intent
            },
            OperationIntent {
                position_generation: Some(2),
                ..ok_intent
            },
            OperationIntent {
                position: None,
                position_generation: None,
                ..ok_intent
            },
        ];
        for bad in bad_intents {
            check(
                empty,
                base.clone(),
                new(0, 500),
                bad,
                PendingShieldError::AttemptBindingMismatch,
            );
        }
        check(
            empty,
            base.clone(),
            new(1, 500),
            ok_intent,
            PendingShieldError::SequenceMismatch,
        );
        let mut advanced = base.clone();
        advanced.next_shield_seq = 2;
        check(
            empty,
            advanced,
            new(1, 500),
            ok_intent,
            PendingShieldError::SequenceMismatch,
        );
        let mut saturated = base;
        saturated.next_shield_seq = u64::MAX;
        check(
            empty,
            saturated,
            new(u64::MAX, 500),
            ok_intent,
            PendingShieldError::SequenceOverflow,
        );
    }

    #[test]
    fn a_replayed_sequence_cannot_open_a_second_shield() {
        let (_, mut position, _) = opened();
        let mut second = PendingShield::default();
        assert_eq!(
            open_pending_shield(&mut second, &mut position, &new(0, 9), &intent(1, 101, 3)),
            Err(PendingShieldError::SequenceMismatch)
        );
        open_pending_shield(&mut second, &mut position, &new(1, 9), &intent(1, 101, 3)).unwrap();
        assert_eq!(second.sequence, 1);
        assert_eq!(position.next_shield_seq, 2);
    }

    // ---- Settle ------------------------------------------------------------

    #[test]
    fn applied_current_attempt_credits_once() {
        let (mut shield, _, first) = opened();
        let applied = ended(first, IntentStatus::Applied);
        assert_eq!(
            settle_shield_attempt(&mut shield, &applied),
            Ok(ShieldAttemptOutcome::Credited {
                asset: ShieldAsset::Usdc,
                amount: 500
            })
        );
        assert_eq!(shield.status, PendingShieldStatus::Credited);
        let before = shield;
        // A duplicate callback is a no-op.
        assert_eq!(
            settle_shield_attempt(&mut shield, &applied),
            Ok(ShieldAttemptOutcome::Ignored)
        );
        assert_eq!(shield, before);
        // Refund and retry are refused after the credit.
        assert_eq!(
            refund_pending_shield(&mut shield, &OWNER, &applied),
            Err(PendingShieldError::NotDeposited)
        );
        assert_eq!(shield, before);
    }

    #[test]
    fn non_applied_outcomes_leave_deposited() {
        for status in [
            IntentStatus::Zero,
            IntentStatus::Failed,
            IntentStatus::Stale,
            IntentStatus::Expired,
            IntentStatus::Abandoned,
        ] {
            let (mut shield, _, first) = opened();
            let before = shield;
            assert_eq!(
                settle_shield_attempt(&mut shield, &ended(first, status)),
                Ok(ShieldAttemptOutcome::Retryable(status))
            );
            assert_eq!(shield, before);
        }
    }

    #[test]
    fn settle_rejects_a_queued_or_misbound_current_attempt() {
        let (mut shield, _, first) = opened();
        let before = shield;
        assert_eq!(
            settle_shield_attempt(&mut shield, &first),
            Err(PendingShieldError::AttemptQueued)
        );
        let applied = ended(first, IntentStatus::Applied);
        for bad in [
            OperationIntent {
                kind: OperationKind::Unshield,
                ..applied
            },
            OperationIntent {
                owner: STRANGER,
                ..applied
            },
            OperationIntent {
                position: Some(STRANGER),
                ..applied
            },
            OperationIntent {
                position_generation: Some(4),
                ..applied
            },
        ] {
            assert_eq!(
                settle_shield_attempt(&mut shield, &bad),
                Err(PendingShieldError::AttemptBindingMismatch)
            );
        }
        assert_eq!(shield, before);
    }

    #[test]
    fn foreign_intent_and_wrong_offset_are_ignored() {
        let (mut shield, _, first) = opened();
        let before = shield;
        let foreign = ended(intent(9, 100, 3), IntentStatus::Applied);
        let wrong_offset = OperationIntent {
            computation_offset: 101,
            ..ended(first, IntentStatus::Applied)
        };
        for other in [foreign, wrong_offset] {
            assert_eq!(
                settle_shield_attempt(&mut shield, &other),
                Ok(ShieldAttemptOutcome::Ignored)
            );
            assert_eq!(shield, before);
        }
    }

    // ---- Retry -------------------------------------------------------------

    #[test]
    fn retry_installs_a_new_attempt_and_old_callbacks_are_no_ops() {
        let (mut shield, mut position, first) = opened();
        let failed = ended(first, IntentStatus::Failed);
        settle_shield_attempt(&mut shield, &failed).unwrap();
        // The Position moved on before the retry.
        position.coordination.generation = 7;
        let second = intent(1, 200, 7);
        assert_eq!(
            retry_pending_shield(
                &mut shield,
                &OWNER,
                &position,
                &position_key(),
                &failed,
                &second
            ),
            Ok(2)
        );
        assert_eq!(shield.attempt_sequence, 2);
        assert_eq!(shield.active_intent, second.key);
        assert_eq!(shield.active_computation_offset, 200);
        assert_eq!(shield.captured_position_generation, 7);
        assert_eq!(shield.status, PendingShieldStatus::Deposited);

        // The old attempt cannot credit, even reported as Applied.
        let before = shield;
        for status in [IntentStatus::Applied, IntentStatus::Failed] {
            assert_eq!(
                settle_shield_attempt(&mut shield, &ended(first, status)),
                Ok(ShieldAttemptOutcome::Ignored)
            );
        }
        assert_eq!(shield, before);

        // The new attempt can.
        assert_eq!(
            settle_shield_attempt(&mut shield, &ended(second, IntentStatus::Applied)),
            Ok(ShieldAttemptOutcome::Credited {
                asset: ShieldAsset::Usdc,
                amount: 500
            })
        );
    }

    #[test]
    fn retry_rejects_each_bad_input_without_change() {
        let (base, position, first) = opened();
        let failed = ended(first, IntentStatus::Failed);
        let next = intent(1, 200, 3);
        let check = |shield: PendingShield,
                     signer: Pubkey,
                     position: &Position,
                     position_key: Pubkey,
                     prior: OperationIntent,
                     next: OperationIntent,
                     expected: PendingShieldError| {
            let mut s = shield;
            assert_eq!(
                retry_pending_shield(&mut s, &signer, position, &position_key, &prior, &next),
                Err(expected)
            );
            assert_eq!(s, shield);
        };
        let pk = position_key();

        check(
            base,
            STRANGER,
            &position,
            pk,
            failed,
            next,
            PendingShieldError::NotOwner,
        );
        for status in [
            PendingShieldStatus::Uninitialized,
            PendingShieldStatus::Credited,
            PendingShieldStatus::Refunded,
        ] {
            let shield = PendingShield { status, ..base };
            check(
                shield,
                OWNER,
                &position,
                pk,
                failed,
                next,
                PendingShieldError::NotDeposited,
            );
        }
        check(
            base,
            OWNER,
            &position,
            STRANGER,
            failed,
            next,
            PendingShieldError::PositionMismatch,
        );
        let mut foreign = position.clone();
        foreign.owner = STRANGER;
        check(
            base,
            OWNER,
            &foreign,
            pk,
            failed,
            next,
            PendingShieldError::PositionMismatch,
        );
        let wrong_offset = OperationIntent {
            computation_offset: 1,
            ..failed
        };
        let other = ended(intent(5, 100, 3), IntentStatus::Failed);
        for prior in [wrong_offset, other] {
            check(
                base,
                OWNER,
                &position,
                pk,
                prior,
                next,
                PendingShieldError::ActiveIntentMismatch,
            );
        }
        check(
            base,
            OWNER,
            &position,
            pk,
            first,
            next,
            PendingShieldError::AttemptQueued,
        );
        check(
            base,
            OWNER,
            &position,
            pk,
            ended(first, IntentStatus::Applied),
            next,
            PendingShieldError::AttemptApplied,
        );
        check(
            base,
            OWNER,
            &position,
            pk,
            failed,
            OperationIntent {
                computation_offset: 300,
                ..first
            },
            PendingShieldError::AttemptReused,
        );
        let mut inactive = position.clone();
        inactive.status = PositionStatus::InitFailed;
        check(
            base,
            OWNER,
            &inactive,
            pk,
            failed,
            next,
            PendingShieldError::PositionNotActive,
        );
        for bad in [
            intent(1, 200, 2),
            ended(next, IntentStatus::Failed),
            OperationIntent {
                kind: OperationKind::Repay,
                ..next
            },
        ] {
            check(
                base,
                OWNER,
                &position,
                pk,
                failed,
                bad,
                PendingShieldError::AttemptBindingMismatch,
            );
        }
        let saturated = PendingShield {
            attempt_sequence: u64::MAX,
            ..base
        };
        check(
            saturated,
            OWNER,
            &position,
            pk,
            failed,
            next,
            PendingShieldError::AttemptOverflow,
        );
    }

    // ---- Refund ------------------------------------------------------------

    #[test]
    fn refund_returns_the_recorded_value_exactly_once() {
        let (mut shield, _, first) = opened();
        let abandoned = ended(first, IntentStatus::Abandoned);
        assert_eq!(
            refund_pending_shield(&mut shield, &OWNER, &abandoned),
            Ok(ShieldRefund {
                asset: ShieldAsset::Usdc,
                amount: 500,
                destination: SOURCE
            })
        );
        assert_eq!(shield.status, PendingShieldStatus::Refunded);
        let before = shield;
        assert_eq!(
            refund_pending_shield(&mut shield, &OWNER, &abandoned),
            Err(PendingShieldError::NotDeposited)
        );
        // A late callback of the refunded attempt cannot credit.
        assert_eq!(
            settle_shield_attempt(&mut shield, &ended(first, IntentStatus::Applied)),
            Ok(ShieldAttemptOutcome::Ignored)
        );
        assert_eq!(shield, before);
    }

    #[test]
    fn refund_rejects_each_bad_input_without_change() {
        let (base, _, first) = opened();
        let check = |shield: PendingShield,
                     signer: Pubkey,
                     active: OperationIntent,
                     expected: PendingShieldError| {
            let mut s = shield;
            assert_eq!(
                refund_pending_shield(&mut s, &signer, &active),
                Err(expected)
            );
            assert_eq!(s, shield);
        };
        let failed = ended(first, IntentStatus::Failed);
        check(base, STRANGER, failed, PendingShieldError::NotOwner);
        let uninitialized = PendingShield {
            status: PendingShieldStatus::Uninitialized,
            ..base
        };
        check(
            uninitialized,
            OWNER,
            failed,
            PendingShieldError::NotDeposited,
        );
        check(
            base,
            OWNER,
            ended(intent(4, 100, 3), IntentStatus::Failed),
            PendingShieldError::ActiveIntentMismatch,
        );
        check(base, OWNER, first, PendingShieldError::AttemptQueued);
        check(
            base,
            OWNER,
            ended(first, IntentStatus::Applied),
            PendingShieldError::AttemptApplied,
        );
    }

    // ---- Property test -----------------------------------------------------

    /// Deterministic linear-congruential sampler (no external crate).
    struct Lcg(u64);
    impl Lcg {
        fn sample(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 11
        }

        fn below(&mut self, n: u64) -> u64 {
            self.sample() % n
        }
    }

    const TERMINAL: [IntentStatus; 6] = [
        IntentStatus::Applied,
        IntentStatus::Zero,
        IntentStatus::Failed,
        IntentStatus::Stale,
        IntentStatus::Expired,
        IntentStatus::Abandoned,
    ];

    /// One simulated Position with its Pending Shields and Shield Intents.
    struct World {
        position: Position,
        next_intent_seq: u64,
        shields: Vec<PendingShield>,
        /// Every Intent ever created, with the index of its shield.
        intents: Vec<(OperationIntent, usize)>,
        /// Value outcomes per shield: credits and refunds.
        credits: Vec<u32>,
        refunds: Vec<u32>,
    }

    impl World {
        fn new_intent(&mut self, shield: usize) -> OperationIntent {
            let sequence = self.next_intent_seq;
            self.next_intent_seq += 1;
            let made = intent(
                sequence,
                0xC0_0000 + sequence,
                self.position.coordination.generation,
            );
            self.intents.push((made, shield));
            made
        }

        fn queued(&self, shield: usize) -> usize {
            self.intents
                .iter()
                .filter(|(i, s)| *s == shield && i.status == IntentStatus::Queued)
                .count()
        }

        fn active(&self, shield: usize) -> OperationIntent {
            let s = &self.shields[shield];
            self.intents
                .iter()
                .find(|(i, _)| i.key == s.active_intent)
                .map(|(i, _)| *i)
                .unwrap()
        }
    }

    /// Random interleavings of open, attempt outcomes, duplicate and old-attempt
    /// callbacks, retries, and refunds (real and with stale Intents). After
    /// every step: only `Deposited -> Credited | Refunded` happens; a shield
    /// gets at most one value outcome (credit XOR refund, once); retry and
    /// refund never succeed while the active attempt is queued; old-attempt
    /// callbacks change nothing; at most one attempt per shield is queued and
    /// it is the active one; sequences and addresses are unique.
    ///
    /// **Validates: Requirements 5.4, 5.6**
    #[test]
    fn prop_pending_shield_lifecycle_is_exclusive_and_replay_safe() {
        for seed in 0..96u64 {
            let mut rng = Lcg(0x5EED_5200 ^ seed);
            let mut world = World {
                position: active_position(),
                next_intent_seq: 0,
                shields: Vec::new(),
                intents: Vec::new(),
                credits: Vec::new(),
                refunds: Vec::new(),
            };

            for _ in 0..160 {
                let before_shields = world.shields.clone();
                let action = if world.shields.is_empty() {
                    0
                } else {
                    rng.below(7)
                };
                match action {
                    0 => {
                        // Open (sometimes with a replayed or skipped sequence).
                        let next = world.position.next_shield_seq;
                        let sequence = match rng.below(5) {
                            0 if next > 0 => next - 1,
                            1 => next + 1,
                            _ => next,
                        };
                        let index = world.shields.len();
                        let attempt = world.new_intent(index);
                        let mut shield = PendingShield::default();
                        let amount = rng.below(4);
                        let result = open_pending_shield(
                            &mut shield,
                            &mut world.position,
                            &NewPendingShield {
                                amount,
                                asset: if rng.below(2) == 0 {
                                    ShieldAsset::Sol
                                } else {
                                    ShieldAsset::Usdc
                                },
                                ..new(sequence, amount)
                            },
                            &attempt,
                        );
                        if amount == 0 {
                            assert_eq!(result, Err(PendingShieldError::ZeroAmount));
                        } else if sequence != next {
                            assert_eq!(result, Err(PendingShieldError::SequenceMismatch));
                        } else {
                            assert_eq!(result, Ok(()));
                        }
                        if result.is_ok() {
                            assert_eq!(world.position.next_shield_seq, next + 1);
                            world.shields.push(shield);
                            world.credits.push(0);
                            world.refunds.push(0);
                        } else {
                            assert_eq!(world.position.next_shield_seq, next);
                            // The unused Intent is dropped by the reverted tx.
                            world.intents.pop();
                            world.next_intent_seq -= 1;
                        }
                    }
                    1 | 2 => {
                        // A callback for a random Intent: the first one for a
                        // queued Intent, or a duplicate/late one otherwise.
                        let pick = rng.below(world.intents.len() as u64) as usize;
                        let (mut target, shield_index) = world.intents[pick];
                        let was_queued = target.status == IntentStatus::Queued;
                        if was_queued {
                            target.status = TERMINAL[rng.below(6) as usize];
                            world.intents[pick].0 = target;
                        }
                        let is_current = world.shields[shield_index]
                            .is_current_attempt(&target.key, target.computation_offset);
                        let before = world.shields[shield_index];
                        let outcome =
                            settle_shield_attempt(&mut world.shields[shield_index], &target)
                                .unwrap();
                        match outcome {
                            ShieldAttemptOutcome::Credited { amount, .. } => {
                                assert!(is_current && target.status == IntentStatus::Applied);
                                assert_eq!(amount, before.amount);
                                world.credits[shield_index] += 1;
                                // The credit wrote the Position.
                                world.position.coordination.generation += 1;
                            }
                            ShieldAttemptOutcome::Retryable(status) => {
                                assert!(is_current);
                                assert_eq!(status, target.status);
                                assert_eq!(world.shields[shield_index], before);
                            }
                            ShieldAttemptOutcome::Ignored => {
                                assert!(!is_current);
                                assert_eq!(world.shields[shield_index], before);
                            }
                        }
                        if !is_current {
                            assert_eq!(outcome, ShieldAttemptOutcome::Ignored);
                        }
                    }
                    3 | 4 => {
                        // Retry, presenting the real active Intent or a stale one.
                        let index = rng.below(world.shields.len() as u64) as usize;
                        let active = world.active(index);
                        let presented = if rng.below(4) == 0 {
                            let pick = rng.below(world.intents.len() as u64) as usize;
                            world.intents[pick].0
                        } else {
                            active
                        };
                        let queued_before = world.queued(index);
                        let next = world.new_intent(index);
                        let before = world.shields[index];
                        let position = world.position.clone();
                        let result = retry_pending_shield(
                            &mut world.shields[index],
                            &OWNER,
                            &position,
                            &position_key(),
                            &presented,
                            &next,
                        );
                        match result {
                            Ok(attempt) => {
                                assert_eq!(before.status, PendingShieldStatus::Deposited);
                                assert_eq!(presented, active);
                                assert!(active.status.is_terminal());
                                assert_ne!(active.status, IntentStatus::Applied);
                                assert_eq!(queued_before, 0, "retry while queued");
                                assert_eq!(attempt, before.attempt_sequence + 1);
                                assert_eq!(world.shields[index].active_intent, next.key);
                            }
                            Err(_) => {
                                assert_eq!(world.shields[index], before);
                                world.intents.pop();
                                world.next_intent_seq -= 1;
                            }
                        }
                    }
                    5 => {
                        // Refund, presenting the real active Intent or a stale one.
                        let index = rng.below(world.shields.len() as u64) as usize;
                        let active = world.active(index);
                        let presented = if rng.below(4) == 0 {
                            let pick = rng.below(world.intents.len() as u64) as usize;
                            world.intents[pick].0
                        } else {
                            active
                        };
                        let queued_before = world.queued(index);
                        let before = world.shields[index];
                        let signer = if rng.below(8) == 0 { STRANGER } else { OWNER };
                        match refund_pending_shield(&mut world.shields[index], &signer, &presented)
                        {
                            Ok(refund) => {
                                assert_eq!(signer, OWNER);
                                assert_eq!(before.status, PendingShieldStatus::Deposited);
                                assert_eq!(presented, active);
                                assert_eq!(queued_before, 0, "refund while queued");
                                assert_ne!(active.status, IntentStatus::Applied);
                                assert_eq!(refund.amount, before.amount);
                                assert_eq!(refund.destination, SOURCE);
                                world.refunds[index] += 1;
                            }
                            Err(_) => assert_eq!(world.shields[index], before),
                        }
                    }
                    _ => {
                        // An old-attempt callback: any Intent of this shield
                        // that is not its active one, reported as Applied.
                        let index = rng.below(world.shields.len() as u64) as usize;
                        let active_key = world.shields[index].active_intent;
                        let old: Vec<OperationIntent> = world
                            .intents
                            .iter()
                            .filter(|(i, s)| *s == index && i.key != active_key)
                            .map(|(i, _)| ended(*i, IntentStatus::Applied))
                            .collect();
                        if let Some(stale) = old.first() {
                            let before = world.shields[index];
                            assert_eq!(
                                settle_shield_attempt(&mut world.shields[index], stale),
                                Ok(ShieldAttemptOutcome::Ignored)
                            );
                            assert_eq!(world.shields[index], before);
                        }
                    }
                }

                // ---- Invariants ---------------------------------------------
                for (index, shield) in world.shields.iter().enumerate() {
                    if let Some(prior) = before_shields.get(index) {
                        let legal = prior.status == shield.status
                            || (prior.status == PendingShieldStatus::Deposited
                                && shield.status.is_final());
                        assert!(legal, "{:?} -> {:?}", prior.status, shield.status);
                        if prior.status.is_final() {
                            assert_eq!(prior, shield, "final shields are immutable");
                        }
                        assert_eq!(prior.amount, shield.amount);
                        assert_eq!(prior.sequence, shield.sequence);
                        assert!(shield.attempt_sequence >= prior.attempt_sequence);
                    }
                    let outcomes = world.credits[index] + world.refunds[index];
                    assert!(outcomes <= 1, "credit XOR refund, exactly once");
                    assert_eq!(
                        world.credits[index] == 1,
                        shield.status == PendingShieldStatus::Credited
                    );
                    assert_eq!(
                        world.refunds[index] == 1,
                        shield.status == PendingShieldStatus::Refunded
                    );
                    let queued = world.queued(index);
                    assert!(queued <= 1, "two queued attempts");
                    if queued == 1 {
                        assert_eq!(world.active(index).status, IntentStatus::Queued);
                        assert_eq!(shield.status, PendingShieldStatus::Deposited);
                    }
                    assert_eq!(shield.sequence, index as u64, "sequences unique");
                }
                assert_eq!(world.position.next_shield_seq, world.shields.len() as u64);
            }

            let mut addresses: Vec<Pubkey> = world
                .shields
                .iter()
                .map(|s| derive_pending_shield_address(&s.position, s.sequence, &crate::ID).0)
                .collect();
            addresses.sort();
            addresses.dedup();
            assert_eq!(addresses.len(), world.shields.len());
        }
    }
}
