//! `abandon_position_op`: timeout recovery of a queued position operation
//! after Lock expiry. Rules live in `state::position_op`.

use anchor_lang::prelude::*;

use crate::events::PositionOpAbandoned;
use crate::operation::OperationIntent;
use crate::state::position::POSITION_ACCOUNT_SEED;
use crate::state::position_op as rules;
use crate::state::Position;

/// Accounts of `abandon_position_op`.
#[derive(Accounts)]
pub struct AbandonPositionOp<'info> {
    /// Any signer; nothing moves.
    pub signer: Signer<'info>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    /// The queued operation; `state::position_op` checks it.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

/// Permissionless: abandons a queued position operation after Lock expiry.
pub fn abandon_position_op_handler(ctx: Context<AbandonPositionOp>) -> Result<()> {
    let clock = Clock::get()?;
    let accounts = &mut *ctx.accounts;
    let position_key = accounts.position.key();
    rules::abandon_position_op(
        &mut accounts.position,
        position_key,
        &mut accounts.operation_intent,
        clock.slot,
    )?;
    emit!(PositionOpAbandoned {
        position: position_key,
        owner: accounts.position.owner,
        operation_intent: accounts.operation_intent.key(),
        generation: accounts.position.coordination.generation,
        signer: accounts.signer.key(),
        slot: clock.slot,
    });
    Ok(())
}
