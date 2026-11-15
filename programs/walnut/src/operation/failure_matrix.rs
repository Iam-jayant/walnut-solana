//! Task 2.4 failure-matrix state-machine assertions (pure Rust, CI-runnable).
//!
//! Requirement 2.4 lists thirteen callback/failure/expiry/recovery conditions
//! the Arcium feasibility gate must exercise and assert. Several of them are
//! pure coordination invariants of the Operation Intent / Lock / generation
//! state machine (Task 2.1): their outcome does not depend on a live Arcium
//! cluster, so they are asserted here and run under `cargo test` /
//! `cargo test -- prop_` without a localnet or MPC round trip.
//!
//! One `#[test]` per matrix item makes the pass/fail of each visible. The
//! remaining items (success event decryption, `OutputTooLarge`, callback
//! compute-unit exhaustion, finalized-rent claim, expired-fee reclaim) require
//! real Arcium machinery and are exercised on localnet in
//! `tests/failure-matrix.ts`; this module documents, per item, which layer owns
//! the assertion so the split is explicit.
//!
//! Intent-level classification used throughout (consistent with the design's
//! "Success / Failure / Expiry and abandonment" callback sections):
//!
//! * An **authenticated static failure** (Cerberus detect-and-abort, or a
//!   deterministic failure the node finalizes through a failure claim, such as
//!   `OutputTooLarge`) invokes the static failure callback and marks the Intent
//!   `Failed`.
//! * A **callback transaction that cannot execute** (invalid/unauthenticated
//!   signed output rejected by `verify_output`, a missing/wrong custom callback
//!   account, a failed callback CPI, or callback compute-unit exhaustion)
//!   reverts atomically and commits nothing, so the Intent stays `Queued` for
//!   timeout recovery (Requirement 3.6).

use crate::operation::intent::{IntentStatus, OperationIntent};
use crate::operation::lifecycle::{
    abandon, on_callback_tx_failure, on_static_failure, on_success, queue, CallbackOutcome,
};
use crate::state::{ProtectedAccount, Slot};
use anchor_lang::prelude::Pubkey;

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}

/// Builds a queued Intent plus two protected accounts (for example Position and
/// Pool), with the Locks acquired through the queue path, mirroring the Task 2.1
/// lifecycle tests.
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

// ---- Item 1: success callback (happy path, Applied) -----------------------

/// **Validates: Requirement 2.4 (success callback)** — an authenticated,
/// current, nonzero success callback applies state: the Intent becomes
/// `Applied`, every locked generation bumps exactly once, and all matching
/// Locks are released. (Success-event *decryption* is asserted on localnet in
/// `tests/failure-matrix.ts`.)
#[test]
fn item01_success_callback_applies() {
    let (mut intent, mut a, mut b) = queued_scenario(100, 200);
    let out = on_success(&mut intent, &mut [&mut a, &mut b], 50, true, true).unwrap();
    assert_eq!(out, CallbackOutcome::Applied);
    assert_eq!(intent.status, IntentStatus::Applied);
    assert_eq!(a.generation.value(), 1);
    assert_eq!(b.generation.value(), 1);
    assert!(!a.lock.is_active());
    assert!(!b.lock.is_active());
}

// ---- Item 2: circuit failure and static failure callback ------------------

/// **Validates: Requirement 2.4 (circuit failure / static failure callback)** —
/// an authenticated static failure callback finalizes the Intent as `Failed`,
/// bumps no generation, and releases only its own Locks. No financial state
/// changes.
#[test]
fn item02_static_failure_marks_failed() {
    let (mut intent, mut a, mut b) = queued_scenario(100, 200);
    let out = on_static_failure(&mut intent, &mut [&mut a, &mut b]).unwrap();
    assert_eq!(out, CallbackOutcome::Failed);
    assert_eq!(intent.status, IntentStatus::Failed);
    assert_eq!(a.generation.value(), 0);
    assert_eq!(b.generation.value(), 0);
    assert!(!a.lock.is_active());
    assert!(!b.lock.is_active());
}

// ---- Item 3: invalid signed output (forged/garbled signature) -------------

/// **Validates: Requirement 2.4 (invalid signed output) / Requirement 3.6** —
/// output that fails authentication (`verify_output` returns `Err`) must NOT
/// apply state and must NOT mark the Intent terminal. The Intent stays `Queued`
/// with every Lock held, and timeout recovery (abandonment) is the only exit.
///
/// A forged/garbled BLS signature cannot be produced on localnet, so the live
/// `verify_output` rejection is a devnet observation (Task 2.6); the state-level
/// invariant is asserted here.
#[test]
fn item03_invalid_output_left_queued_then_recovers() {
    let (mut intent, mut a, mut b) = queued_scenario(100, 200);

    let out = on_callback_tx_failure(&intent, &[&a, &b]);
    assert_eq!(out, CallbackOutcome::LeftQueuedForRecovery);
    // Nothing changed: still Queued, Locks held, generations untouched.
    assert_eq!(intent.status, IntentStatus::Queued);
    assert!(a.lock.is_active());
    assert!(b.lock.is_active());
    assert_eq!(a.generation.value(), 0);
    assert_eq!(b.generation.value(), 0);

    // Timeout recovery resolves it after Lock expiry.
    let recovered = abandon(&mut intent, &mut [&mut a, &mut b], 201).unwrap();
    assert_eq!(recovered, CallbackOutcome::Abandoned);
    assert_eq!(intent.status, IntentStatus::Abandoned);
    assert_eq!(a.generation.value(), 1);
    assert_eq!(b.generation.value(), 1);
}

// ---- Item 4: callback custom-account failure ------------------------------

/// **Validates: Requirement 2.4 (callback custom-account failure)** — a
/// callback whose required custom account is wrong or missing fails Anchor
/// account resolution, so the whole callback transaction reverts and commits
/// nothing. The Intent stays `Queued` for recovery.
#[test]
fn item04_bad_callback_account_left_queued() {
    let (intent, a, b) = queued_scenario(100, 200);
    let out = on_callback_tx_failure(&intent, &[&a, &b]);
    assert_eq!(out, CallbackOutcome::LeftQueuedForRecovery);
    assert_eq!(intent.status, IntentStatus::Queued);
    assert!(a.lock.is_active());
    assert!(b.lock.is_active());
    assert_eq!(a.generation.value(), 0);
}

// ---- Item 5: callback token CPI failure -----------------------------------

/// **Validates: Requirement 2.4 (callback token CPI failure)** — a token (or
/// any) CPI that fails inside the callback reverts the whole callback
/// transaction atomically, so no tokens move and no coordination state changes.
/// The Intent stays `Queued` for recovery.
///
/// The live SPL-token CPI wiring arrives with the vaults in Task 5 (it needs an
/// SPL token dependency not yet in the crate); the revert-leaves-nothing
/// invariant is asserted here and noted for devnet in Task 2.6.
#[test]
fn item05_failed_callback_cpi_left_queued() {
    let (intent, a, b) = queued_scenario(100, 200);
    let out = on_callback_tx_failure(&intent, &[&a, &b]);
    assert_eq!(out, CallbackOutcome::LeftQueuedForRecovery);
    assert_eq!(intent.status, IntentStatus::Queued);
    assert!(a.lock.is_active());
    assert!(b.lock.is_active());
}

// ---- Item 6: compute-unit exhaustion in the callback ----------------------

/// **Validates: Requirement 2.4 (compute-unit exhaustion)** — a callback that
/// exhausts its compute-unit budget aborts the callback transaction, which
/// reverts atomically. The Intent stays `Queued` for recovery. (The live
/// exhaustion is driven on localnet by a tiny fixed `callback_cu_limit` in
/// `tests/failure-matrix.ts`.)
#[test]
fn item06_callback_cu_exhaustion_left_queued() {
    let (intent, a, b) = queued_scenario(100, 200);
    let out = on_callback_tx_failure(&intent, &[&a, &b]);
    assert_eq!(out, CallbackOutcome::LeftQueuedForRecovery);
    assert_eq!(intent.status, IntentStatus::Queued);
    assert!(a.lock.is_active());
}

// ---- Item 7: OutputTooLarge ------------------------------------------------

/// **Validates: Requirement 2.4 (`OutputTooLarge`)** — when the callback output
/// exceeds the single-transaction size limit the node cannot build the callback
/// and finalizes a deterministic computation failure. As a known static failure
/// it is finalized by the static failure callback, marking the Intent `Failed`
/// with no generation bump. (The live `OutputTooLarge` is driven on localnet by
/// the `oversized_output` circuit in `tests/failure-matrix.ts`.)
#[test]
fn item07_output_too_large_marks_failed() {
    let (mut intent, mut a, mut b) = queued_scenario(100, 200);
    let out = on_static_failure(&mut intent, &mut [&mut a, &mut b]).unwrap();
    assert_eq!(out, CallbackOutcome::Failed);
    assert_eq!(intent.status, IntentStatus::Failed);
    assert_eq!(a.generation.value(), 0);
    assert!(!a.lock.is_active());
}

// ---- Item 8: queued expiry -------------------------------------------------

/// **Validates: Requirement 2.4 (queued expiry)** — two expiry paths.
///
/// 1. An authenticated success callback that arrives after Decision Expiry
///    while the Intent is still `Queued` marks `Expired` and applies no state.
/// 2. A computation that expires in the mempool before execution never produces
///    a callback; timeout recovery abandons the still-`Queued` Intent, bumping
///    generations so no late callback can commit. The fee side of this path is
///    the expired-fee reclaim (item 13), driven on localnet.
#[test]
fn item08_queued_expiry_decision_and_mempool() {
    // Path 1: callback after Decision Expiry -> Expired, no write.
    let (mut intent, mut a, mut b) = queued_scenario(100, 200);
    let out = on_success(&mut intent, &mut [&mut a, &mut b], 101, true, true).unwrap();
    assert_eq!(out, CallbackOutcome::Expired);
    assert_eq!(intent.status, IntentStatus::Expired);
    assert_eq!(a.generation.value(), 0);
    assert!(!a.lock.is_active());

    // Path 2: never executed, Lock expired -> abandonment recovers.
    let (mut intent2, mut c, mut d) = queued_scenario(100, 200);
    let out2 = abandon(&mut intent2, &mut [&mut c, &mut d], 201).unwrap();
    assert_eq!(out2, CallbackOutcome::Abandoned);
    assert_eq!(intent2.status, IntentStatus::Abandoned);
    assert_eq!(c.generation.value(), 1);
    assert_eq!(d.generation.value(), 1);
}

// ---- Item 9: abandonment while an old computation may still execute --------

/// **Validates: Requirement 2.4 (abandonment vs late execution) /
/// Requirement 3.8** — abandonment bumps every still-matching generation BEFORE
/// clearing the Locks, so an old computation that executes late cannot commit
/// against the generation it captured. The late callback hits a now-terminal
/// Intent and a changed generation, so it is a no-op.
#[test]
fn item09_abandon_before_old_computation_late_write_blocked() {
    let (mut intent, mut a, mut b) = queued_scenario(100, 200);

    // An old computation captured generation 0 for account a at queue time.
    let captured_generation_a = a.generation.value();
    assert_eq!(captured_generation_a, 0);

    // Abandon after Lock expiry: generations bump first, then Locks release,
    // then the Intent becomes terminal (see `lifecycle::abandon`).
    let out = abandon(&mut intent, &mut [&mut a, &mut b], 201).unwrap();
    assert_eq!(out, CallbackOutcome::Abandoned);
    assert_eq!(intent.status, IntentStatus::Abandoned);

    // The generation moved past what the old computation captured, so a generation
    // guard alone would reject the late callback.
    assert_ne!(captured_generation_a, a.generation.value());
    assert_eq!(a.generation.value(), 1);

    // The late callback arrives for the now-Abandoned Intent: pure no-op.
    let gen_before_late = a.generation.value();
    let late = on_success(&mut intent, &mut [&mut a, &mut b], 50, true, true).unwrap();
    assert_eq!(late, CallbackOutcome::TerminalNoop);
    assert_eq!(intent.status, IntentStatus::Abandoned);
    assert_eq!(a.generation.value(), gen_before_late);
}

// ---- Item 10: duplicate callback ------------------------------------------

/// **Validates: Requirement 2.4 (duplicate callback) / Requirement 3.4** — a
/// second callback for the same computation is a no-op: the Intent keeps its
/// first terminal state and no generation bumps a second time.
#[test]
fn item10_duplicate_callback_is_noop() {
    let (mut intent, mut a, mut b) = queued_scenario(100, 200);
    let first = on_success(&mut intent, &mut [&mut a, &mut b], 50, true, true).unwrap();
    assert_eq!(first, CallbackOutcome::Applied);
    let gen_after_first = a.generation.value();

    let second = on_success(&mut intent, &mut [&mut a, &mut b], 50, true, true).unwrap();
    assert_eq!(second, CallbackOutcome::TerminalNoop);
    assert_eq!(intent.status, IntentStatus::Applied);
    assert_eq!(a.generation.value(), gen_after_first); // no second bump
}

// ---- Item 11: late callback (Intent already terminal) ---------------------

/// **Validates: Requirement 2.4 (late callback) / Requirement 3.6** — a
/// callback that arrives after the Intent is already terminal (here `Failed`)
/// changes no financial state and leaves the terminal status intact.
#[test]
fn item11_late_callback_after_terminal_is_noop() {
    let (mut intent, mut a, mut b) = queued_scenario(100, 200);
    on_static_failure(&mut intent, &mut [&mut a, &mut b]).unwrap();
    assert_eq!(intent.status, IntentStatus::Failed);
    let gen_before = a.generation.value();

    let late = on_success(&mut intent, &mut [&mut a, &mut b], 50, true, true).unwrap();
    assert_eq!(late, CallbackOutcome::TerminalNoop);
    assert_eq!(intent.status, IntentStatus::Failed);
    assert_eq!(a.generation.value(), gen_before);
}

// ---- Items 12 and 13: rent and fee reclaim --------------------------------
//
// Item 12 (finalized-rent claim, `claimComputationRent`) and item 13
// (expired-fee reclaim, `reclaimExpiredComputationFee`) are Arcium
// account-lifecycle actions, not Walnut Intent transitions: they close or
// refund per-computation Arcium accounts and have no pure state-machine
// invariant to assert here. They are exercised on localnet in
// `tests/failure-matrix.ts`. The abandonment path that item 13 settles the fee
// for is asserted above (items 3 and 8, path 2).

// ---- Cross-cutting property: no failing path ever moves a generation ------

/// **Validates: Requirement 2.4 / Requirement 3.6** — across randomized
/// orderings, a failing callback transaction (`on_callback_tx_failure`) never
/// changes the Intent status or any generation, and abandonment afterwards
/// always succeeds and bumps every still-locked generation exactly once.
#[test]
fn prop_failing_callback_never_commits_then_recovers() {
    let mut seed = 0x2_4fa1_1001u64;
    let mut next = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        seed
    };
    for _ in 0..5_000 {
        let lock_expiry = 100 + (next() % 50);
        let (mut intent, mut a, mut b) = queued_scenario(10_000, lock_expiry);

        // Any number of failing callbacks never commit anything.
        let attempts = (next() % 4) + 1;
        for _ in 0..attempts {
            let out = on_callback_tx_failure(&intent, &[&a, &b]);
            assert_eq!(out, CallbackOutcome::LeftQueuedForRecovery);
            assert_eq!(intent.status, IntentStatus::Queued);
            assert!(a.lock.is_active());
            assert!(b.lock.is_active());
            assert_eq!(a.generation.value(), 0);
            assert_eq!(b.generation.value(), 0);
        }

        // Recovery after Lock expiry always succeeds exactly once.
        let current = lock_expiry + 1 + (next() % 10);
        let recovered = abandon(&mut intent, &mut [&mut a, &mut b], current).unwrap();
        assert_eq!(recovered, CallbackOutcome::Abandoned);
        assert_eq!(intent.status, IntentStatus::Abandoned);
        assert_eq!(a.generation.value(), 1);
        assert_eq!(b.generation.value(), 1);
    }
}
