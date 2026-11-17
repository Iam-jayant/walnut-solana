//! Unshield, its callback, and abandonment (Task 5.4; Requirements 5.7, 5.8,
//! 3.1-3.10, 4.8).
//!
//! Pure Rust rules behind the `unshield` instruction, the `unshield` callback,
//! and `abandon_unshield`. The Anchor handlers in `lib.rs` and
//! `instructions/unshield.rs` load accounts, perform the SPL transfer, and
//! queue the computation; every state change goes through these functions.
//!
//! # Flow
//!
//! 1. `unshield(asset, amount)` (owner-signed): [`queue_unshield`] checks the
//!    Market, the `UNSHIELD` pause, the nonzero public `amount`, that the
//!    Position is `Active` with a free Lock, and that the destination is a
//!    token account of the asset's mint owned by the Position owner (and not
//!    the vault). It creates the Intent, Locks the Position, and the handler
//!    queues the `unshield` circuit. No token moves yet.
//! 2. The circuit approves exactly the requested amount or zero (never a
//!    partial amount) and re-seals the Position. The approved and requested
//!    amounts are revealed.
//! 3. Callback ([`settle_unshield`]): `Applied` (approved) stores the new
//!    state, bumps the generation, releases the Lock, and returns
//!    [`UnshieldSettlement::Approved`]; the handler then transfers exactly
//!    that amount from the vault to the committed destination, signed by the
//!    vault-authority PDA. `Zero` (insufficient free balance) releases the
//!    Lock, stores nothing, and moves nothing. `Expired`, `Stale`, and failed
//!    outcomes release the Lock and move nothing. A terminal Intent is a
//!    no-op. Unauthenticated output and every binding mismatch revert, so the
//!    Intent stays `Queued` until [`abandon_unshield`].
//! 4. `abandon_unshield` (any signer, after Lock expiry): releases the Lock.
//!    Nothing was transferred, so there is nothing to refund.
//!
//! A token transfer therefore happens only for a verified `Applied` callback
//! with a nonzero approved amount equal to the committed request.
//!
//! # SOL
//!
//! The asset's mint is the wSOL mint, so the callback delivers wSOL to the
//! owner's wSOL token account. Native SOL unwrap is a separate owner-signed
//! transaction (`buildUnwrapWsolTransaction` in the SDK): the program never
//! closes token accounts.
//!
//! # Intent binding
//!
//! `accounts_commitment` covers, in order: owner (signer, payer), Market,
//! Asset State, Position (captured generation), the asset vault, the mint,
//! the destination token account (mint, owner), the vault-authority PDA, and
//! the SPL Token program. The callback rebuilds the bindings from the
//! accounts it actually received, so substituting any of them (notably the
//! destination) fails the commitment check. `inputs_commitment` covers the
//! Viewing public key, the key epoch, the stored state nonce, the asset, the
//! requested amount, the five stored state ciphertexts, and the output-nonce
//! input (`state::nonce`). The circuit echoes the requested amount so the
//! callback can rebuild the commitment even when the approved amount is zero.

use anchor_lang::prelude::Pubkey;

use super::accounts::{EncryptedPositionPayload, Position, PositionStatus};
use super::coordination::{acquire_locks, CoordinationSlot};
use super::nonce::{derive_output_nonce_input, INPUT_OUTPUT_NONCE};
use super::pending_shield::ShieldAsset;
use super::position::{INPUT_NONCE, INPUT_VIEWING_KEY_EPOCH, INPUT_VIEWING_PUBKEY};
use super::shield::{
    asset_mint, asset_tag, asset_vault, TokenAccountView, INPUT_AMOUNT, INPUT_ASSET,
    INPUT_STATE_CIPHERTEXT_FIRST,
};
use super::Slot;
use crate::errors::ErrorCode;
use crate::market::{ensure_not_paused, Market, MarketStatus, PauseCategory, PauseState};
use crate::operation::callback::{
    apply_decision_to_slots, classify_callback, CallbackContext, CallbackDecision, CallbackReport,
    CurrentDependencies,
};
use crate::operation::commitment::{
    account_commitment, input_commitment, AccountBinding, AccountRole, InputField, InputValue,
};
use crate::operation::seeds::reserve_intent;
use crate::operation::{IntentStatus, OperationCounter, OperationIntent, OperationKind};

/// The upgradeable-free BPF loader that owns the SPL Token program account.
const BPF_LOADER_PROGRAM: Pubkey =
    Pubkey::from_str_const("BPFLoader2111111111111111111111111111111111");

/// Decision Expiry distance of an unshield, in slots. The circuit reads no
/// price or index, so a fixed window is used, as for shield.
pub const UNSHIELD_DECISION_SLOTS: u64 = 1_500;

/// Slots between Decision Expiry and Lock expiry.
pub const UNSHIELD_LOCK_GRACE_SLOTS: u64 = 300;

/// Unshield failures. Mapped onto program errors in `errors.rs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnshieldError {
    /// The requested amount is zero.
    ZeroAmount,
    /// The signer is not the Position owner.
    NotOwner,
    /// The Position is not `Active`.
    PositionNotActive,
    /// The mint is not the Market mint of the asset.
    MintMismatch,
    /// The vault is not the Market vault of the asset.
    VaultMismatch,
    /// The destination has the wrong mint or owner, or is the vault.
    DestinationInvalid,
    /// The Intent is not this Position's queued unshield.
    IntentMismatch,
    /// The revealed approved amount is not the request or zero.
    ApprovalInvalid,
}

/// The mint-to-asset mapping of the Market, or `None` for any other mint.
pub fn asset_of_mint(market: &Market, mint: &Pubkey) -> Option<ShieldAsset> {
    if *mint == market.wsol_mint {
        Some(ShieldAsset::Sol)
    } else if *mint == market.usdc_mint {
        Some(ShieldAsset::Usdc)
    } else {
        None
    }
}

/// `unshield` account rule: `mint` and `vault` are the Market's for `asset`,
/// and `destination` holds that mint, is owned by the Position owner, and is
/// not the vault. Value can only go to the owner.
pub fn check_unshield_accounts(
    market: &Market,
    asset: ShieldAsset,
    owner: &Pubkey,
    mint: &Pubkey,
    vault: &Pubkey,
    destination: &TokenAccountView,
) -> Result<(), UnshieldError> {
    if *mint != asset_mint(market, asset) {
        return Err(UnshieldError::MintMismatch);
    }
    if *vault != asset_vault(market, asset) {
        return Err(UnshieldError::VaultMismatch);
    }
    if destination.mint != *mint || destination.owner != *owner || destination.key == *vault {
        return Err(UnshieldError::DestinationInvalid);
    }
    Ok(())
}

/// The accounts an Unshield Intent binds besides the owner, Market, Asset
/// State, and Position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnshieldKeys {
    /// The asset vault.
    pub vault: Pubkey,
    /// The asset mint.
    pub mint: Pubkey,
    /// The destination token account.
    pub destination: Pubkey,
    /// The vault-authority PDA that signs the transfer.
    pub vault_authority: Pubkey,
    /// The SPL Token program.
    pub token_program: Pubkey,
}

/// The ordered accounts bound by an Unshield Intent.
pub fn unshield_bindings(
    owner: Pubkey,
    market_key: Pubkey,
    asset_state: Pubkey,
    position: Pubkey,
    position_generation: Option<u64>,
    keys: &UnshieldKeys,
) -> [AccountBinding; 9] {
    let program = crate::ID;
    let token_program = anchor_spl::token::ID;
    [
        AccountBinding {
            key: owner,
            role: AccountRole::Owner,
            expected_owner: anchor_lang::system_program::ID,
            mint: None,
            authority: None,
            writable: true,
            generation: None,
        },
        AccountBinding {
            key: market_key,
            role: AccountRole::Market,
            expected_owner: program,
            mint: None,
            authority: None,
            writable: false,
            generation: None,
        },
        AccountBinding {
            key: asset_state,
            role: AccountRole::AssetState,
            expected_owner: program,
            mint: None,
            authority: None,
            writable: false,
            generation: None,
        },
        AccountBinding {
            key: position,
            role: AccountRole::Position,
            expected_owner: program,
            mint: None,
            authority: Some(owner),
            writable: true,
            generation: position_generation,
        },
        AccountBinding {
            key: keys.vault,
            role: AccountRole::Vault,
            expected_owner: token_program,
            mint: Some(keys.mint),
            authority: Some(keys.vault_authority),
            writable: true,
            generation: None,
        },
        AccountBinding {
            key: keys.mint,
            role: AccountRole::Mint,
            expected_owner: token_program,
            mint: None,
            authority: None,
            writable: false,
            generation: None,
        },
        AccountBinding {
            key: keys.destination,
            role: AccountRole::TokenAccount,
            expected_owner: token_program,
            mint: Some(keys.mint),
            authority: Some(owner),
            writable: true,
            generation: None,
        },
        AccountBinding {
            key: keys.vault_authority,
            role: AccountRole::Other,
            expected_owner: program,
            mint: None,
            authority: None,
            writable: false,
            generation: None,
        },
        AccountBinding {
            key: keys.token_program,
            role: AccountRole::Other,
            expected_owner: BPF_LOADER_PROGRAM,
            mint: None,
            authority: None,
            writable: false,
            generation: None,
        },
    ]
}

/// The ordered inputs bound by an Unshield Intent.
pub fn unshield_inputs(
    viewing_pubkey: [u8; 32],
    viewing_key_epoch: u64,
    state: &EncryptedPositionPayload,
    asset: ShieldAsset,
    amount: u64,
    output_nonce_input: u128,
) -> [InputField<'static>; 11] {
    let c = &state.ciphertexts;
    let first = INPUT_STATE_CIPHERTEXT_FIRST;
    [
        InputField::new(INPUT_VIEWING_PUBKEY, InputValue::Bytes32(viewing_pubkey)),
        InputField::new(INPUT_VIEWING_KEY_EPOCH, InputValue::U64(viewing_key_epoch)),
        InputField::new(INPUT_NONCE, InputValue::U128(state.nonce)),
        InputField::new(INPUT_ASSET, InputValue::U8(asset_tag(asset))),
        InputField::new(INPUT_AMOUNT, InputValue::U64(amount)),
        InputField::new(first, InputValue::Bytes32(c[0])),
        InputField::new(first + 1, InputValue::Bytes32(c[1])),
        InputField::new(first + 2, InputValue::Bytes32(c[2])),
        InputField::new(first + 3, InputValue::Bytes32(c[3])),
        InputField::new(first + 4, InputValue::Bytes32(c[4])),
        InputField::new(INPUT_OUTPUT_NONCE, InputValue::U128(output_nonce_input)),
    ]
}

/// One `unshield` queue request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnshieldRequest {
    /// The Operation Intent PDA address.
    pub intent_key: Pubkey,
    /// The owner's next Intent sequence.
    pub sequence: u64,
    /// The Arcium computation offset.
    pub computation_offset: u64,
    /// The current slot.
    pub queued_slot: Slot,
    /// The asset to unshield.
    pub asset: ShieldAsset,
    /// The public requested amount.
    pub amount: u64,
    /// The destination token account.
    pub destination: Pubkey,
}

/// The keys an Unshield Intent binds for `asset`, from the Market.
pub fn unshield_keys(market: &Market, asset: ShieldAsset, destination: Pubkey) -> UnshieldKeys {
    UnshieldKeys {
        vault: asset_vault(market, asset),
        mint: asset_mint(market, asset),
        destination,
        vault_authority: market.vault_authority,
        token_program: anchor_spl::token::ID,
    }
}

/// Builds the Unshield Intent for `request` against `position`.
pub fn build_unshield_intent(
    market: &Market,
    market_key: Pubkey,
    position: &Position,
    position_key: Pubkey,
    request: &UnshieldRequest,
) -> Result<OperationIntent, ErrorCode> {
    let decision_expiry_slot = request
        .queued_slot
        .checked_add(UNSHIELD_DECISION_SLOTS)
        .ok_or(ErrorCode::InvalidIntentTiming)?;
    let lock_expiry_slot = decision_expiry_slot
        .checked_add(UNSHIELD_LOCK_GRACE_SLOTS)
        .ok_or(ErrorCode::InvalidIntentTiming)?;
    let generation = Some(position.coordination.generation);
    let bindings = unshield_bindings(
        position.owner,
        market_key,
        market.asset_state,
        position_key,
        generation,
        &unshield_keys(market, request.asset, request.destination),
    );
    let inputs = unshield_inputs(
        position.viewing_pubkey,
        position.viewing_key_epoch,
        &position.encrypted_state,
        request.asset,
        request.amount,
        derive_output_nonce_input(&position_key, request.sequence),
    );
    Ok(OperationIntent {
        key: request.intent_key,
        owner: position.owner,
        payer: position.owner,
        reclaim_authority: position.owner,
        kind: OperationKind::Unshield,
        status: IntentStatus::Queued,
        sequence: request.sequence,
        computation_offset: request.computation_offset,
        accounts_commitment: account_commitment(&bindings),
        inputs_commitment: input_commitment(OperationKind::Unshield, &inputs),
        queued_slot: request.queued_slot,
        decision_expiry_slot,
        lock_expiry_slot,
        position: Some(position_key),
        position_generation: generation,
        ..OperationIntent::default()
    })
}

/// `unshield`: checks the Market, the `UNSHIELD` pause, the amount, the
/// signer, the Position, and the token accounts; builds and reserves the
/// Intent; and Locks the Position. Atomic: on error the Position and the
/// counter are unchanged.
#[allow(clippy::too_many_arguments)]
pub fn queue_unshield(
    market: &Market,
    market_key: Pubkey,
    pauses: &PauseState,
    position: &mut Position,
    position_key: Pubkey,
    counter: &mut OperationCounter,
    signer: &Pubkey,
    destination: &TokenAccountView,
    request: &UnshieldRequest,
) -> Result<OperationIntent, ErrorCode> {
    if market.status != MarketStatus::Active {
        return Err(ErrorCode::MarketNotActive);
    }
    ensure_not_paused(pauses, PauseCategory::Unshield, request.queued_slot)?;
    if request.amount == 0 {
        return Err(UnshieldError::ZeroAmount.into());
    }
    if *signer != position.owner {
        return Err(UnshieldError::NotOwner.into());
    }
    if position.status != PositionStatus::Active {
        return Err(UnshieldError::PositionNotActive.into());
    }
    if destination.key != request.destination {
        return Err(UnshieldError::DestinationInvalid.into());
    }
    check_unshield_accounts(
        market,
        request.asset,
        &position.owner,
        &asset_mint(market, request.asset),
        &asset_vault(market, request.asset),
        destination,
    )?;
    let intent = build_unshield_intent(market, market_key, position, position_key, request)?;
    let mut next_counter = *counter;
    reserve_intent(&mut next_counter, &intent)?;
    acquire_locks(
        intent.lock_owner(),
        intent.lock_expiry_slot,
        &mut [CoordinationSlot::new(
            position_key,
            &mut position.coordination,
        )],
    )?;
    *counter = next_counter;
    Ok(intent)
}

/// What the `unshield` callback observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnshieldReport {
    /// Whether Arcium `verify_output` succeeded.
    pub output_authenticated: bool,
    /// The computation offset the callback's computation account belongs to.
    pub computation_offset: u64,
    /// The Operation Intent account the callback received.
    pub intent_key: Pubkey,
    /// The output's encryption key (must be the registered Viewing Key).
    pub encryption_key: [u8; 32],
    /// The output nonce (output-nonce input + 1, see `state::nonce`).
    pub output_nonce: u128,
    /// The output ciphertexts in `PositionSecret` order.
    pub ciphertexts: [[u8; 32]; 5],
    /// The revealed approved amount: the request or zero.
    pub approved_amount: u64,
    /// The revealed request echo.
    pub requested_amount: u64,
    /// The destination token account the callback received.
    pub destination: TokenAccountView,
    /// The mint account the callback received.
    pub mint: Pubkey,
    /// The vault the callback received.
    pub vault: Pubkey,
    /// The vault-authority account the callback received.
    pub vault_authority: Pubkey,
    /// The token program account the callback received.
    pub token_program: Pubkey,
    /// The current slot.
    pub current_slot: Slot,
}

/// What the callback did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnshieldSettlement {
    /// `Applied`: the new state is stored. The handler must transfer exactly
    /// `amount` from the vault to `destination`, signed by the vault
    /// authority.
    Approved {
        /// The unshielded asset.
        asset: ShieldAsset,
        /// The exact amount to transfer.
        amount: u64,
        /// The committed destination token account.
        destination: Pubkey,
    },
    /// The circuit approved zero: nothing stored, nothing moves, Lock
    /// released.
    Zero {
        /// The requested asset.
        asset: ShieldAsset,
    },
    /// The attempt ended with this status (`Expired`, `Stale`, `Failed`);
    /// nothing stored, nothing moves, Lock released.
    Failed(IntentStatus),
    /// The Intent was already terminal; nothing changed.
    Ignored,
}

/// Verifies and applies an `unshield` callback (Task 3.3 helpers).
///
/// Unauthenticated output and commitment, offset, or binding mismatches are
/// errors, so the transaction reverts and the Intent stays `Queued`. A
/// terminal Intent is `Ignored`. Otherwise the decision releases the Lock;
/// only `Applied` stores the payload (bumping the generation) and returns
/// [`UnshieldSettlement::Approved`]. Atomic on error.
pub fn settle_unshield(
    market: &Market,
    market_key: Pubkey,
    position: &mut Position,
    position_key: Pubkey,
    intent: &mut OperationIntent,
    report: &UnshieldReport,
) -> Result<UnshieldSettlement, ErrorCode> {
    let asset =
        asset_of_mint(market, &report.mint).ok_or(ErrorCode::from(UnshieldError::MintMismatch))?;
    let bindings = unshield_bindings(
        position.owner,
        market_key,
        market.asset_state,
        position_key,
        intent.position_generation,
        &UnshieldKeys {
            vault: report.vault,
            mint: report.mint,
            destination: report.destination.key,
            vault_authority: report.vault_authority,
            token_program: report.token_program,
        },
    );
    // The output-nonce input is output nonce - 1; the commitment binds it to
    // the value derived at queue time from the Intent sequence.
    let inputs = unshield_inputs(
        report.encryption_key,
        position.viewing_key_epoch,
        &position.encrypted_state,
        asset,
        report.requested_amount,
        report.output_nonce.wrapping_sub(1),
    );
    let context = CallbackContext {
        output_authenticated: report.output_authenticated,
        computation_offset: report.computation_offset,
        intent_key: report.intent_key,
        kind: OperationKind::Unshield,
        accounts: &bindings,
        input_fields: &inputs,
        current: CurrentDependencies {
            position_generation: Some(position.coordination.generation),
            ..CurrentDependencies::captured_by(intent)
        },
        current_slot: report.current_slot,
        report: CallbackReport::Success {
            nonzero: report.approved_amount != 0,
        },
    };
    let decision = classify_callback(intent, &context)?;
    match decision {
        CallbackDecision::LeaveQueued => return Err(ErrorCode::CallbackOutputNotAuthenticated),
        CallbackDecision::Ignore => return Ok(UnshieldSettlement::Ignored),
        _ => {}
    }
    if position.status != PositionStatus::Active {
        return Err(UnshieldError::PositionNotActive.into());
    }
    if !intent.lock_owner().holds(&position.coordination.lock) {
        return Err(UnshieldError::IntentMismatch.into());
    }
    if decision == CallbackDecision::Applied {
        // The circuit approves the request or zero, never a partial amount.
        if report.approved_amount == 0 || report.approved_amount != report.requested_amount {
            return Err(UnshieldError::ApprovalInvalid.into());
        }
        // The destination must still be the owner's token account of the mint
        // (its token owner may have changed since the commitment was made).
        let destination = &report.destination;
        if destination.mint != report.mint
            || destination.owner != position.owner
            || destination.key == report.vault
        {
            return Err(UnshieldError::DestinationInvalid.into());
        }
    }

    let mut next_intent = *intent;
    let mut coordination = position.coordination;
    let result = apply_decision_to_slots(
        &mut next_intent,
        &mut [CoordinationSlot::new(position_key, &mut coordination)],
        decision,
        report.current_slot,
    )?;
    let settlement = match result.status {
        IntentStatus::Applied => UnshieldSettlement::Approved {
            asset,
            amount: report.approved_amount,
            destination: report.destination.key,
        },
        IntentStatus::Zero => UnshieldSettlement::Zero { asset },
        status => UnshieldSettlement::Failed(status),
    };

    *intent = next_intent;
    position.coordination = coordination;
    if let UnshieldSettlement::Approved { .. } = settlement {
        position.encrypted_state = EncryptedPositionPayload {
            nonce: report.output_nonce,
            ciphertexts: report.ciphertexts,
        };
    }
    Ok(settlement)
}

/// `abandon_unshield`: permissionless timeout recovery of a queued unshield
/// after Lock expiry. Bumps the Position generation, releases the Lock, and
/// marks the Intent `Abandoned`. No token moved, so nothing needs refunding.
/// Atomic. Returns the number of Locks cleared.
pub fn abandon_unshield(
    position: &mut Position,
    position_key: Pubkey,
    intent: &mut OperationIntent,
    current_slot: Slot,
) -> Result<usize, ErrorCode> {
    if intent.kind != OperationKind::Unshield || intent.position != Some(position_key) {
        return Err(UnshieldError::IntentMismatch.into());
    }
    if intent.status.is_terminal() {
        return Err(ErrorCode::IntentNotQueued);
    }
    if !intent.lock_owner().holds(&position.coordination.lock) {
        return Err(UnshieldError::IntentMismatch.into());
    }
    let cleared = crate::operation::abandon_intent(
        intent,
        current_slot,
        &mut [CoordinationSlot::new(
            position_key,
            &mut position.coordination,
        )],
    )?;
    Ok(cleared)
}

#[cfg(test)]
mod tests;
