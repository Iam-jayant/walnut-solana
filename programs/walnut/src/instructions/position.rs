//! Position initialization recovery and close (Task 5.1).
//!
//! `initialize_position`, `retry_position_init`, and the `init_position_state`
//! callback queue or receive Arcium computations, so their account structs
//! live in `lib.rs`. The two plain instructions are here:
//!
//! * `abandon_position_init` (any signer): timeout recovery of a queued
//!   attempt after Lock expiry; the Position becomes `InitFailed`.
//! * `close_position` (owner): closes an `InitFailed` Position, returns its
//!   rent to the owner, and decrements `position_count`.
//!
//! Rules live in `state::position`.

use anchor_lang::prelude::*;

use crate::events::{PositionClosed, PositionCountChanged, PositionInitFailed};
use crate::market::{Market, MARKET_SEED};
use crate::operation::{IntentStatus, OperationIntent};
use crate::state::position::{self as rules, POSITION_ACCOUNT_SEED};
use crate::state::Position;

/// Accounts of `abandon_position_init`.
#[derive(Accounts)]
pub struct AbandonPositionInit<'info> {
    /// Any signer. Abandonment cannot redirect value (Requirement 3.10).
    pub signer: Signer<'info>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    /// The queued initialization Intent. `state::position` checks that it
    /// belongs to this Position and holds its Lock.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

/// Permissionless: abandons a queued initialization attempt after Lock expiry.
pub fn abandon_position_init_handler(ctx: Context<AbandonPositionInit>) -> Result<()> {
    let clock = Clock::get()?;
    let accounts = &mut *ctx.accounts;
    let position_key = accounts.position.key();
    rules::abandon_position_init(
        &mut accounts.position,
        position_key,
        &mut accounts.operation_intent,
        clock.slot,
    )?;
    emit!(PositionInitFailed {
        market: accounts.position.market,
        position: position_key,
        owner: accounts.position.owner,
        operation_intent: accounts.operation_intent.key(),
        intent_status: IntentStatus::Abandoned,
        generation: accounts.position.coordination.generation,
        slot: clock.slot,
    });
    Ok(())
}

/// Accounts of `close_position`.
#[derive(Accounts)]
pub struct ClosePosition<'info> {
    /// The Position owner; receives the rent.
    #[account(mut)]
    pub owner: Signer<'info>,
    #[account(mut, seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(
        mut,
        close = owner,
        seeds = [POSITION_ACCOUNT_SEED, market.key().as_ref(), owner.key().as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
}

/// Owner-signed: closes an `InitFailed` Position.
pub fn close_position_handler(ctx: Context<ClosePosition>) -> Result<()> {
    let clock = Clock::get()?;
    let accounts = &mut *ctx.accounts;
    let change = rules::close_position(&mut accounts.market, &accounts.position)?;
    let market = accounts.market.key();
    let position = accounts.position.key();
    emit!(PositionCountChanged {
        market,
        position,
        old_count: change.old_count,
        new_count: change.new_count,
        cap: change.cap,
        slot: clock.slot,
    });
    emit!(PositionClosed {
        market,
        position,
        owner: accounts.owner.key(),
        slot: clock.slot,
    });
    Ok(())
}
