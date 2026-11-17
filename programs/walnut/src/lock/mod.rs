//! Writer Locks.
//!
//! A Lock records that one operation holds write authority over an account
//! until a Lock-expiry slot. The safety invariant is NOT "only one computation
//! is executing" (Walnut cannot prove that after expiry); it is "only one
//! generation is current, and only a callback bound to that generation may
//! commit." Lock expiry therefore permits *abandonment*, it does not assert
//! that an old computation has stopped. (Design: "Generations and Locks";
//! Requirements 3.7, 3.8.)
//!
//! The same type is the production on-chain Lock (Task 3.1): it is Borsh
//! serializable with a fixed 49-byte layout and is embedded in `Position` and
//! `PoolState` through [`Coordination`](crate::state::Coordination). It carries
//! no financial state.

use anchor_lang::prelude::{AnchorDeserialize, AnchorSerialize, InitSpace, Pubkey};

use crate::state::{CoordinationError, Slot};

/// A writer Lock over one protected account.
///
/// Serialized layout (49 bytes, no padding, little-endian integers):
/// `active` (1) | `computation_offset` (8) | `operation_intent` (32) |
/// `expiry_slot` (8).
#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub struct Lock {
    /// Whether the Lock is currently held.
    pub active: bool,
    /// The queued computation that owns the Lock.
    pub computation_offset: u64,
    /// The Operation Intent PDA that owns the Lock.
    pub operation_intent: Pubkey,
    /// The slot after which the Lock may be abandoned. Written as `u64` (the
    /// type behind [`Slot`]) so the `InitSpace` derive sees a primitive.
    pub expiry_slot: u64,
}

impl Lock {
    /// Returns a released (inactive) Lock.
    pub fn unlocked() -> Self {
        Self {
            active: false,
            computation_offset: 0,
            operation_intent: Pubkey::default(),
            expiry_slot: 0,
        }
    }

    /// Returns whether the Lock is currently held.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Returns whether the Lock is held but past its expiry slot, i.e. eligible
    /// for permissionless abandonment.
    pub fn is_expired(&self, current_slot: Slot) -> bool {
        self.active && current_slot > self.expiry_slot
    }

    /// Returns whether the Lock is held by exactly `intent` / `computation_offset`.
    pub fn matches(&self, intent: &Pubkey, computation_offset: u64) -> bool {
        self.active
            && self.operation_intent == *intent
            && self.computation_offset == computation_offset
    }

    /// Acquires the Lock for `intent` / `computation_offset` until `expiry_slot`.
    ///
    /// Fails with [`CoordinationError::LockHeld`] if the Lock is already active,
    /// so acquisition is never silent overwrite.
    pub fn acquire(
        &mut self,
        intent: Pubkey,
        computation_offset: u64,
        expiry_slot: Slot,
    ) -> Result<(), CoordinationError> {
        if self.active {
            return Err(CoordinationError::LockHeld);
        }
        self.active = true;
        self.computation_offset = computation_offset;
        self.operation_intent = intent;
        self.expiry_slot = expiry_slot;
        Ok(())
    }

    /// Releases the Lock only if it is held by exactly `intent` /
    /// `computation_offset`. Returns whether a release occurred.
    ///
    /// A non-matching call is a no-op: a callback or abandonment clears only
    /// the Locks it owns, never another operation's Lock.
    pub fn release_matching(&mut self, intent: &Pubkey, computation_offset: u64) -> bool {
        if self.matches(intent, computation_offset) {
            *self = Self::unlocked();
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    #[test]
    fn acquire_then_double_acquire_fails() {
        let mut lock = Lock::unlocked();
        lock.acquire(key(1), 7, 100).unwrap();
        assert!(lock.is_active());
        assert_eq!(
            lock.acquire(key(2), 8, 200),
            Err(CoordinationError::LockHeld)
        );
        // The original owner is untouched.
        assert!(lock.matches(&key(1), 7));
    }

    #[test]
    fn release_requires_exact_match() {
        let mut lock = Lock::unlocked();
        lock.acquire(key(1), 7, 100).unwrap();

        assert!(!lock.release_matching(&key(2), 7)); // wrong intent
        assert!(!lock.release_matching(&key(1), 8)); // wrong offset
        assert!(lock.is_active());

        assert!(lock.release_matching(&key(1), 7));
        assert!(!lock.is_active());
    }

    #[test]
    fn expiry_is_strictly_after_expiry_slot() {
        let mut lock = Lock::unlocked();
        lock.acquire(key(1), 7, 100).unwrap();
        assert!(!lock.is_expired(100));
        assert!(lock.is_expired(101));

        // An inactive Lock is never "expired".
        let idle = Lock::unlocked();
        assert!(!idle.is_expired(u64::MAX));
    }

    /// **Validates: Requirements 3.7** — a Lock release touches only the exact
    /// owner; any mismatch in intent or offset leaves the Lock held.
    #[test]
    fn prop_release_is_exact_owner_only() {
        let mut seed = 0x5eed_1234u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            seed
        };
        for _ in 0..5_000 {
            let owner = (next() % 4) as u8;
            let offset = next() % 4;
            let mut lock = Lock::unlocked();
            lock.acquire(key(owner), offset, 10).unwrap();

            let caller = (next() % 4) as u8;
            let caller_offset = next() % 4;
            let released = lock.release_matching(&key(caller), caller_offset);
            let should_release = caller == owner && caller_offset == offset;
            assert_eq!(released, should_release);
            assert_eq!(lock.is_active(), !should_release);
        }
    }
}
