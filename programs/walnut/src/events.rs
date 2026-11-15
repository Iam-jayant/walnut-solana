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
