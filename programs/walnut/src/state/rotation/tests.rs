use std::collections::HashSet;

use super::*;
use crate::operation::callback::CallbackError;
use crate::state::nonce::output_nonce;

const OWNER: Pubkey = Pubkey::new_from_array([0x11; 32]);
const STRANGER: Pubkey = Pubkey::new_from_array([0x12; 32]);
const MARKET_KEY: Pubkey = Pubkey::new_from_array([0x22; 32]);
const POSITION_KEY: Pubkey = Pubkey::new_from_array([0x23; 32]);
const VIEWING: [u8; 32] = [0x33; 32];

fn intent_key(sequence: u64) -> Pubkey {
    let mut bytes = [0x60; 32];
    bytes[..8].copy_from_slice(&sequence.to_le_bytes());
    Pubkey::new_from_array(bytes)
}

fn c<T>(result: Result<T, ErrorCode>) -> Result<T, u32> {
    result.map_err(u32::from)
}

fn code(error: ErrorCode) -> u32 {
    u32::from(error)
}

/// A Position whose payload is "sealed" to `VIEWING`: the first ciphertext
/// byte tags the key it is sealed to, so tests can check that payload and key
/// always move together.
fn active_position() -> Position {
    let mut position = Position::blank(OWNER, MARKET_KEY, 0xFC);
    position.status = PositionStatus::Active;
    position.viewing_pubkey = VIEWING;
    position.coordination.generation = 1;
    position.encrypted_state = EncryptedPositionPayload {
        nonce: 1_000,
        ciphertexts: [[VIEWING[0]; 32]; 5],
    };
    position
}

/// The comparable part of a Position (it is not `PartialEq`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Snap {
    viewing_pubkey: [u8; 32],
    viewing_key_epoch: u64,
    encrypted_state: EncryptedPositionPayload,
    coordination: super::super::Coordination,
    status: PositionStatus,
}

fn snap(p: &Position) -> Snap {
    Snap {
        viewing_pubkey: p.viewing_pubkey,
        viewing_key_epoch: p.viewing_key_epoch,
        encrypted_state: p.encrypted_state,
        coordination: p.coordination,
        status: p.status,
    }
}
struct World {
    position: Position,
    counter: OperationCounter,
}

fn world() -> World {
    World {
        position: active_position(),
        counter: OperationCounter::new(OWNER, 0xFA),
    }
}

impl World {
    fn request(&self, new_key: [u8; 32], slot: Slot) -> RotationRequest {
        let sequence = self.counter.next_sequence;
        RotationRequest {
            intent_key: intent_key(sequence),
            sequence,
            computation_offset: 9_000 + sequence,
            queued_slot: slot,
            expected_old_epoch: self.position.viewing_key_epoch,
            expected_new_epoch: self.position.viewing_key_epoch + 1,
            expected_old_pubkey: self.position.viewing_pubkey,
            new_viewing_pubkey: new_key,
        }
    }

    fn queue_request(
        &mut self,
        request: &RotationRequest,
        signer: &Pubkey,
    ) -> Result<OperationIntent, ErrorCode> {
        queue_rotation(
            &mut self.position,
            POSITION_KEY,
            &mut self.counter,
            signer,
            request,
        )
    }

    fn queue(&mut self, new_key: [u8; 32], slot: Slot) -> Result<OperationIntent, ErrorCode> {
        let request = self.request(new_key, slot);
        self.queue_request(&request, &OWNER)
    }

    /// The honest callback of `intent`: sealed to `new_key` under the derived
    /// output nonce, with the payload tagged by the new key.
    fn report(&self, intent: &OperationIntent, new_key: [u8; 32], slot: Slot) -> RotationReport {
        RotationReport {
            output_authenticated: true,
            computation_offset: intent.computation_offset,
            intent_key: intent.key,
            encryption_key: new_key,
            output_nonce: output_nonce(&POSITION_KEY, intent.sequence),
            ciphertexts: [[new_key[0]; 32]; 5],
            current_slot: slot,
        }
    }

    fn settle(
        &mut self,
        intent: &mut OperationIntent,
        report: &RotationReport,
    ) -> Result<RotationSettlement, ErrorCode> {
        settle_rotation(&mut self.position, POSITION_KEY, intent, report)
    }
}

fn key(n: u8) -> [u8; 32] {
    [n; 32]
}

#[test]
fn queue_binds_the_intent_and_locks_the_position() {
    let mut w = world();
    let intent = w.queue(key(0x44), 10).unwrap();
    assert_eq!(intent.kind, OperationKind::ViewingKeyRotation);
    assert_eq!(intent.status, IntentStatus::Queued);
    assert_eq!(intent.owner, OWNER);
    assert_eq!(intent.position, Some(POSITION_KEY));
    assert_eq!(intent.decision_expiry_slot, 10 + ROTATION_DECISION_SLOTS);
    assert_eq!(
        intent.lock_expiry_slot,
        10 + ROTATION_DECISION_SLOTS + ROTATION_LOCK_GRACE_SLOTS
    );
    assert!(intent.lock_owner().holds(&w.position.coordination.lock));
    assert_eq!(w.counter.next_sequence, 1);
    // Nothing changes until the callback: the old key and epoch stay valid.
    assert_eq!(w.position.viewing_pubkey, VIEWING);
    assert_eq!(w.position.viewing_key_epoch, 0);
    assert_eq!(
        w.position.encrypted_state,
        active_position().encrypted_state
    );
}

#[test]
fn commitment_binds_every_rotation_field() {
    let mut w = world();
    let intent = w.queue(key(0x44), 10).unwrap();
    let keys = RotationKeys {
        old_pubkey: VIEWING,
        old_epoch: 0,
        new_pubkey: key(0x44),
        new_epoch: 1,
    };
    let nonce_in = derive_output_nonce_input(&POSITION_KEY, intent.sequence);
    let state = active_position().encrypted_state;
    let base = |keys: &RotationKeys, state: &EncryptedPositionPayload, nonce: u128| {
        input_commitment(
            OperationKind::ViewingKeyRotation,
            &rotation_inputs(keys, state, nonce),
        )
    };
    assert_eq!(intent.inputs_commitment, base(&keys, &state, nonce_in));
    let mut other_state = state;
    other_state.ciphertexts[4][0] ^= 1;
    let variants = [
        base(
            &RotationKeys {
                new_pubkey: key(0x45),
                ..keys
            },
            &state,
            nonce_in,
        ),
        base(
            &RotationKeys {
                new_epoch: 2,
                ..keys
            },
            &state,
            nonce_in,
        ),
        base(
            &RotationKeys {
                old_epoch: 1,
                ..keys
            },
            &state,
            nonce_in,
        ),
        base(
            &RotationKeys {
                old_pubkey: key(0x34),
                ..keys
            },
            &state,
            nonce_in,
        ),
        base(&keys, &other_state, nonce_in),
        base(&keys, &state, nonce_in + 1),
    ];
    for variant in variants {
        assert_ne!(variant, intent.inputs_commitment);
    }
    // The kind is part of the domain: an unshield-style commitment differs.
    assert_ne!(
        input_commitment(
            OperationKind::Unshield,
            &rotation_inputs(&keys, &state, nonce_in)
        ),
        intent.inputs_commitment
    );
}

#[test]
fn queue_rejects_bad_requests_atomically() {
    let new = key(0x44);
    let mut w = world();
    let good = w.request(new, 10);
    let before = (snap(&w.position), w.counter);

    // Not the owner.
    assert_eq!(
        c(w.queue_request(&good, &STRANGER).map(|_| ())),
        Err(code(ErrorCode::RotationNotOwner))
    );
    // Zero new key.
    let zero = RotationRequest {
        new_viewing_pubkey: [0; 32],
        ..good
    };
    assert_eq!(
        c(w.queue_request(&zero, &OWNER).map(|_| ())),
        Err(code(ErrorCode::InvalidViewingKey))
    );
    // New key equals the old key.
    let same = RotationRequest {
        new_viewing_pubkey: VIEWING,
        ..good
    };
    assert_eq!(
        c(w.queue_request(&same, &OWNER).map(|_| ())),
        Err(code(ErrorCode::RotationKeyUnchanged))
    );
    // Wrong expected old key.
    let old_key = RotationRequest {
        expected_old_pubkey: key(0x01),
        ..good
    };
    assert_eq!(
        c(w.queue_request(&old_key, &OWNER).map(|_| ())),
        Err(code(ErrorCode::RotationOldKeyMismatch))
    );
    // Wrong expected epochs: stale old, skipped new, repeated new.
    for (old, new_epoch) in [(1, 2), (0, 2), (0, 0), (0, 5)] {
        let epochs = RotationRequest {
            expected_old_epoch: old,
            expected_new_epoch: new_epoch,
            ..good
        };
        assert_eq!(
            c(w.queue_request(&epochs, &OWNER).map(|_| ())),
            Err(code(ErrorCode::RotationEpochMismatch)),
            "({old}, {new_epoch})"
        );
    }
    // The Position must be Active.
    for status in [
        PositionStatus::Uninitialized,
        PositionStatus::Initializing,
        PositionStatus::InitFailed,
    ] {
        w.position.status = status;
        assert_eq!(
            c(w.queue_request(&good, &OWNER).map(|_| ())),
            Err(code(ErrorCode::RotationPositionNotActive))
        );
    }
    w.position.status = PositionStatus::Active;
    // Nothing above changed anything.
    assert_eq!(snap(&w.position), before.0);
    assert_eq!(w.counter, before.1);

    // Epoch overflow: the epoch cannot be incremented.
    w.position.viewing_key_epoch = u64::MAX;
    let overflow = RotationRequest {
        expected_old_epoch: u64::MAX,
        expected_new_epoch: 0,
        ..good
    };
    assert_eq!(
        c(w.queue_request(&overflow, &OWNER).map(|_| ())),
        Err(code(ErrorCode::RotationEpochOverflow))
    );
    w.position.viewing_key_epoch = 0;

    // A held Lock rejects a second queue.
    w.queue_request(&good, &OWNER).unwrap();
    let again = w.request(key(0x55), 11);
    assert_eq!(
        c(w.queue_request(&again, &OWNER).map(|_| ())),
        Err(code(ErrorCode::LockHeld))
    );
}

#[test]
fn applied_callback_changes_payload_key_and_epoch_together() {
    let mut w = world();
    let mut intent = w.queue(key(0x44), 10).unwrap();
    let report = w.report(&intent, key(0x44), 11);
    let generation = w.position.coordination.generation;
    assert_eq!(
        c(w.settle(&mut intent, &report)),
        Ok(RotationSettlement::Rotated {
            old_epoch: 0,
            new_epoch: 1,
            old_pubkey: VIEWING,
            new_pubkey: key(0x44),
        })
    );
    assert_eq!(intent.status, IntentStatus::Applied);
    assert_eq!(w.position.viewing_pubkey, key(0x44));
    assert_eq!(w.position.viewing_key_epoch, 1);
    assert_eq!(w.position.encrypted_state.nonce, report.output_nonce);
    assert_eq!(w.position.encrypted_state.ciphertexts, report.ciphertexts);
    assert_eq!(w.position.coordination.generation, generation + 1);
    assert!(!w.position.coordination.lock.is_active());
    assert_eq!(w.position.status, PositionStatus::Active);

    // A replayed callback is a no-op.
    let snapshot = snap(&w.position);
    assert_eq!(
        c(w.settle(&mut intent, &report)),
        Ok(RotationSettlement::Ignored)
    );
    assert_eq!(snap(&w.position), snapshot);
}

#[test]
fn consecutive_rotations_advance_one_epoch_each_with_fresh_nonces() {
    let mut w = world();
    let mut seen = HashSet::from([(VIEWING, 1_000u128)]);
    for round in 0..5u8 {
        let new = key(0x50 + round);
        let mut intent = w.queue(new, 10 + u64::from(round)).unwrap();
        let report = w.report(&intent, new, 11 + u64::from(round));
        assert!(seen.insert((report.encryption_key, report.output_nonce)));
        w.settle(&mut intent, &report).unwrap();
        assert_eq!(w.position.viewing_key_epoch, u64::from(round) + 1);
        assert_eq!(w.position.viewing_pubkey, new);
    }
}

#[test]
fn non_applied_outcomes_keep_the_old_key_epoch_and_payload() {
    let before = active_position();
    // Expired: delivered after Decision Expiry.
    let mut w = world();
    let mut intent = w.queue(key(0x44), 10).unwrap();
    let late = intent.decision_expiry_slot + 1;
    let report = w.report(&intent, key(0x44), late);
    assert_eq!(
        c(w.settle(&mut intent, &report)),
        Ok(RotationSettlement::Failed(IntentStatus::Expired))
    );
    assert_unchanged_but_unlocked(&w.position, &before, 0);

    // Stale: the Position generation moved since the Intent captured it.
    let mut w = world();
    let mut intent = w.queue(key(0x44), 10).unwrap();
    w.position.coordination.generation += 1;
    let report = w.report(&intent, key(0x44), 11);
    assert_eq!(
        c(w.settle(&mut intent, &report)),
        Ok(RotationSettlement::Failed(IntentStatus::Stale))
    );
    assert_unchanged_but_unlocked(&w.position, &before, 1);

    // After a failed attempt the owner can queue a new one, reusing the key
    // under a new sequence and so a new output nonce.
    let mut again = w.queue(key(0x44), 20).unwrap();
    let report = w.report(&again, key(0x44), 21);
    w.settle(&mut again, &report).unwrap();
    assert_eq!(w.position.viewing_key_epoch, 1);
}

fn assert_unchanged_but_unlocked(position: &Position, before: &Position, extra_generation: u64) {
    assert_eq!(position.viewing_pubkey, before.viewing_pubkey);
    assert_eq!(position.viewing_key_epoch, before.viewing_key_epoch);
    assert_eq!(position.encrypted_state, before.encrypted_state);
    assert!(!position.coordination.lock.is_active());
    assert!(position.coordination.generation >= before.coordination.generation + extra_generation);
}

#[test]
fn unauthenticated_output_reverts_and_leaves_the_intent_queued() {
    let mut w = world();
    let mut intent = w.queue(key(0x44), 10).unwrap();
    let report = RotationReport {
        output_authenticated: false,
        ..w.report(&intent, key(0x44), 11)
    };
    assert_eq!(
        c(w.settle(&mut intent, &report)),
        Err(code(ErrorCode::CallbackOutputNotAuthenticated))
    );
    assert_eq!(intent.status, IntentStatus::Queued);
    assert!(w.position.coordination.lock.is_active());
    assert_eq!(w.position.viewing_key_epoch, 0);
    assert_eq!(w.position.viewing_pubkey, VIEWING);
}

#[test]
fn substituted_keys_nonces_and_intents_are_rejected() {
    let mismatch = code(ErrorCode::from(CallbackError::InputsCommitmentMismatch));
    let reject = |tweak: &dyn Fn(&mut RotationReport), expect: u32| {
        let mut w = world();
        let mut intent = w.queue(key(0x44), 10).unwrap();
        let mut report = w.report(&intent, key(0x44), 11);
        tweak(&mut report);
        let snapshot = (snap(&w.position), intent);
        assert_eq!(c(w.settle(&mut intent, &report)), Err(expect));
        assert_eq!((snap(&w.position), intent), snapshot);
    };
    // Sealed to a different key than the committed new key.
    reject(&|r| r.encryption_key = key(0x45), mismatch);
    // Sealed to the old key (a no-op rotation).
    reject(&|r| r.encryption_key = VIEWING, mismatch);
    // Zero key.
    reject(&|r| r.encryption_key = [0; 32], mismatch);
    // An output nonce other than the derived one.
    reject(&|r| r.output_nonce += 1, mismatch);
    reject(&|r| r.output_nonce = 1_001, mismatch);
    // A different Intent or computation offset.
    reject(
        &|r| r.intent_key = intent_key(99),
        code(ErrorCode::from(CallbackError::IntentKeyMismatch)),
    );
    reject(
        &|r| r.computation_offset += 1,
        code(ErrorCode::from(CallbackError::OffsetMismatch)),
    );

    // An Intent whose committed new epoch is not old + 1 can never apply.
    let mut w = world();
    let request = RotationRequest {
        expected_new_epoch: 2,
        ..w.request(key(0x44), 10)
    };
    let mut intent = build_rotation_intent(&w.position, POSITION_KEY, &request).unwrap();
    w.counter.next_sequence = 1;
    acquire_locks(
        intent.lock_owner(),
        intent.lock_expiry_slot,
        &mut [CoordinationSlot::new(
            POSITION_KEY,
            &mut w.position.coordination,
        )],
    )
    .unwrap();
    let report = w.report(&intent, key(0x44), 11);
    assert_eq!(c(w.settle(&mut intent, &report)), Err(mismatch));
    assert_eq!(w.position.viewing_key_epoch, 0);
}

#[test]
fn abandon_requires_lock_expiry_and_changes_nothing_else() {
    let mut w = world();
    let mut intent = w.queue(key(0x44), 10).unwrap();
    assert!(abandon_rotation(&mut w.position, POSITION_KEY, &mut intent, 11).is_err());
    assert!(w.position.coordination.lock.is_active());
    let expired = intent.lock_expiry_slot + 1;
    abandon_rotation(&mut w.position, POSITION_KEY, &mut intent, expired).unwrap();
    assert_eq!(intent.status, IntentStatus::Abandoned);
    assert!(!w.position.coordination.lock.is_active());
    assert_eq!(w.position.viewing_pubkey, VIEWING);
    assert_eq!(w.position.viewing_key_epoch, 0);
    assert_eq!(
        w.position.encrypted_state,
        active_position().encrypted_state
    );
    // A late callback of the abandoned attempt changes nothing.
    let report = w.report(&intent, key(0x44), expired + 1);
    assert_eq!(
        c(w.settle(&mut intent, &report)),
        Ok(RotationSettlement::Ignored)
    );
    assert_eq!(w.position.viewing_key_epoch, 0);
    // A terminal Intent cannot be abandoned twice; a foreign one is rejected.
    assert_eq!(
        c(abandon_rotation(&mut w.position, POSITION_KEY, &mut intent, 99_999).map(|_| ())),
        Err(code(ErrorCode::IntentNotQueued))
    );
    let mut other = w.queue(key(0x46), 20).unwrap();
    assert_eq!(
        c(abandon_rotation(&mut w.position, STRANGER, &mut other, 99_999).map(|_| ())),
        Err(code(ErrorCode::RotationIntentMismatch))
    );
}

#[test]
fn rotation_is_not_pausable() {
    use crate::market::pause::{action_pause_category, pause_category, Action};
    assert_eq!(
        pause_category(OperationKind::ViewingKeyRotation),
        None,
        "queue_rotation takes no pause state"
    );
    assert_eq!(action_pause_category(Action::ViewingKeyRotation), None);
}

// ---- Property: the key, epoch, and payload move together or not at all ------

struct Lcg(u64);

impl Lcg {
    fn sample(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, n: u64) -> u64 {
        self.sample() % n
    }
}

/// Random rotation sequences through the real rules: valid and invalid
/// queues, applied, expired, stale, unauthenticated, substituted-key,
/// replayed, and abandoned callbacks. After every step:
///
/// * the epoch only ever increases, by exactly 1, and only on `Rotated`;
/// * the stored payload is always sealed to the stored key (the payload tag
///   equals the key tag), so key and payload change atomically or not at all;
/// * `Failed`, `Ignored`, rejected, and unauthenticated steps change neither
///   key, epoch, nor payload;
/// * a substituted new key or a wrong epoch is never accepted;
/// * no output (key, nonce) pair repeats across any attempt, applied or not.
///
/// **Validates: Requirements 5.10, 5.11, 2.3**
#[test]
fn prop_rotation_is_atomic_and_never_reuses_an_output_nonce() {
    let mut rng = Lcg(0x0707_5EED);
    for _ in 0..300 {
        let mut w = world();
        let mut slot: Slot = 10;
        let mut inflight: Option<(OperationIntent, [u8; 32])> = None;
        let mut outputs = HashSet::from([(VIEWING, 1_000u128)]);
        let mut expected_epoch = 0u64;
        for _ in 0..40 {
            let before = snap(&w.position);
            slot += 1 + rng.below(5);
            let tag_matches =
                |p: &Position| p.encrypted_state.ciphertexts[0][0] == p.viewing_pubkey[0];
            match (rng.below(9), inflight.take()) {
                (0..=2, None) => {
                    let new = key(0x40 + rng.below(8) as u8);
                    let mut request = w.request(new, slot);
                    // Sometimes lie about the epochs or the old key.
                    let lie = rng.below(4);
                    match lie {
                        0 => request.expected_old_epoch += 1,
                        1 => request.expected_new_epoch += 1,
                        2 => request.expected_old_pubkey = key(0x01),
                        _ => {}
                    }
                    let result = w.queue_request(&request, &OWNER);
                    let valid = lie == 3 && new != w.position.viewing_pubkey;
                    assert_eq!(result.is_ok(), valid, "lie {lie}");
                    match result {
                        Ok(intent) => inflight = Some((intent, new)),
                        Err(_) => assert_eq!(snap(&w.position), before),
                    }
                }
                (0..=2, Some(op)) => {
                    // The Lock is held: a new rotation is rejected untouched.
                    let request = w.request(key(0x4F), slot);
                    assert_eq!(
                        c(w.queue_request(&request, &OWNER).map(|_| ())),
                        Err(code(ErrorCode::LockHeld))
                    );
                    assert_eq!(snap(&w.position), before);
                    inflight = Some(op);
                }
                (3 | 4, Some((mut intent, new))) => {
                    // Honest, on time.
                    let report = w.report(&intent, new, slot.min(intent.decision_expiry_slot));
                    assert!(outputs.insert((report.encryption_key, report.output_nonce)));
                    let settled = w.settle(&mut intent, &report).unwrap();
                    assert!(matches!(settled, RotationSettlement::Rotated { .. }));
                    expected_epoch += 1;
                    inflight = None;
                    // A replay is ignored.
                    let after = snap(&w.position);
                    assert_eq!(
                        c(w.settle(&mut intent, &report)),
                        Ok(RotationSettlement::Ignored)
                    );
                    assert_eq!(snap(&w.position), after);
                }
                (5, Some((mut intent, new))) => {
                    // Expired or stale: nothing changes but the Lock.
                    let stale = rng.below(2) == 0;
                    let at = if stale {
                        w.position.coordination.generation += 1;
                        slot.min(intent.decision_expiry_slot)
                    } else {
                        intent.decision_expiry_slot + 1
                    };
                    let report = w.report(&intent, new, at);
                    assert!(outputs.insert((report.encryption_key, report.output_nonce)));
                    let settled = w.settle(&mut intent, &report).unwrap();
                    assert!(matches!(settled, RotationSettlement::Failed(_)));
                    assert_eq!(w.position.viewing_pubkey, before.viewing_pubkey);
                    assert_eq!(w.position.encrypted_state, before.encrypted_state);
                    inflight = None;
                }
                (6, Some((mut intent, new))) => {
                    // Unauthenticated or substituted: rejected, still queued.
                    let mut report = w.report(&intent, new, slot.min(intent.decision_expiry_slot));
                    if rng.below(2) == 0 {
                        report.output_authenticated = false;
                    } else {
                        report.encryption_key = key(0x7F);
                    }
                    assert!(w.settle(&mut intent, &report).is_err());
                    assert_eq!(snap(&w.position), before);
                    assert_eq!(intent.status, IntentStatus::Queued);
                    inflight = Some((intent, new));
                }
                (7, Some((mut intent, _))) => {
                    // Abandonment needs Lock expiry; then nothing changes.
                    let (early, expired) = (intent.queued_slot + 1, intent.lock_expiry_slot + 1);
                    assert!(
                        abandon_rotation(&mut w.position, POSITION_KEY, &mut intent, early)
                            .is_err()
                    );
                    abandon_rotation(&mut w.position, POSITION_KEY, &mut intent, expired).unwrap();
                    assert_eq!(w.position.viewing_pubkey, before.viewing_pubkey);
                    assert_eq!(w.position.encrypted_state, before.encrypted_state);
                    inflight = None;
                }
                (_, op) => inflight = op,
            }
            assert_eq!(w.position.viewing_key_epoch, expected_epoch);
            assert!(
                w.position.viewing_key_epoch == before.viewing_key_epoch
                    || w.position.viewing_key_epoch == before.viewing_key_epoch + 1
            );
            assert!(tag_matches(&w.position), "payload sealed to the stored key");
            assert_eq!(w.position.status, PositionStatus::Active);
            if inflight.is_some() {
                assert!(w.position.coordination.lock.is_active());
            }
        }
    }
}
