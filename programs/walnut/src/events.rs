use anchor_lang::prelude::*;

/// Emitted by the scaffold `add_together` callback. The sum stays encrypted
/// to the caller's shared key; only the ciphertext and nonce are public.
#[event]
pub struct SumEvent {
    pub sum: [u8; 32],
    pub nonce: [u8; 16],
}
