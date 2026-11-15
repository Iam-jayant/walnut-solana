use anchor_lang::prelude::*;

/// Emitted by the scaffold `add_together` callback. The sum stays encrypted
/// to the caller's shared key; only the ciphertext and nonce are public.
#[event]
pub struct SumEvent {
    pub sum: [u8; 32],
    pub nonce: [u8; 16],
}

/// Emitted by the Task 2.2 Position callbacks when a new encrypted revision is
/// persisted. The payload stays encrypted in the account; only the Position
/// key, the new revision counter, and the output nonce are public. The nonce
/// lets a client locate and decrypt the stored ciphertext locally.
#[event]
pub struct PositionStateStored {
    pub position: Pubkey,
    pub revision: u64,
    pub nonce: [u8; 16],
}

/// Emitted by the Task 2.3 `liquidation_receipts` callback. Proves one
/// computation sealed two encrypted Receipts to two different owners. Each
/// Receipt stays encrypted; only its owner's effective encryption key, the
/// output nonce, and the ciphertexts are public. The design emits borrower and
/// liquidator Receipts as authenticated callback events rather than creating
/// Receipt accounts, so the test decrypts each owner's ciphertexts locally and
/// records the per-owner (key, nonce) identities for the no-repeat assertion.
#[event]
pub struct LiquidationReceiptsEmitted {
    pub borrower_key: [u8; 32],
    pub borrower_nonce: [u8; 16],
    pub borrower_ciphertexts: [[u8; 32]; 2],
    pub liquidator_key: [u8; 32],
    pub liquidator_nonce: [u8; 16],
    pub liquidator_ciphertexts: [[u8; 32]; 2],
}

/// Emitted by the Task 2.4 `oversized_output` success callback. In practice the
/// computation fails with `OutputTooLarge` before this callback runs, so the
/// event is the negative control: the localnet test asserts it is NOT observed.
/// Only the output nonce is carried; the oversized ciphertexts are ignored.
#[event]
pub struct OversizedOutputEmitted {
    pub nonce: [u8; 16],
}

/// Emitted by the Task 2.3 `rotate_viewing_key` callback when a Position is
/// re-encrypted to a new Viewing Key. Commits the old and new key epochs and
/// public keys (Requirement 5.11). The re-encrypted payload stays in the
/// account; only the epoch/public-key transition and the new output nonce are
/// public.
#[event]
pub struct ViewingKeyRotated {
    pub position: Pubkey,
    pub old_epoch: u64,
    pub new_epoch: u64,
    pub old_pubkey: [u8; 32],
    pub new_pubkey: [u8; 32],
    pub new_nonce: [u8; 16],
}
