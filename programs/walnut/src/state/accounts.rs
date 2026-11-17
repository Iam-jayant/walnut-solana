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
//! ones, so the offsets asserted by the tests below stay valid. Both are
//! separate from the Task 2.2 `SpikePosition`, which is unchanged.
//!
//! Layout (offsets include the 8-byte Anchor discriminator):
//!
//! * `Position` (99 bytes): discriminator 0..8, `owner` 8..40, `generation`
//!   40..48, `lock` 48..97, `bump` 97, then the Task 4.5 public `has_debt`
//!   flag at 98.
//! * `PoolState` (260 bytes): discriminator 0..8, `generation` 8..16, `lock`
//!   16..65, `bump` 65, then the Task 4.1 fields: `layout_version` 66,
//!   `status` 67, `market` 68..100, `accounted_borrow_index` 100..116,
//!   `encrypted.nonce` 116..132, `encrypted.ciphertexts` 132..260.
//!
//! Task 4.1 creates the Pool State and the Recovery Position (a `Position`
//! whose `owner` is the Market PDA) in `initialize_market`.

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
    /// Public "this Position has debt" flag (Requirement 4.11, Task 4.5). It
    /// changes only through `market::caps::settle_debt_intent`, driven by the
    /// zero/nonzero transition a verified callback reveals; never the amount.
    pub has_debt: bool,
}

impl Position {
    /// Account data length including Anchor's 8-byte discriminator.
    pub const ACCOUNT_LEN: usize = 8 + Self::INIT_SPACE;
}

/// Number of encrypted scalars in the Pool payload (`cash_usdc`,
/// `total_supply_shares`, `total_debt_shares`, `reserves_usdc`). This is the
/// `N` in Arcium's `MXEEncryptedStruct<N>` for `Enc<Mxe, PoolSecret>`.
pub const POOL_SECRET_FIELDS: usize = 4;

/// Lifecycle of the Pool State. Zeroed data reads as `Uninitialized`.
#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub enum PoolStatus {
    /// Account space exists but `initialize_market` has not written it.
    #[default]
    Uninitialized,
    /// Created by `initialize_market`; waiting for the `init_pool_state`
    /// callback to store the encrypted zero payload. No Pool operation may
    /// run in this state.
    Initializing,
    /// The encrypted payload is stored and Pool operations may run.
    Active,
}

/// The encrypted Pool payload: the nonce and ciphertexts of an
/// `MXEEncryptedStruct<POOL_SECRET_FIELDS>` (`Enc<Mxe, PoolSecret>`).
///
/// Ciphertext order matches the Arcis `PoolSecret` field order:
/// `[cash_usdc, total_supply_shares, total_debt_shares, reserves_usdc]`.
/// `Enc<Mxe>` has no client key, so no public key is stored.
#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub struct EncryptedPoolState {
    /// The MXE output nonce of this revision.
    pub nonce: u128,
    /// One 32-byte ciphertext per Pool scalar, in payload order. The literal
    /// length must equal [`POOL_SECRET_FIELDS`].
    pub ciphertexts: [[u8; 32]; 4],
}

/// The USDC Pool State (design: "Pool State").
///
/// The Task 3.1 coordination fields keep their offsets; Task 4.1 appends the
/// Pool fields after `bump`.
#[account]
#[derive(InitSpace)]
pub struct PoolState {
    /// The Pool's generation and writer Lock.
    pub coordination: Coordination,
    /// The PDA bump.
    pub bump: u8,
    /// Layout version (`market::LAYOUT_VERSION`).
    pub layout_version: u8,
    /// Lifecycle status.
    pub status: PoolStatus,
    /// The owning Market.
    pub market: Pubkey,
    /// The Borrow Index the encrypted payload was last checkpointed at (WAD).
    pub accounted_borrow_index: u128,
    /// The `Enc<Mxe, PoolSecret>` payload. All zero until `Active`.
    pub encrypted: EncryptedPoolState,
}

impl PoolState {
    /// Account data length including Anchor's 8-byte discriminator.
    pub const ACCOUNT_LEN: usize = 8 + Self::INIT_SPACE;

    /// Applies the authenticated `init_pool_state` output.
    ///
    /// Only an `Initializing` Pool is written: the encrypted zero payload is
    /// stored, the status becomes `Active`, and the generation is bumped once.
    /// Any other status is a duplicate or late callback and changes nothing.
    /// Returns whether the Pool was activated. Atomic on generation overflow.
    pub fn activate(
        &mut self,
        encrypted: EncryptedPoolState,
    ) -> core::result::Result<bool, super::CoordinationError> {
        if self.status != PoolStatus::Initializing {
            return Ok(false);
        }
        let generation = self
            .coordination
            .generation
            .checked_add(1)
            .ok_or(super::CoordinationError::GenerationOverflow)?;
        self.coordination.generation = generation;
        self.encrypted = encrypted;
        self.status = PoolStatus::Active;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::Lock;
    use anchor_lang::{AccountDeserialize, AccountSerialize, Discriminator, Space};

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
        assert_eq!(Position::INIT_SPACE, 32 + 57 + 1 + 1);
        assert_eq!(Position::INIT_SPACE, 91);
        assert_eq!(Position::ACCOUNT_LEN, 99);
    }

    #[test]
    fn pool_state_space_is_fixed() {
        assert_eq!(PoolStatus::INIT_SPACE, 1);
        assert_eq!(EncryptedPoolState::INIT_SPACE, 16 + POOL_SECRET_FIELDS * 32);
        assert_eq!(EncryptedPoolState::INIT_SPACE, 144);
        assert_eq!(PoolState::INIT_SPACE, 57 + 1 + 1 + 1 + 32 + 16 + 144);
        assert_eq!(PoolState::INIT_SPACE, 252);
        assert_eq!(PoolState::ACCOUNT_LEN, 260);
    }

    #[test]
    fn position_byte_layout() {
        let position = Position {
            owner: Pubkey::new_from_array([0x11; 32]),
            coordination: sample_coordination(),
            bump: 0xFE,
            has_debt: true,
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
        assert_eq!(bytes[98], 1, "has_debt");
    }

    #[test]
    fn pool_state_byte_layout() {
        let pool = PoolState {
            coordination: sample_coordination(),
            bump: 0xFD,
            layout_version: 1,
            status: PoolStatus::Active,
            market: Pubkey::new_from_array([0x33; 32]),
            accounted_borrow_index: 0x4142_4344_4546_4748_494A_4B4C_4D4E_4F50,
            encrypted: EncryptedPoolState {
                nonce: 0x5152_5354_5556_5758_595A_5B5C_5D5E_5F60,
                ciphertexts: [[0x61; 32], [0x62; 32], [0x63; 32], [0x64; 32]],
            },
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
        // Task 4.1 fields.
        assert_eq!(bytes[66], 1);
        assert_eq!(bytes[67], 2);
        assert_eq!(bytes[68..100], [0x33; 32]);
        assert_eq!(
            u128::from_le_bytes(bytes[100..116].try_into().unwrap()),
            pool.accounted_borrow_index
        );
        assert_eq!(
            u128::from_le_bytes(bytes[116..132].try_into().unwrap()),
            pool.encrypted.nonce
        );
        for (index, fill) in [0x61u8, 0x62, 0x63, 0x64].iter().enumerate() {
            let at = 132 + 32 * index;
            assert_eq!(bytes[at..at + 32], [*fill; 32]);
        }
    }

    fn pool_with(status: PoolStatus) -> PoolState {
        PoolState {
            coordination: Coordination::default(),
            bump: 1,
            layout_version: 1,
            status,
            market: Pubkey::new_from_array([0x33; 32]),
            accounted_borrow_index: 1,
            encrypted: EncryptedPoolState::default(),
        }
    }

    fn payload(fill: u8) -> EncryptedPoolState {
        EncryptedPoolState {
            nonce: u128::from(fill),
            ciphertexts: [[fill; 32]; 4],
        }
    }

    #[test]
    fn activation_writes_once_and_bumps_the_generation() {
        let mut pool = pool_with(PoolStatus::Initializing);
        assert_eq!(pool.activate(payload(7)), Ok(true));
        assert_eq!(pool.status, PoolStatus::Active);
        assert_eq!(pool.coordination.generation, 1);
        assert_eq!(pool.encrypted, payload(7));

        // A duplicate or late callback changes nothing.
        assert_eq!(pool.activate(payload(9)), Ok(false));
        assert_eq!(pool.coordination.generation, 1);
        assert_eq!(pool.encrypted, payload(7));
    }

    #[test]
    fn activation_ignores_an_unwritten_pool_and_is_atomic_on_overflow() {
        let mut unwritten = pool_with(PoolStatus::Uninitialized);
        assert_eq!(unwritten.activate(payload(7)), Ok(false));
        assert_eq!(unwritten.encrypted, EncryptedPoolState::default());

        let mut saturated = pool_with(PoolStatus::Initializing);
        saturated.coordination.generation = u64::MAX;
        assert_eq!(
            saturated.activate(payload(7)),
            Err(crate::state::CoordinationError::GenerationOverflow)
        );
        assert_eq!(saturated.status, PoolStatus::Initializing);
        assert_eq!(saturated.encrypted, EncryptedPoolState::default());
    }

    #[test]
    fn zeroed_pool_reads_as_uninitialized() {
        let mut bytes = PoolState::DISCRIMINATOR.to_vec();
        bytes.extend([0u8; PoolState::INIT_SPACE]);
        let pool = PoolState::try_deserialize(&mut bytes.as_slice()).unwrap();
        assert_eq!(pool.status, PoolStatus::Uninitialized);
        assert_eq!(pool.encrypted, EncryptedPoolState::default());
    }

    #[test]
    fn discriminators_are_distinct_from_the_spike_position() {
        use crate::position::SpikePosition;
        assert_ne!(Position::DISCRIMINATOR, SpikePosition::DISCRIMINATOR);
        assert_ne!(PoolState::DISCRIMINATOR, SpikePosition::DISCRIMINATOR);
        assert_ne!(Position::DISCRIMINATOR, PoolState::DISCRIMINATOR);
    }
}
