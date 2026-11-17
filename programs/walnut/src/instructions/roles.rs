//! Role instructions (Task 4.2, Requirements 4.3 and 4.4).
//!
//! Each handler reads the signer, runs one pure transition from
//! `market::roles` (which authorizes through `market::authorize`), and emits
//! one event with the old key, the new key, and the slot. Signers are plain
//! Anchor `Signer`s, so a Squads multisig vault PDA signing through CPI is
//! accepted like a keypair. None of these accounts pays rent, so no signer is
//! writable.
//!
//! Role changes apply immediately: the design does not place them behind the
//! Task 4.4 timelock. A timelocked path would authorize its own proposal and
//! then call `market::apply_role_change`.

use anchor_lang::prelude::*;

use crate::events::{
    AdminTransferAccepted, AdminTransferCancelled, AdminTransferProposed, RoleAuthorityChanged,
};
use crate::market::roles as rules;
use crate::market::{Market, ProgramAccountView, RoleKind, MARKET_SEED};

/// Accounts of `propose_admin`.
#[derive(Accounts)]
pub struct ProposeAdmin<'info> {
    /// The current admin.
    pub admin: Signer<'info>,
    #[account(mut, seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
}

/// Accounts of `accept_admin`.
#[derive(Accounts)]
pub struct AcceptAdmin<'info> {
    /// The pending admin.
    pub pending_admin: Signer<'info>,
    #[account(mut, seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
}

/// Accounts of `cancel_admin_transfer`.
#[derive(Accounts)]
pub struct CancelAdminTransfer<'info> {
    /// The current admin.
    pub admin: Signer<'info>,
    #[account(mut, seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
}

/// Accounts of `set_risk_authority`, `set_treasury_authority`, and
/// `set_guardian_authority`.
#[derive(Accounts)]
pub struct SetRoleAuthority<'info> {
    /// The current admin.
    pub admin: Signer<'info>,
    #[account(mut, seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
}

/// Accounts of `set_upgrade_authority`.
#[derive(Accounts)]
pub struct SetUpgradeAuthority<'info> {
    /// The current admin.
    pub admin: Signer<'info>,
    #[account(mut, seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    /// CHECK: the Walnut program account. Its address is fixed here and its
    /// loader state is parsed by `check_loader_upgrade_authority`.
    #[account(address = crate::ID)]
    pub walnut_program: UncheckedAccount<'info>,
    /// CHECK: the program-data account. `check_loader_upgrade_authority`
    /// requires it to be the one the program account points at, owned by the
    /// loader, and naming the new upgrade authority.
    pub program_data: UncheckedAccount<'info>,
}

/// Step one of the admin transfer.
pub fn propose_admin_handler(ctx: Context<ProposeAdmin>, new_admin: Pubkey) -> Result<()> {
    let market_key = ctx.accounts.market.key();
    let admin = ctx.accounts.admin.key();
    let old_pending_admin =
        rules::propose_admin(&mut ctx.accounts.market, &market_key, &admin, new_admin)?;
    emit!(AdminTransferProposed {
        market: market_key,
        admin,
        old_pending_admin,
        new_pending_admin: new_admin,
        slot: Clock::get()?.slot,
    });
    Ok(())
}

/// Step two of the admin transfer.
pub fn accept_admin_handler(ctx: Context<AcceptAdmin>) -> Result<()> {
    let market_key = ctx.accounts.market.key();
    let new_admin = ctx.accounts.pending_admin.key();
    let old_admin = rules::accept_admin(&mut ctx.accounts.market, &new_admin)?;
    emit!(AdminTransferAccepted {
        market: market_key,
        old_admin,
        new_admin,
        slot: Clock::get()?.slot,
    });
    Ok(())
}

/// Withdraws a pending admin transfer.
pub fn cancel_admin_transfer_handler(ctx: Context<CancelAdminTransfer>) -> Result<()> {
    let market_key = ctx.accounts.market.key();
    let admin = ctx.accounts.admin.key();
    let old_pending_admin = rules::cancel_admin_transfer(&mut ctx.accounts.market, &admin)?;
    emit!(AdminTransferCancelled {
        market: market_key,
        admin,
        old_pending_admin,
        new_pending_admin: Pubkey::default(),
        slot: Clock::get()?.slot,
    });
    Ok(())
}

/// Sets the risk, treasury, or guardian authority.
pub fn set_role_authority_handler(
    ctx: Context<SetRoleAuthority>,
    role: RoleKind,
    new_authority: Pubkey,
) -> Result<()> {
    let market_key = ctx.accounts.market.key();
    let admin = ctx.accounts.admin.key();
    let old_authority = rules::set_role(
        &mut ctx.accounts.market,
        &market_key,
        &admin,
        role,
        new_authority,
    )?;
    emit!(RoleAuthorityChanged {
        market: market_key,
        role,
        old_authority,
        new_authority,
        changed_by: admin,
        slot: Clock::get()?.slot,
    });
    Ok(())
}

/// Records a rotated program upgrade authority that the loader already names.
pub fn set_upgrade_authority_handler(
    ctx: Context<SetUpgradeAuthority>,
    new_authority: Pubkey,
) -> Result<()> {
    let accounts = &mut *ctx.accounts;
    let market_key = accounts.market.key();
    let admin = accounts.admin.key();
    let program_info = accounts.walnut_program.to_account_info();
    let program_data_info = accounts.program_data.to_account_info();
    let old_authority = {
        let program_bytes = program_info.try_borrow_data()?;
        let program_data_bytes = program_data_info.try_borrow_data()?;
        rules::set_upgrade_authority(
            &mut accounts.market,
            &market_key,
            &admin,
            &ProgramAccountView {
                key: program_info.key(),
                owner: *program_info.owner,
                data: &program_bytes,
            },
            &ProgramAccountView {
                key: program_data_info.key(),
                owner: *program_data_info.owner,
                data: &program_data_bytes,
            },
            new_authority,
        )?
    };
    emit!(RoleAuthorityChanged {
        market: market_key,
        role: RoleKind::Upgrade,
        old_authority,
        new_authority,
        changed_by: admin,
        slot: Clock::get()?.slot,
    });
    Ok(())
}
