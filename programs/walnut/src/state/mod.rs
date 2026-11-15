//! Shared on-chain state primitives for the Walnut coordination layer.
//!
//! Task 2.1 (Arcium feasibility gate) adds only the minimal, test-only state
//! machine needed to exercise the generation / Lock / Operation Intent
//! lifecycle. No financial state (balances, shares, prices, indexes used for
//! value movement) lives here: this module only proves the coordination
//! scaffolding required before the feasibility measurements in Tasks 2.2-2.7.
//!
//! The types are deliberately plain Rust (no Anchor account machinery) so the
//! state transitions can be unit- and property-tested in CI without a localnet
//! or an MPC round trip.

pub mod generation;
pub mod protected;

pub use generation::Generation;
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
}
