//! Pause instructions (Task 4.3, Requirements 4.5, 4.7, 4.8, and 4.9).
//!
//! Each handler reads the slot, runs one pure transition from `market::pause`
//! (which authorizes through `market::authorize`), and emits one
//! `PauseUpdated` event with the old and new pause state. Pauses apply
//! immediately and do not bump the Asset State generation, so already-queued
//! work still completes through its callback.

use anchor_lang::prelude::*;

use crate::events::PauseUpdated;
use crate::market::pause as rules;
use crate::market::{AssetState, Market, PauseChange, PauseState, ASSET_STATE_SEED, MARKET_SEED};

/// Accounts of `guardian_pause`, `set_pause`, `renew_guardian_pause`, and
/// `clear_guardian_pause`.
#[derive(Accounts)]
pub struct UpdatePause<'info> {
    /// The guardian (`guardian_pause`) or the risk authority (the others).
    pub authority: Signer<'info>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(
        mut,
        seeds = [ASSET_STATE_SEED, market.key().as_ref()],
        bump = asset_state.bump,
    )]
    pub asset_state: Box<Account<'info, AssetState>>,
}

fn update(
    ctx: Context<UpdatePause>,
    change: PauseChange,
    flags: u16,
    apply: impl FnOnce(
        &Market,
        &mut PauseState,
        &Pubkey,
        u64,
    ) -> core::result::Result<(), crate::market::MarketError>,
) -> Result<()> {
    let slot = Clock::get()?.slot;
    let accounts = &mut *ctx.accounts;
    let actor = accounts.authority.key();
    let old = accounts.asset_state.pauses;
    let mut new = old;
    apply(&accounts.market, &mut new, &actor, slot)?;
    accounts.asset_state.pauses = new;
    emit!(PauseUpdated {
        market: accounts.market.key(),
        asset_state: accounts.asset_state.key(),
        change,
        actor,
        flags,
        old_flags: old.flags,
        new_flags: new.flags,
        old_guardian_flags: old.guardian_flags,
        new_guardian_flags: new.guardian_flags,
        old_guardian_expiry_slot: old.guardian_expiry_slot,
        new_guardian_expiry_slot: new.guardian_expiry_slot,
        slot,
    });
    Ok(())
}

/// Guardian: pauses `flags` until `slot + duration_slots`.
pub fn guardian_pause_handler(
    ctx: Context<UpdatePause>,
    flags: u16,
    duration_slots: u64,
) -> Result<()> {
    update(
        ctx,
        PauseChange::GuardianPause,
        flags,
        |market, pauses, signer, slot| {
            rules::guardian_pause(market, pauses, signer, flags, duration_slots, slot)
        },
    )
}

/// Risk authority: sets (`paused`) or clears durable `flags`.
pub fn set_pause_handler(ctx: Context<UpdatePause>, flags: u16, paused: bool) -> Result<()> {
    let change = if paused {
        PauseChange::GovernancePause
    } else {
        PauseChange::GovernanceUnpause
    };
    update(ctx, change, flags, |market, pauses, signer, _| {
        rules::set_pause(market, pauses, signer, flags, paused)
    })
}

/// Risk authority: moves the active guardian pause expiry to
/// `new_expiry_slot`.
pub fn renew_guardian_pause_handler(ctx: Context<UpdatePause>, new_expiry_slot: u64) -> Result<()> {
    update(
        ctx,
        PauseChange::GuardianRenew,
        0,
        |market, pauses, signer, slot| {
            rules::renew_guardian_pause(market, pauses, signer, new_expiry_slot, slot)
        },
    )
}

/// Risk authority: clears every guardian pause.
pub fn clear_guardian_pause_handler(ctx: Context<UpdatePause>) -> Result<()> {
    update(
        ctx,
        PauseChange::GuardianClear,
        0,
        |market, pauses, signer, _| rules::clear_guardian_pause(market, pauses, signer),
    )
}
