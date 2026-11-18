//! `abandon_disclosure`: recovery of a queued `disclose_position` whose
//! callback never arrived. The Position is not Locked, so only the Intent is
//! closed and the previous Disclosure stays in place.

use anchor_lang::prelude::*;

use crate::events::DisclosureAbandoned;
use crate::operation::OperationIntent;
use crate::state::disclosure as rules;

/// Accounts of `abandon_disclosure`.
#[derive(Accounts)]
pub struct AbandonDisclosure<'info> {
    /// Any signer; nothing moves.
    pub signer: Signer<'info>,
    /// The queued disclosure.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

/// Permissionless: abandons a queued disclosure after its grace window.
pub fn abandon_disclosure_handler(ctx: Context<AbandonDisclosure>) -> Result<()> {
    let clock = Clock::get()?;
    let accounts = &mut *ctx.accounts;
    let position_key = accounts
        .operation_intent
        .position
        .ok_or(crate::errors::ErrorCode::DisclosureIntentMismatch)?;
    rules::abandon_disclosure(position_key, &mut accounts.operation_intent, clock.slot)?;
    emit!(DisclosureAbandoned {
        position: position_key,
        operation_intent: accounts.operation_intent.key(),
        signer: accounts.signer.key(),
        slot: clock.slot,
    });
    Ok(())
}
