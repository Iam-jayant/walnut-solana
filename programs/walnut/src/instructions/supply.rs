//! Supply and withdraw-supply abandonment (Task 8).
//!
//! `supply_flow` and its callback queue or receive an Arcium computation, so
//! their account structs live in `lib.rs`. The plain `abandon_supply_flow`
//! instruction is here: timeout recovery after Lock expiry. Rules live in
//! `state::supply`.

use anchor_lang::prelude::*;

use crate::events::SupplyFlowAbandoned;
use crate::market::{Market, MARKET_SEED, POOL_STATE_SEED};
use crate::operation::OperationIntent;
use crate::state::position::POSITION_ACCOUNT_SEED;
use crate::state::supply as rules;
use crate::state::{PoolState, Position};

/// Accounts of `abandon_supply_flow`.
#[derive(Accounts)]
pub struct AbandonSupplyFlow<'info> {
    /// Any signer. Abandonment moves no value: nothing moved at queue time.
    pub signer: Signer<'info>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(
        mut,
        address = market.pool_state,
        seeds = [POOL_STATE_SEED, market.key().as_ref()],
        bump = pool_state.bump,
    )]
    pub pool_state: Box<Account<'info, PoolState>>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    /// The queued supply or withdrawal. `state::supply` checks that it
    /// belongs to this Position and holds the Pool and Position Locks.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

/// Permissionless: abandons a queued supply or withdrawal after Lock expiry.
pub fn abandon_supply_flow_handler(ctx: Context<AbandonSupplyFlow>) -> Result<()> {
    let clock = Clock::get()?;
    let accounts = &mut *ctx.accounts;
    let pool_key = accounts.pool_state.key();
    let position_key = accounts.position.key();
    rules::abandon_supply_flow(
        &mut accounts.pool_state,
        pool_key,
        &mut accounts.position,
        position_key,
        &mut accounts.operation_intent,
        clock.slot,
    )?;
    emit!(SupplyFlowAbandoned {
        position: position_key,
        pool_state: pool_key,
        operation_intent: accounts.operation_intent.key(),
        signer: accounts.signer.key(),
        slot: clock.slot,
    });
    Ok(())
}
