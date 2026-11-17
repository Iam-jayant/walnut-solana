//! Production generation + Lock coordination (Task 3.1).
//!
//! This module is the single implementation of the Lock and generation
//! transition rules. Three layers share it:
//!
//! 1. The pure primitives ([`acquire_all`], [`release_all`],
//!    [`bump_matching_generations`], [`abandon_all`]) work on
//!    `&mut [&mut ProtectedAccount]`. The Task 2.1 lifecycle handlers
//!    (`operation::lifecycle`) call exactly these functions.
//! 2. [`Coordination`] is the fixed-layout on-chain field group (`generation`
//!    plus `lock`) that `Position` and `PoolState` embed.
//! 3. The adapter ([`acquire_locks`], [`release_locks`],
//!    [`apply_and_release_locks`], [`abandon_locks`], [`callback_may_write`])
//!    lets an Anchor instruction run the same primitives over the
//!    `Coordination` fields of loaded accounts. It copies the fields into
//!    working [`ProtectedAccount`]s, runs a primitive, and writes the fields
//!    back only when the primitive succeeded, so a failure leaves every account
//!    exactly as it was.
//!
//! Safety model (design: "Generations and Locks"): Lock expiry permits
//! abandonment, not an assertion that the old computation stopped. The
//! invariant is that only one generation is current and only a callback bound
//! to that generation (and still holding the Lock) may commit.
//!
//! Operation Intent status (`Queued -> terminal`) is not stored here. The
//! caller owns the status change and must apply it in the same instruction as
//! the Lock change: after [`abandon_locks`] succeeds the instruction marks the
//! Intent `Abandoned`, and a later callback sees a terminal Intent and a
//! changed generation, so it cannot write.

use anchor_lang::prelude::{AnchorDeserialize, AnchorSerialize, InitSpace, Pubkey};

use super::{CoordinationError, Generation, ProtectedAccount, Slot};
use crate::lock::Lock;

/// Identifies the operation that owns a Lock: its Operation Intent PDA and
/// queued computation offset. A Lock is released or abandoned only by an exact
/// match on both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockOwner {
    /// The Operation Intent PDA.
    pub operation_intent: Pubkey,
    /// The queued computation offset.
    pub computation_offset: u64,
}

impl LockOwner {
    /// Creates an owner identity.
    pub const fn new(operation_intent: Pubkey, computation_offset: u64) -> Self {
        Self {
            operation_intent,
            computation_offset,
        }
    }

    /// Returns whether `lock` is active and held by exactly this owner.
    pub fn holds(&self, lock: &Lock) -> bool {
        lock.matches(&self.operation_intent, self.computation_offset)
    }
}

/// The on-chain generation and Lock of one protected account.
///
/// Embedded in `Position` and `PoolState`. Borsh lays the two fields out
/// back to back, so embedding a `Coordination` is byte-identical to declaring
/// `generation: u64` and `lock: Lock` as adjacent fields.
///
/// Serialized layout (57 bytes): `generation` (8, little-endian) | `lock`
/// (49, see [`Lock`]).
#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub struct Coordination {
    /// The current accepted revision.
    pub generation: u64,
    /// The writer Lock over the account.
    pub lock: Lock,
}

impl Coordination {
    /// Serialized size in bytes (generation 8 + Lock 49).
    pub const LEN: usize = 8 + 49;

    /// Returns whether a callback that captured `captured_generation` and is
    /// owned by `owner` may still commit against this account: the Lock must be
    /// held by `owner` and the generation must be unchanged.
    pub fn is_current_writer(&self, owner: &LockOwner, captured_generation: u64) -> bool {
        owner.holds(&self.lock) && self.generation == captured_generation
    }
}

impl From<&ProtectedAccount> for Coordination {
    fn from(account: &ProtectedAccount) -> Self {
        Self {
            generation: account.generation.value(),
            lock: account.lock,
        }
    }
}

// ---- Pure primitives -------------------------------------------------------

/// Atomically acquires the writer Lock on every account for `owner`.
///
/// All-or-nothing: if any account is already locked, no Lock is acquired and
/// [`CoordinationError::LockHeld`] is returned. `lock_expiry_slot` is the slot
/// after which the Locks become abandonable.
pub fn acquire_all(
    owner: LockOwner,
    accounts: &mut [&mut ProtectedAccount],
    lock_expiry_slot: Slot,
) -> Result<(), CoordinationError> {
    if accounts.iter().any(|account| account.lock.is_active()) {
        return Err(CoordinationError::LockHeld);
    }
    for account in accounts.iter_mut() {
        account.lock.acquire(
            owner.operation_intent,
            owner.computation_offset,
            lock_expiry_slot,
        )?;
    }
    Ok(())
}

/// Releases every Lock held by exactly `owner`; leaves all other Locks alone.
///
/// Returns the number of Locks released. A non-matching owner (wrong Intent,
/// wrong offset, or an unlocked account) releases nothing and returns `0`; it
/// is never an error.
pub fn release_all(owner: LockOwner, accounts: &mut [&mut ProtectedAccount]) -> usize {
    let mut released = 0;
    for account in accounts.iter_mut() {
        if account
            .lock
            .release_matching(&owner.operation_intent, owner.computation_offset)
        {
            released += 1;
        }
    }
    released
}

/// Bumps the generation of every account whose Lock is held by `owner`.
///
/// Fully atomic: it first checks that no matching generation is saturated, so
/// [`CoordinationError::GenerationOverflow`] leaves every generation unchanged.
/// Returns the number of generations bumped.
pub fn bump_matching_generations(
    owner: LockOwner,
    accounts: &mut [&mut ProtectedAccount],
) -> Result<usize, CoordinationError> {
    if accounts
        .iter()
        .any(|account| owner.holds(&account.lock) && account.generation.value() == u64::MAX)
    {
        return Err(CoordinationError::GenerationOverflow);
    }
    let mut bumped = 0;
    for account in accounts.iter_mut() {
        if owner.holds(&account.lock) {
            account.generation.bump()?;
            bumped += 1;
        }
    }
    Ok(bumped)
}

/// Abandons the Locks held by `owner` once every one of them has expired.
///
/// Requirement 3.8: it increments every still-matching generation *before*
/// clearing the Locks, so no later writer can commit against a generation an
/// old (possibly still executing) computation captured. Atomic: on
/// [`CoordinationError::LockNotExpired`] or
/// [`CoordinationError::GenerationOverflow`] nothing changes. Returns the number
/// of Locks cleared (equal to the number of generations bumped).
pub fn abandon_all(
    owner: LockOwner,
    accounts: &mut [&mut ProtectedAccount],
    current_slot: Slot,
) -> Result<usize, CoordinationError> {
    if accounts
        .iter()
        .any(|account| owner.holds(&account.lock) && !account.lock.is_expired(current_slot))
    {
        return Err(CoordinationError::LockNotExpired);
    }
    let bumped = bump_matching_generations(owner, accounts)?;
    release_all(owner, accounts);
    Ok(bumped)
}

// ---- Anchor adapter --------------------------------------------------------

/// One protected account as seen by an Anchor instruction: its key and a
/// mutable borrow of its [`Coordination`] fields.
///
/// Build it from loaded accounts, for example
/// `CoordinationSlot::new(position.key(), &mut position.coordination)`.
pub struct CoordinationSlot<'a> {
    /// The account's key. Used to reject the same account being passed twice.
    pub key: Pubkey,
    /// The account's generation and Lock.
    pub state: &'a mut Coordination,
}

impl<'a> CoordinationSlot<'a> {
    /// Creates a slot over `state`.
    pub fn new(key: Pubkey, state: &'a mut Coordination) -> Self {
        Self { key, state }
    }
}

/// Runs a pure primitive over working copies of `slots` and commits the result
/// back only on success.
///
/// Rejects duplicate keys first: two deserialized copies of one account would
/// otherwise let the later write-back silently overwrite the earlier one.
///
/// Generic over the error type so the callback helpers (Task 3.3) can reuse the
/// same copy-in, write-back-on-success logic with their own error enum.
pub(crate) fn with_protected<R, E: From<CoordinationError>>(
    slots: &mut [CoordinationSlot<'_>],
    operation: impl FnOnce(&mut [&mut ProtectedAccount]) -> Result<R, E>,
) -> Result<R, E> {
    for (index, slot) in slots.iter().enumerate() {
        if slots
            .iter()
            .skip(index + 1)
            .any(|other| other.key == slot.key)
        {
            return Err(CoordinationError::DuplicateAccount.into());
        }
    }

    let mut working: Vec<ProtectedAccount> = slots
        .iter()
        .map(|slot| ProtectedAccount {
            key: slot.key,
            generation: Generation::new(slot.state.generation),
            lock: slot.state.lock,
        })
        .collect();

    let result = {
        let mut refs: Vec<&mut ProtectedAccount> = working.iter_mut().collect();
        operation(&mut refs)?
    };

    for (slot, account) in slots.iter_mut().zip(working.iter()) {
        slot.state.generation = account.generation.value();
        slot.state.lock = account.lock;
    }
    Ok(result)
}

/// Queue path: atomically acquires the Lock on every slot for `owner`.
///
/// All or none. Fails with [`CoordinationError::LockHeld`] if any slot is
/// already locked (no slot is changed) or
/// [`CoordinationError::DuplicateAccount`] if a key repeats.
pub fn acquire_locks(
    owner: LockOwner,
    lock_expiry_slot: Slot,
    slots: &mut [CoordinationSlot<'_>],
) -> Result<(), CoordinationError> {
    with_protected(slots, |accounts| {
        acquire_all(owner, accounts, lock_expiry_slot)
    })
}

/// Callback release for outcomes that write nothing (`Zero`, `Stale`,
/// `Expired`, `Failed`).
///
/// Releases only the Locks held by exactly `owner` and bumps no generation.
/// Returns `Ok(n)` with the number of Locks released. A non-matching owner is a
/// no-op that returns `Ok(0)`.
pub fn release_locks(
    owner: LockOwner,
    slots: &mut [CoordinationSlot<'_>],
) -> Result<usize, CoordinationError> {
    with_protected(slots, |accounts| Ok(release_all(owner, accounts)))
}

/// Callback release for the `Applied` outcome.
///
/// Bumps the generation of every slot whose Lock is held by `owner`, then
/// releases those Locks. Returns `Ok(n)` with the number of generations bumped
/// (equal to the Locks released). Atomic on generation overflow. A non-matching
/// owner changes nothing and returns `Ok(0)`.
pub fn apply_and_release_locks(
    owner: LockOwner,
    slots: &mut [CoordinationSlot<'_>],
) -> Result<usize, CoordinationError> {
    with_protected(slots, |accounts| {
        let bumped = bump_matching_generations(owner, accounts)?;
        release_all(owner, accounts);
        Ok(bumped)
    })
}

/// Permissionless abandonment of an expired operation's Locks.
///
/// Every Lock held by `owner` must be past its expiry slot. The matching
/// generations are bumped first and the Locks cleared second. Returns `Ok(n)`
/// with the number of Locks cleared. Errors with
/// [`CoordinationError::LockNotExpired`] (nothing changes) while any matching
/// Lock is unexpired. The caller must have verified the Intent is still
/// `Queued` and must mark it `Abandoned` in the same instruction.
pub fn abandon_locks(
    owner: LockOwner,
    current_slot: Slot,
    slots: &mut [CoordinationSlot<'_>],
) -> Result<usize, CoordinationError> {
    with_protected(slots, |accounts| abandon_all(owner, accounts, current_slot))
}

/// Returns whether a callback owned by `owner` may still write.
///
/// Each entry pairs an account's [`Coordination`] with the generation the
/// Intent captured for it. True only when the list is non-empty and every
/// account still holds the Lock for `owner` at the captured generation. After
/// abandonment the Locks are cleared and the generations moved on, so a late
/// callback always gets `false`.
pub fn callback_may_write(owner: LockOwner, accounts: &[(&Coordination, u64)]) -> bool {
    !accounts.is_empty()
        && accounts
            .iter()
            .all(|(state, captured)| state.is_current_writer(&owner, *captured))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anchor_lang::Space;

    fn key(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    fn owner(n: u8, offset: u64) -> LockOwner {
        LockOwner::new(key(n), offset)
    }

    fn locked(owner: LockOwner, expiry: Slot) -> Coordination {
        let mut state = Coordination::default();
        state
            .lock
            .acquire(owner.operation_intent, owner.computation_offset, expiry)
            .unwrap();
        state
    }

    fn serialize<T: AnchorSerialize>(value: &T) -> Vec<u8> {
        let mut bytes = Vec::new();
        value.serialize(&mut bytes).unwrap();
        bytes
    }

    // ---- Layout ------------------------------------------------------------

    #[test]
    fn lock_layout_is_49_bytes() {
        assert_eq!(Lock::INIT_SPACE, 1 + 8 + 32 + 8);
        assert_eq!(Lock::INIT_SPACE, 49);

        let lock = Lock {
            active: true,
            computation_offset: 0x0102_0304_0506_0708,
            operation_intent: key(0xAB),
            expiry_slot: 0x1112_1314_1516_1718,
        };
        let bytes = serialize(&lock);
        assert_eq!(bytes.len(), 49);
        assert_eq!(bytes[0], 1);
        assert_eq!(bytes[1..9], 0x0102_0304_0506_0708u64.to_le_bytes());
        assert_eq!(bytes[9..41], [0xAB; 32]);
        assert_eq!(bytes[41..49], 0x1112_1314_1516_1718u64.to_le_bytes());
    }

    #[test]
    fn unlocked_lock_serializes_to_zeroes() {
        assert_eq!(serialize(&Lock::unlocked()), [0u8; 49]);
        assert_eq!(Lock::unlocked(), Lock::default());
    }

    #[test]
    fn coordination_layout_is_generation_then_lock() {
        assert_eq!(Coordination::INIT_SPACE, 8 + 49);
        assert_eq!(Coordination::INIT_SPACE, 57);
        assert_eq!(Coordination::LEN, Coordination::INIT_SPACE);

        let state = Coordination {
            generation: 0x2122_2324_2526_2728,
            lock: Lock {
                active: true,
                computation_offset: 9,
                operation_intent: key(7),
                expiry_slot: 11,
            },
        };
        let bytes = serialize(&state);
        assert_eq!(bytes.len(), Coordination::LEN);
        assert_eq!(bytes[0..8], 0x2122_2324_2526_2728u64.to_le_bytes());
        assert_eq!(bytes[8..], serialize(&state.lock)[..]);
    }

    #[test]
    fn coordination_round_trips() {
        let state = locked(owner(3, 44), 500);
        let bytes = serialize(&state);
        assert_eq!(Coordination::try_from_slice(&bytes).unwrap(), state);
    }

    // ---- Adapter -----------------------------------------------------------

    #[test]
    fn acquire_locks_is_all_or_none() {
        let mut a = Coordination::default();
        let mut b = locked(owner(9, 1), 10);
        let b_before = b;

        let result = acquire_locks(
            owner(1, 7),
            100,
            &mut [
                CoordinationSlot::new(key(1), &mut a),
                CoordinationSlot::new(key(2), &mut b),
            ],
        );
        assert_eq!(result, Err(CoordinationError::LockHeld));
        assert_eq!(a, Coordination::default());
        assert_eq!(b, b_before);
    }

    #[test]
    fn acquire_locks_sets_every_lock() {
        let mut a = Coordination::default();
        let mut b = Coordination::default();
        acquire_locks(
            owner(1, 7),
            100,
            &mut [
                CoordinationSlot::new(key(1), &mut a),
                CoordinationSlot::new(key(2), &mut b),
            ],
        )
        .unwrap();
        for state in [a, b] {
            assert!(owner(1, 7).holds(&state.lock));
            assert_eq!(state.lock.expiry_slot, 100);
            assert_eq!(state.generation, 0);
        }
    }

    #[test]
    fn duplicate_account_is_rejected_without_change() {
        let mut a = Coordination::default();
        let mut a_copy = Coordination::default();
        let result = acquire_locks(
            owner(1, 7),
            100,
            &mut [
                CoordinationSlot::new(key(1), &mut a),
                CoordinationSlot::new(key(1), &mut a_copy),
            ],
        );
        assert_eq!(result, Err(CoordinationError::DuplicateAccount));
        assert_eq!(a, Coordination::default());
        assert_eq!(a_copy, Coordination::default());
    }

    #[test]
    fn release_returns_count_and_ignores_wrong_owner() {
        let mut a = locked(owner(1, 7), 100);
        let mut b = locked(owner(2, 8), 100);
        let before = (a, b);

        // Wrong intent, wrong offset, and an unrelated owner release nothing.
        for wrong in [owner(2, 7), owner(1, 8), owner(5, 5)] {
            let released = release_locks(
                wrong,
                &mut [
                    CoordinationSlot::new(key(1), &mut a),
                    CoordinationSlot::new(key(2), &mut b),
                ],
            );
            assert_eq!(released, Ok(0));
            assert_eq!((a, b), before);
        }

        // The exact owner releases only its own Lock.
        let released = release_locks(
            owner(1, 7),
            &mut [
                CoordinationSlot::new(key(1), &mut a),
                CoordinationSlot::new(key(2), &mut b),
            ],
        );
        assert_eq!(released, Ok(1));
        assert!(!a.lock.is_active());
        assert!(b.lock.is_active());
        assert_eq!(a.generation, 0);
    }

    #[test]
    fn apply_and_release_bumps_then_clears() {
        let mut a = locked(owner(1, 7), 100);
        let mut b = locked(owner(1, 7), 100);
        a.generation = 4;
        b.generation = 9;

        let bumped = apply_and_release_locks(
            owner(1, 7),
            &mut [
                CoordinationSlot::new(key(1), &mut a),
                CoordinationSlot::new(key(2), &mut b),
            ],
        );
        assert_eq!(bumped, Ok(2));
        assert_eq!((a.generation, b.generation), (5, 10));
        assert!(!a.lock.is_active());
        assert!(!b.lock.is_active());
    }

    #[test]
    fn apply_and_release_is_atomic_on_overflow() {
        let mut a = locked(owner(1, 7), 100);
        let mut b = locked(owner(1, 7), 100);
        a.generation = 5;
        b.generation = u64::MAX;
        let before = (a, b);

        let result = apply_and_release_locks(
            owner(1, 7),
            &mut [
                CoordinationSlot::new(key(1), &mut a),
                CoordinationSlot::new(key(2), &mut b),
            ],
        );
        assert_eq!(result, Err(CoordinationError::GenerationOverflow));
        assert_eq!((a, b), before);
    }

    #[test]
    fn abandon_requires_expiry_and_bumps_before_clearing() {
        let mut a = locked(owner(1, 7), 100);
        let mut b = locked(owner(1, 7), 150);
        let before = (a, b);

        // At slot 120 only `a` is expired, so nothing may change.
        let early = abandon_locks(
            owner(1, 7),
            120,
            &mut [
                CoordinationSlot::new(key(1), &mut a),
                CoordinationSlot::new(key(2), &mut b),
            ],
        );
        assert_eq!(early, Err(CoordinationError::LockNotExpired));
        assert_eq!((a, b), before);

        let done = abandon_locks(
            owner(1, 7),
            151,
            &mut [
                CoordinationSlot::new(key(1), &mut a),
                CoordinationSlot::new(key(2), &mut b),
            ],
        );
        assert_eq!(done, Ok(2));
        assert_eq!((a.generation, b.generation), (1, 1));
        assert!(!a.lock.is_active());
        assert!(!b.lock.is_active());
    }

    #[test]
    fn abandon_is_atomic_on_overflow() {
        let mut a = locked(owner(1, 7), 100);
        let mut b = locked(owner(1, 7), 100);
        b.generation = u64::MAX;
        let before = (a, b);

        let result = abandon_locks(
            owner(1, 7),
            101,
            &mut [
                CoordinationSlot::new(key(1), &mut a),
                CoordinationSlot::new(key(2), &mut b),
            ],
        );
        assert_eq!(result, Err(CoordinationError::GenerationOverflow));
        assert_eq!((a, b), before);
    }

    #[test]
    fn abandon_leaves_other_owners_untouched() {
        let mut a = locked(owner(1, 7), 100);
        let mut b = locked(owner(2, 8), 100);
        let b_before = b;

        let done = abandon_locks(
            owner(1, 7),
            101,
            &mut [
                CoordinationSlot::new(key(1), &mut a),
                CoordinationSlot::new(key(2), &mut b),
            ],
        );
        assert_eq!(done, Ok(1));
        assert_eq!(a.generation, 1);
        assert_eq!(b, b_before);
    }

    #[test]
    fn late_callback_cannot_write_after_abandon() {
        let mut a = locked(owner(1, 7), 100);
        let captured = a.generation;
        assert!(callback_may_write(owner(1, 7), &[(&a, captured)]));

        abandon_locks(
            owner(1, 7),
            101,
            &mut [CoordinationSlot::new(key(1), &mut a)],
        )
        .unwrap();
        assert!(!callback_may_write(owner(1, 7), &[(&a, captured)]));

        // Even if another operation takes the Lock, the old owner is rejected.
        a.lock.acquire(key(2), 8, 500).unwrap();
        assert!(!callback_may_write(owner(1, 7), &[(&a, captured)]));
        assert!(!callback_may_write(owner(1, 7), &[]));
    }
}
