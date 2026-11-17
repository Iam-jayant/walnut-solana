//! Timelocked configuration instructions (Task 4.4, Requirements 4.5, 4.6,
//! 4.12, and 4.13).
//!
//! Each handler reads the clock, runs one pure transition from
//! `market::config`, and emits one event. Only execution changes the Asset
//! State configuration, and it increments the Asset State generation once, so
//! every Intent queued before it resolves `Stale` at its callback.

use anchor_lang::prelude::*;

use crate::events::{ConfigChangeCancelled, ConfigChangeExecuted, ConfigChangeProposed};
use crate::market::config as rules;
use crate::market::{
    AssetState, ConfigGroup, ConfigProposal, ConfigValue, Market, ASSET_STATE_SEED,
    CONFIG_PROPOSAL_SEED, MARKET_SEED, POOL_STATE_SEED,
};
use crate::state::pool_mutation::ensure_pool_unlocked_for_index_mutation;
use crate::state::PoolState;

/// Accounts of `propose_config_change`.
#[derive(Accounts)]
#[instruction(proposal_id: u64)]
pub struct ProposeConfigChange<'info> {
    /// The risk authority.
    pub proposer: Signer<'info>,
    /// Pays the proposal rent; may be the proposer.
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(
        mut,
        seeds = [ASSET_STATE_SEED, market.key().as_ref()],
        bump = asset_state.bump,
    )]
    pub asset_state: Box<Account<'info, AssetState>>,
    #[account(
        init,
        payer = payer,
        space = ConfigProposal::ACCOUNT_LEN,
        seeds = [CONFIG_PROPOSAL_SEED, market.key().as_ref(), &proposal_id.to_le_bytes()],
        bump,
    )]
    pub proposal: Box<Account<'info, ConfigProposal>>,
    pub system_program: Program<'info, System>,
}

/// Accounts of `execute_config_change`.
#[derive(Accounts)]
#[instruction(proposal_id: u64)]
pub struct ExecuteConfigChange<'info> {
    /// Anyone; recorded as the executor.
    pub executor: Signer<'info>,
    /// Writable because a `CountCaps` proposal writes the Market count caps.
    #[account(mut, seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(
        mut,
        seeds = [ASSET_STATE_SEED, market.key().as_ref()],
        bump = asset_state.bump,
    )]
    pub asset_state: Box<Account<'info, AssetState>>,
    #[account(
        mut,
        seeds = [CONFIG_PROPOSAL_SEED, market.key().as_ref(), &proposal_id.to_le_bytes()],
        bump = proposal.bump,
    )]
    pub proposal: Box<Account<'info, ConfigProposal>>,
    /// Read for the Lock: a `Rates` change accrues the accounting Borrow Index
    /// and must not run while the Pool is Locked (Task 7.4).
    #[account(
        address = market.pool_state,
        seeds = [POOL_STATE_SEED, market.key().as_ref()],
        bump = pool_state.bump,
    )]
    pub pool_state: Box<Account<'info, PoolState>>,
}

/// Accounts of `cancel_config_change`.
#[derive(Accounts)]
#[instruction(proposal_id: u64)]
pub struct CancelConfigChange<'info> {
    /// The risk authority or the guardian.
    pub authority: Signer<'info>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(
        mut,
        seeds = [CONFIG_PROPOSAL_SEED, market.key().as_ref(), &proposal_id.to_le_bytes()],
        bump = proposal.bump,
    )]
    pub proposal: Box<Account<'info, ConfigProposal>>,
}

/// Risk authority: stores a pending change executable after the delay.
pub fn propose_config_change_handler(
    ctx: Context<ProposeConfigChange>,
    proposal_id: u64,
    change: ConfigValue,
) -> Result<()> {
    let clock = Clock::get()?;
    let accounts = &mut *ctx.accounts;
    let proposer = accounts.proposer.key();
    let mut proposal = rules::propose(
        &accounts.market,
        &mut accounts.asset_state,
        &proposer,
        proposal_id,
        change,
        clock.unix_timestamp,
    )?;
    proposal.bump = ctx.bumps.proposal;
    emit!(ConfigChangeProposed {
        market: accounts.market.key(),
        asset_state: accounts.asset_state.key(),
        proposal: accounts.proposal.key(),
        proposal_id,
        group: proposal.group,
        proposer,
        old_value: proposal.old_value(),
        new_value: proposal.new_value(),
        proposed_at: proposal.proposed_at,
        earliest_execute_at: proposal.earliest_execute_at,
        expires_at: proposal.expires_at,
        proposal_generation: proposal.proposal_generation,
        slot: clock.slot,
    });
    let stored: &mut ConfigProposal = &mut accounts.proposal;
    *stored = proposal;
    Ok(())
}

/// Anyone: applies a pending change after its delay and bumps the Asset
/// State generation.
pub fn execute_config_change_handler(
    ctx: Context<ExecuteConfigChange>,
    proposal_id: u64,
) -> Result<()> {
    let clock = Clock::get()?;
    let accounts = &mut *ctx.accounts;
    let executor = accounts.executor.key();
    if accounts.proposal.group == ConfigGroup::Rates {
        ensure_pool_unlocked_for_index_mutation(&accounts.pool_state)?;
    }
    let done = rules::execute(
        &mut accounts.market,
        &mut accounts.asset_state,
        &mut accounts.proposal,
        &executor,
        clock.unix_timestamp,
    )?;
    emit!(ConfigChangeExecuted {
        market: accounts.market.key(),
        asset_state: accounts.asset_state.key(),
        proposal: accounts.proposal.key(),
        proposal_id,
        group: accounts.proposal.group,
        executor,
        old_value: done.old_value,
        new_value: done.new_value,
        old_generation: done.old_generation,
        new_generation: done.new_generation,
        executed_at: clock.unix_timestamp,
        slot: clock.slot,
    });
    Ok(())
}

/// Risk authority or guardian: cancels a pending change.
pub fn cancel_config_change_handler(
    ctx: Context<CancelConfigChange>,
    proposal_id: u64,
) -> Result<()> {
    let clock = Clock::get()?;
    let accounts = &mut *ctx.accounts;
    let signer = accounts.authority.key();
    rules::cancel(
        &accounts.market,
        &mut accounts.proposal,
        &signer,
        clock.unix_timestamp,
    )?;
    emit!(ConfigChangeCancelled {
        market: accounts.market.key(),
        proposal: accounts.proposal.key(),
        proposal_id,
        group: accounts.proposal.group,
        cancelled_by: signer,
        cancelled_at: clock.unix_timestamp,
        slot: clock.slot,
    });
    Ok(())
}
