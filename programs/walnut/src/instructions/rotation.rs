//! Viewing Key rotation abandonment (Task 5.5).
//!
//! `rotate_position_key` and its callback queue or receive an Arcium
//! computation, so their account structs live in `lib.rs`. The plain
//! `abandon_rotation` instruction is here: timeout recovery of a queued
//! rotation after Lock expiry. Rules live in `state::rotation`.

use anchor_lang::prelude::*;

use crate::events::ViewingKeyRotationAbandoned;
use crate::operation::OperationIntent;
use crate::state::position::POSITION_ACCOUNT_SEED;
use crate::state::rotation as rules;
use crate::state::Position;

/// Accounts of `abandon_rotation`.
#[derive(Accounts)]
pub struct AbandonRotation<'info> {
    /// Any signer. Abandonment cannot change the key, the epoch, or the
    /// payload; it only releases the Lock.
    pub signer: Signer<'info>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    /// The queued rotation. `state::rotation` checks that it belongs to this
    /// Position and holds its Lock.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

/// Permissionless: abandons a queued rotation after Lock expiry.
pub fn abandon_rotation_handler(ctx: Context<AbandonRotation>) -> Result<()> {
    let clock = Clock::get()?;
    let accounts = &mut *ctx.accounts;
    let position_key = accounts.position.key();
    rules::abandon_rotation(
        &mut accounts.position,
        position_key,
        &mut accounts.operation_intent,
        clock.slot,
    )?;
    emit!(ViewingKeyRotationAbandoned {
        position: position_key,
        owner: accounts.position.owner,
        operation_intent: accounts.operation_intent.key(),
        generation: accounts.position.coordination.generation,
        signer: accounts.signer.key(),
        slot: clock.slot,
    });
    Ok(())
}
