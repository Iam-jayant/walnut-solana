//! Walnut-Lend `lend_op` support: `abandon_lend_op` and the vault transfer
//! helpers shared with the callback. Queue and callback account structs live
//! in `lib.rs`; the rules live in `state::lend_op`.
//!
//! Liquidation accounts travel as `remaining_accounts` in this order:
//! `[usdc_vault, wsol_vault, vault_authority, token_program,
//! liquidator_usdc, liquidator_wsol]`. Their addresses are bound in the
//! Intent's accounts commitment.

use anchor_lang::prelude::*;
use anchor_spl::token::{self as spl, Transfer};

use crate::errors::ErrorCode;
use crate::events::LendOpAbandoned;
use crate::market::{Market, MARKET_SEED, VAULT_AUTHORITY_SEED};
use crate::operation::commitment::account_commitment;
use crate::operation::OperationIntent;
use crate::state::lend_op::{self as rules, lend_bindings, LendKeys, LiquidationKeys};
use crate::state::lending_pool::{LendingPool, LENDING_POOL_SEED};
use crate::state::position::POSITION_ACCOUNT_SEED;
use crate::state::Position;

/// Number of liquidation accounts passed as `remaining_accounts`.
pub const LIQUIDATION_REMAINING_ACCOUNTS: usize = 6;
/// Index of the USDC vault.
pub const REMAINING_USDC_VAULT: usize = 0;
/// Index of the wSOL vault.
pub const REMAINING_WSOL_VAULT: usize = 1;
/// Index of the vault authority.
pub const REMAINING_VAULT_AUTHORITY: usize = 2;
/// Index of the token program.
pub const REMAINING_TOKEN_PROGRAM: usize = 3;
/// Index of the Liquidator's USDC account.
pub const REMAINING_LIQUIDATOR_USDC: usize = 4;
/// Index of the Liquidator's wSOL account.
pub const REMAINING_LIQUIDATOR_WSOL: usize = 5;

/// Reads the liquidation keys from `remaining_accounts`.
pub fn liquidation_keys_from_remaining(remaining: &[AccountInfo]) -> Result<LiquidationKeys> {
    if remaining.len() < LIQUIDATION_REMAINING_ACCOUNTS {
        return Err(ErrorCode::LiquidationAccountsInvalid.into());
    }
    Ok(LiquidationKeys {
        usdc_vault: remaining[REMAINING_USDC_VAULT].key(),
        wsol_vault: remaining[REMAINING_WSOL_VAULT].key(),
        vault_authority: remaining[REMAINING_VAULT_AUTHORITY].key(),
        token_program: remaining[REMAINING_TOKEN_PROGRAM].key(),
        liquidator_usdc: remaining[REMAINING_LIQUIDATOR_USDC].key(),
        liquidator_wsol: remaining[REMAINING_LIQUIDATOR_WSOL].key(),
    })
}

/// Moves `amount` from the vault at `from` to the Liquidator account at `to`,
/// signed by the vault authority. The caller has already matched the
/// addresses against the Intent commitment and the Market.
pub fn transfer_from_vault<'info>(
    market: &Market,
    market_key: &Pubkey,
    remaining: &[AccountInfo<'info>],
    from: usize,
    to: usize,
    amount: u64,
) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    let bump = [market.vault_authority_bump];
    let seeds: &[&[u8]] = &[VAULT_AUTHORITY_SEED, market_key.as_ref(), &bump];
    spl::transfer(
        CpiContext::new_with_signer(
            remaining[REMAINING_TOKEN_PROGRAM].key(),
            Transfer {
                from: remaining[from].clone(),
                to: remaining[to].clone(),
                authority: remaining[REMAINING_VAULT_AUTHORITY].clone(),
            },
            &[seeds],
        ),
        amount,
    )
}

/// Accounts of `abandon_lend_op`.
#[derive(Accounts)]
pub struct AbandonLendOp<'info> {
    /// Any signer. Abandonment cannot redirect value: a liquidation refund
    /// goes to the committed Liquidator account.
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
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    /// The queued operation; `state::lend_op` checks it.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

/// Permissionless: abandons a queued lend operation after Lock expiry. A
/// queued liquidation also refunds the Liquidator's USDC (the six
/// liquidation accounts are passed as `remaining_accounts`).
pub fn abandon_lend_op_handler(ctx: Context<AbandonLendOp>) -> Result<()> {
    let clock = Clock::get()?;
    let remaining = ctx.remaining_accounts;
    let accounts = &mut *ctx.accounts;
    let position_key = accounts.position.key();
    let market_key = accounts.market.key();
    let liquidation = rules::is_liquidation(&accounts.operation_intent);
    let repay = rules::liquidation_repay_amount(&accounts.operation_intent);
    if liquidation {
        let keys = liquidation_keys_from_remaining(remaining)?;
        keys.check_market(&accounts.market)?;
        let bound = LendKeys {
            payer: accounts.operation_intent.payer,
            market: market_key,
            asset_state: accounts.market.asset_state,
            lending_pool: accounts.lending_pool.key(),
            position: position_key,
            position_owner: accounts.position.owner,
            position_generation: accounts.operation_intent.position_generation,
            liquidation: Some(keys),
        };
        if account_commitment(&lend_bindings(&bound))
            != accounts.operation_intent.accounts_commitment
        {
            return Err(ErrorCode::LiquidationAccountsInvalid.into());
        }
    }
    rules::abandon_lend_op(
        &mut accounts.position,
        position_key,
        &mut accounts.operation_intent,
        clock.slot,
    )?;
    if liquidation {
        transfer_from_vault(
            &accounts.market,
            &market_key,
            remaining,
            REMAINING_USDC_VAULT,
            REMAINING_LIQUIDATOR_USDC,
            repay,
        )?;
    }
    emit!(LendOpAbandoned {
        position: position_key,
        operation_intent: accounts.operation_intent.key(),
        op: accounts.operation_intent.asset_generation,
        refunded_usdc: if liquidation { repay } else { 0 },
        signer: accounts.signer.key(),
        slot: clock.slot,
    });
    Ok(())
}
