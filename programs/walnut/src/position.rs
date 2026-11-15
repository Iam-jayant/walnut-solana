//! Persistent encrypted Position state for the Task 2.2 feasibility gate.
//!
//! This models the design's on-chain `Position` closely enough to prove that
//! the complete encrypted payload persists in an account and can be reread by
//! a later computation (Requirement 2.2). It is NOT the production Position:
//! the coordination fields (`generation`, `lock`, sequences, freeze state) are
//! exercised separately by the Task 2.1 state machine, and the financial
//! lifecycle (`status`, `has_debt`, shield/unshield) arrives in Task 5. The
//! name `SpikePosition` marks it as feasibility-gate scaffolding.
//!
//! The encrypted payload is `Enc<Shared, PositionSecret>` with five scalar
//! fields (design: `PositionSecret`). An Arcium `SharedEncryptedStruct<5>`
//! carries, for that payload, one `encryption_key` (the Viewing public key),
//! one `nonce`, and five 32-byte ciphertexts. The Viewing public key is a
//! long-lived Position field, so it is stored once as `viewing_pubkey`; the
//! per-revision `nonce` and `ciphertexts` live in [`EncryptedPositionState`]
//! (design: `EncGroup`).

use anchor_lang::prelude::*;

/// Number of encrypted scalar fields in the Position payload
/// (`free_sol`, `free_usdc`, `collateral_sol`, `supply_shares_usdc`,
/// `debt_shares_usdc`). This is the `N` in Arcium's
/// `SharedEncryptedStruct<N>` for `Enc<Shared, PositionSecret>`.
pub const POSITION_SECRET_FIELDS: usize = 5;

/// The per-revision encrypted Position payload: the nonce and ciphertexts of a
/// `SharedEncryptedStruct<POSITION_SECRET_FIELDS>` output.
///
/// The ciphertext order matches the Arcis `PositionSecret` field order exactly:
/// `[free_sol, free_usdc, collateral_sol, supply_shares_usdc,
/// debt_shares_usdc]`. The queue instruction re-feeds these in the same order,
/// and the client decrypts them in the same order.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, InitSpace)]
pub struct EncryptedPositionState {
    /// The nonce the MXE used to encrypt this revision (the output
    /// `SharedEncryptedStruct::nonce`). Fed back as the input nonce when this
    /// revision is read by a later computation.
    pub nonce: u128,
    /// One 32-byte ciphertext per encrypted Position scalar, in payload order.
    /// The literal length must equal [`POSITION_SECRET_FIELDS`]; the length is
    /// written as a literal because Anchor's `InitSpace` derive expects one.
    pub ciphertexts: [[u8; 32]; 5],
}

/// A Position account holding persistent encrypted state for the feasibility
/// gate.
#[account]
#[derive(InitSpace)]
pub struct SpikePosition {
    /// The Position owner (payer of the initialization).
    pub owner: Pubkey,
    /// The Viewing public key the payload is encrypted to (the
    /// `SharedEncryptedStruct::encryption_key`). Equivalent to the design's
    /// `viewing_pubkey`.
    pub viewing_pubkey: [u8; 32],
    /// The Viewing Key epoch. Initialization uses epoch zero; rotation
    /// (Task 5.5) increments it. Stored here so the spike account shape matches
    /// the design.
    pub viewing_key_epoch: u64,
    /// A local revision counter bumped on each stored write. Distinct from the
    /// coordination `generation` modelled by the Task 2.1 state machine; it
    /// lets the test confirm every computation produced a fresh revision.
    pub revision: u64,
    /// The current encrypted payload.
    pub state: EncryptedPositionState,
    /// The PDA bump.
    pub bump: u8,
}

impl SpikePosition {
    /// Account data length including Anchor's 8-byte discriminator.
    pub const ACCOUNT_LEN: usize = 8 + Self::INIT_SPACE;
}

#[cfg(test)]
mod tests {
    use super::*;
    use anchor_lang::Space;

    /// The encrypted payload holds exactly the five design `PositionSecret`
    /// scalars.
    #[test]
    fn payload_has_five_ciphertexts() {
        assert_eq!(POSITION_SECRET_FIELDS, 5);
        let state = EncryptedPositionState {
            nonce: 0,
            ciphertexts: [[0u8; 32]; POSITION_SECRET_FIELDS],
        };
        assert_eq!(state.ciphertexts.len(), 5);
    }

    /// The encrypted payload byte size is fixed and small: 16 (u128 nonce) plus
    /// five 32-byte ciphertexts = 176 bytes. Fixing this guards the account and
    /// transaction-envelope budgets (Requirement 2.2 / Task 2.5).
    #[test]
    fn encrypted_payload_space_is_fixed() {
        assert_eq!(EncryptedPositionState::INIT_SPACE, 16 + 5 * 32);
        assert_eq!(EncryptedPositionState::INIT_SPACE, 176);
    }

    /// The whole Position account is 265 bytes: 8 discriminator + 32 owner +
    /// 32 viewing key + 8 epoch + 8 revision + 176 payload + 1 bump. Far under
    /// Solana account and transaction limits, so the single-structure
    /// `Enc<Shared>` layout fits comfortably.
    #[test]
    fn account_len_is_expected() {
        assert_eq!(SpikePosition::INIT_SPACE, 32 + 32 + 8 + 8 + 176 + 1);
        assert_eq!(SpikePosition::INIT_SPACE, 257);
        assert_eq!(SpikePosition::ACCOUNT_LEN, 265);
    }
}
