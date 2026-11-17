//! Unique output nonces for Position-mutating circuits (Task 5.4; ADR-0003
//! nonce safety, Requirement 2.3 / 5).
//!
//! # The problem
//!
//! Arcium seals an `Enc<Shared, _>` output under the owner key with nonce =
//! the owning input's nonce + 1, and Rescue is a stream-style cipher: two
//! ciphertexts under one (key, nonce) leak the difference of their
//! plaintexts. If a circuit re-sealed the Position under *stored nonce + 1*,
//! an attempt that does not apply (`Zero`, `Stale`, `Expired`, abandoned, or
//! a callback that reverts) leaves the stored nonce unchanged, so the next
//! attempt would seal a different plaintext under the same (key, nonce), and
//! the rejected output is still public in its callback transaction.
//!
//! # The rule
//!
//! Every Position-mutating circuit (`shield_credit`, `unshield`, and the later
//! ones) takes a bare `Shared` input carrying the Viewing public key and a
//! fresh *output-nonce input* `n`, and seals its output to it, so the output
//! nonce is `n + 1`. The program derives `n` itself from the Position address
//! and the Operation Intent sequence:
//!
//! ```text
//! n = u128_le( sha256( "walnut:position-output-nonce:v1" | position | sequence_le )[0..16] )
//!     & (2^127 - 1)
//! ```
//!
//! Intent sequences come from the owner's `OperationCounter` and are
//! reserved exactly once, even when the attempt later fails, so no two
//! attempts of one Position share `n`. The top bit is cleared so `n + 1`
//! never wraps. `n` is also committed in the Intent's `inputs_commitment`,
//! and the callback rebuilds the commitment from the output nonce - 1, so an
//! output sealed under any other nonce is rejected.
//!
//! The client does not choose the nonce, so a buggy or malicious client
//! cannot cause a repeat. Residual risk: distinct (position, sequence) pairs
//! map to nonces through a truncated hash, so a collision with another
//! nonce under the same Viewing Key (a client-chosen random input nonce, or
//! another derived one) has probability about `2^-127` per pair, the same
//! order as a collision of two random 128-bit client nonces.

use anchor_lang::prelude::Pubkey;

use crate::operation::sha256::sha256;

/// Domain separator of the derivation.
pub const OUTPUT_NONCE_DOMAIN: &[u8] = b"walnut:position-output-nonce:v1";

/// Input field id: the output-nonce input `n` (`U128`). Append-only; ids 1-12
/// are taken by the Position and shield fields.
pub const INPUT_OUTPUT_NONCE: u8 = 13;

/// The output-nonce input `n` of the attempt with `intent_sequence` on
/// `position`. The circuit's output nonce is `n + 1`.
pub fn derive_output_nonce_input(position: &Pubkey, intent_sequence: u64) -> u128 {
    let mut bytes = Vec::with_capacity(OUTPUT_NONCE_DOMAIN.len() + 40);
    bytes.extend_from_slice(OUTPUT_NONCE_DOMAIN);
    bytes.extend_from_slice(position.as_ref());
    bytes.extend_from_slice(&intent_sequence.to_le_bytes());
    let digest = sha256(&bytes);
    let mut low = [0u8; 16];
    low.copy_from_slice(&digest[..16]);
    u128::from_le_bytes(low) & (u128::MAX >> 1)
}

/// The nonce the circuit seals its output under: `n + 1`.
pub fn output_nonce(position: &Pubkey, intent_sequence: u64) -> u128 {
    derive_output_nonce_input(position, intent_sequence) + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn position(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    #[test]
    fn derivation_matches_the_published_vectors() {
        // The same vectors are asserted by the SDK (`output-nonce.test.ts`).
        let p = position(0x23);
        assert_eq!(
            derive_output_nonce_input(&p, 0),
            0x7636_69ec_ad9c_e051_199b_4389_985c_02cc
        );
        assert_eq!(
            derive_output_nonce_input(&p, 1),
            0x5e39_c77f_c4f2_4515_e0f2_5a9d_9021_4f9d
        );
        assert_eq!(
            derive_output_nonce_input(&p, 7),
            0x08b1_6e46_33c9_1b0d_9b42_baa8_e2b8_1605
        );
        assert_eq!(output_nonce(&p, 0), derive_output_nonce_input(&p, 0) + 1);
    }

    #[test]
    fn nonce_depends_on_position_and_sequence() {
        let a = derive_output_nonce_input(&position(1), 5);
        assert_ne!(a, derive_output_nonce_input(&position(2), 5));
        assert_ne!(a, derive_output_nonce_input(&position(1), 6));
        assert!(a < (1u128 << 127), "top bit is clear so n + 1 cannot wrap");
    }

    #[test]
    fn many_sequences_never_collide_even_with_adjacent_outputs() {
        // No input nonce, output nonce, or adjacent value repeats.
        let p = position(9);
        let mut seen = HashSet::new();
        for sequence in 0..2_000u64 {
            let n = derive_output_nonce_input(&p, sequence);
            assert!(seen.insert(n));
            assert!(seen.insert(n + 1));
        }
    }

    /// Distinct attempts (distinct sequences, any Position) always get
    /// disjoint input and output nonce pairs.
    ///
    /// **Validates: Requirements 2.3, 5.7**
    #[test]
    fn prop_distinct_sequences_get_disjoint_nonce_pairs() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            state
        };
        for _ in 0..256 {
            let mut key = [0u8; 32];
            for chunk in key.chunks_mut(8) {
                chunk.copy_from_slice(&next().to_le_bytes());
            }
            let p = Pubkey::new_from_array(key);
            let (a, b) = (next(), next());
            if a == b {
                continue;
            }
            let (na, nb) = (
                derive_output_nonce_input(&p, a),
                derive_output_nonce_input(&p, b),
            );
            assert_ne!(na, nb);
            assert_ne!(na + 1, nb);
            assert_ne!(na, nb + 1);
        }
    }
}
