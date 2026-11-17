//! Operation Intent addressing and sequences (Requirement 3.1).
//!
//! Every queued computation creates one Operation Intent PDA. The address must
//! be unique and must never be reusable, so replaying a queue request, or
//! queueing a second operation under an old address, is impossible.
//!
//! # Seed scheme
//!
//! ```text
//! OperationIntent  = PDA[ "operation_intent" | owner (32) | sequence (u64 LE) ]
//! OperationCounter = PDA[ "operation_counter" | owner (32) ]
//! ```
//!
//! * The program-level constant separates Intents from every other Walnut PDA.
//! * `owner` is the operation owner. Two owners can never share an address.
//! * `sequence` is a monotonically increasing number taken from the owner's
//!   [`OperationCounter`]. One sequence value is consumed per Intent, so two
//!   Intents of one owner never share an address.
//! * The operation kind is deliberately **not** a seed. If it were, one sequence
//!   number could be reused for a different kind and the counter would no
//!   longer make addresses single-use. A sequence belongs to exactly one
//!   operation, whatever its kind.
//!
//! Anchor accounts structs use the same bytes:
//! `seeds = [OPERATION_INTENT_SEED, owner.key().as_ref(), &sequence.to_le_bytes()]`.
//!
//! # Sequence rule
//!
//! The queue instruction passes the sequence it expects. It must equal the
//! counter's `next_sequence` exactly, and the counter then advances by one with
//! checked arithmetic ([`OperationCounter::consume`]). A smaller value is a
//! reuse ([`IntentError::SequenceReused`]), a larger value is out of order
//! ([`IntentError::SequenceOutOfOrder`]), and `u64::MAX` cannot be consumed
//! ([`IntentError::SequenceOverflow`]). Two transactions that race for the same
//! sequence cannot both succeed, because the second sees the advanced counter.
//!
//! The design's `Position.next_operation_seq` and this counter are the same
//! idea. The production Position (Task 5) is a separate account, and
//! owners such as keepers and the Market have no Position, so the counter is a
//! small account of its own: 49 bytes (8 discriminator, 32 owner, 8
//! `next_sequence`, 1 bump).

use anchor_lang::prelude::{
    account, AnchorDeserialize, AnchorSerialize, Discriminator, InitSpace, Pubkey, Space,
};

use super::intent::OperationIntent;
use super::ownership::IntentError;

/// Program-level seed constant for Operation Intent PDAs.
pub const OPERATION_INTENT_SEED: &[u8] = b"operation_intent";

/// Program-level seed constant for per-owner sequence counters.
pub const OPERATION_COUNTER_SEED: &[u8] = b"operation_counter";

/// The seed bytes of an Operation Intent PDA, ready to pass to
/// `find_program_address` or to an Anchor `seeds = [...]` constraint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntentSeeds {
    owner: [u8; 32],
    sequence: [u8; 8],
}

impl IntentSeeds {
    /// Builds the seeds for `owner` and `sequence`.
    pub fn new(owner: &Pubkey, sequence: u64) -> Self {
        Self {
            owner: owner.to_bytes(),
            sequence: sequence.to_le_bytes(),
        }
    }

    /// The seed slices, in order: constant, owner, sequence.
    pub fn as_slices(&self) -> [&[u8]; 3] {
        [OPERATION_INTENT_SEED, &self.owner, &self.sequence]
    }
}

/// Derives the Operation Intent address and bump for `owner` and `sequence`
/// under `program_id`.
///
/// Uses the same derivation as `Pubkey::find_program_address` but needs no
/// syscall, so it also runs in host tests. On chain, Anchor's `seeds`
/// constraint is the authority; this is for clients and tests.
pub fn derive_intent_address(owner: &Pubkey, sequence: u64, program_id: &Pubkey) -> (Pubkey, u8) {
    let seeds = IntentSeeds::new(owner, sequence);
    let (address, bump) =
        anchor_lang::derive_program_address(&seeds.as_slices(), &program_id.to_bytes());
    (Pubkey::new_from_array(address), bump)
}

/// Derives the sequence counter address and bump for `owner` under `program_id`.
pub fn derive_counter_address(owner: &Pubkey, program_id: &Pubkey) -> (Pubkey, u8) {
    let owner_bytes = owner.to_bytes();
    let (address, bump) = anchor_lang::derive_program_address(
        &[OPERATION_COUNTER_SEED, &owner_bytes],
        &program_id.to_bytes(),
    );
    (Pubkey::new_from_array(address), bump)
}

/// A per-owner counter that hands out Operation Intent sequence numbers.
///
/// Serialized layout (49 bytes with the discriminator): discriminator 0..8,
/// `owner` 8..40, `next_sequence` 40..48 (little-endian), `bump` 48.
#[account]
#[derive(InitSpace, Copy, Debug, Default, PartialEq, Eq)]
pub struct OperationCounter {
    /// The owner this counter belongs to. Part of the PDA seeds.
    pub owner: Pubkey,
    /// The sequence the next Intent of this owner must use.
    pub next_sequence: u64,
    /// The PDA bump.
    pub bump: u8,
}

impl OperationCounter {
    /// Account data length including Anchor's 8-byte discriminator.
    pub const ACCOUNT_LEN: usize = 8 + Self::INIT_SPACE;

    /// Creates a counter whose first sequence is zero.
    pub fn new(owner: Pubkey, bump: u8) -> Self {
        Self {
            owner,
            next_sequence: 0,
            bump,
        }
    }

    /// Consumes `sequence`, which must equal `next_sequence`, and advances the
    /// counter by one.
    ///
    /// The counter is unchanged on error.
    pub fn consume(&mut self, sequence: u64) -> Result<(), IntentError> {
        if sequence < self.next_sequence {
            return Err(IntentError::SequenceReused);
        }
        if sequence > self.next_sequence {
            return Err(IntentError::SequenceOutOfOrder);
        }
        let next = self
            .next_sequence
            .checked_add(1)
            .ok_or(IntentError::SequenceOverflow)?;
        self.next_sequence = next;
        Ok(())
    }
}

/// Validates a freshly populated Intent and reserves its sequence.
///
/// Runs [`OperationIntent::validate_new`], checks that the counter belongs to
/// the Intent's owner, then consumes `intent.sequence`. The counter is
/// unchanged on any error. The queue instruction calls this once, before it
/// writes the Intent and acquires Locks.
pub fn reserve_intent(
    counter: &mut OperationCounter,
    intent: &OperationIntent,
) -> Result<(), IntentError> {
    intent.validate_new()?;
    if counter.owner != intent.owner {
        return Err(IntentError::CounterOwnerMismatch);
    }
    counter.consume(intent.sequence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::intent::OperationKind;
    use crate::operation::sha256::sha256;
    use anchor_lang::{AccountDeserialize, AccountSerialize};

    fn key(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    fn program() -> Pubkey {
        crate::ID
    }

    fn valid_intent(owner: Pubkey, sequence: u64) -> OperationIntent {
        OperationIntent {
            key: key(0x40),
            owner,
            payer: key(0x50),
            reclaim_authority: key(0x50),
            kind: OperationKind::Supply,
            sequence,
            queued_slot: 10,
            decision_expiry_slot: 20,
            lock_expiry_slot: 30,
            ..OperationIntent::default()
        }
    }

    // ---- Seeds -------------------------------------------------------------

    #[test]
    fn seed_bytes_are_constant_owner_sequence() {
        let seeds = IntentSeeds::new(&key(7), 0x0102_0304_0506_0708);
        let slices = seeds.as_slices();
        assert_eq!(slices[0], b"operation_intent");
        assert_eq!(slices[1], [7u8; 32]);
        assert_eq!(slices[2], 0x0102_0304_0506_0708u64.to_le_bytes());
        // Every seed fits Solana's 32-byte limit.
        assert!(slices.iter().all(|seed| seed.len() <= 32));
    }

    #[test]
    fn distinct_owner_or_sequence_gives_distinct_seeds_and_addresses() {
        let base = derive_intent_address(&key(1), 0, &program());
        assert_ne!(base.0, derive_intent_address(&key(2), 0, &program()).0);
        assert_ne!(base.0, derive_intent_address(&key(1), 1, &program()).0);
        assert_ne!(
            derive_intent_address(&key(1), 1, &program()).0,
            derive_intent_address(&key(1), 256, &program()).0
        );
        // Deterministic.
        assert_eq!(base, derive_intent_address(&key(1), 0, &program()));
    }

    #[test]
    fn the_operation_kind_is_not_part_of_the_address() {
        // There is no kind parameter: one (owner, sequence) maps to one
        // address, so a sequence cannot be reused under another kind.
        let first = valid_intent(key(1), 5);
        let second = OperationIntent {
            kind: OperationKind::Borrow,
            ..first
        };
        assert_eq!(
            IntentSeeds::new(&first.owner, first.sequence),
            IntentSeeds::new(&second.owner, second.sequence)
        );
    }

    #[test]
    fn intent_and_counter_addresses_are_separated() {
        let owner = key(3);
        assert_ne!(
            derive_intent_address(&owner, 0, &program()).0,
            derive_counter_address(&owner, &program()).0
        );
        assert_ne!(
            derive_counter_address(&key(1), &program()).0,
            derive_counter_address(&key(2), &program()).0
        );
    }

    // ---- Sequence counter --------------------------------------------------

    #[test]
    fn sequences_are_consumed_in_order() {
        let mut counter = OperationCounter::new(key(1), 255);
        assert_eq!(counter.next_sequence, 0);
        assert_eq!(counter.consume(0), Ok(()));
        assert_eq!(counter.consume(1), Ok(()));
        assert_eq!(counter.next_sequence, 2);
    }

    #[test]
    fn sequence_reuse_is_rejected() {
        let mut counter = OperationCounter::new(key(1), 255);
        counter.consume(0).unwrap();
        counter.consume(1).unwrap();
        let before = counter;
        assert_eq!(counter.consume(0), Err(IntentError::SequenceReused));
        assert_eq!(counter.consume(1), Err(IntentError::SequenceReused));
        assert_eq!(counter, before);
    }

    #[test]
    fn a_skipped_sequence_is_rejected() {
        let mut counter = OperationCounter::new(key(1), 255);
        let before = counter;
        assert_eq!(counter.consume(1), Err(IntentError::SequenceOutOfOrder));
        assert_eq!(counter, before);
    }

    #[test]
    fn the_counter_cannot_overflow() {
        let mut counter = OperationCounter::new(key(1), 255);
        counter.next_sequence = u64::MAX - 1;
        assert_eq!(counter.consume(u64::MAX - 1), Ok(()));
        assert_eq!(counter.next_sequence, u64::MAX);

        let before = counter;
        assert_eq!(
            counter.consume(u64::MAX),
            Err(IntentError::SequenceOverflow)
        );
        assert_eq!(counter, before);
    }

    #[test]
    fn reserve_intent_consumes_the_sequence_once() {
        let mut counter = OperationCounter::new(key(1), 255);
        let intent = valid_intent(key(1), 0);
        assert_eq!(reserve_intent(&mut counter, &intent), Ok(()));
        assert_eq!(counter.next_sequence, 1);
        // Replaying the same Intent is rejected and changes nothing.
        assert_eq!(
            reserve_intent(&mut counter, &intent),
            Err(IntentError::SequenceReused)
        );
        assert_eq!(counter.next_sequence, 1);
    }

    #[test]
    fn reserve_intent_rejects_another_owners_counter() {
        let mut counter = OperationCounter::new(key(9), 255);
        let intent = valid_intent(key(1), 0);
        assert_eq!(
            reserve_intent(&mut counter, &intent),
            Err(IntentError::CounterOwnerMismatch)
        );
        assert_eq!(counter.next_sequence, 0);
    }

    #[test]
    fn reserve_intent_does_not_consume_for_an_invalid_intent() {
        let mut counter = OperationCounter::new(key(1), 255);
        let mut intent = valid_intent(key(1), 0);
        intent.decision_expiry_slot = intent.queued_slot;
        assert_eq!(
            reserve_intent(&mut counter, &intent),
            Err(IntentError::InvalidTiming)
        );
        assert_eq!(counter.next_sequence, 0);
    }

    // ---- Counter layout ----------------------------------------------------

    #[test]
    fn counter_space_and_byte_layout() {
        assert_eq!(OperationCounter::INIT_SPACE, 32 + 8 + 1);
        assert_eq!(OperationCounter::ACCOUNT_LEN, 49);

        let counter = OperationCounter {
            owner: key(0x11),
            next_sequence: 0x0102_0304_0506_0708,
            bump: 0xFD,
        };
        let mut bytes = Vec::new();
        counter.try_serialize(&mut bytes).unwrap();
        assert_eq!(bytes.len(), OperationCounter::ACCOUNT_LEN);
        assert_eq!(&bytes[..8], OperationCounter::DISCRIMINATOR);
        assert_eq!(bytes[8..40], [0x11; 32]);
        assert_eq!(bytes[40..48], 0x0102_0304_0506_0708u64.to_le_bytes());
        assert_eq!(bytes[48], 0xFD);

        let decoded = OperationCounter::try_deserialize(&mut bytes.as_slice()).unwrap();
        assert_eq!(decoded, counter);
    }

    #[test]
    fn counter_discriminator_is_the_anchor_account_hash() {
        let hash = sha256(b"account:OperationCounter");
        assert_eq!(OperationCounter::DISCRIMINATOR, &hash[..8]);
        assert_ne!(
            OperationCounter::DISCRIMINATOR,
            OperationIntent::DISCRIMINATOR
        );
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

        fn key(&mut self) -> Pubkey {
            let mut bytes = [0u8; 32];
            bytes[0..8].copy_from_slice(&self.sample().to_le_bytes());
            bytes[8..16].copy_from_slice(&self.sample().to_le_bytes());
            bytes[16..24].copy_from_slice(&self.sample().to_le_bytes());
            bytes[24..32].copy_from_slice(&self.sample().to_le_bytes());
            Pubkey::new_from_array(bytes)
        }
    }

    /// **Validates: Requirements 3.1** ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â distinct `(owner, sequence)` pairs
    /// never produce the same seed bytes, and the operation kind never changes
    /// them.
    #[test]
    fn prop_seed_bytes_are_unique_per_owner_and_sequence() {
        let mut rng = Lcg(0x5EED_0001);
        let mut seen: Vec<(Pubkey, u64, [u8; 8], [u8; 32])> = Vec::new();
        for _ in 0..2_000 {
            // Few owners and small sequences force many near-collisions.
            let owner = key((rng.sample() % 6) as u8);
            let sequence = rng.sample() % 40;
            let seeds = IntentSeeds::new(&owner, sequence);
            let slices = seeds.as_slices();
            let mut owner_bytes = [0u8; 32];
            owner_bytes.copy_from_slice(slices[1]);
            let mut sequence_bytes = [0u8; 8];
            sequence_bytes.copy_from_slice(slices[2]);

            for (other_owner, other_sequence, other_seq_bytes, other_owner_bytes) in &seen {
                let same_pair = *other_owner == owner && *other_sequence == sequence;
                let same_seeds =
                    *other_seq_bytes == sequence_bytes && *other_owner_bytes == owner_bytes;
                assert_eq!(same_pair, same_seeds);
            }
            seen.push((owner, sequence, sequence_bytes, owner_bytes));
        }
    }

    /// **Validates: Requirements 3.1** ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â distinct `(owner, sequence)` pairs
    /// never derive the same PDA address.
    #[test]
    fn prop_derived_addresses_are_unique() {
        let mut rng = Lcg(0x5EED_0002);
        let mut seen: Vec<(Pubkey, u64, Pubkey)> = Vec::new();
        for _ in 0..120 {
            let owner = rng.key();
            let sequence = rng.sample() % 8;
            let (address, _) = derive_intent_address(&owner, sequence, &program());
            for (other_owner, other_sequence, other_address) in &seen {
                let same_pair = *other_owner == owner && *other_sequence == sequence;
                assert_eq!(same_pair, *other_address == address);
            }
            seen.push((owner, sequence, address));
        }
    }

    /// **Validates: Requirements 3.1** ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â over any run of attempts, each
    /// sequence is accepted at most once, in order, and the counter only moves
    /// forward.
    #[test]
    fn prop_sequences_are_single_use_and_monotonic() {
        let mut rng = Lcg(0x5EED_0003);
        for _ in 0..500 {
            let mut counter = OperationCounter::new(key(1), 255);
            let mut accepted: Vec<u64> = Vec::new();
            for _ in 0..30 {
                let before = counter.next_sequence;
                // Mostly the next value, sometimes a replay or a jump ahead.
                let attempt = match rng.sample() % 4 {
                    0 => before.saturating_sub(rng.sample() % 3),
                    1 => before + 1 + rng.sample() % 3,
                    _ => before,
                };
                match counter.consume(attempt) {
                    Ok(()) => {
                        assert_eq!(attempt, before);
                        assert!(!accepted.contains(&attempt));
                        accepted.push(attempt);
                        assert_eq!(counter.next_sequence, before + 1);
                    }
                    Err(error) => {
                        assert!(matches!(
                            error,
                            IntentError::SequenceReused | IntentError::SequenceOutOfOrder
                        ));
                        assert_ne!(attempt, before);
                        assert_eq!(counter.next_sequence, before);
                    }
                }
            }
            assert!(accepted.windows(2).all(|pair| pair[1] == pair[0] + 1));
        }
    }
}
