//! Operation Intent lifecycle handlers.
//!
//! This models the four terminal outcomes the Task 2.1 feasibility gate must
//! exercise, plus the queue path that acquires Locks:
//!
//! * success  -> `Applied` (nonzero) or `Zero` (approved zero), else `Stale`
//!   (dependency/commitment changed) or `Expired` (past Decision
//!   Expiry);
//! * static failure -> `Failed`;
//! * expiry   -> `Expired` (authenticated callback after Decision Expiry);
//! * abandonment -> `Abandoned` (permissionless timeout after Lock expiry).
//!
//! Generation bumps follow the design rule "a generation changes whenever
//! accepted state changes or an old operation is abandoned":
//!
//! | outcome   | written generations | matching Locks |
//! |-----------|---------------------|----------------|
//! | Applied   | bumped              | released       |
//! | Zero      | unchanged           | released       |
//! | Stale     | unchanged           | released       |
//! | Expired   | unchanged           | released       |
//! | Failed    | unchanged           | released       |
//! | Abandoned | bumped              | released       |
//!
//! A failing callback transaction (`LeftQueuedForRecovery`) writes nothing and
//! releases no Lock: the Intent stays `Queued`.
//!
//! A callback for an already-terminal Intent (duplicate, late, or
//! already-resolved) is a no-op that changes no financial state
//! ([`CallbackOutcome::TerminalNoop`]).
//!
//! A callback transaction that *fails to execute* -- invalid or unauthenticated
//! signed output, a missing or wrong custom callback account, a failed callback
//! CPI (for example a token transfer), or callback compute-unit exhaustion --
//! commits nothing. On Solana the whole callback transaction reverts
//! atomically, so no status change, generation bump, Lock release, or financial
//! write survives. The Intent stays `Queued` and only timeout recovery
//! (abandonment, once the Lock expires) resolves it (Requirement 3.6).
//! [`on_callback_tx_failure`] models this shared outcome.
//!
//! All of this is pure Rust with no financial fields, so it is fully
//! CI-testable without a localnet or MPC round trip.

use crate::operation::intent::OperationIntent;
use crate::state::coordination::{
    abandon_all, acquire_all, bump_matching_generations, release_all, LockOwner,
};
use crate::state::{CoordinationError, ProtectedAccount, Slot};

/// The resolved outcome of a lifecycle handler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackOutcome {
    /// Authenticated success with a nonzero transition; generations bumped.
    Applied,
    /// Authenticated success approving no value; no generation change.
    Zero,
    /// Authenticated static failure; no financial change.
    Failed,
    /// Authenticated success whose dependency/commitment changed; no change.
    Stale,
    /// Authenticated success after Decision Expiry; no change.
    Expired,
    /// Permissionless timeout recovery after Lock expiry; generations bumped.
    Abandoned,
    /// Callback for an already-terminal Intent; nothing happened.
    TerminalNoop,
    /// The callback transaction failed to execute (invalid/unauthenticated
    /// output, missing or wrong custom account, failed callback CPI, or
    /// compute-unit exhaustion). Nothing was committed; the Intent stays
    /// `Queued` for timeout recovery.
    LeftQueuedForRecovery,
}

/// The Lock owner identity of `intent`: its PDA key and computation offset.
fn owner_of(intent: &OperationIntent) -> LockOwner {
    LockOwner::new(intent.key, intent.computation_offset)
}

/// Releases every Lock owned by `intent`; leaves all other Locks untouched.
///
/// The Lock and generation rules live in `state::coordination`, which the
/// production Anchor adapter shares; this handler only supplies the owner.
fn release_matching_locks(intent: &OperationIntent, accounts: &mut [&mut ProtectedAccount]) {
    release_all(owner_of(intent), accounts);
}

/// Queue path: atomically acquire the writer Locks for every affected account.
///
/// All-or-nothing: if any account is already locked, no Lock is acquired and
/// [`CoordinationError::LockHeld`] is returned. `lock_expiry_slot` is the slot
/// after which the Locks become abandonable; it is distinct from the Intent's
/// Decision Expiry.
pub fn queue(
    intent: &OperationIntent,
    accounts: &mut [&mut ProtectedAccount],
    lock_expiry_slot: Slot,
) -> Result<(), CoordinationError> {
    acquire_all(owner_of(intent), accounts, lock_expiry_slot)
}

/// Success callback.
///
/// Resolves to `Stale`, `Expired`, `Zero`, or `Applied` per the table in the
/// module docs. Dependency/commitment mismatch is checked before expiry, per
/// the design's success-callback ordering. A terminal Intent yields
/// [`CallbackOutcome::TerminalNoop`] with no state change.
pub fn on_success(
    intent: &mut OperationIntent,
    accounts: &mut [&mut ProtectedAccount],
    current_slot: Slot,
    dependencies_match: bool,
    nonzero: bool,
) -> Result<CallbackOutcome, CoordinationError> {
    if intent.status.is_terminal() {
        return Ok(CallbackOutcome::TerminalNoop);
    }

    if !dependencies_match {
        intent.mark_stale()?;
        release_matching_locks(intent, accounts);
        return Ok(CallbackOutcome::Stale);
    }

    if intent.is_past_decision_expiry(current_slot) {
        intent.mark_expired()?;
        release_matching_locks(intent, accounts);
        return Ok(CallbackOutcome::Expired);
    }

    if nonzero {
        // Applied is the only success outcome that changes accepted state, so
        // it is the only success outcome that bumps generations. Bump first so
        // an overflow aborts before the status becomes terminal.
        bump_matching_generations(owner_of(intent), accounts)?;
        intent.mark_applied()?;
        release_matching_locks(intent, accounts);
        Ok(CallbackOutcome::Applied)
    } else {
        intent.mark_zero()?;
        release_matching_locks(intent, accounts);
        Ok(CallbackOutcome::Zero)
    }
}

/// Static-failure callback.
///
/// An authenticated static failure marks the Intent `Failed` and clears only
/// its Locks. It changes no financial state and bumps no generation. A terminal
/// Intent yields [`CallbackOutcome::TerminalNoop`].
pub fn on_static_failure(
    intent: &mut OperationIntent,
    accounts: &mut [&mut ProtectedAccount],
) -> Result<CallbackOutcome, CoordinationError> {
    if intent.status.is_terminal() {
        return Ok(CallbackOutcome::TerminalNoop);
    }
    intent.mark_failed()?;
    release_matching_locks(intent, accounts);
    Ok(CallbackOutcome::Failed)
}

/// A callback transaction that fails to execute.
///
/// Covers every failing callback transaction that is NOT an authenticated
/// static failure: invalid or unauthenticated signed output, a missing or wrong
/// custom callback account, a failed callback CPI (such as a token transfer),
/// and callback compute-unit exhaustion. On Solana such a transaction reverts
/// atomically, so this is a read-only no-op: the Intent stays `Queued` and every
/// Lock it owns stays held. Only timeout recovery (abandonment after Lock
/// expiry) can then resolve the Intent (Requirement 3.6).
///
/// It takes shared references precisely because it must not mutate anything; the
/// signature documents that a failed callback leaves all coordination state
/// exactly as the queue path left it.
pub fn on_callback_tx_failure(
    intent: &OperationIntent,
    accounts: &[&ProtectedAccount],
) -> CallbackOutcome {
    let _ = (intent, accounts);
    CallbackOutcome::LeftQueuedForRecovery
}

/// Permissionless abandonment after Lock expiry.
///
/// Requires the Intent to still be queued and every Lock it owns to be expired.
/// It increments every still-matching generation *before* clearing the Locks,
/// so no later writer can commit against a generation an old (possibly still
/// executing) computation captured (Requirement 3.8). Funds are untouched.
pub fn abandon(
    intent: &mut OperationIntent,
    accounts: &mut [&mut ProtectedAccount],
    current_slot: Slot,
) -> Result<CallbackOutcome, CoordinationError> {
    if intent.status.is_terminal() {
        return Err(CoordinationError::IntentNotQueued);
    }
    // Every Lock this Intent still owns must be past its expiry slot. The shared
    // primitive checks that, bumps generations first (atomic on overflow), then
    // clears the Locks. Only then does the Intent become terminal.
    abandon_all(owner_of(intent), accounts, current_slot)?;
    intent.mark_abandoned()?;
    Ok(CallbackOutcome::Abandoned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::intent::IntentStatus;
    use anchor_lang::prelude::Pubkey;

    fn key(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    /// Builds a queued Intent plus two protected accounts (e.g. Position +
    /// Pool), with the Locks acquired through the queue path.
    fn queued_scenario(
        decision_expiry: Slot,
        lock_expiry: Slot,
    ) -> (OperationIntent, ProtectedAccount, ProtectedAccount) {
        let intent = OperationIntent {
            key: key(42),
            computation_offset: 7,
            decision_expiry_slot: decision_expiry,
            ..OperationIntent::default()
        };
        let mut a = ProtectedAccount::new(key(1));
        let mut b = ProtectedAccount::new(key(2));
        queue(&intent, &mut [&mut a, &mut b], lock_expiry).unwrap();
        (intent, a, b)
    }

    #[test]
    fn queue_is_atomic_when_one_account_is_locked() {
        let intent = OperationIntent {
            key: key(42),
            computation_offset: 7,
            ..OperationIntent::default()
        };
        let mut a = ProtectedAccount::new(key(1));
        let mut b = ProtectedAccount::new(key(2));
        // Pre-lock b with a different operation.
        b.lock.acquire(key(99), 1, 10).unwrap();

        let err = queue(&intent, &mut [&mut a, &mut b], 10);
        assert_eq!(err, Err(CoordinationError::LockHeld));
        // a must not have been locked by the failed atomic acquisition.
        assert!(!a.lock.is_active());
    }

    #[test]
    fn success_applied_bumps_generations_and_releases_locks() {
        let (mut intent, mut a, mut b) = queued_scenario(100, 200);
        let out = on_success(&mut intent, &mut [&mut a, &mut b], 50, true, true).unwrap();
        assert_eq!(out, CallbackOutcome::Applied);
        assert_eq!(intent.status, IntentStatus::Applied);
        assert_eq!(a.generation.value(), 1);
        assert_eq!(b.generation.value(), 1);
        assert!(!a.lock.is_active());
        assert!(!b.lock.is_active());
    }

    #[test]
    fn success_zero_releases_without_generation_change() {
        let (mut intent, mut a, mut b) = queued_scenario(100, 200);
        let out = on_success(&mut intent, &mut [&mut a, &mut b], 50, true, false).unwrap();
        assert_eq!(out, CallbackOutcome::Zero);
        assert_eq!(intent.status, IntentStatus::Zero);
        assert_eq!(a.generation.value(), 0);
        assert_eq!(b.generation.value(), 0);
        assert!(!a.lock.is_active());
        assert!(!b.lock.is_active());
    }

    #[test]
    fn success_with_changed_dependency_is_stale() {
        let (mut intent, mut a, mut b) = queued_scenario(100, 200);
        let out = on_success(&mut intent, &mut [&mut a, &mut b], 50, false, true).unwrap();
        assert_eq!(out, CallbackOutcome::Stale);
        assert_eq!(intent.status, IntentStatus::Stale);
        assert_eq!(a.generation.value(), 0);
        assert!(!a.lock.is_active());
    }

    #[test]
    fn success_after_decision_expiry_is_expired() {
        let (mut intent, mut a, mut b) = queued_scenario(100, 200);
        // current_slot 101 > decision_expiry 100, dependencies still match.
        let out = on_success(&mut intent, &mut [&mut a, &mut b], 101, true, true).unwrap();
        assert_eq!(out, CallbackOutcome::Expired);
        assert_eq!(intent.status, IntentStatus::Expired);
        assert_eq!(a.generation.value(), 0);
        assert!(!a.lock.is_active());
    }

    #[test]
    fn static_failure_releases_without_generation_change() {
        let (mut intent, mut a, mut b) = queued_scenario(100, 200);
        let out = on_static_failure(&mut intent, &mut [&mut a, &mut b]).unwrap();
        assert_eq!(out, CallbackOutcome::Failed);
        assert_eq!(intent.status, IntentStatus::Failed);
        assert_eq!(a.generation.value(), 0);
        assert!(!a.lock.is_active());
    }

    #[test]
    fn abandon_requires_lock_expiry() {
        let (mut intent, mut a, mut b) = queued_scenario(100, 200);
        // Lock expiry is slot 200; at 200 it is not yet expired.
        let err = abandon(&mut intent, &mut [&mut a, &mut b], 200);
        assert_eq!(err, Err(CoordinationError::LockNotExpired));
        assert_eq!(intent.status, IntentStatus::Queued);
        assert!(a.lock.is_active());
    }

    #[test]
    fn abandon_after_lock_expiry_bumps_generations_and_releases() {
        let (mut intent, mut a, mut b) = queued_scenario(100, 200);
        let out = abandon(&mut intent, &mut [&mut a, &mut b], 201).unwrap();
        assert_eq!(out, CallbackOutcome::Abandoned);
        assert_eq!(intent.status, IntentStatus::Abandoned);
        // Generations advance so any late callback cannot commit.
        assert_eq!(a.generation.value(), 1);
        assert_eq!(b.generation.value(), 1);
        assert!(!a.lock.is_active());
        assert!(!b.lock.is_active());
    }

    #[test]
    fn abandon_of_terminal_intent_is_rejected() {
        let (mut intent, mut a, mut b) = queued_scenario(100, 200);
        on_static_failure(&mut intent, &mut [&mut a, &mut b]).unwrap();
        let err = abandon(&mut intent, &mut [&mut a, &mut b], 999);
        assert_eq!(err, Err(CoordinationError::IntentNotQueued));
        assert_eq!(intent.status, IntentStatus::Failed);
    }

    #[test]
    fn late_callback_after_abandon_is_noop() {
        let (mut intent, mut a, mut b) = queued_scenario(100, 200);
        abandon(&mut intent, &mut [&mut a, &mut b], 201).unwrap();
        let gen_a = a.generation.value();

        // A late success callback arrives for the now-Abandoned Intent.
        let out = on_success(&mut intent, &mut [&mut a, &mut b], 50, true, true).unwrap();
        assert_eq!(out, CallbackOutcome::TerminalNoop);
        assert_eq!(intent.status, IntentStatus::Abandoned);
        // No further state change.
        assert_eq!(a.generation.value(), gen_a);
    }

    #[test]
    fn duplicate_success_callback_is_noop() {
        let (mut intent, mut a, mut b) = queued_scenario(100, 200);
        on_success(&mut intent, &mut [&mut a, &mut b], 50, true, true).unwrap();
        let gen_a = a.generation.value();
        let out = on_success(&mut intent, &mut [&mut a, &mut b], 50, true, true).unwrap();
        assert_eq!(out, CallbackOutcome::TerminalNoop);
        assert_eq!(a.generation.value(), gen_a); // no second bump
    }

    // ---- Property tests (run via `cargo test -- prop_`) -------------------

    /// Deterministic linear-congruential sampler, so property tests need no
    /// external crate and stay reproducible under `--locked`.
    struct Lcg(u64);
    impl Lcg {
        fn sample(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0
        }
    }

    /// **Validates: Requirements 3.4, 3.6** — only the first terminal handler
    /// takes effect; every later callback is a no-op and the status never
    /// leaves its first terminal value.
    #[test]
    fn prop_terminal_transition_is_write_once() {
        let mut rng = Lcg(0xabcd_0001);
        for _ in 0..3_000 {
            let (mut intent, mut a, mut b) = queued_scenario(100, 200);

            // Drive the Intent to some terminal state first.
            let first = rng.sample() % 6;
            match first {
                0 => {
                    on_success(&mut intent, &mut [&mut a, &mut b], 50, true, true).unwrap();
                }
                1 => {
                    on_success(&mut intent, &mut [&mut a, &mut b], 50, true, false).unwrap();
                }
                2 => {
                    on_success(&mut intent, &mut [&mut a, &mut b], 50, false, true).unwrap();
                }
                3 => {
                    on_success(&mut intent, &mut [&mut a, &mut b], 101, true, true).unwrap();
                }
                4 => {
                    on_static_failure(&mut intent, &mut [&mut a, &mut b]).unwrap();
                }
                _ => {
                    abandon(&mut intent, &mut [&mut a, &mut b], 201).unwrap();
                }
            }

            let terminal = intent.status;
            assert!(terminal.is_terminal());
            let gen_a = a.generation.value();
            let gen_b = b.generation.value();

            // Fire several more callbacks; none may alter the terminal state.
            for _ in 0..5 {
                match rng.sample() % 3 {
                    0 => {
                        let out =
                            on_success(&mut intent, &mut [&mut a, &mut b], 50, true, true).unwrap();
                        assert_eq!(out, CallbackOutcome::TerminalNoop);
                    }
                    1 => {
                        let out = on_static_failure(&mut intent, &mut [&mut a, &mut b]).unwrap();
                        assert_eq!(out, CallbackOutcome::TerminalNoop);
                    }
                    _ => {
                        // Abandon of a terminal Intent is explicitly rejected.
                        assert_eq!(
                            abandon(&mut intent, &mut [&mut a, &mut b], 999),
                            Err(CoordinationError::IntentNotQueued)
                        );
                    }
                }
                assert_eq!(intent.status, terminal);
                assert_eq!(a.generation.value(), gen_a);
                assert_eq!(b.generation.value(), gen_b);
            }
        }
    }

    /// **Validates: Requirements 3.6, 3.8** — a generation advances only on
    /// `Applied` or `Abandoned`; `Zero`, `Stale`, `Expired`, and `Failed` leave
    /// every generation unchanged.
    #[test]
    fn prop_generation_bumps_only_on_applied_or_abandoned() {
        let mut rng = Lcg(0x1234_5678);
        for _ in 0..3_000 {
            let (mut intent, mut a, mut b) = queued_scenario(100, 200);
            let before = a.generation.value();

            let outcome = match rng.sample() % 6 {
                0 => on_success(&mut intent, &mut [&mut a, &mut b], 50, true, true).unwrap(),
                1 => on_success(&mut intent, &mut [&mut a, &mut b], 50, true, false).unwrap(),
                2 => on_success(&mut intent, &mut [&mut a, &mut b], 50, false, true).unwrap(),
                3 => on_success(&mut intent, &mut [&mut a, &mut b], 101, true, true).unwrap(),
                4 => on_static_failure(&mut intent, &mut [&mut a, &mut b]).unwrap(),
                _ => abandon(&mut intent, &mut [&mut a, &mut b], 201).unwrap(),
            };

            let bumped = a.generation.value() != before;
            let should_bump = matches!(
                outcome,
                CallbackOutcome::Applied | CallbackOutcome::Abandoned
            );
            assert_eq!(bumped, should_bump);
            if bumped {
                assert_eq!(a.generation.value(), before + 1);
                assert_eq!(b.generation.value(), before + 1);
            }
        }
    }

    /// **Validates: Requirements 3.7** — every terminal outcome releases the
    /// Locks the Intent owned; no account remains locked by it afterwards.
    #[test]
    fn prop_terminal_outcome_releases_all_matching_locks() {
        let mut rng = Lcg(0x0f0f_0f0f);
        for _ in 0..3_000 {
            let (mut intent, mut a, mut b) = queued_scenario(100, 200);
            match rng.sample() % 6 {
                0 => {
                    on_success(&mut intent, &mut [&mut a, &mut b], 50, true, true).unwrap();
                }
                1 => {
                    on_success(&mut intent, &mut [&mut a, &mut b], 50, true, false).unwrap();
                }
                2 => {
                    on_success(&mut intent, &mut [&mut a, &mut b], 50, false, true).unwrap();
                }
                3 => {
                    on_success(&mut intent, &mut [&mut a, &mut b], 101, true, true).unwrap();
                }
                4 => {
                    on_static_failure(&mut intent, &mut [&mut a, &mut b]).unwrap();
                }
                _ => {
                    abandon(&mut intent, &mut [&mut a, &mut b], 201).unwrap();
                }
            }
            assert!(intent.status.is_terminal());
            assert!(!a.is_locked_by(&intent.key, intent.computation_offset));
            assert!(!b.is_locked_by(&intent.key, intent.computation_offset));
        }
    }

    /// **Validates: Requirements 3.8** — abandonment is permitted only once
    /// every matching Lock has passed its expiry slot.
    #[test]
    fn prop_abandon_requires_all_locks_expired() {
        let mut rng = Lcg(0xdead_beef);
        for _ in 0..3_000 {
            let lock_expiry = 100 + (rng.sample() % 50);
            let (mut intent, mut a, mut b) = queued_scenario(1_000, lock_expiry);
            let current = rng.sample() % 200;

            let result = abandon(&mut intent, &mut [&mut a, &mut b], current);
            if current > lock_expiry {
                assert_eq!(result, Ok(CallbackOutcome::Abandoned));
                assert_eq!(a.generation.value(), 1);
            } else {
                assert_eq!(result, Err(CoordinationError::LockNotExpired));
                assert_eq!(intent.status, IntentStatus::Queued);
                assert!(a.lock.is_active());
            }
        }
    }
}
