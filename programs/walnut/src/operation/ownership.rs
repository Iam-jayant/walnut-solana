//! Exact transition ownership for Operation Intents (Requirements 3.4, 3.6,
//! 3.8, 3.10, 3.11).
//!
//! An Intent leaves `Queued` exactly once. This module decides *who* may drive
//! each transition and *when*, and who may reclaim computation rent and fees
//! afterwards. It is pure Rust; the Anchor instructions (Task 3.3 and later)
//! call it, so the rules have one implementation.
//!
//! # Transition ownership
//!
//! | Transition  | Actor              | Timing rule                          |
//! |-------------|--------------------|--------------------------------------|
//! | `Applied`   | verified callback  | at or before Decision Expiry         |
//! | `Zero`      | verified callback  | at or before Decision Expiry         |
//! | `Failed`    | verified callback  | any slot                             |
//! | `Stale`     | verified callback  | any slot                             |
//! | `Expired`   | verified callback  | strictly after Decision Expiry       |
//! | `Abandoned` | any signer         | strictly after Lock expiry           |
//!
//! The verified callback is the Arcium callback instruction after the generated
//! helper has checked the cluster-signed output (Task 3.3). No user, owner,
//! payer, or keeper can move an Intent into a callback outcome, and a callback
//! can never abandon: `Abandoned` is the permissionless timeout path only.
//! Checks run in a fixed order, so every failure has one distinct error:
//!
//! 1. the Intent must still be `Queued` ([`IntentError::NotQueued`]);
//! 2. the actor must be the transition's owner ([`IntentError::WrongActor`]);
//! 3. the timing rule must hold ([`IntentError::DecisionExpired`],
//!    [`IntentError::DecisionNotExpired`], [`IntentError::LockNotExpired`]).
//!
//! # Reclaim ownership
//!
//! The Intent records the `payer` and a `reclaim_authority`, and creation
//! requires them to be equal (see [`OperationIntent::validate_new`]). The
//! Arcium program only lets the original payer claim a computation's rent, so a
//! different authority could never succeed.
//!
//! Reclaim never touches value held in custody. It maps to two Arcium
//! instructions that the SDK builds and the payer signs (ADR-0003):
//!
//! * [`ReclaimKind::ComputationRent`] is `claimComputationRent`: the rent of a
//!   finalized computation account returns to the payer. It is allowed after any
//!   terminal status.
//! * [`ReclaimKind::ExpiredComputationFee`] is `reclaimExpiredComputationFee`:
//!   the fee of a computation that expired without executing returns to the
//!   payer. It is allowed only after `Expired` or `Abandoned`. Arcium enforces
//!   that the computation really expired; Walnut only refuses the request when
//!   the Intent's own outcome rules it out.
//!
//! Neither is allowed while the Intent is `Queued`, because a queued
//! computation may still finalize and its callback still needs its accounts.
//! The reclaim CPI itself is not part of Task 3.2.

use anchor_lang::prelude::Pubkey;

use super::intent::{IntentStatus, OperationIntent};
use crate::state::coordination::{abandon_locks, CoordinationSlot};
use crate::state::{CoordinationError, Slot};

/// Errors from Operation Intent creation, transitions, sequences, and reclaim.
///
/// Plain values so the logic stays testable without the Solana runtime. The
/// mapping onto program errors is in `errors.rs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntentError {
    /// The Intent is not `Queued`, so it cannot transition.
    NotQueued,
    /// The actor may not drive this transition.
    WrongActor,
    /// Decision Expiry has passed, so no value may be applied.
    DecisionExpired,
    /// Decision Expiry has not passed, so the Intent cannot be `Expired`.
    DecisionNotExpired,
    /// The Lock has not expired, so the Intent cannot be abandoned.
    LockNotExpired,
    /// The sequence number was already used.
    SequenceReused,
    /// The sequence number is ahead of the counter.
    SequenceOutOfOrder,
    /// The sequence counter would overflow `u64`.
    SequenceOverflow,
    /// The counter belongs to a different owner than the Intent.
    CounterOwnerMismatch,
    /// Reclaim was requested before the Intent reached a terminal status.
    ReclaimBeforeTerminal,
    /// The signer is not the recorded reclaim authority.
    ReclaimNotAuthorized,
    /// This kind of reclaim is not allowed for the Intent's status.
    ReclaimNotAllowedForStatus,
    /// The reclaim authority is not the payer.
    ReclaimAuthorityNotPayer,
    /// Decision Expiry and Lock expiry are not ordered after the queue slot.
    InvalidTiming,
    /// A Position and its captured generation do not pair up.
    SnapshotMismatch,
    /// A Lock or generation failure from the shared coordination rules.
    Coordination(CoordinationError),
}

impl From<CoordinationError> for IntentError {
    /// Maps the two conditions that have an Intent-level meaning onto their
    /// Intent variants, so each condition has one representation.
    fn from(error: CoordinationError) -> Self {
        match error {
            CoordinationError::IntentNotQueued => IntentError::NotQueued,
            CoordinationError::LockNotExpired => IntentError::LockNotExpired,
            other => IntentError::Coordination(other),
        }
    }
}

/// Who is driving a transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Actor {
    /// The Arcium callback instruction, after the signed output was verified.
    VerifiedCallback,
    /// Any transaction signer, including the owner, the payer, and keepers.
    Signer,
}

/// A terminal transition of an Operation Intent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    /// Authenticated success, dependencies matched, nonzero transition.
    Applied,
    /// Authenticated success, dependencies matched, no value approved.
    Zero,
    /// Authenticated static failure.
    Failed,
    /// Authenticated success whose captured dependency or commitment changed.
    Stale,
    /// Authenticated success arriving after Decision Expiry.
    Expired,
    /// Permissionless timeout recovery after Lock expiry.
    Abandoned,
}

impl Transition {
    /// Every transition, in declaration order.
    pub const ALL: [Transition; 6] = [
        Transition::Applied,
        Transition::Zero,
        Transition::Failed,
        Transition::Stale,
        Transition::Expired,
        Transition::Abandoned,
    ];

    /// The status this transition produces.
    pub const fn status(self) -> IntentStatus {
        match self {
            Transition::Applied => IntentStatus::Applied,
            Transition::Zero => IntentStatus::Zero,
            Transition::Failed => IntentStatus::Failed,
            Transition::Stale => IntentStatus::Stale,
            Transition::Expired => IntentStatus::Expired,
            Transition::Abandoned => IntentStatus::Abandoned,
        }
    }

    /// The only actor allowed to drive this transition.
    pub const fn required_actor(self) -> Actor {
        match self {
            Transition::Abandoned => Actor::Signer,
            _ => Actor::VerifiedCallback,
        }
    }
}

impl OperationIntent {
    /// Checks whether `actor` may drive `transition` at `current_slot`,
    /// without changing anything. See the module docs for the rules and the
    /// order of the checks.
    pub fn check_transition(
        &self,
        transition: Transition,
        actor: Actor,
        current_slot: Slot,
    ) -> Result<(), IntentError> {
        if self.status.is_terminal() {
            return Err(IntentError::NotQueued);
        }
        if actor != transition.required_actor() {
            return Err(IntentError::WrongActor);
        }
        match transition {
            Transition::Applied | Transition::Zero => {
                if self.is_past_decision_expiry(current_slot) {
                    return Err(IntentError::DecisionExpired);
                }
            }
            Transition::Expired => {
                if !self.is_past_decision_expiry(current_slot) {
                    return Err(IntentError::DecisionNotExpired);
                }
            }
            Transition::Failed | Transition::Stale => {}
            Transition::Abandoned => {
                if !self.is_past_lock_expiry(current_slot) {
                    return Err(IntentError::LockNotExpired);
                }
            }
        }
        Ok(())
    }

    /// Drives `transition` after checking ownership and timing.
    ///
    /// On error the Intent is unchanged. This does not touch Locks or
    /// generations: the callback helpers (Task 3.3) pair it with the shared
    /// coordination rules, and [`abandon_intent`] does so for abandonment.
    pub fn transition(
        &mut self,
        transition: Transition,
        actor: Actor,
        current_slot: Slot,
    ) -> Result<(), IntentError> {
        self.check_transition(transition, actor, current_slot)?;
        let result = match transition {
            Transition::Applied => self.mark_applied(),
            Transition::Zero => self.mark_zero(),
            Transition::Failed => self.mark_failed(),
            Transition::Stale => self.mark_stale(),
            Transition::Expired => self.mark_expired(),
            Transition::Abandoned => self.mark_abandoned(),
        };
        result.map_err(IntentError::from)
    }
}

/// Permissionless abandonment of an expired, still-queued Intent, together
/// with its Locks.
///
/// The Intent-level checks run first (still `Queued`, `current_slot` strictly
/// after `lock_expiry_slot`). Then the shared coordination rules bump every
/// matching generation and clear the matching Locks, and only then does the
/// Intent become `Abandoned`. Any failure leaves the Intent and every slot
/// unchanged. Returns the number of Locks cleared.
pub fn abandon_intent(
    intent: &mut OperationIntent,
    current_slot: Slot,
    slots: &mut [CoordinationSlot<'_>],
) -> Result<usize, IntentError> {
    intent.check_transition(Transition::Abandoned, Actor::Signer, current_slot)?;
    let cleared = abandon_locks(intent.lock_owner(), current_slot, slots)?;
    intent.transition(Transition::Abandoned, Actor::Signer, current_slot)?;
    Ok(cleared)
}

/// The kind of rent or fee reclaim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReclaimKind {
    /// Arcium `claimComputationRent`: the rent of a finalized computation.
    ComputationRent,
    /// Arcium `reclaimExpiredComputationFee`: the fee of an expired computation.
    ExpiredComputationFee,
}

/// Checks whether `signer` may reclaim `kind` for `intent`.
///
/// Allowed only when the Intent is terminal and `signer` is the recorded
/// reclaim authority. An expired-fee reclaim additionally needs the Intent to
/// be `Expired` or `Abandoned`.
pub fn check_reclaim(
    intent: &OperationIntent,
    signer: &Pubkey,
    kind: ReclaimKind,
) -> Result<(), IntentError> {
    if !intent.status.is_terminal() {
        return Err(IntentError::ReclaimBeforeTerminal);
    }
    if *signer != intent.reclaim_authority {
        return Err(IntentError::ReclaimNotAuthorized);
    }
    if kind == ReclaimKind::ExpiredComputationFee
        && !matches!(
            intent.status,
            IntentStatus::Expired | IntentStatus::Abandoned
        )
    {
        return Err(IntentError::ReclaimNotAllowedForStatus);
    }
    Ok(())
}

/// Returns whether `signer` may reclaim `kind` for `intent`.
pub fn can_reclaim(intent: &OperationIntent, signer: &Pubkey, kind: ReclaimKind) -> bool {
    check_reclaim(intent, signer, kind).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::coordination::{acquire_locks, Coordination};

    fn key(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    /// A queued Intent: Decision Expiry 100, Lock expiry 200.
    fn queued() -> OperationIntent {
        OperationIntent {
            key: key(42),
            owner: key(1),
            payer: key(2),
            reclaim_authority: key(2),
            computation_offset: 7,
            queued_slot: 10,
            decision_expiry_slot: 100,
            lock_expiry_slot: 200,
            ..OperationIntent::default()
        }
    }

    fn terminal(status: IntentStatus) -> OperationIntent {
        OperationIntent { status, ..queued() }
    }

    /// The slot at which `transition` is allowed for [`queued`].
    fn valid_slot(transition: Transition) -> Slot {
        match transition {
            Transition::Expired => 101,
            Transition::Abandoned => 201,
            _ => 50,
        }
    }

    // ---- Transition ownership ---------------------------------------------

    #[test]
    fn the_ownership_table_is_exact() {
        for transition in Transition::ALL {
            let expected = if transition == Transition::Abandoned {
                Actor::Signer
            } else {
                Actor::VerifiedCallback
            };
            assert_eq!(transition.required_actor(), expected);
        }
        assert_eq!(Transition::Applied.status(), IntentStatus::Applied);
        assert_eq!(Transition::Zero.status(), IntentStatus::Zero);
        assert_eq!(Transition::Failed.status(), IntentStatus::Failed);
        assert_eq!(Transition::Stale.status(), IntentStatus::Stale);
        assert_eq!(Transition::Expired.status(), IntentStatus::Expired);
        assert_eq!(Transition::Abandoned.status(), IntentStatus::Abandoned);
    }

    #[test]
    fn the_right_actor_moves_queued_to_each_terminal_status() {
        for transition in Transition::ALL {
            let mut intent = queued();
            intent
                .transition(
                    transition,
                    transition.required_actor(),
                    valid_slot(transition),
                )
                .unwrap();
            assert_eq!(intent.status, transition.status());
        }
    }

    #[test]
    fn a_wrong_actor_is_rejected_for_every_transition() {
        for transition in Transition::ALL {
            let wrong = match transition.required_actor() {
                Actor::VerifiedCallback => Actor::Signer,
                Actor::Signer => Actor::VerifiedCallback,
            };
            let mut intent = queued();
            let result = intent.transition(transition, wrong, valid_slot(transition));
            assert_eq!(result, Err(IntentError::WrongActor));
            assert_eq!(intent.status, IntentStatus::Queued);
        }
    }

    #[test]
    fn every_non_queued_source_state_is_rejected() {
        let terminals = [
            IntentStatus::Applied,
            IntentStatus::Zero,
            IntentStatus::Failed,
            IntentStatus::Stale,
            IntentStatus::Expired,
            IntentStatus::Abandoned,
        ];
        for status in terminals {
            for transition in Transition::ALL {
                for actor in [Actor::VerifiedCallback, Actor::Signer] {
                    let mut intent = terminal(status);
                    let result = intent.transition(transition, actor, valid_slot(transition));
                    assert_eq!(result, Err(IntentError::NotQueued));
                    assert_eq!(intent.status, status);
                }
            }
        }
    }

    #[test]
    fn decision_expiry_gates_applied_zero_and_expired() {
        // Applied and Zero need a slot at or before Decision Expiry.
        for transition in [Transition::Applied, Transition::Zero] {
            let actor = Actor::VerifiedCallback;
            assert_eq!(queued().check_transition(transition, actor, 100), Ok(()));
            assert_eq!(
                queued().check_transition(transition, actor, 101),
                Err(IntentError::DecisionExpired)
            );
        }
        // Expired needs a slot strictly after it.
        let actor = Actor::VerifiedCallback;
        assert_eq!(
            queued().check_transition(Transition::Expired, actor, 100),
            Err(IntentError::DecisionNotExpired)
        );
        assert_eq!(
            queued().check_transition(Transition::Expired, actor, 101),
            Ok(())
        );
        // Failed and Stale do not depend on the clock.
        for slot in [0, 100, 101, u64::MAX] {
            assert_eq!(
                queued().check_transition(Transition::Failed, actor, slot),
                Ok(())
            );
            assert_eq!(
                queued().check_transition(Transition::Stale, actor, slot),
                Ok(())
            );
        }
    }

    #[test]
    fn abandonment_needs_the_lock_to_have_expired() {
        let actor = Actor::Signer;
        assert_eq!(
            queued().check_transition(Transition::Abandoned, actor, 200),
            Err(IntentError::LockNotExpired)
        );
        assert_eq!(
            queued().check_transition(Transition::Abandoned, actor, 201),
            Ok(())
        );
    }

    #[test]
    fn a_failed_transition_changes_nothing() {
        let mut intent = queued();
        let before = intent;
        assert!(intent
            .transition(Transition::Expired, Actor::VerifiedCallback, 50)
            .is_err());
        assert!(intent
            .transition(Transition::Applied, Actor::Signer, 50)
            .is_err());
        assert_eq!(intent, before);
    }

    #[test]
    fn coordination_errors_keep_one_representation() {
        assert_eq!(
            IntentError::from(CoordinationError::IntentNotQueued),
            IntentError::NotQueued
        );
        assert_eq!(
            IntentError::from(CoordinationError::LockNotExpired),
            IntentError::LockNotExpired
        );
        assert_eq!(
            IntentError::from(CoordinationError::LockHeld),
            IntentError::Coordination(CoordinationError::LockHeld)
        );
    }

    // ---- Abandonment adapter ----------------------------------------------

    fn locked_pair(intent: &OperationIntent) -> (Coordination, Coordination) {
        let mut a = Coordination::default();
        let mut b = Coordination::default();
        acquire_locks(
            intent.lock_owner(),
            intent.lock_expiry_slot,
            &mut [
                CoordinationSlot::new(key(1), &mut a),
                CoordinationSlot::new(key(2), &mut b),
            ],
        )
        .unwrap();
        (a, b)
    }

    #[test]
    fn abandon_intent_bumps_generations_clears_locks_and_marks_abandoned() {
        let mut intent = queued();
        let (mut a, mut b) = locked_pair(&intent);

        let cleared = abandon_intent(
            &mut intent,
            201,
            &mut [
                CoordinationSlot::new(key(1), &mut a),
                CoordinationSlot::new(key(2), &mut b),
            ],
        );
        assert_eq!(cleared, Ok(2));
        assert_eq!(intent.status, IntentStatus::Abandoned);
        assert_eq!((a.generation, b.generation), (1, 1));
        assert!(!a.lock.is_active());
        assert!(!b.lock.is_active());
    }

    #[test]
    fn abandon_intent_before_lock_expiry_changes_nothing() {
        let mut intent = queued();
        let (mut a, mut b) = locked_pair(&intent);
        let before = (intent, a, b);

        let result = abandon_intent(
            &mut intent,
            200,
            &mut [
                CoordinationSlot::new(key(1), &mut a),
                CoordinationSlot::new(key(2), &mut b),
            ],
        );
        assert_eq!(result, Err(IntentError::LockNotExpired));
        assert_eq!((intent, a, b), before);
    }

    #[test]
    fn abandon_intent_rejects_a_terminal_intent_without_touching_locks() {
        let mut intent = terminal(IntentStatus::Failed);
        let (mut a, mut b) = locked_pair(&intent);
        let before = (a, b);

        let result = abandon_intent(
            &mut intent,
            999,
            &mut [
                CoordinationSlot::new(key(1), &mut a),
                CoordinationSlot::new(key(2), &mut b),
            ],
        );
        assert_eq!(result, Err(IntentError::NotQueued));
        assert_eq!((a, b), before);
        assert_eq!(intent.status, IntentStatus::Failed);
    }

    #[test]
    fn abandon_intent_is_atomic_when_a_generation_would_overflow() {
        let mut intent = queued();
        let (mut a, mut b) = locked_pair(&intent);
        b.generation = u64::MAX;
        let before = (intent, a, b);

        let result = abandon_intent(
            &mut intent,
            201,
            &mut [
                CoordinationSlot::new(key(1), &mut a),
                CoordinationSlot::new(key(2), &mut b),
            ],
        );
        assert_eq!(
            result,
            Err(IntentError::Coordination(
                CoordinationError::GenerationOverflow
            ))
        );
        assert_eq!((intent, a, b), before);
    }

    // ---- Reclaim -----------------------------------------------------------

    const TERMINALS: [IntentStatus; 6] = [
        IntentStatus::Applied,
        IntentStatus::Zero,
        IntentStatus::Failed,
        IntentStatus::Stale,
        IntentStatus::Expired,
        IntentStatus::Abandoned,
    ];

    #[test]
    fn reclaim_is_refused_while_queued() {
        let intent = queued();
        for kind in [
            ReclaimKind::ComputationRent,
            ReclaimKind::ExpiredComputationFee,
        ] {
            assert_eq!(
                check_reclaim(&intent, &key(2), kind),
                Err(IntentError::ReclaimBeforeTerminal)
            );
            assert!(!can_reclaim(&intent, &key(2), kind));
        }
    }

    #[test]
    fn reclaim_needs_the_recorded_reclaim_authority() {
        for status in TERMINALS {
            let intent = terminal(status);
            // The owner is not the reclaim authority; neither is a stranger.
            for signer in [key(1), key(99), Pubkey::default()] {
                assert_eq!(
                    check_reclaim(&intent, &signer, ReclaimKind::ComputationRent),
                    Err(IntentError::ReclaimNotAuthorized)
                );
            }
        }
    }

    #[test]
    fn computation_rent_reclaim_is_allowed_after_every_terminal_status() {
        for status in TERMINALS {
            let intent = terminal(status);
            assert_eq!(
                check_reclaim(&intent, &key(2), ReclaimKind::ComputationRent),
                Ok(())
            );
        }
    }

    #[test]
    fn expired_fee_reclaim_is_allowed_only_after_expired_or_abandoned() {
        for status in TERMINALS {
            let intent = terminal(status);
            let result = check_reclaim(&intent, &key(2), ReclaimKind::ExpiredComputationFee);
            if matches!(status, IntentStatus::Expired | IntentStatus::Abandoned) {
                assert_eq!(result, Ok(()));
            } else {
                assert_eq!(result, Err(IntentError::ReclaimNotAllowedForStatus));
            }
        }
    }

    // ---- Property tests (run via `cargo test -- prop_`) -------------------

    /// Deterministic linear-congruential sampler (no external crate).
    struct Lcg(u64);
    impl Lcg {
        fn sample(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 11
        }
    }

    /// An independent oracle for which `(transition, actor, slot)` triples a
    /// queued Intent (decision 100, lock 200) accepts.
    fn oracle_allows(transition: Transition, actor: Actor, slot: Slot) -> bool {
        let actor_ok = match transition {
            Transition::Abandoned => actor == Actor::Signer,
            _ => actor == Actor::VerifiedCallback,
        };
        let timing_ok = match transition {
            Transition::Applied | Transition::Zero => slot <= 100,
            Transition::Expired => slot > 100,
            Transition::Failed | Transition::Stale => true,
            Transition::Abandoned => slot > 200,
        };
        actor_ok && timing_ok
    }

    /// **Validates: Requirements 3.4, 3.6** — terminal states are write-once,
    /// and only the right actor at the right time moves an Intent out of
    /// `Queued`.
    #[test]
    fn prop_terminal_states_are_write_once_and_actor_gated() {
        let mut rng = Lcg(0x0A11_0001);
        for _ in 0..4_000 {
            let mut intent = queued();
            for _ in 0..8 {
                let transition = Transition::ALL[(rng.sample() % 6) as usize];
                let actor = if rng.sample().is_multiple_of(2) {
                    Actor::VerifiedCallback
                } else {
                    Actor::Signer
                };
                let slot = rng.sample() % 300;

                let before = intent;
                let result = intent.transition(transition, actor, slot);
                if before.status.is_terminal() {
                    assert_eq!(result, Err(IntentError::NotQueued));
                    assert_eq!(intent, before);
                } else if oracle_allows(transition, actor, slot) {
                    assert_eq!(result, Ok(()));
                    assert_eq!(intent.status, transition.status());
                } else {
                    assert!(result.is_err());
                    assert_eq!(intent, before);
                }
            }
        }
    }

    /// **Validates: Requirements 3.4** — once terminal, the status never
    /// changes again, whatever is attempted.
    #[test]
    fn prop_a_terminal_status_never_changes() {
        let mut rng = Lcg(0x0A11_0002);
        for _ in 0..3_000 {
            let first = Transition::ALL[(rng.sample() % 6) as usize];
            let mut intent = queued();
            intent
                .transition(first, first.required_actor(), valid_slot(first))
                .unwrap();
            let terminal_status = intent.status;

            for _ in 0..10 {
                let transition = Transition::ALL[(rng.sample() % 6) as usize];
                let actor = if rng.sample().is_multiple_of(2) {
                    Actor::VerifiedCallback
                } else {
                    Actor::Signer
                };
                let _ = intent.transition(transition, actor, rng.sample() % 400);
                assert_eq!(intent.status, terminal_status);
            }
        }
    }

    /// **Validates: Requirements 3.11** — reclaim is never allowed before a
    /// terminal state, and never to anyone but the reclaim authority.
    #[test]
    fn prop_reclaim_needs_a_terminal_status_and_the_reclaim_authority() {
        let all_statuses = [
            IntentStatus::Queued,
            IntentStatus::Applied,
            IntentStatus::Zero,
            IntentStatus::Failed,
            IntentStatus::Stale,
            IntentStatus::Expired,
            IntentStatus::Abandoned,
        ];
        let mut rng = Lcg(0x0A11_0003);
        for _ in 0..4_000 {
            let status = all_statuses[(rng.sample() % 7) as usize];
            let mut intent = terminal(status);
            intent.reclaim_authority = key((rng.sample() % 4) as u8);
            let signer = key((rng.sample() % 4) as u8);
            let kind = if rng.sample().is_multiple_of(2) {
                ReclaimKind::ComputationRent
            } else {
                ReclaimKind::ExpiredComputationFee
            };

            let allowed = can_reclaim(&intent, &signer, kind);
            if status == IntentStatus::Queued || signer != intent.reclaim_authority {
                assert!(!allowed);
            }
            let expected = status != IntentStatus::Queued
                && signer == intent.reclaim_authority
                && (kind == ReclaimKind::ComputationRent
                    || matches!(status, IntentStatus::Expired | IntentStatus::Abandoned));
            assert_eq!(allowed, expected);
        }
    }
}
