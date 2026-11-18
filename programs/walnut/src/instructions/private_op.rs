//! Abandonment of private operations (ADR-0004).
//!
//! `pool_op`, `position_private` and `liquidate_pool` queue Arcium
//! computations, so their account structs live in `lib.rs`. The plain
//! `abandon_private_op` instruction is here: permissionless timeout recovery
//! after Lock expiry. Rules live in `state::private_op`.
use anchor_lang::prelude::*;

use crate::events::PrivateOpAbandoned;
use crate::market::{Market, MARKET_SEED, POOL_STATE_SEED};
use crate::operation::OperationIntent;
use crate::state::lending_pool::{LendingPool, LENDING_POOL_SEED};
use crate::state::position::POSITION_ACCOUNT_SEED;
use crate::state::private_op::{self as rules, PrivateKeys};
use crate::state::{PoolState, Position};

/// Accounts of `abandon_private_op`. `pool_state` is required for `PoolOp`
/// and `LiquidatePool`; `second_position` (the liquidator's Position) for
/// `LiquidatePool` only.
#[derive(Accounts)]
pub struct AbandonPrivateOp<'info> {
    /// Any signer. Nothing moved at queue time, so nothing is refunded.
    pub signer: Signer<'info>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(
        seeds = [LENDING_POOL_SEED, market.key().as_ref()],
        bump = lending_pool.bump,
    )]
    pub lending_pool: Box<Account<'info, LendingPool>>,
    #[account(
        mut,
        address = market.pool_state,
        seeds = [POOL_STATE_SEED, market.key().as_ref()],
        bump = pool_state.bump,
    )]
    pub pool_state: Option<Box<Account<'info, PoolState>>>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, second_position.market.as_ref(), second_position.owner.as_ref()],
        bump = second_position.bump,
    )]
    pub second_position: Option<Box<Account<'info, Position>>>,
    /// The queued private operation; `state::private_op` checks it.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

/// Permissionless: abandons a queued private operation after Lock expiry.
pub fn abandon_private_op_handler(ctx: Context<AbandonPrivateOp>) -> Result<()> {
    let clock = Clock::get()?;
    let accounts = &mut *ctx.accounts;
    let position_key = accounts.position.key();
    let second_key = accounts.second_position.as_ref().map(|p| p.key());
    let keys = PrivateKeys {
        payer: accounts.operation_intent.payer,
        market: accounts.market.key(),
        asset_state: accounts.market.asset_state,
        lending_pool: accounts.lending_pool.key(),
        pool: accounts.pool_state.as_ref().map(|p| (p.key(), None)),
        position: (position_key, accounts.position.owner, None),
        second: accounts
            .second_position
            .as_ref()
            .map(|p| (p.key(), p.owner, None)),
    };
    let kind = accounts.operation_intent.kind;
    rules::abandon_private(
        &keys,
        accounts.pool_state.as_deref_mut().map(|p| &mut **p),
        &mut accounts.position,
        accounts.second_position.as_deref_mut().map(|p| &mut **p),
        &mut accounts.operation_intent,
        clock.slot,
    )?;
    emit!(PrivateOpAbandoned {
        position: position_key,
        second_position: second_key,
        operation_intent: accounts.operation_intent.key(),
        kind: kind.tag(),
        signer: accounts.signer.key(),
        slot: clock.slot,
    });
    Ok(())
}
