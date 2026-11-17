//! Immediate cap decreases (Task 4.5, Requirements 4.5, 4.6, and 4.10).
//!
//! Lowering a USDC cap or a count cap only blocks new risk, so the risk
//! authority may apply it at once. Raises go through the Task 4.4 timelock
//! (`propose_config_change` with a `Caps` or `CountCaps` value). Rules live in
//! `market::caps`.

use anchor_lang::prelude::*;

use crate::events::CapsDecreased;
use crate::market::caps as rules;
use crate::market::{AssetState, ConfigValue, Market, ASSET_STATE_SEED, MARKET_SEED};

/// Accounts of `decrease_caps`.
#[derive(Accounts)]
pub struct DecreaseCaps<'info> {
    /// The risk authority.
    pub authority: Signer<'info>,
    #[account(mut, seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(
        mut,
        seeds = [ASSET_STATE_SEED, market.key().as_ref()],
        bump = asset_state.bump,
    )]
    pub asset_state: Box<Account<'info, AssetState>>,
}

/// Risk authority: lowers the USDC caps or the count caps immediately and
/// bumps the Asset State generation.
pub fn decrease_caps_handler(ctx: Context<DecreaseCaps>, change: ConfigValue) -> Result<()> {
    let clock = Clock::get()?;
    let accounts = &mut *ctx.accounts;
    let authority = accounts.authority.key();
    let done = rules::decrease_caps(
        &mut accounts.market,
        &mut accounts.asset_state,
        &authority,
        change,
    )?;
    emit!(CapsDecreased {
        market: accounts.market.key(),
        asset_state: accounts.asset_state.key(),
        group: change.group(),
        authority,
        old_value: done.old_value,
        new_value: done.new_value,
        old_generation: done.old_generation,
        new_generation: done.new_generation,
        slot: clock.slot,
    });
    Ok(())
}
