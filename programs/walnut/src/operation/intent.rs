//! Operation Intents.
//!
//! An Operation Intent is the pre-created PDA that records one queued
//! computation, its dependency snapshots, account/input commitments, Decision
//! Expiry, and a one-way terminal status. (Requirements 3.1-3.4.)
//!
//! This is the minimal, test-only Intent used by the Task 2.1 feasibility gate.
//! The dependency-snapshot fields are present so the scaffolding is faithful to
//! the design, but no financial logic reads or mutates them here.

use anchor_lang::prelude::Pubkey;

use crate::state::{CoordinationError, Slot};

/// The kind of operation an Intent coordinates.
///
/// No behaviour in this scaffolding branches on the kind; it is stored so the
/// Intent record matches the design shape. Financial semantics arrive in later
/// tasks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
}

/// The one-way terminal status of an Operation Intent.
///
/// Every Intent starts [`IntentStatus::Queued`] and transitions exactly once to
/// one terminal state. (Requirement 3.4.)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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

/// A pre-created Operation Intent record.
///
/// Fields mirror the design's `OperationIntent`. Snapshot fields are captured
/// at queue time; the status is the only mutable lifecycle field here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OperationIntent {
    /// The Intent PDA's own key (referenced by the Locks it owns).
    pub key: Pubkey,
    /// The operation owner.
    pub owner: Pubkey,
    /// The rent/fee payer and reclaim owner.
    pub payer: Pubkey,
    /// The operation kind.
    pub kind: OperationKind,
    /// The queued computation offset.
    pub computation_offset: u64,
    /// Primary Position and its captured generation.
    pub position: Option<Pubkey>,
    pub position_generation: Option<u64>,
    /// Second Position (e.g. liquidation) and its captured generation.
    pub second_position: Option<Pubkey>,
    pub second_position_generation: Option<u64>,
    /// Captured Pool generation, if the Pool is a dependency.
    pub pool_generation: Option<u64>,
    /// Captured Asset State generation.
    pub asset_generation: u64,
    /// Captured oracle generation.
    pub oracle_generation: u64,
    /// Captured rate-epoch generation.
    pub rate_epoch_generation: u64,
    /// Accounting Borrow Index and its timestamp.
    pub accounting_borrow_index: u128,
    pub accounting_index_timestamp: i64,
    /// Risk Borrow Index projected through Decision Expiry.
    pub risk_borrow_index_at_expiry: u128,
    /// Oracle publish times where applicable.
    pub oracle_publish_time_sol: Option<i64>,
    pub oracle_publish_time_usdc: Option<i64>,
    /// Canonical account and input commitments.
    pub accounts_commitment: [u8; 32],
    pub inputs_commitment: [u8; 32],
    /// The slot the computation was queued.
    pub queued_slot: Slot,
    /// The latest slot at which the callback may commit.
    pub decision_expiry_slot: Slot,
    /// The one-way terminal status.
    pub status: IntentStatus,
}

impl OperationIntent {
    /// Returns whether Decision Expiry has passed at `current_slot`.
    pub fn is_past_decision_expiry(&self, current_slot: Slot) -> bool {
        current_slot > self.decision_expiry_slot
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
        assert_eq!(intent.mark_failed(), Err(CoordinationError::IntentNotQueued));
        assert_eq!(intent.status, IntentStatus::Applied);
    }
}
