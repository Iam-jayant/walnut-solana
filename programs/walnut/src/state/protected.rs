//! A protected account: encrypted/custody state guarded by a generation and a
//! Lock.
//!
//! In the production design both `Position` and `PoolState` carry a
//! `generation` and a `lock`. This minimal, test-only stand-in models that
//! shared shape so the Lock + generation lifecycle can be exercised without any
//! financial fields. (Design: "Generations and Locks".)

use anchor_lang::prelude::Pubkey;

use super::Generation;
use crate::lock::Lock;

/// An account whose mutation is coordinated by a generation and a writer Lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtectedAccount {
    /// The account's identifying key.
    pub key: Pubkey,
    /// The current accepted revision.
    pub generation: Generation,
    /// The writer Lock, if any, held over this account.
    pub lock: Lock,
}

impl ProtectedAccount {
    /// Creates an unlocked account at generation zero.
    pub fn new(key: Pubkey) -> Self {
        Self {
            key,
            generation: Generation::default(),
            lock: Lock::unlocked(),
        }
    }

    /// Returns whether this account's Lock is currently held by `intent` /
    /// `computation_offset`.
    pub fn is_locked_by(&self, intent: &Pubkey, computation_offset: u64) -> bool {
        self.lock.matches(intent, computation_offset)
    }
}
