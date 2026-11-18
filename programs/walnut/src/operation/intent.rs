//! Operation Intents.
//!
//! An Operation Intent is the pre-created PDA that records one queued
//! computation, its dependency snapshots, account/input commitments, Decision
//! Expiry, and a one-way terminal status. (Requirements 3.1-3.4.)
//!
//! Task 2.1 proved the status machine as plain Rust. Task 3.2 promotes the same
//! struct to the production Anchor account, so the transition rules stay in one
//! place:
//!
//! * the `mark_*` methods here are the only code that writes `status`;
//! * `operation::ownership` adds the actor and timing rules in front of them
//!   ([`OperationIntent::transition`]);
//! * the Task 2.1 lifecycle handlers keep calling `mark_*` directly, and the
//!   Task 3.1 ordering tests keep exercising them unchanged.
//!
//! No financial math lives here. Indexes, generations, and expiries are plain
//! fixed-size integers captured at queue time and compared by callbacks.
//!
//! # Account layout
//!
//! `OperationIntent` is 418 bytes: an 8-byte Anchor discriminator plus 410 bytes
//! of Borsh data. Fields with a fixed width come first, so their offsets never
//! move. The optional fields come last because Borsh writes `None` in one byte,
//! which shifts any field that follows. The account is always allocated at the
//! full 418 bytes, so a `None` simply leaves trailing zero bytes.
//!
//! ```text
//! offset  size  field
//!      0     8  discriminator
//!      8    32  key                        this Intent's own PDA address
//!     40    32  owner                      operation owner
//!     72    32  payer                      pays rent and Arcium fees
//!    104    32  reclaim_authority          signs rent and fee reclaim
//!    136     1  kind                       OperationKind tag
//!    137     1  status                     IntentStatus tag
//!    138     1  bump                       PDA bump
//!    139     8  sequence                   per-owner sequence (PDA seed)
//!    147     8  computation_offset         queued Arcium computation
//!    155     8  asset_generation
//!    163     8  oracle_generation
//!    171     8  rate_epoch_generation
//!    179    16  accounting_borrow_index    u128
//!    195     8  accounting_index_timestamp i64
//!    203    16  risk_borrow_index_at_expiry u128
//!    219    32  accounts_commitment
//!    251    32  inputs_commitment
//!    283     8  queued_slot
//!    291     8  decision_expiry_slot
//!    299     8  lock_expiry_slot
//!    307    33  position                   Option<Pubkey>
//!    340     9  position_generation        Option<u64>
//!    349    33  second_position            Option<Pubkey>
//!    382     9  second_position_generation Option<u64>
//!    391     9  pool_generation            Option<u64>
//!    400     9  oracle_publish_time_sol    Option<i64>
//!    409     9  oracle_publish_time_usdc   Option<i64>
//!    418        end
//! ```
//!
//! Integers are little-endian. `Option<T>` is a 1-byte flag (`0` or `1`)
//! followed by `T` when present. Later tasks may append fields at the end
//! (after growing the allocation) and must never reorder or resize these.

use anchor_lang::prelude::{
    account, AnchorDeserialize, AnchorSerialize, Discriminator, InitSpace, Pubkey, Space,
};

use super::ownership::IntentError;
use crate::state::coordination::LockOwner;
use crate::state::{CoordinationError, Slot};

/// The kind of operation an Intent coordinates.
///
/// Variants are append-only. The tag (`kind as u8`, also the Borsh variant
/// index) is stored on the account and hashed into the input commitment, so
/// reordering would change both.
#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub enum OperationKind {
    /// Placeholder lifecycle probe used by the feasibility gate.
    #[default]
    Probe,
    /// Position initialization.
    PositionInit,
    /// Shield (deposit into encrypted balance).
    Shield,
    /// Unshield (withdraw from encrypted balance).
    Unshield,
    /// USDC supply.
    Supply,
    /// USDC supply withdrawal.
    WithdrawSupply,
    /// USDC borrow.
    Borrow,
    /// USDC repay.
    Repay,
    /// Non-locking health check.
    Health,
    /// Partial liquidation.
    Liquidation,
    /// Viewing Key rotation (Task 5.5): re-encrypts the Position to a new key.
    ViewingKeyRotation,
    /// Read-only utilization band update (Task 7.2): takes no Lock.
    UtilizationBand,
    /// Reserve withdrawal to the treasury (Task 7.5): Locks the Pool.
    ReserveWithdrawal,
    /// Admin recovery of an empty Pool's dust residue (Task 8.1): Locks the Pool.
    RecoverEmptyPool,
    /// Demo position operation (add/withdraw collateral, borrow, repay).
    PositionOp,
}

impl OperationKind {
    /// The stable one-byte tag stored on the account and hashed into the input
    /// commitment.
    pub const fn tag(self) -> u8 {
        self as u8
    }
}

/// The one-way terminal status of an Operation Intent.
///
/// Every Intent starts [`IntentStatus::Queued`] and transitions exactly once to
/// one terminal state. (Requirement 3.4.) Variants are append-only; the tag is
/// the Borsh variant index.
#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub enum IntentStatus {
    /// Awaiting its callback or timeout recovery.
    #[default]
    Queued,
    /// Authenticated success, dependencies matched, nonzero transition applied.
    Applied,
    /// Authenticated success, dependencies matched, no value approved.
    Zero,
    /// Authenticated static failure callback.
    Failed,
    /// Authenticated success whose captured dependency/commitment changed.
    Stale,
    /// Authenticated success arriving after Decision Expiry while still queued.
    Expired,
    /// Permissionless timeout recovery after Lock expiry without a terminal callback.
    Abandoned,
}

impl IntentStatus {
    /// Returns whether this status is terminal (anything other than `Queued`).
    pub fn is_terminal(self) -> bool {
        !matches!(self, IntentStatus::Queued)
    }
}

/// An Operation Intent PDA.
///
/// Fields mirror the design's `OperationIntent`, plus the addressing and
/// ownership fields the production account needs (`key`, `sequence`,
/// `reclaim_authority`, `lock_expiry_slot`, `bump`). Snapshot fields are
/// captured at queue time and never change; `status` is the only mutable field
/// and it moves exactly once.
///
/// Slots are `u64` (the type behind [`Slot`]) so the `InitSpace` derive sees a
/// primitive.
#[account]
#[derive(InitSpace, Copy, Debug, Default, PartialEq, Eq)]
pub struct OperationIntent {
    /// The Intent PDA's own address, written at creation. Locks reference it as
    /// their owner.
    pub key: Pubkey,
    /// The operation owner. Part of the PDA seeds.
    pub owner: Pubkey,
    /// The account that paid the Intent rent and the Arcium computation fee.
    pub payer: Pubkey,
    /// The only signer allowed to reclaim computation rent and fees. It must
    /// equal `payer`, because the Arcium program requires the original payer.
    pub reclaim_authority: Pubkey,
    /// The operation kind.
    pub kind: OperationKind,
    /// The one-way terminal status.
    pub status: IntentStatus,
    /// The PDA bump.
    pub bump: u8,
    /// The per-owner sequence number. Part of the PDA seeds.
    pub sequence: u64,
    /// The queued computation offset.
    pub computation_offset: u64,
    /// Captured Asset State generation.
    pub asset_generation: u64,
    /// Captured oracle generation.
    pub oracle_generation: u64,
    /// Captured rate-epoch generation.
    pub rate_epoch_generation: u64,
    /// Accounting Borrow Index (WAD), the sole authority for share and Cash
    /// conversion.
    pub accounting_borrow_index: u128,
    /// The timestamp the accounting index was accrued to.
    pub accounting_index_timestamp: i64,
    /// Risk Borrow Index projected through Decision Expiry, the sole authority
    /// for health and cap checks.
    pub risk_borrow_index_at_expiry: u128,
    /// Canonical commitment over the ordered accounts (see `commitment`).
    pub accounts_commitment: [u8; 32],
    /// Canonical commitment over the operation kind and inputs.
    pub inputs_commitment: [u8; 32],
    /// The slot the computation was queued.
    pub queued_slot: u64,
    /// The latest slot at which the callback may commit.
    pub decision_expiry_slot: u64,
    /// The slot after which the Locks may be abandoned. Not before
    /// `decision_expiry_slot`.
    pub lock_expiry_slot: u64,
    /// Primary Position.
    pub position: Option<Pubkey>,
    /// Captured generation of the primary Position.
    pub position_generation: Option<u64>,
    /// Second Position (for example a liquidator).
    pub second_position: Option<Pubkey>,
    /// Captured generation of the second Position.
    pub second_position_generation: Option<u64>,
    /// Captured Pool generation, if the Pool is a dependency.
    pub pool_generation: Option<u64>,
    /// SOL/USD oracle publish time, where applicable.
    pub oracle_publish_time_sol: Option<i64>,
    /// USDC/USD oracle publish time, where applicable.
    pub oracle_publish_time_usdc: Option<i64>,
}

impl OperationIntent {
    /// Account data length including Anchor's 8-byte discriminator.
    pub const ACCOUNT_LEN: usize = 8 + Self::INIT_SPACE;

    /// The Lock owner identity of this Intent: its PDA address and computation
    /// offset. A Lock is released or abandoned only by an exact match on both.
    pub fn lock_owner(&self) -> LockOwner {
        LockOwner::new(self.key, self.computation_offset)
    }

    /// Returns whether Decision Expiry has passed at `current_slot`.
    pub fn is_past_decision_expiry(&self, current_slot: Slot) -> bool {
        current_slot > self.decision_expiry_slot
    }

    /// Returns whether the Locks are abandonable at `current_slot`. Matches
    /// `Lock::is_expired`: strictly after the expiry slot.
    pub fn is_past_lock_expiry(&self, current_slot: Slot) -> bool {
        current_slot > self.lock_expiry_slot
    }

    /// Checks a freshly populated Intent before it is stored.
    ///
    /// The queue instruction calls this once, after filling every field and
    /// before the Intent is written. It rejects:
    ///
    /// * a status other than `Queued`;
    /// * timing that breaks `queued_slot < decision_expiry_slot <=
    ///   lock_expiry_slot`, which guarantees abandonment can never become
    ///   possible while a callback may still apply a decision;
    /// * a `reclaim_authority` that is not the `payer`;
    /// * a Position without a captured generation (or a generation without a
    ///   Position), a second Position without a first, and the same Position
    ///   listed twice.
    pub fn validate_new(&self) -> Result<(), IntentError> {
        if self.status.is_terminal() {
            return Err(IntentError::NotQueued);
        }
        if self.queued_slot >= self.decision_expiry_slot
            || self.decision_expiry_slot > self.lock_expiry_slot
        {
            return Err(IntentError::InvalidTiming);
        }
        if self.reclaim_authority != self.payer {
            return Err(IntentError::ReclaimAuthorityNotPayer);
        }

        let first_paired = self.position.is_some() == self.position_generation.is_some();
        let second_paired =
            self.second_position.is_some() == self.second_position_generation.is_some();
        let second_has_first = self.second_position.is_none() || self.position.is_some();
        let distinct = self.position.is_none() || self.position != self.second_position;
        let consistent = first_paired && second_paired && second_has_first && distinct;
        if !consistent {
            return Err(IntentError::SnapshotMismatch);
        }
        Ok(())
    }

    /// Performs the single allowed `Queued -> terminal` transition.
    ///
    /// Fails with [`CoordinationError::IntentNotQueued`] if the Intent is
    /// already terminal, enforcing "transition exactly once". A `next` of
    /// `Queued` is rejected for the same reason.
    fn set_terminal(&mut self, next: IntentStatus) -> Result<(), CoordinationError> {
        if self.status.is_terminal() || next == IntentStatus::Queued {
            return Err(CoordinationError::IntentNotQueued);
        }
        self.status = next;
        Ok(())
    }

    /// Marks the Intent `Applied`.
    pub fn mark_applied(&mut self) -> Result<(), CoordinationError> {
        self.set_terminal(IntentStatus::Applied)
    }

    /// Marks the Intent `Zero`.
    pub fn mark_zero(&mut self) -> Result<(), CoordinationError> {
        self.set_terminal(IntentStatus::Zero)
    }

    /// Marks the Intent `Failed`.
    pub fn mark_failed(&mut self) -> Result<(), CoordinationError> {
        self.set_terminal(IntentStatus::Failed)
    }

    /// Marks the Intent `Stale`.
    pub fn mark_stale(&mut self) -> Result<(), CoordinationError> {
        self.set_terminal(IntentStatus::Stale)
    }

    /// Marks the Intent `Expired`.
    pub fn mark_expired(&mut self) -> Result<(), CoordinationError> {
        self.set_terminal(IntentStatus::Expired)
    }

    /// Marks the Intent `Abandoned`.
    pub fn mark_abandoned(&mut self) -> Result<(), CoordinationError> {
        self.set_terminal(IntentStatus::Abandoned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::sha256::sha256;
    use anchor_lang::{AccountDeserialize, AccountSerialize};

    fn key(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    /// A fully populated, valid Intent whose every field has a distinct value.
    fn full_intent() -> OperationIntent {
        OperationIntent {
            key: key(0x01),
            owner: key(0x02),
            payer: key(0x03),
            reclaim_authority: key(0x03),
            kind: OperationKind::Borrow,
            status: IntentStatus::Queued,
            bump: 0xFE,
            sequence: 0x0102_0304_0506_0708,
            computation_offset: 0x1112_1314_1516_1718,
            asset_generation: 0x2122_2324_2526_2728,
            oracle_generation: 0x3132_3334_3536_3738,
            rate_epoch_generation: 0x4142_4344_4546_4748,
            accounting_borrow_index: 0x5152_5354_5556_5758_5960_6162_6364_6566,
            accounting_index_timestamp: 0x7172_7374_7576_7778,
            risk_borrow_index_at_expiry: 0x8182_8384_8586_8788_8990_9192_9394_9596,
            accounts_commitment: [0xA1; 32],
            inputs_commitment: [0xA2; 32],
            queued_slot: 100,
            decision_expiry_slot: 150,
            lock_expiry_slot: 300,
            position: Some(key(0xB1)),
            position_generation: Some(0xC1C2_C3C4_C5C6_C7C8),
            second_position: Some(key(0xB2)),
            second_position_generation: Some(0xD1D2_D3D4_D5D6_D7D8),
            pool_generation: Some(0xE1E2_E3E4_E5E6_E7E8),
            oracle_publish_time_sol: Some(0x0102_0304_0506_0708),
            oracle_publish_time_usdc: Some(-2),
        }
    }

    fn serialize(intent: &OperationIntent) -> Vec<u8> {
        let mut bytes = Vec::new();
        intent.try_serialize(&mut bytes).unwrap();
        bytes
    }

    // ---- Status machine (Task 2.1) ----------------------------------------

    #[test]
    fn queued_is_not_terminal() {
        assert!(!IntentStatus::Queued.is_terminal());
        assert!(IntentStatus::Applied.is_terminal());
        assert!(IntentStatus::Zero.is_terminal());
        assert!(IntentStatus::Failed.is_terminal());
        assert!(IntentStatus::Stale.is_terminal());
        assert!(IntentStatus::Expired.is_terminal());
        assert!(IntentStatus::Abandoned.is_terminal());
    }

    #[test]
    fn transition_happens_exactly_once() {
        let mut intent = OperationIntent::default();
        assert_eq!(intent.status, IntentStatus::Queued);
        intent.mark_applied().unwrap();
        assert_eq!(intent.status, IntentStatus::Applied);
        // A second transition of any kind is rejected and leaves status intact.
        assert_eq!(intent.mark_zero(), Err(CoordinationError::IntentNotQueued));
        assert_eq!(
            intent.mark_failed(),
            Err(CoordinationError::IntentNotQueued)
        );
        assert_eq!(intent.status, IntentStatus::Applied);
    }

    // ---- Layout (Task 3.2) -------------------------------------------------

    #[test]
    fn space_is_fixed() {
        let fixed = 4 * 32 // key, owner, payer, reclaim_authority
            + 3 // kind, status, bump
            + 8 * 5 // sequence, offset, three generations
            + 16 + 8 + 16 // accounting index, timestamp, risk index
            + 2 * 32 // commitments
            + 3 * 8; // queued, decision expiry, lock expiry
        let optional = 2 * 33 + 5 * 9;
        assert_eq!(fixed, 299);
        assert_eq!(optional, 111);
        assert_eq!(OperationIntent::INIT_SPACE, fixed + optional);
        assert_eq!(OperationIntent::INIT_SPACE, 410);
        assert_eq!(OperationIntent::ACCOUNT_LEN, 418);
    }

    #[test]
    fn enum_spaces_are_one_byte() {
        assert_eq!(OperationKind::INIT_SPACE, 1);
        assert_eq!(IntentStatus::INIT_SPACE, 1);
    }

    #[test]
    fn byte_layout_matches_the_documented_offsets() {
        let intent = full_intent();
        let bytes = serialize(&intent);

        assert_eq!(bytes.len(), OperationIntent::ACCOUNT_LEN);
        assert_eq!(&bytes[..8], OperationIntent::DISCRIMINATOR);
        assert_eq!(bytes[8..40], [0x01; 32]);
        assert_eq!(bytes[40..72], [0x02; 32]);
        assert_eq!(bytes[72..104], [0x03; 32]);
        assert_eq!(bytes[104..136], [0x03; 32]);
        assert_eq!(bytes[136], OperationKind::Borrow.tag());
        assert_eq!(bytes[136], 6);
        assert_eq!(bytes[137], 0); // Queued
        assert_eq!(bytes[138], 0xFE);
        assert_eq!(bytes[139..147], 0x0102_0304_0506_0708u64.to_le_bytes());
        assert_eq!(bytes[147..155], 0x1112_1314_1516_1718u64.to_le_bytes());
        assert_eq!(bytes[155..163], 0x2122_2324_2526_2728u64.to_le_bytes());
        assert_eq!(bytes[163..171], 0x3132_3334_3536_3738u64.to_le_bytes());
        assert_eq!(bytes[171..179], 0x4142_4344_4546_4748u64.to_le_bytes());
        assert_eq!(
            bytes[179..195],
            0x5152_5354_5556_5758_5960_6162_6364_6566u128.to_le_bytes()
        );
        assert_eq!(bytes[195..203], 0x7172_7374_7576_7778i64.to_le_bytes());
        assert_eq!(
            bytes[203..219],
            0x8182_8384_8586_8788_8990_9192_9394_9596u128.to_le_bytes()
        );
        assert_eq!(bytes[219..251], [0xA1; 32]);
        assert_eq!(bytes[251..283], [0xA2; 32]);
        assert_eq!(bytes[283..291], 100u64.to_le_bytes());
        assert_eq!(bytes[291..299], 150u64.to_le_bytes());
        assert_eq!(bytes[299..307], 300u64.to_le_bytes());
        assert_eq!(bytes[307], 1);
        assert_eq!(bytes[308..340], [0xB1; 32]);
        assert_eq!(bytes[340], 1);
        assert_eq!(bytes[341..349], 0xC1C2_C3C4_C5C6_C7C8u64.to_le_bytes());
        assert_eq!(bytes[349], 1);
        assert_eq!(bytes[350..382], [0xB2; 32]);
        assert_eq!(bytes[382], 1);
        assert_eq!(bytes[383..391], 0xD1D2_D3D4_D5D6_D7D8u64.to_le_bytes());
        assert_eq!(bytes[391], 1);
        assert_eq!(bytes[392..400], 0xE1E2_E3E4_E5E6_E7E8u64.to_le_bytes());
        assert_eq!(bytes[400], 1);
        assert_eq!(bytes[401..409], 0x0102_0304_0506_0708i64.to_le_bytes());
        assert_eq!(bytes[409], 1);
        assert_eq!(bytes[410..418], (-2i64).to_le_bytes());
    }

    #[test]
    fn status_and_kind_tags_are_stable() {
        let tags: Vec<u8> = [
            IntentStatus::Queued,
            IntentStatus::Applied,
            IntentStatus::Zero,
            IntentStatus::Failed,
            IntentStatus::Stale,
            IntentStatus::Expired,
            IntentStatus::Abandoned,
        ]
        .iter()
        .map(|status| {
            let mut intent = full_intent();
            intent.status = *status;
            serialize(&intent)[137]
        })
        .collect();
        assert_eq!(tags, [0, 1, 2, 3, 4, 5, 6]);

        assert_eq!(OperationKind::Probe.tag(), 0);
        assert_eq!(OperationKind::Shield.tag(), 2);
        assert_eq!(OperationKind::Liquidation.tag(), 9);
        assert_eq!(OperationKind::ViewingKeyRotation.tag(), 10);
    }

    #[test]
    fn discriminator_is_the_anchor_account_hash() {
        let hash = sha256(b"account:OperationIntent");
        assert_eq!(OperationIntent::DISCRIMINATOR, &hash[..8]);
        assert_eq!(
            OperationIntent::DISCRIMINATOR,
            [142, 188, 133, 102, 75, 252, 225, 87]
        );
    }

    #[test]
    fn full_intent_round_trips() {
        let intent = full_intent();
        let bytes = serialize(&intent);
        let decoded = OperationIntent::try_deserialize(&mut bytes.as_slice()).unwrap();
        assert_eq!(decoded, intent);
    }

    #[test]
    fn absent_options_still_fit_the_allocated_account() {
        let intent = OperationIntent {
            position: None,
            position_generation: None,
            second_position: None,
            second_position_generation: None,
            pool_generation: None,
            oracle_publish_time_sol: None,
            oracle_publish_time_usdc: None,
            ..full_intent()
        };
        let bytes = serialize(&intent);
        // `None` is one byte, so the serialized form is shorter than the account.
        assert_eq!(bytes.len(), 8 + 299 + 7);
        assert_eq!(bytes[..307], serialize(&full_intent())[..307]);

        // On chain the account is zero-padded to the full allocation.
        let mut padded = bytes.clone();
        padded.resize(OperationIntent::ACCOUNT_LEN, 0);
        let decoded = OperationIntent::try_deserialize(&mut padded.as_slice()).unwrap();
        assert_eq!(decoded, intent);
    }

    #[test]
    fn default_intent_is_queued_and_zeroed() {
        let bytes = serialize(&OperationIntent::default());
        assert_eq!(bytes[137], 0);
        assert!(bytes[8..307].iter().all(|byte| *byte == 0));
    }

    // ---- Creation checks ---------------------------------------------------

    #[test]
    fn a_well_formed_intent_validates() {
        assert_eq!(full_intent().validate_new(), Ok(()));
    }

    #[test]
    fn validate_new_rejects_a_terminal_status() {
        let mut intent = full_intent();
        intent.status = IntentStatus::Applied;
        assert_eq!(intent.validate_new(), Err(IntentError::NotQueued));
    }

    #[test]
    fn validate_new_enforces_expiry_ordering() {
        let mut intent = full_intent();
        intent.decision_expiry_slot = intent.queued_slot;
        assert_eq!(intent.validate_new(), Err(IntentError::InvalidTiming));

        let mut intent = full_intent();
        intent.lock_expiry_slot = intent.decision_expiry_slot - 1;
        assert_eq!(intent.validate_new(), Err(IntentError::InvalidTiming));

        // Lock expiry equal to Decision Expiry is allowed: the windows are
        // disjoint because abandonment needs a slot strictly after the expiry.
        let mut intent = full_intent();
        intent.lock_expiry_slot = intent.decision_expiry_slot;
        assert_eq!(intent.validate_new(), Ok(()));
    }

    #[test]
    fn validate_new_requires_reclaim_authority_to_be_the_payer() {
        let mut intent = full_intent();
        intent.reclaim_authority = key(0x99);
        assert_eq!(
            intent.validate_new(),
            Err(IntentError::ReclaimAuthorityNotPayer)
        );
    }

    #[test]
    fn validate_new_requires_a_generation_for_every_position() {
        let mut intent = full_intent();
        intent.position_generation = None;
        assert_eq!(intent.validate_new(), Err(IntentError::SnapshotMismatch));

        let mut intent = full_intent();
        intent.second_position_generation = None;
        assert_eq!(intent.validate_new(), Err(IntentError::SnapshotMismatch));

        let mut intent = full_intent();
        intent.position = None;
        intent.position_generation = None;
        assert_eq!(intent.validate_new(), Err(IntentError::SnapshotMismatch));

        let mut intent = full_intent();
        intent.second_position = intent.position;
        assert_eq!(intent.validate_new(), Err(IntentError::SnapshotMismatch));

        let mut intent = full_intent();
        intent.second_position = None;
        intent.second_position_generation = None;
        assert_eq!(intent.validate_new(), Ok(()));
    }

    #[test]
    fn lock_owner_pairs_the_intent_key_and_offset() {
        let intent = full_intent();
        assert_eq!(
            intent.lock_owner(),
            LockOwner::new(key(0x01), 0x1112_1314_1516_1718)
        );
    }

    #[test]
    fn expiry_checks_are_strictly_after() {
        let intent = full_intent();
        assert!(!intent.is_past_decision_expiry(150));
        assert!(intent.is_past_decision_expiry(151));
        assert!(!intent.is_past_lock_expiry(300));
        assert!(intent.is_past_lock_expiry(301));
    }
}
