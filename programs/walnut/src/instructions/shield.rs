//! Shield refund and abandonment (Task 5.3).
//!
//! `shield`, `retry_shield`, and the `shield_credit` callback queue or receive
//! Arcium computations, so their account structs live in `lib.rs`. The two
//! plain instructions are here:
//!
//! * `refund_shield` (owner): returns exactly the recorded amount from the
//!   asset vault, signed by the vault-authority PDA, once the active attempt
//!   ended without a credit. Not pausable.
//! * `abandon_shield` (any signer): timeout recovery of a queued attempt after
//!   Lock expiry. The Pending Shield stays `Deposited`.
//!
//! Rules live in `state::shield`.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, TransferChecked};

use crate::events::{ShieldAbandoned, ShieldRefunded};
use crate::market::{Market, VaultAuthority, MARKET_SEED, VAULT_AUTHORITY_SEED};
use crate::operation::OperationIntent;
use crate::state::pending_shield::PENDING_SHIELD_SEED;
use crate::state::position::POSITION_ACCOUNT_SEED;
use crate::state::shield::{self as rules, TokenAccountView};
use crate::state::{PendingShield, Position};

/// Accounts of `refund_shield`.
#[derive(Accounts)]
pub struct RefundShield<'info> {
    /// The Pending Shield owner. `state::shield` checks it.
    pub owner: Signer<'info>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    /// Signs the vault transfer.
    #[account(
        seeds = [VAULT_AUTHORITY_SEED, market.key().as_ref()],
        bump = market.vault_authority_bump,
        address = market.vault_authority,
    )]
    pub vault_authority: Box<Account<'info, VaultAuthority>>,
    #[account(
        mut,
        seeds = [
            PENDING_SHIELD_SEED,
            pending_shield.position.as_ref(),
            &pending_shield.sequence.to_le_bytes(),
        ],
        bump = pending_shield.bump,
    )]
    pub pending_shield: Box<Account<'info, PendingShield>>,
    /// The Pending Shield's active attempt; it must be terminal.
    #[account(address = pending_shield.active_intent)]
    pub active_intent: Box<Account<'info, OperationIntent>>,
    /// The asset mint (classic SPL Token only).
    pub mint: Box<Account<'info, Mint>>,
    /// The asset vault.
    #[account(mut)]
    pub vault: Box<Account<'info, TokenAccount>>,
    /// A token account of the asset owned by the Pending Shield owner.
    #[account(mut)]
    pub destination: Box<Account<'info, TokenAccount>>,
    /// Classic SPL Token only; Token-2022 is rejected.
    pub token_program: Program<'info, Token>,
}

/// Owner-signed: refunds a `Deposited` Pending Shield whose active attempt
/// ended without a credit, moving exactly the recorded amount out of the
/// vault in this instruction.
pub fn refund_shield_handler(ctx: Context<RefundShield>) -> Result<()> {
    let clock = Clock::get()?;
    let accounts = &mut *ctx.accounts;
    let market_key = accounts.market.key();
    let vault_key = accounts.vault.key();
    let destination = TokenAccountView {
        key: accounts.destination.key(),
        mint: accounts.destination.mint,
        owner: accounts.destination.owner,
    };
    let refund = rules::refund_shield(
        &accounts.market,
        &mut accounts.pending_shield,
        &accounts.owner.key(),
        &accounts.active_intent,
        &accounts.mint.key(),
        &vault_key,
        &destination,
    )?;

    let bump = [accounts.market.vault_authority_bump];
    let seeds: &[&[u8]] = &[VAULT_AUTHORITY_SEED, market_key.as_ref(), &bump];
    let signer = &[seeds];
    token::transfer_checked(
        CpiContext::new_with_signer(
            accounts.token_program.key(),
            TransferChecked {
                from: accounts.vault.to_account_info(),
                mint: accounts.mint.to_account_info(),
                to: accounts.destination.to_account_info(),
                authority: accounts.vault_authority.to_account_info(),
            },
            signer,
        ),
        refund.amount,
        accounts.mint.decimals,
    )?;

    emit!(ShieldRefunded {
        market: market_key,
        position: accounts.pending_shield.position,
        owner: accounts.pending_shield.owner,
        pending_shield: accounts.pending_shield.key(),
        asset: refund.asset,
        amount: refund.amount,
        vault: vault_key,
        destination: refund.destination,
        slot: clock.slot,
    });
    Ok(())
}

/// Accounts of `abandon_shield`.
#[derive(Accounts)]
pub struct AbandonShield<'info> {
    /// Any signer. Abandonment cannot redirect value (Requirement 3.10).
    pub signer: Signer<'info>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    #[account(
        seeds = [
            PENDING_SHIELD_SEED,
            position.key().as_ref(),
            &pending_shield.sequence.to_le_bytes(),
        ],
        bump = pending_shield.bump,
    )]
    pub pending_shield: Box<Account<'info, PendingShield>>,
    /// The queued attempt. `state::shield` checks that it is the Pending
    /// Shield's current attempt and holds the Position Lock.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

/// Permissionless: abandons a queued shield attempt after Lock expiry.
pub fn abandon_shield_handler(ctx: Context<AbandonShield>) -> Result<()> {
    let clock = Clock::get()?;
    let accounts = &mut *ctx.accounts;
    let position_key = accounts.position.key();
    rules::abandon_shield(
        &mut accounts.position,
        position_key,
        &accounts.pending_shield,
        &mut accounts.operation_intent,
        clock.slot,
    )?;
    emit!(ShieldAbandoned {
        position: position_key,
        owner: accounts.position.owner,
        pending_shield: accounts.pending_shield.key(),
        operation_intent: accounts.operation_intent.key(),
        generation: accounts.position.coordination.generation,
        signer: accounts.signer.key(),
        slot: clock.slot,
    });
    Ok(())
}
