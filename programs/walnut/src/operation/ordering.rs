//! Task 3.1 event-ordering property tests (pure Rust, CI-runnable).
//!
//! These drive the production Lock and generation rules through every ordering
//! of the coordination events and assert the safety invariants after each step.
//! Each event runs through two paths that must agree exactly:
//!
//! * the Task 2.1 lifecycle handlers (`queue`, `on_success`,
//!   `on_static_failure`, `abandon`) over [`ProtectedAccount`]s, and
//! * the Anchor adapter (`acquire_locks`, `release_locks`,
//!   `apply_and_release_locks`, `abandon_locks`) over the on-chain
//!   [`Coordination`] fields.
//!
//! World: three protected accounts `P`, `Q`, `R` (for example Position, Pool,
//! and a second Position) and two Operation Intents. Intent 0 locks `{P, Q}`
//! and Intent 1 locks `{Q, R}`, so they contend on `Q` while each also owns an
//! account the other does not. The clock moves through four fixed stages. An
//! Intent queued at slot `s` has Decision Expiry `s + 50` and Lock expiry
//! `s + 150`.
//!
//! Events (per Intent unless noted):
//!
//! * `Advance` (global): move the clock to the next stage.
//! * `Queue`: acquire the Intent's Locks. When another Intent holds any of
//!   them this is the "concurrent queue attempt on a locked account".
//! * `SuccessApplied`, `SuccessZero`, `Stale`, `ExpiredCallback`: authenticated
//!   success callbacks. The outcome is decided by the dependency flag and the
//!   clock, and the tests compare it with an independent oracle.
//! * `StaticFailure`: authenticated static failure callback.
//! * `Abandon`: permissionless abandonment at the current clock.
//! * `Duplicate`: replay of the callback that already resolved the Intent.
//! * `Late`: a success callback that arrives after the clock reached its last
//!   stage, for an Intent that is already terminal.
//! * `WrongOwner`: Lock release by identities that do not own any Lock.
//!
//! Events that do not apply (a callback for an Intent that was never queued, a
//! duplicate with nothing to replay, a second queue of the same Intent) are
//! recorded as `Skipped` and must change nothing.
//!
//! Invariants asserted after every step (see `check`):
//!
//! 1. At most one Lock owner per account; an active Lock belongs to a queued,
//!    non-terminal Intent whose footprint contains the account.
//! 2. Generations never decrease. They advance by exactly one, and only on the
//!    accounts the Intent owned, for `Applied` and `Abandoned`.
//! 3. A terminal Intent never changes again; no event touches another Intent's
//!    record.
//! 4. A terminal outcome releases exactly the Locks its Intent owned.
//! 5. `Abandoned` happens only when every owned Lock is past its expiry.
//! 6. Duplicate, late, refused, and wrong-owner events change nothing.
//! 7. A terminal Intent can never pass `callback_may_write`; a queued one can.

use crate::lock::Lock;
use crate::operation::intent::{IntentStatus, OperationIntent};
use crate::operation::lifecycle::{abandon, on_static_failure, on_success, queue, CallbackOutcome};
use crate::state::coordination::{
    abandon_locks, acquire_locks, apply_and_release_locks, callback_may_write, release_all,
    release_locks, Coordination, CoordinationSlot, LockOwner,
};
use crate::state::{CoordinationError, Generation, ProtectedAccount, Slot};
use anchor_lang::prelude::Pubkey;

const ACCOUNTS: usize = 3;
const INTENTS: usize = 2;
/// The clock stages an event sequence can move through.
const SLOT_STAGES: [Slot; 4] = [0, 100, 200, 300];
const DECISION_WINDOW: Slot = 50;
const LOCK_WINDOW: Slot = 150;
/// The accounts each Intent locks: Intent 0 -> {P, Q}, Intent 1 -> {Q, R}.
const FOOTPRINT: [[usize; 2]; INTENTS] = [[0, 1], [1, 2]];

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}

fn account_key(position: usize) -> Pubkey {
    key(1 + position as u8)
}

fn intent_key(index: usize) -> Pubkey {
    key(40 + index as u8)
}

fn offset_of(index: usize) -> u64 {
    7 + index as u64
}

fn owner_of(index: usize) -> LockOwner {
    LockOwner::new(intent_key(index), offset_of(index))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Queue,
    SuccessApplied,
    SuccessZero,
    Stale,
    ExpiredCallback,
    StaticFailure,
    Abandon,
    Duplicate,
    Late,
    WrongOwner,
}

const ALL_KINDS: [Kind; 10] = [
    Kind::Queue,
    Kind::SuccessApplied,
    Kind::SuccessZero,
    Kind::Stale,
    Kind::ExpiredCallback,
    Kind::StaticFailure,
    Kind::Abandon,
    Kind::Duplicate,
    Kind::Late,
    Kind::WrongOwner,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    Advance,
    On(Kind, usize),
}

fn all_events() -> Vec<Event> {
    std::iter::once(Event::Advance)
        .chain(
            (0..INTENTS).flat_map(|index| ALL_KINDS.into_iter().map(move |k| Event::On(k, index))),
        )
        .collect()
}

/// The callback that resolved an Intent, kept so `Duplicate` can replay it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Replay {
    Success {
        dependencies_match: bool,
        nonzero: bool,
    },
    Failure,
    Abandon,
}

/// What one event did, as seen by the test harness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Observed {
    Advanced,
    Skipped,
    Acquired(usize),
    QueueRefused,
    Terminal(usize, IntentStatus),
    Noop,
    AbandonRefused,
    ReleaseNoop,
}

const CATEGORIES: usize = 13;

/// Maps an observation to a coverage bucket so the tests can prove that every
/// kind of outcome was actually reached.
fn category(observed: Observed) -> usize {
    match observed {
        Observed::Advanced => 0,
        Observed::Skipped => 1,
        Observed::Acquired(_) => 2,
        Observed::QueueRefused => 3,
        Observed::Terminal(_, IntentStatus::Applied) => 4,
        Observed::Terminal(_, IntentStatus::Zero) => 5,
        Observed::Terminal(_, IntentStatus::Stale) => 6,
        Observed::Terminal(_, IntentStatus::Expired) => 7,
        Observed::Terminal(_, IntentStatus::Failed) => 8,
        Observed::Terminal(_, IntentStatus::Abandoned) => 9,
        Observed::Terminal(_, IntentStatus::Queued) => unreachable!("Queued is not terminal"),
        Observed::Noop => 10,
        Observed::AbandonRefused => 11,
        Observed::ReleaseNoop => 12,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct World {
    slot_stage: usize,
    /// The pure-Rust accounts the Task 2.1 handlers operate on.
    accounts: [ProtectedAccount; ACCOUNTS],
    /// The on-chain field groups the Anchor adapter operates on.
    coords: [Coordination; ACCOUNTS],
    intents: [OperationIntent; INTENTS],
    queued: [bool; INTENTS],
    lock_expiry: [Slot; INTENTS],
    /// The generations each Intent captured for its footprint at queue time.
    captured: [[u64; 2]; INTENTS],
    replay: [Option<Replay>; INTENTS],
}

impl World {
    fn new() -> Self {
        Self {
            slot_stage: 0,
            accounts: std::array::from_fn(|position| ProtectedAccount::new(account_key(position))),
            coords: [Coordination::default(); ACCOUNTS],
            intents: [OperationIntent::default(); INTENTS],
            queued: [false; INTENTS],
            lock_expiry: [0; INTENTS],
            captured: [[0; 2]; INTENTS],
            replay: [None; INTENTS],
        }
    }

    fn slot(&self) -> Slot {
        SLOT_STAGES[self.slot_stage]
    }
}

/// Runs `operation` over the accounts in `index`'s footprint.
fn with_footprint<R>(
    accounts: &mut [ProtectedAccount; ACCOUNTS],
    index: usize,
    operation: impl FnOnce(&mut [&mut ProtectedAccount]) -> R,
) -> R {
    let [first, second, third] = accounts;
    if index == 0 {
        operation(&mut [first, second])
    } else {
        operation(&mut [second, third])
    }
}

/// Runs `operation` over every account.
fn with_all<R>(
    accounts: &mut [ProtectedAccount; ACCOUNTS],
    operation: impl FnOnce(&mut [&mut ProtectedAccount]) -> R,
) -> R {
    let [first, second, third] = accounts;
    operation(&mut [first, second, third])
}

/// Runs `operation` over the adapter slots of `index`'s footprint.
fn with_slots<R>(
    coords: &mut [Coordination; ACCOUNTS],
    index: usize,
    operation: impl FnOnce(&mut [CoordinationSlot<'_>]) -> R,
) -> R {
    let [first, second, third] = coords;
    if index == 0 {
        operation(&mut [
            CoordinationSlot::new(account_key(0), first),
            CoordinationSlot::new(account_key(1), second),
        ])
    } else {
        operation(&mut [
            CoordinationSlot::new(account_key(1), second),
            CoordinationSlot::new(account_key(2), third),
        ])
    }
}

/// Runs `operation` over the adapter slots of every account.
fn with_all_slots<R>(
    coords: &mut [Coordination; ACCOUNTS],
    operation: impl FnOnce(&mut [CoordinationSlot<'_>]) -> R,
) -> R {
    let [first, second, third] = coords;
    operation(&mut [
        CoordinationSlot::new(account_key(0), first),
        CoordinationSlot::new(account_key(1), second),
        CoordinationSlot::new(account_key(2), third),
    ])
}

fn owned_count(world: &World, owner: LockOwner) -> usize {
    world
        .accounts
        .iter()
        .filter(|account| owner.holds(&account.lock))
        .count()
}

// ---- Event application ------------------------------------------------------

fn apply_queue(world: &mut World, index: usize) -> Observed {
    if world.queued[index] {
        return Observed::Skipped;
    }
    let slot = world.slot();
    let lock_expiry = slot + LOCK_WINDOW;
    let owner = owner_of(index);
    let intent = OperationIntent {
        key: owner.operation_intent,
        computation_offset: owner.computation_offset,
        queued_slot: slot,
        decision_expiry_slot: slot + DECISION_WINDOW,
        ..OperationIntent::default()
    };
    let contended = FOOTPRINT[index]
        .iter()
        .any(|&position| world.accounts[position].lock.is_active());

    let pure = with_footprint(&mut world.accounts, index, |accounts| {
        queue(&intent, accounts, lock_expiry)
    });
    let adapter = with_slots(&mut world.coords, index, |slots| {
        acquire_locks(owner, lock_expiry, slots)
    });
    assert_eq!(pure, adapter, "lifecycle and adapter must agree on queue");

    if pure.is_ok() {
        assert!(!contended, "queue succeeded over a held Lock");
        world.intents[index] = intent;
        world.queued[index] = true;
        world.lock_expiry[index] = lock_expiry;
        world.captured[index] =
            FOOTPRINT[index].map(|position| world.accounts[position].generation.value());
        Observed::Acquired(index)
    } else {
        assert_eq!(pure, Err(CoordinationError::LockHeld));
        assert!(contended, "queue refused without a held Lock");
        Observed::QueueRefused
    }
}

fn apply_success(
    world: &mut World,
    index: usize,
    dependencies_match: bool,
    nonzero: bool,
    past_decision_expiry: bool,
) -> Observed {
    if !world.queued[index] {
        return Observed::Skipped;
    }
    if past_decision_expiry {
        // Move the clock to the first stage after this Intent's Decision Expiry,
        // or to the last stage if none is later.
        while world.slot() <= world.intents[index].decision_expiry_slot
            && world.slot_stage + 1 < SLOT_STAGES.len()
        {
            world.slot_stage += 1;
        }
    }
    let slot = world.slot();
    let owner = owner_of(index);
    let was_terminal = world.intents[index].status.is_terminal();
    let owned = owned_count(world, owner);

    let outcome = with_footprint(&mut world.accounts, index, |accounts| {
        on_success(
            &mut world.intents[index],
            accounts,
            slot,
            dependencies_match,
            nonzero,
        )
    })
    .unwrap();

    // Independent oracle for the transition table.
    let expected = if was_terminal {
        CallbackOutcome::TerminalNoop
    } else if !dependencies_match {
        CallbackOutcome::Stale
    } else if slot > world.intents[index].decision_expiry_slot {
        CallbackOutcome::Expired
    } else if nonzero {
        CallbackOutcome::Applied
    } else {
        CallbackOutcome::Zero
    };
    assert_eq!(outcome, expected);

    match outcome {
        CallbackOutcome::Applied => {
            let bumped = with_slots(&mut world.coords, index, |slots| {
                apply_and_release_locks(owner, slots)
            });
            assert_eq!(bumped, Ok(owned));
        }
        CallbackOutcome::Zero | CallbackOutcome::Stale | CallbackOutcome::Expired => {
            let released = with_slots(&mut world.coords, index, |slots| {
                release_locks(owner, slots)
            });
            assert_eq!(released, Ok(owned));
        }
        CallbackOutcome::TerminalNoop => {}
        other => panic!("unexpected success outcome {other:?}"),
    }

    if outcome == CallbackOutcome::TerminalNoop {
        Observed::Noop
    } else {
        world.replay[index] = Some(Replay::Success {
            dependencies_match,
            nonzero,
        });
        Observed::Terminal(index, world.intents[index].status)
    }
}

fn apply_static_failure(world: &mut World, index: usize) -> Observed {
    if !world.queued[index] {
        return Observed::Skipped;
    }
    let owner = owner_of(index);
    let was_terminal = world.intents[index].status.is_terminal();
    let owned = owned_count(world, owner);

    let outcome = with_footprint(&mut world.accounts, index, |accounts| {
        on_static_failure(&mut world.intents[index], accounts)
    })
    .unwrap();

    if was_terminal {
        assert_eq!(outcome, CallbackOutcome::TerminalNoop);
        return Observed::Noop;
    }
    assert_eq!(outcome, CallbackOutcome::Failed);
    let released = with_slots(&mut world.coords, index, |slots| {
        release_locks(owner, slots)
    });
    assert_eq!(released, Ok(owned));
    world.replay[index] = Some(Replay::Failure);
    Observed::Terminal(index, IntentStatus::Failed)
}

fn apply_abandon(world: &mut World, index: usize) -> Observed {
    if !world.queued[index] {
        return Observed::Skipped;
    }
    let slot = world.slot();
    let owner = owner_of(index);
    let was_terminal = world.intents[index].status.is_terminal();
    let owned = owned_count(world, owner);
    let all_expired = world
        .accounts
        .iter()
        .filter(|account| owner.holds(&account.lock))
        .all(|account| account.lock.is_expired(slot));

    let pure = with_footprint(&mut world.accounts, index, |accounts| {
        abandon(&mut world.intents[index], accounts, slot)
    });
    if was_terminal {
        assert_eq!(pure, Err(CoordinationError::IntentNotQueued));
        return Observed::Noop;
    }

    let adapter = with_slots(&mut world.coords, index, |slots| {
        abandon_locks(owner, slot, slots)
    });
    if all_expired {
        assert_eq!(pure, Ok(CallbackOutcome::Abandoned));
        assert_eq!(adapter, Ok(owned));
        world.replay[index] = Some(Replay::Abandon);
        Observed::Terminal(index, IntentStatus::Abandoned)
    } else {
        assert_eq!(pure, Err(CoordinationError::LockNotExpired));
        assert_eq!(adapter, Err(CoordinationError::LockNotExpired));
        Observed::AbandonRefused
    }
}

fn apply_duplicate(world: &mut World, index: usize) -> Observed {
    let Some(replay) = world.replay[index] else {
        return Observed::Skipped;
    };
    let slot = world.slot();
    match replay {
        Replay::Success {
            dependencies_match,
            nonzero,
        } => {
            let outcome = with_footprint(&mut world.accounts, index, |accounts| {
                on_success(
                    &mut world.intents[index],
                    accounts,
                    slot,
                    dependencies_match,
                    nonzero,
                )
            });
            assert_eq!(outcome, Ok(CallbackOutcome::TerminalNoop));
        }
        Replay::Failure => {
            let outcome = with_footprint(&mut world.accounts, index, |accounts| {
                on_static_failure(&mut world.intents[index], accounts)
            });
            assert_eq!(outcome, Ok(CallbackOutcome::TerminalNoop));
        }
        Replay::Abandon => {
            let outcome = with_footprint(&mut world.accounts, index, |accounts| {
                abandon(&mut world.intents[index], accounts, slot)
            });
            assert_eq!(outcome, Err(CoordinationError::IntentNotQueued));
        }
    }
    Observed::Noop
}

fn apply_late(world: &mut World, index: usize) -> Observed {
    if !world.queued[index] || !world.intents[index].status.is_terminal() {
        return Observed::Skipped;
    }
    let slot = SLOT_STAGES[SLOT_STAGES.len() - 1];
    let outcome = with_footprint(&mut world.accounts, index, |accounts| {
        on_success(&mut world.intents[index], accounts, slot, true, true)
    });
    assert_eq!(outcome, Ok(CallbackOutcome::TerminalNoop));
    Observed::Noop
}

fn apply_wrong_owner(world: &mut World, index: usize) -> Observed {
    let other = (index + 1) % INTENTS;
    let impostors = [
        // Right Intent, wrong computation offset.
        LockOwner::new(intent_key(index), offset_of(index) + 1_000),
        // Another Intent's key with this Intent's offset.
        LockOwner::new(intent_key(other), offset_of(index)),
        // An unrelated key.
        LockOwner::new(key(200), offset_of(index)),
    ];
    for impostor in impostors {
        let pure = with_all(&mut world.accounts, |accounts| {
            release_all(impostor, accounts)
        });
        let adapter = with_all_slots(&mut world.coords, |slots| release_locks(impostor, slots));
        assert_eq!(pure, 0, "a wrong owner released a Lock");
        assert_eq!(adapter, Ok(0));
    }
    Observed::ReleaseNoop
}

fn apply(world: &mut World, kind: Kind, index: usize) -> Observed {
    match kind {
        Kind::Queue => apply_queue(world, index),
        Kind::SuccessApplied => apply_success(world, index, true, true, false),
        Kind::SuccessZero => apply_success(world, index, true, false, false),
        Kind::Stale => apply_success(world, index, false, true, false),
        Kind::ExpiredCallback => apply_success(world, index, true, true, true),
        Kind::StaticFailure => apply_static_failure(world, index),
        Kind::Abandon => apply_abandon(world, index),
        Kind::Duplicate => apply_duplicate(world, index),
        Kind::Late => apply_late(world, index),
        Kind::WrongOwner => apply_wrong_owner(world, index),
    }
}

// ---- Invariants ----------------------------------------------------------------

fn check_acquired(before: &World, after: &World, index: usize) {
    let owner = owner_of(index);
    assert!(!before.queued[index]);
    assert!(after.queued[index]);
    assert_eq!(after.intents[index].status, IntentStatus::Queued);
    for (position, (old, new)) in before
        .accounts
        .iter()
        .zip(after.accounts.iter())
        .enumerate()
    {
        if FOOTPRINT[index].contains(&position) {
            assert!(!old.lock.is_active(), "acquired over a held Lock");
            assert!(owner.holds(&new.lock));
            assert_eq!(new.lock.expiry_slot, after.lock_expiry[index]);
            assert_eq!(new.generation, old.generation);
        } else {
            assert_eq!(
                new, old,
                "acquisition touched an account outside its footprint"
            );
        }
    }
    for (other, (old, new)) in before.intents.iter().zip(after.intents.iter()).enumerate() {
        if other != index {
            assert_eq!(new, old);
        }
    }
    assert_eq!(after.replay, before.replay);
}

fn check_terminal(before: &World, after: &World, index: usize, status: IntentStatus) {
    let owner = owner_of(index);
    let bumps = matches!(status, IntentStatus::Applied | IntentStatus::Abandoned);
    assert!(before.queued[index]);
    assert!(!before.intents[index].status.is_terminal());
    assert_ne!(status, IntentStatus::Queued);

    // Only the status of the resolved Intent changes.
    let mut expected_intent = before.intents[index];
    expected_intent.status = status;
    assert_eq!(after.intents[index], expected_intent);
    for (other, (old, new)) in before.intents.iter().zip(after.intents.iter()).enumerate() {
        if other != index {
            assert_eq!(new, old, "an event changed another Intent");
        }
    }

    for (position, (old, new)) in before
        .accounts
        .iter()
        .zip(after.accounts.iter())
        .enumerate()
    {
        if FOOTPRINT[index].contains(&position) {
            assert!(
                owner.holds(&old.lock),
                "a queued Intent must own its whole footprint"
            );
            assert_eq!(new.lock, Lock::unlocked(), "a terminal outcome left a Lock");
            assert_eq!(
                new.generation.value(),
                old.generation.value() + u64::from(bumps),
                "{status:?} generation change"
            );
            if status == IntentStatus::Abandoned {
                assert!(old.lock.is_expired(SLOT_STAGES[after.slot_stage]));
            }
        } else {
            assert_eq!(new, old, "a terminal outcome touched an unowned account");
        }
    }
}

/// Invariants that must hold in every reachable state.
fn check_standing(world: &World) {
    // The adapter fields mirror the lifecycle accounts exactly.
    for (coord, account) in world.coords.iter().zip(world.accounts.iter()) {
        assert_eq!(*coord, Coordination::from(account));
    }

    // At most one Lock owner per account.
    for (position, account) in world.accounts.iter().enumerate() {
        let holders: Vec<usize> = (0..INTENTS)
            .filter(|&index| owner_of(index).holds(&account.lock))
            .collect();
        assert!(holders.len() <= 1, "two owners hold account {position}");
        match holders.first() {
            Some(&index) => {
                assert!(FOOTPRINT[index].contains(&position));
                assert!(world.queued[index]);
                assert!(!world.intents[index].status.is_terminal());
                assert_eq!(account.lock.expiry_slot, world.lock_expiry[index]);
            }
            None => assert_eq!(account.lock, Lock::unlocked()),
        }
    }

    for (index, intent) in world.intents.iter().enumerate() {
        if !world.queued[index] {
            assert_eq!(*intent, OperationIntent::default());
            continue;
        }
        // Only a queued Intent may write, and a terminal one never may.
        let pairs: Vec<(&Coordination, u64)> = FOOTPRINT[index]
            .iter()
            .zip(world.captured[index])
            .map(|(&position, captured)| (&world.coords[position], captured))
            .collect();
        assert_eq!(
            callback_may_write(owner_of(index), &pairs),
            !intent.status.is_terminal(),
            "callback_may_write disagrees with the Intent status"
        );
    }
}

fn check(before: &World, after: &World, observed: Observed) {
    assert!(
        after.slot_stage >= before.slot_stage,
        "the clock moved back"
    );
    for (old, new) in before.accounts.iter().zip(after.accounts.iter()) {
        assert!(new.generation >= old.generation, "a generation decreased");
    }

    // `ExpiredCallback` may also move the clock, so compare against the old
    // world with the new clock.
    let mut baseline = *before;
    baseline.slot_stage = after.slot_stage;

    match observed {
        Observed::Advanced
        | Observed::Skipped
        | Observed::QueueRefused
        | Observed::Noop
        | Observed::AbandonRefused
        | Observed::ReleaseNoop => {
            assert_eq!(*after, baseline, "{observed:?} must change nothing");
        }
        Observed::Acquired(index) => check_acquired(&baseline, after, index),
        Observed::Terminal(index, status) => check_terminal(&baseline, after, index, status),
    }
    check_standing(after);
}

fn step(world: &mut World, event: Event) -> Observed {
    let before = *world;
    let observed = match event {
        Event::Advance => {
            world.slot_stage = (world.slot_stage + 1).min(SLOT_STAGES.len() - 1);
            Observed::Advanced
        }
        Event::On(kind, index) => apply(world, kind, index),
    };
    check(&before, world, observed);
    observed
}

// ---- Drivers -----------------------------------------------------------------

fn explore(
    world: &World,
    depth: usize,
    events: &[Event],
    visited: &mut u64,
    coverage: &mut [u64; CATEGORIES],
) {
    if depth == 0 {
        return;
    }
    for &event in events {
        let mut next = *world;
        let observed = step(&mut next, event);
        *visited += 1;
        coverage[category(observed)] += 1;
        explore(&next, depth - 1, events, visited, coverage);
    }
}

fn assert_full_coverage(coverage: &[u64; CATEGORIES]) {
    for (bucket, count) in coverage.iter().enumerate() {
        assert!(*count > 0, "outcome category {bucket} was never reached");
    }
}

/// Deterministic linear-congruential sampler: no external crate, reproducible
/// under `--locked`.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, bound: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % bound
    }
}

/// **Validates: Requirements 3.4, 3.6, 3.7, 3.8** - every ordering of every
/// event (21 events, lengths 1 to 4) keeps all invariants.
#[test]
fn prop_every_ordering_of_all_events_up_to_length_four() {
    let events = all_events();
    assert_eq!(events.len(), 1 + INTENTS * ALL_KINDS.len());

    let mut visited = 0;
    let mut coverage = [0u64; CATEGORIES];
    explore(&World::new(), 4, &events, &mut visited, &mut coverage);

    let count = events.len() as u64;
    assert_eq!(visited, (1..=4).map(|depth| count.pow(depth)).sum::<u64>());
    assert_full_coverage(&coverage);
}

/// **Validates: Requirements 3.6, 3.7, 3.8** - every ordering up to length six
/// of the events that exercise contention, abandonment, and late callbacks.
#[test]
fn prop_every_contention_ordering_up_to_length_six() {
    let events = [
        Event::Advance,
        Event::On(Kind::Queue, 0),
        Event::On(Kind::Queue, 1),
        Event::On(Kind::SuccessApplied, 0),
        Event::On(Kind::Stale, 1),
        Event::On(Kind::Abandon, 0),
        Event::On(Kind::Abandon, 1),
        Event::On(Kind::Late, 0),
    ];
    let mut visited = 0;
    let mut coverage = [0u64; CATEGORIES];
    explore(&World::new(), 6, &events, &mut visited, &mut coverage);

    let count = events.len() as u64;
    assert_eq!(visited, (1..=6).map(|depth| count.pow(depth)).sum::<u64>());
    // This reduced set never fires static failure, zero success, or the expired
    // callback, so those buckets stay empty; the rest must all be reached.
    for bucket in [2, 3, 4, 6, 9, 10, 11] {
        assert!(coverage[bucket] > 0, "category {bucket} was never reached");
    }
}

/// **Validates: Requirements 3.4, 3.6, 3.7, 3.8** - long pseudo-random event
/// sequences keep all invariants.
#[test]
fn prop_random_long_event_orderings() {
    let events = all_events();
    let mut rng = Lcg(0x0031_c0ff_ee00_0001);
    let mut coverage = [0u64; CATEGORIES];
    for _ in 0..20_000 {
        let mut world = World::new();
        for _ in 0..24 {
            let event = events[rng.below(events.len())];
            let observed = step(&mut world, event);
            coverage[category(observed)] += 1;
        }
    }
    assert_full_coverage(&coverage);
}

// ---- Named scenarios -------------------------------------------------------------

#[test]
fn concurrent_queue_on_a_locked_account_is_refused_atomically() {
    let mut world = World::new();
    assert_eq!(
        step(&mut world, Event::On(Kind::Queue, 0)),
        Observed::Acquired(0)
    );
    // Intent 1 needs Q (held by Intent 0) and R (free): all or nothing.
    assert_eq!(
        step(&mut world, Event::On(Kind::Queue, 1)),
        Observed::QueueRefused
    );
    assert_eq!(world.accounts[2].lock, Lock::unlocked());
    assert!(owner_of(0).holds(&world.accounts[1].lock));
}

#[test]
fn abandoned_intent_cannot_disturb_the_next_writer() {
    let mut world = World::new();
    for event in [
        Event::On(Kind::Queue, 0),
        Event::Advance,
        Event::Advance,
        Event::On(Kind::Abandon, 0),
        Event::On(Kind::Queue, 1),
        Event::On(Kind::Late, 0),
        Event::On(Kind::SuccessApplied, 0),
        Event::On(Kind::WrongOwner, 0),
    ] {
        step(&mut world, event);
    }
    assert_eq!(world.intents[0].status, IntentStatus::Abandoned);
    assert_eq!(world.accounts[0].generation.value(), 1);
    assert_eq!(world.accounts[1].generation.value(), 1);
    assert_eq!(world.accounts[2].generation.value(), 0);
    // Intent 1 holds Q and R, untouched by the late callbacks for Intent 0.
    assert_eq!(world.intents[1].status, IntentStatus::Queued);
    assert!(owner_of(1).holds(&world.accounts[1].lock));
    assert!(owner_of(1).holds(&world.accounts[2].lock));
}

#[test]
fn abandonment_is_refused_until_every_lock_expires() {
    let mut world = World::new();
    step(&mut world, Event::On(Kind::Queue, 0));
    // Clock 0 and clock 100 are both before the Lock expiry at slot 150.
    assert_eq!(
        step(&mut world, Event::On(Kind::Abandon, 0)),
        Observed::AbandonRefused
    );
    step(&mut world, Event::Advance);
    assert_eq!(
        step(&mut world, Event::On(Kind::Abandon, 0)),
        Observed::AbandonRefused
    );
    step(&mut world, Event::Advance);
    assert_eq!(
        step(&mut world, Event::On(Kind::Abandon, 0)),
        Observed::Terminal(0, IntentStatus::Abandoned)
    );
}

#[test]
fn abandon_overflow_leaves_the_intent_queued_and_locks_held() {
    let mut world = World::new();
    step(&mut world, Event::On(Kind::Queue, 0));
    world.accounts[1].generation = Generation::new(u64::MAX);
    world.coords[1].generation = u64::MAX;
    world.slot_stage = SLOT_STAGES.len() - 1;
    let before = world;

    let slot = world.slot();
    let result = with_footprint(&mut world.accounts, 0, |accounts| {
        abandon(&mut world.intents[0], accounts, slot)
    });
    assert_eq!(result, Err(CoordinationError::GenerationOverflow));
    assert_eq!(world, before);
}
