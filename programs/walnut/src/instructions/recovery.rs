//! `abandon_recover_empty_pool` (Task 8.1): timeout recovery after Lock expiry.
//!
//! `recover_empty_pool` and its callback queue or receive an Arcium
//! computation, so their account structs live in `lib.rs`. Rules live in
//! `state::recovery`.

use anchor_lang::prelude::*;

use crate::events::EmptyPoolRecoveryAbandoned;
use crate::market::{Market, MARKET_SEED, POOL_STATE_SEED};
use crate::operation::OperationIntent;
use crate::state::recovery as rules;
use crate::state::PoolState;

/// Accounts of `abandon_recover_empty_pool`.
#[derive(Accounts)]
pub struct AbandonRecoverEmptyPool<'info> {
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
    /// The queued recovery. `state::pool_mutation` checks its kind and that it
    /// holds the Pool Lock.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

/// Permissionless: abandons a queued recovery after Lock expiry.
pub fn abandon_recover_empty_pool_handler(ctx: Context<AbandonRecoverEmptyPool>) -> Result<()> {
    let clock = Clock::get()?;
    let accounts = &mut *ctx.accounts;
    let pool_key = accounts.pool_state.key();
    rules::abandon_recover_empty_pool(
        &mut accounts.pool_state,
        pool_key,
        &mut accounts.operation_intent,
        clock.slot,
    )?;
    emit!(EmptyPoolRecoveryAbandoned {
        pool_state: pool_key,
        operation_intent: accounts.operation_intent.key(),
        generation: accounts.pool_state.coordination.generation,
        signer: accounts.signer.key(),
        slot: clock.slot,
    });
    Ok(())
}
