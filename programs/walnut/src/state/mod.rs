//! Shared on-chain state primitives for the Walnut coordination layer.
//!
//! Task 2.1 added the pure-Rust generation / Lock / Operation Intent state
//! machine. Task 3.1 promotes it to the production foundation:
//!
//! * [`coordination`] holds the single implementation of the Lock and
//!   generation transition rules, the fixed-layout on-chain [`Coordination`]
//!   field group, and a thin adapter over `&mut` account data used by Anchor
//!   instructions. The lifecycle handlers in `operation::lifecycle` call the
//!   same primitives, so there is one source of truth.
//! * [`accounts`] holds the production `Position` and `PoolState` accounts
//!   that carry a `Coordination`.
//! * [`pending_shield`] holds the Task 5.2 Pending Shield account and its
//!   pure `Deposited -> Credited | Refunded` state machine.
//! * [`shield`] holds the Task 5.3 shield, retry, refund, abandonment, and
//!   `shield_credit` callback rules built on it.
//!
//! No financial state (balances, shares, prices, indexes used for value
//! movement) lives here. The transition logic stays plain Rust so it can be
//! unit- and property-tested in CI without a localnet or an MPC round trip.

pub mod accounts;
pub mod band;
pub mod coordination;
pub mod generation;
pub mod nonce;
pub mod pending_shield;
pub mod pool_mutation;
pub mod position;
pub mod protected;
pub mod reserve_withdrawal;
pub mod rotation;
pub mod shield;
pub mod supply;
pub mod unshield;

// The Task 5.6 balance-layer checkpoint: a model-based test that drives the
// real shield, unshield, and rotation rules next to a plaintext reference.
#[cfg(test)]
mod balance_checkpoint;

// The Task 7.7 Pool accounting checkpoint: a model-based test that drives the
// real Pool mutation, reserve withdrawal, and band rules next to a plaintext
// reference model.
#[cfg(test)]
mod pool_checkpoint;

pub use accounts::{
    EncryptedPoolState, EncryptedPositionPayload, PoolState, PoolStatus, Position, PositionStatus,
    POOL_SECRET_FIELDS, POSITION_PAYLOAD_FIELDS,
};
pub use coordination::{Coordination, CoordinationSlot, LockOwner};
pub use generation::Generation;
pub use pending_shield::{PendingShield, PendingShieldStatus, ShieldAsset};
pub use protected::ProtectedAccount;

/// A Solana slot number. Used for Lock expiry and Decision Expiry.
pub type Slot = u64;

/// Errors produced by the generation / Lock / Operation Intent state machine.
///
/// These are plain values (not Anchor `ErrorCode`s) so the pure state-machine
/// logic stays testable on the host without the Solana runtime. Task 3 maps
/// the production versions of these conditions onto program errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoordinationError {
    /// A Lock is already held by a live operation, so it cannot be acquired.
    LockHeld,
    /// Abandonment was attempted while a matching Lock had not yet expired.
    LockNotExpired,
    /// An Intent transition was attempted on an already-terminal Intent.
    IntentNotQueued,
    /// A generation counter would overflow `u64` on increment.
    GenerationOverflow,
    /// The same protected account was supplied more than once to one
    /// multi-account operation (adapter-level guard).
    DuplicateAccount,
}
