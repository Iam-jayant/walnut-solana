//! Production protected accounts (Task 3.1).
//!
//! `Position` and `PoolState` are the two accounts the design says carry a
//! generation and a writer Lock (design: "Position", "Pool State",
//! "Generations and Locks"). Task 3.1 defines only their coordination layout:
//! the `owner` identity, the [`Coordination`] field group (generation + Lock),
//! and the PDA bump. No financial or encrypted fields exist yet.
//!
//! Later tasks (Position in Task 5, Pool State in Task 4) append their own
//! fields **after** `coordination` and never reorder or resize the existing
//! ones, so the offsets asserted by the tests below stay valid. These accounts
//! are not created by any instruction yet and are separate from the Task 2.2
//! `SpikePosition`, which is unchanged.
//!
//! Layout (offsets include the 8-byte Anchor discriminator):
//!
//! * `Position` (98 bytes): discriminator 0..8, `owner` 8..40, `generation`
//!   40..48, `lock` 48..97, `bump` 97.
//! * `PoolState` (66 bytes): discriminator 0..8, `generation` 8..16, `lock`
//!   16..65, `bump` 65.

use anchor_lang::prelude::*;

use super::coordination::Coordination;

/// A user Position's coordination layout.
#[account]
#[derive(InitSpace)]
pub struct Position {
    /// The Position owner.
    pub owner: Pubkey,
    /// The Position's generation and writer Lock.
    pub coordination: Coordination,
    /// The PDA bump.
    pub bump: u8,
}

impl Position {
    /// Account data length including Anchor's 8-byte discriminator.
    pub const ACCOUNT_LEN: usize = 8 + Self::INIT_SPACE;
}

/// The Pool State's coordination layout.
#[account]
#[derive(InitSpace)]
pub struct PoolState {
    /// The Pool's generation and writer Lock.
    pub coordination: Coordination,
    /// The PDA bump.
    pub bump: u8,
}

impl PoolState {
    /// Account data length including Anchor's 8-byte discriminator.
    pub const ACCOUNT_LEN: usize = 8 + Self::INIT_SPACE;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::Lock;
    use anchor_lang::{AccountSerialize, Discriminator, Space};

    fn sample_coordination() -> Coordination {
        Coordination {
            generation: 0x0102_0304_0506_0708,
            lock: Lock {
                active: true,
                computation_offset: 0x1112_1314_1516_1718,
                operation_intent: Pubkey::new_from_array([0xAB; 32]),
                expiry_slot: 0x2122_2324_2526_2728,
            },
        }
    }

    fn read_u64(bytes: &[u8], at: usize) -> u64 {
        u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
    }

    #[test]
    fn position_space_is_fixed() {
        assert_eq!(Position::INIT_SPACE, 32 + 57 + 1);
        assert_eq!(Position::INIT_SPACE, 90);
        assert_eq!(Position::ACCOUNT_LEN, 98);
    }

    #[test]
    fn pool_state_space_is_fixed() {
        assert_eq!(PoolState::INIT_SPACE, 57 + 1);
        assert_eq!(PoolState::INIT_SPACE, 58);
        assert_eq!(PoolState::ACCOUNT_LEN, 66);
    }

    #[test]
    fn position_byte_layout() {
        let position = Position {
            owner: Pubkey::new_from_array([0x11; 32]),
            coordination: sample_coordination(),
            bump: 0xFE,
        };
        let mut bytes = Vec::new();
        position.try_serialize(&mut bytes).unwrap();

        assert_eq!(bytes.len(), Position::ACCOUNT_LEN);
        assert_eq!(&bytes[..8], Position::DISCRIMINATOR);
        assert_eq!(bytes[8..40], [0x11; 32]);
        assert_eq!(read_u64(&bytes, 40), 0x0102_0304_0506_0708);
        assert_eq!(bytes[48], 1);
        assert_eq!(read_u64(&bytes, 49), 0x1112_1314_1516_1718);
        assert_eq!(bytes[57..89], [0xAB; 32]);
        assert_eq!(read_u64(&bytes, 89), 0x2122_2324_2526_2728);
        assert_eq!(bytes[97], 0xFE);
    }

    #[test]
    fn pool_state_byte_layout() {
        let pool = PoolState {
            coordination: sample_coordination(),
            bump: 0xFD,
        };
        let mut bytes = Vec::new();
        pool.try_serialize(&mut bytes).unwrap();

        assert_eq!(bytes.len(), PoolState::ACCOUNT_LEN);
        assert_eq!(&bytes[..8], PoolState::DISCRIMINATOR);
        assert_eq!(read_u64(&bytes, 8), 0x0102_0304_0506_0708);
        assert_eq!(bytes[16], 1);
        assert_eq!(read_u64(&bytes, 17), 0x1112_1314_1516_1718);
        assert_eq!(bytes[25..57], [0xAB; 32]);
        assert_eq!(read_u64(&bytes, 57), 0x2122_2324_2526_2728);
        assert_eq!(bytes[65], 0xFD);
    }

    #[test]
    fn discriminators_are_distinct_from_the_spike_position() {
        use crate::position::SpikePosition;
        assert_ne!(Position::DISCRIMINATOR, SpikePosition::DISCRIMINATOR);
        assert_ne!(PoolState::DISCRIMINATOR, SpikePosition::DISCRIMINATOR);
        assert_ne!(Position::DISCRIMINATOR, PoolState::DISCRIMINATOR);
    }
}
