//! `init_lending_pool` (walnut-lend): creates the `LendingPool`, seeds its
//! supply totals from the USDC vault balance and records the oracle
//! configuration. Admin only and idempotent: a repeat call updates only the
//! oracle account, feed id and maximum price age.

use anchor_lang::prelude::*;
use anchor_spl::token::TokenAccount;

use crate::errors::ErrorCode;
use crate::events::LendingPoolInitialized;
use crate::market::roles::{authorize, Role};
use crate::market::{Market, MARKET_SEED};
use crate::state::lending_pool::{LendingPool, LENDING_POOL_SEED};

/// Accounts of `init_lending_pool`.
#[derive(Accounts)]
pub struct InitLendingPool<'info> {
    /// The Market admin; pays the account rent.
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(
        init_if_needed,
        payer = admin,
        space = LendingPool::ACCOUNT_LEN,
        seeds = [LENDING_POOL_SEED, market.key().as_ref()],
        bump,
    )]
    pub lending_pool: Box<Account<'info, LendingPool>>,
    /// The USDC vault whose balance seeds the supply totals.
    #[account(address = market.usdc_vault)]
    pub usdc_vault: Box<Account<'info, TokenAccount>>,
    pub system_program: Program<'info, System>,
}

/// Admin: creates the pool (seeding `total_supply_shares` from the vault
/// balance at `supply_index = 1e9`) or, when it exists, updates the oracle
/// configuration only.
pub fn init_lending_pool_handler(
    ctx: Context<InitLendingPool>,
    max_price_age_seconds: u32,
    oracle: Pubkey,
    feed_id: [u8; 32],
) -> Result<()> {
    let clock = Clock::get()?;
    let bump = ctx.bumps.lending_pool;
    let accounts = &mut *ctx.accounts;
    authorize(&accounts.market, Role::Admin, &accounts.admin.key())?;
    if max_price_age_seconds == 0 || oracle == Pubkey::default() {
        return Err(ErrorCode::LendPoolConfigInvalid.into());
    }
    let market_key = accounts.market.key();
    let created = accounts.lending_pool.market == Pubkey::default();
    if created {
        let mut pool = LendingPool::new(
            market_key,
            bump,
            oracle,
            feed_id,
            max_price_age_seconds,
            clock.unix_timestamp,
        );
        pool.total_supply_shares = u128::from(accounts.usdc_vault.amount);
        accounts.lending_pool.set_inner(pool);
    } else {
        accounts.lending_pool.oracle = oracle;
        accounts.lending_pool.feed_id = feed_id;
        accounts.lending_pool.max_price_age_seconds = max_price_age_seconds;
    }
    emit!(LendingPoolInitialized {
        market: market_key,
        lending_pool: accounts.lending_pool.key(),
        oracle,
        feed_id,
        max_price_age_seconds,
        created,
        total_supply_shares: accounts.lending_pool.total_supply_shares,
        slot: clock.slot,
    });
    Ok(())
}

/// Accounts of `set_lending_rate_source`.
#[derive(Accounts)]
pub struct SetLendingRateSource<'info> {
    /// The Market admin.
    pub admin: Signer<'info>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    /// Risk and rate parameters and the last revealed utilization band.
    #[account(address = market.asset_state)]
    pub asset_state: Box<Account<'info, crate::market::AssetState>>,
    #[account(
        mut,
        seeds = [LENDING_POOL_SEED, market.key().as_ref()],
        bump = lending_pool.bump,
    )]
    pub lending_pool: Box<Account<'info, LendingPool>>,
}

/// Admin: switches how the LendingPool indexes accrue (ADR-0004).
/// `RATE_SOURCE_TOTALS` (0) uses the public totals; `RATE_SOURCE_BAND` (1)
/// uses the revealed utilization band of the encrypted Pool. Interest up to
/// now accrues with the old source first, so no period is counted twice.
pub fn set_lending_rate_source_handler(
    ctx: Context<SetLendingRateSource>,
    rate_source: u8,
) -> Result<()> {
    use crate::state::lending_pool::{RATE_SOURCE_BAND, RATE_SOURCE_TOTALS};
    let clock = Clock::get()?;
    let accounts = &mut *ctx.accounts;
    authorize(&accounts.market, Role::Admin, &accounts.admin.key())?;
    if rate_source != RATE_SOURCE_TOTALS && rate_source != RATE_SOURCE_BAND {
        return Err(ErrorCode::LendPoolConfigInvalid.into());
    }
    accounts
        .lending_pool
        .accrue_auto(clock.unix_timestamp, &accounts.asset_state)?;
    let previous = accounts.lending_pool.rate_source;
    accounts.lending_pool.rate_source = rate_source;
    emit!(crate::events::LendingRateSourceSet {
        market: accounts.market.key(),
        lending_pool: accounts.lending_pool.key(),
        previous,
        rate_source,
        slot: clock.slot,
    });
    Ok(())
}
