//! Reserve withdrawal abandonment and the treasury account setter (Task 7.5).
//!
//! `reserve_withdrawal` and its callback queue or receive an Arcium
//! computation, so their account structs live in `lib.rs`. The plain
//! instructions are here. Rules live in `state::reserve_withdrawal`.

use anchor_lang::prelude::*;
use anchor_spl::token::TokenAccount;

use crate::events::{ReserveWithdrawalAbandoned, TreasuryUsdcAccountChanged};
use crate::market::{Market, MARKET_SEED, POOL_STATE_SEED};
use crate::operation::OperationIntent;
use crate::state::reserve_withdrawal as rules;
use crate::state::shield::TokenAccountView;
use crate::state::PoolState;

/// Accounts of `abandon_reserve_withdrawal`.
#[derive(Accounts)]
pub struct AbandonReserveWithdrawal<'info> {
    /// Any signer. Abandonment moves no value: no token moved at queue time.
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
    /// The queued reserve withdrawal. `state::pool_mutation` checks its kind
    /// and that it holds the Pool Lock.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

/// Permissionless: abandons a queued reserve withdrawal after Lock expiry.
pub fn abandon_reserve_withdrawal_handler(ctx: Context<AbandonReserveWithdrawal>) -> Result<()> {
    let clock = Clock::get()?;
    let accounts = &mut *ctx.accounts;
    let pool_key = accounts.pool_state.key();
    rules::abandon_reserve_withdrawal(
        &mut accounts.pool_state,
        pool_key,
        &mut accounts.operation_intent,
        clock.slot,
    )?;
    emit!(ReserveWithdrawalAbandoned {
        pool_state: pool_key,
        operation_intent: accounts.operation_intent.key(),
        generation: accounts.pool_state.coordination.generation,
        signer: accounts.signer.key(),
        slot: clock.slot,
    });
    Ok(())
}

/// Accounts of `set_treasury_usdc_account`.
#[derive(Accounts)]
pub struct SetTreasuryUsdcAccount<'info> {
    /// The current admin.
    pub admin: Signer<'info>,
    #[account(mut, seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    /// The new treasury USDC token account (classic SPL Token only).
    pub treasury_usdc_account: Box<Account<'info, TokenAccount>>,
}

/// Admin: records the single treasury USDC token account.
pub fn set_treasury_usdc_account_handler(ctx: Context<SetTreasuryUsdcAccount>) -> Result<()> {
    let accounts = &mut *ctx.accounts;
    let market_key = accounts.market.key();
    let admin = accounts.admin.key();
    let account = TokenAccountView {
        key: accounts.treasury_usdc_account.key(),
        mint: accounts.treasury_usdc_account.mint,
        owner: accounts.treasury_usdc_account.owner,
    };
    let old_account = rules::set_treasury_usdc_account(&mut accounts.market, &admin, &account)?;
    emit!(TreasuryUsdcAccountChanged {
        market: market_key,
        old_account,
        new_account: account.key,
        changed_by: admin,
        slot: Clock::get()?.slot,
    });
    Ok(())
}
