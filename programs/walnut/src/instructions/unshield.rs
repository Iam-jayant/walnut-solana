//! Unshield abandonment (Task 5.4).
//!
//! `unshield` and the `unshield` callback queue or receive an Arcium
//! computation, so their account structs live in `lib.rs`. The plain
//! `abandon_unshield` instruction is here: timeout recovery of a queued
//! unshield after Lock expiry. Rules live in `state::unshield`.

use anchor_lang::prelude::*;

use crate::events::UnshieldAbandoned;
use crate::operation::OperationIntent;
use crate::state::position::POSITION_ACCOUNT_SEED;
use crate::state::unshield as rules;
use crate::state::Position;

/// Accounts of `abandon_unshield`.
#[derive(Accounts)]
pub struct AbandonUnshield<'info> {
    /// Any signer. Abandonment cannot redirect value (Requirement 3.10): no
    /// token moved, and the committed destination cannot change.
    pub signer: Signer<'info>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    /// The queued unshield. `state::unshield` checks that it belongs to this
    /// Position and holds its Lock.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

/// Permissionless: abandons a queued unshield after Lock expiry.
pub fn abandon_unshield_handler(ctx: Context<AbandonUnshield>) -> Result<()> {
    let clock = Clock::get()?;
    let accounts = &mut *ctx.accounts;
    let position_key = accounts.position.key();
    rules::abandon_unshield(
        &mut accounts.position,
        position_key,
        &mut accounts.operation_intent,
        clock.slot,
    )?;
    emit!(UnshieldAbandoned {
        position: position_key,
        owner: accounts.position.owner,
        operation_intent: accounts.operation_intent.key(),
        generation: accounts.position.coordination.generation,
        signer: accounts.signer.key(),
        slot: clock.slot,
    });
    Ok(())
}
