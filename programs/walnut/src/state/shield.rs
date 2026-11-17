//! Shield, shield retry, refund, abandonment, and the `shield_credit` callback
//! (Task 5.3; Requirements 5.4-5.6, 3.1-3.10, 4.8).
//!
//! Pure Rust rules behind the shield instructions. The Anchor handlers in
//! `lib.rs` and `instructions/shield.rs` load accounts, perform the SPL token
//! transfers, and queue the computation; every state change goes through these
//! functions, which call the Task 5.2 Pending Shield state machine.
//!
//! # Flow
//!
//! 1. `shield`: the owner's source token account sends exactly `amount` to the
//!    Market vault of the asset (`transfer_checked`, owner-signed), then
//!    [`queue_shield`] opens the Pending Shield (`Deposited`, attempt 1),
//!    creates the Shield Intent, and Locks the Position. The `shield_credit`
//!    computation adds the public amount to `free_sol` or `free_usdc`.
//! 2. Callback ([`settle_shield_credit`]): `Applied` stores the new encrypted
//!    Position state, bumps the Position generation, and marks the Pending
//!    Shield `Credited`. `Zero` (the credit would overflow `u64`, so the
//!    circuit left the state unchanged), `Expired`, and `Stale` release the
//!    Lock and leave the Pending Shield `Deposited`. A terminal Intent is a
//!    no-op. Unauthenticated output and every binding mismatch revert, so the
//!    Intent stays `Queued` until [`abandon_shield`].
//! 3. `retry_shield` ([`retry_shield`]): owner-only, not pausable. New Intent
//!    and offset, attempt + 1.
//! 4. `refund_shield` ([`refund_shield`]): owner-only, not pausable. Marks
//!    `Refunded`; the handler transfers exactly the recorded amount from the
//!    vault, signed by the vault-authority PDA, in the same instruction.
//! 5. `abandon_shield` ([`abandon_shield`]): any signer after Lock expiry.
//!    The Pending Shield stays `Deposited`, so it can be retried or refunded.
//!
//! # SOL
//!
//! The program accepts only an SPL token account as the source. For SOL the
//! client wraps in the same transaction, before the `shield` instruction:
//! `SystemProgram::transfer` of the lamports into the owner's wSOL token
//! account, then SPL Token `SyncNative`. The program then moves wSOL like any
//! other token, so one code path serves both assets.
//!
//! # Intent binding
//!
//! `accounts_commitment` covers, in order: owner (signer, payer), Market,
//! Asset State, Position (captured generation), Pending Shield, the asset
//! vault (mint, vault-authority PDA), and the source token account (mint,
//! owner). The vault and the source are bound read-only: the computation and
//! its callback never write them. `inputs_commitment` covers the Viewing
//! public key, the key epoch, the stored state nonce (the `Shared` input
//! nonce), the Pending Shield address, the attempt sequence, the asset, the
//! amount, the five stored state ciphertexts, and the output-nonce input. The
//! callback rebuilds the inputs from the output (encryption key, output
//! nonce - 1) and the Locked Position, so any mismatch rejects it.
//!
//! # Nonces
//!
//! Every attempt seals its output under a nonce derived from the Position and
//! the attempt's unique Intent sequence (`state::nonce`), never from the
//! stored nonce, so a retry after an attempt that did not apply (or whose
//! callback was rejected) cannot reuse an output (key, nonce). The derived
//! nonce input is the 13th committed input; the callback rebuilds the
//! commitment from output nonce - 1, so any other nonce is rejected.

use anchor_lang::prelude::Pubkey;

use super::accounts::{EncryptedPositionPayload, Position, PositionStatus};
use super::coordination::{acquire_locks, CoordinationSlot};
use super::nonce::{derive_output_nonce_input, INPUT_OUTPUT_NONCE};
use super::pending_shield::{
    open_pending_shield, refund_pending_shield, retry_pending_shield, settle_shield_attempt,
    NewPendingShield, PendingShield, PendingShieldError, ShieldAsset, ShieldAttemptOutcome,
    ShieldRefund,
};
use super::position::{INPUT_NONCE, INPUT_VIEWING_KEY_EPOCH, INPUT_VIEWING_PUBKEY};
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

/// Decision Expiry distance of a shield credit, in slots. The deposit is
/// already in the vault and the credit reads no price or index, so a fixed
/// window is used, as for Position initialization.
pub const SHIELD_DECISION_SLOTS: u64 = 1_500;

/// Slots between Decision Expiry and Lock expiry.
pub const SHIELD_LOCK_GRACE_SLOTS: u64 = 300;

/// Input field id: the Pending Shield address (`Key`). Append-only.
pub const INPUT_PENDING_SHIELD: u8 = 4;
/// Input field id: the Pending Shield attempt sequence (`U64`).
pub const INPUT_ATTEMPT_SEQUENCE: u8 = 5;
/// Input field id: the asset tag (`U8`, Borsh variant index).
pub const INPUT_ASSET: u8 = 6;
/// Input field id: the public amount (`U64`).
pub const INPUT_AMOUNT: u8 = 7;
/// Input field ids 8..=12: the stored state ciphertexts (`Bytes32`), in
/// `PositionSecret` order.
pub const INPUT_STATE_CIPHERTEXT_FIRST: u8 = 8;

/// Shield instruction failures. Mapped onto program errors in `errors.rs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShieldError {
    /// The mint is not the Market mint of the asset.
    MintMismatch,
    /// The vault is not the Market vault of the asset.
    VaultMismatch,
    /// The source token account has the wrong mint or owner, or is the vault.
    SourceAccountInvalid,
    /// The refund destination has the wrong mint or owner, or is the vault.
    RefundDestinationInvalid,
    /// The Intent is not this Position's queued shield attempt.
    IntentMismatch,
    /// The Intent is not the Pending Shield's current attempt.
    AttemptNotCurrent,
}

/// The fields of an SPL token account the rules read. Token-2022 accounts
/// never reach these checks: the handlers take `Account<TokenAccount>` and
/// `Program<Token>`, which require the classic SPL Token program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenAccountView {
    /// The token account address.
    pub key: Pubkey,
    /// Its mint.
    pub mint: Pubkey,
    /// Its token owner (the authority that may transfer from it).
    pub owner: Pubkey,
}

/// The Market mint of `asset`.
pub fn asset_mint(market: &Market, asset: ShieldAsset) -> Pubkey {
    match asset {
        ShieldAsset::Sol => market.wsol_mint,
        ShieldAsset::Usdc => market.usdc_mint,
    }
}

/// The Market vault of `asset`.
pub fn asset_vault(market: &Market, asset: ShieldAsset) -> Pubkey {
    match asset {
        ShieldAsset::Sol => market.wsol_vault,
        ShieldAsset::Usdc => market.usdc_vault,
    }
}

/// The Borsh tag of `asset`, as committed.
pub fn asset_tag(asset: ShieldAsset) -> u8 {
    match asset {
        ShieldAsset::Sol => 0,
        ShieldAsset::Usdc => 1,
    }
}

fn check_mint_and_vault(
    market: &Market,
    asset: ShieldAsset,
    mint: &Pubkey,
    vault: &Pubkey,
) -> Result<(), ShieldError> {
    if *mint != asset_mint(market, asset) {
        return Err(ShieldError::MintMismatch);
    }
    if *vault != asset_vault(market, asset) {
        return Err(ShieldError::VaultMismatch);
    }
    Ok(())
}

/// `shield` account rule: `mint` and `vault` are the Market's for `asset`,
/// and `source` holds that mint, is owned by `signer`, and is not the vault.
pub fn check_shield_accounts(
    market: &Market,
    asset: ShieldAsset,
    signer: &Pubkey,
    mint: &Pubkey,
    vault: &Pubkey,
    source: &TokenAccountView,
) -> Result<(), ShieldError> {
    check_mint_and_vault(market, asset, mint, vault)?;
    if source.mint != *mint || source.owner != *signer || source.key == *vault {
        return Err(ShieldError::SourceAccountInvalid);
    }
    Ok(())
}

/// `refund_shield` account rule: `mint` and `vault` are the Market's for the
/// recorded asset, and `destination` holds that mint and is owned by the
/// Pending Shield owner (the recorded source qualifies while it exists; the
/// owner may name another of their own accounts, for example after closing a
/// temporary wSOL account). Value can only return to the owner.
pub fn check_refund_accounts(
    market: &Market,
    shield: &PendingShield,
    mint: &Pubkey,
    vault: &Pubkey,
    destination: &TokenAccountView,
) -> Result<(), ShieldError> {
    check_mint_and_vault(market, shield.asset, mint, vault)?;
    if destination.mint != *mint || destination.owner != shield.owner || destination.key == *vault {
        return Err(ShieldError::RefundDestinationInvalid);
    }
    Ok(())
}

/// The ordered accounts bound by a Shield Intent.
#[allow(clippy::too_many_arguments)]
pub fn shield_bindings(
    owner: Pubkey,
    market: &Market,
    market_key: Pubkey,
    position: Pubkey,
    position_generation: Option<u64>,
    pending_shield: Pubkey,
    source_token_account: Pubkey,
    asset: ShieldAsset,
) -> [AccountBinding; 7] {
    let program = crate::ID;
    let token_program = anchor_spl::token::ID;
    let mint = asset_mint(market, asset);
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
            key: market.asset_state,
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
            key: pending_shield,
            role: AccountRole::PendingShield,
            expected_owner: program,
            mint: None,
            authority: Some(owner),
            writable: true,
            generation: None,
        },
        AccountBinding {
            key: asset_vault(market, asset),
            role: AccountRole::Vault,
            expected_owner: token_program,
            mint: Some(mint),
            authority: Some(market.vault_authority),
            writable: false,
            generation: None,
        },
        AccountBinding {
            key: source_token_account,
            role: AccountRole::TokenAccount,
            expected_owner: token_program,
            mint: Some(mint),
            authority: Some(owner),
            writable: false,
            generation: None,
        },
    ]
}

/// The ordered inputs bound by a Shield Intent.
#[allow(clippy::too_many_arguments)]
pub fn shield_inputs(
    viewing_pubkey: [u8; 32],
    viewing_key_epoch: u64,
    state: &EncryptedPositionPayload,
    pending_shield: Pubkey,
    attempt_sequence: u64,
    asset: ShieldAsset,
    amount: u64,
    output_nonce_input: u128,
) -> [InputField<'static>; 13] {
    let c = &state.ciphertexts;
    let first = INPUT_STATE_CIPHERTEXT_FIRST;
    [
        InputField::new(INPUT_VIEWING_PUBKEY, InputValue::Bytes32(viewing_pubkey)),
        InputField::new(INPUT_VIEWING_KEY_EPOCH, InputValue::U64(viewing_key_epoch)),
        InputField::new(INPUT_NONCE, InputValue::U128(state.nonce)),
        InputField::new(INPUT_PENDING_SHIELD, InputValue::Key(pending_shield)),
        InputField::new(INPUT_ATTEMPT_SEQUENCE, InputValue::U64(attempt_sequence)),
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

/// One queue request (shield or retry).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShieldRequest {
    /// The Operation Intent PDA address.
    pub intent_key: Pubkey,
    /// The owner's next Intent sequence.
    pub sequence: u64,
    /// The Arcium computation offset.
    pub computation_offset: u64,
    /// The current slot.
    pub queued_slot: Slot,
}

/// What one attempt credits: the Pending Shield, its source, asset, amount,
/// and attempt number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShieldAttempt {
    /// The Pending Shield address.
    pub pending_shield: Pubkey,
    /// The recorded source token account.
    pub source_token_account: Pubkey,
    /// The asset.
    pub asset: ShieldAsset,
    /// The public amount.
    pub amount: u64,
    /// The attempt sequence (1 for the first attempt).
    pub attempt_sequence: u64,
}

/// Builds the Shield Intent for `attempt` against `position` (owner = payer).
pub fn build_shield_intent(
    market: &Market,
    market_key: Pubkey,
    position: &Position,
    position_key: Pubkey,
    attempt: &ShieldAttempt,
    request: &ShieldRequest,
) -> Result<OperationIntent, ErrorCode> {
    let decision_expiry_slot = request
        .queued_slot
        .checked_add(SHIELD_DECISION_SLOTS)
        .ok_or(ErrorCode::InvalidIntentTiming)?;
    let lock_expiry_slot = decision_expiry_slot
        .checked_add(SHIELD_LOCK_GRACE_SLOTS)
        .ok_or(ErrorCode::InvalidIntentTiming)?;
    let generation = Some(position.coordination.generation);
    let bindings = shield_bindings(
        position.owner,
        market,
        market_key,
        position_key,
        generation,
        attempt.pending_shield,
        attempt.source_token_account,
        attempt.asset,
    );
    let inputs = shield_inputs(
        position.viewing_pubkey,
        position.viewing_key_epoch,
        &position.encrypted_state,
        attempt.pending_shield,
        attempt.attempt_sequence,
        attempt.asset,
        attempt.amount,
        derive_output_nonce_input(&position_key, request.sequence),
    );
    Ok(OperationIntent {
        key: request.intent_key,
        owner: position.owner,
        payer: position.owner,
        reclaim_authority: position.owner,
        kind: OperationKind::Shield,
        status: IntentStatus::Queued,
        sequence: request.sequence,
        computation_offset: request.computation_offset,
        accounts_commitment: account_commitment(&bindings),
        inputs_commitment: input_commitment(OperationKind::Shield, &inputs),
        queued_slot: request.queued_slot,
        decision_expiry_slot,
        lock_expiry_slot,
        position: Some(position_key),
        position_generation: generation,
        ..OperationIntent::default()
    })
}

/// Locks the Position for `intent`. On error `position` is unchanged.
fn lock_position(
    position: &mut Position,
    position_key: Pubkey,
    intent: &OperationIntent,
) -> Result<(), ErrorCode> {
    acquire_locks(
        intent.lock_owner(),
        intent.lock_expiry_slot,
        &mut [CoordinationSlot::new(
            position_key,
            &mut position.coordination,
        )],
    )?;
    Ok(())
}

/// `shield` step 2, after the exact `new.amount` was transferred into the
/// vault: checks the Market and the Shield pause, builds and reserves the
/// Intent, opens the Pending Shield (`Deposited`, attempt 1), and Locks the
/// Position. Atomic: on error the Position, counter, and Pending Shield are
/// unchanged.
#[allow(clippy::too_many_arguments)]
pub fn queue_shield(
    market: &Market,
    market_key: Pubkey,
    pauses: &PauseState,
    position: &mut Position,
    position_key: Pubkey,
    counter: &mut OperationCounter,
    shield: &mut PendingShield,
    shield_key: Pubkey,
    new: &NewPendingShield,
    request: &ShieldRequest,
) -> Result<OperationIntent, ErrorCode> {
    if market.status != MarketStatus::Active {
        return Err(ErrorCode::MarketNotActive);
    }
    ensure_not_paused(pauses, PauseCategory::Shield, request.queued_slot)?;
    if new.position != position_key {
        return Err(PendingShieldError::PositionMismatch.into());
    }
    let attempt = ShieldAttempt {
        pending_shield: shield_key,
        source_token_account: new.source_token_account,
        asset: new.asset,
        amount: new.amount,
        attempt_sequence: 1,
    };
    let intent = build_shield_intent(
        market,
        market_key,
        position,
        position_key,
        &attempt,
        request,
    )?;
    let mut next_counter = *counter;
    reserve_intent(&mut next_counter, &intent)?;

    let saved_shield = *shield;
    let saved_sequence = position.next_shield_seq;
    open_pending_shield(shield, position, new, &intent)?;
    if let Err(error) = lock_position(position, position_key, &intent) {
        *shield = saved_shield;
        position.next_shield_seq = saved_sequence;
        return Err(error);
    }
    *counter = next_counter;
    Ok(intent)
}

/// `retry_shield`: owner-only and not pausable. Builds attempt `n + 1`,
/// installs it on the Pending Shield (the `prior` active Intent must be
/// terminal and not `Applied`), and Locks the Position. Returns the new
/// Intent and attempt sequence. Atomic.
#[allow(clippy::too_many_arguments)]
pub fn retry_shield(
    market: &Market,
    market_key: Pubkey,
    position: &mut Position,
    position_key: Pubkey,
    counter: &mut OperationCounter,
    shield: &mut PendingShield,
    shield_key: Pubkey,
    signer: &Pubkey,
    prior: &OperationIntent,
    request: &ShieldRequest,
) -> Result<(OperationIntent, u64), ErrorCode> {
    if *signer != shield.owner {
        return Err(PendingShieldError::NotOwner.into());
    }
    let attempt_sequence = shield
        .attempt_sequence
        .checked_add(1)
        .ok_or(PendingShieldError::AttemptOverflow)?;
    let attempt = ShieldAttempt {
        pending_shield: shield_key,
        source_token_account: shield.source_token_account,
        asset: shield.asset,
        amount: shield.amount,
        attempt_sequence,
    };
    let intent = build_shield_intent(
        market,
        market_key,
        position,
        position_key,
        &attempt,
        request,
    )?;
    let mut next_counter = *counter;
    reserve_intent(&mut next_counter, &intent)?;

    let saved_shield = *shield;
    let installed = retry_pending_shield(shield, signer, position, &position_key, prior, &intent)?;
    if let Err(error) = lock_position(position, position_key, &intent) {
        *shield = saved_shield;
        return Err(error);
    }
    *counter = next_counter;
    Ok((intent, installed))
}

/// What the `shield_credit` callback observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShieldCallback {
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
    /// The revealed "credit fits in `u64`" flag; `false` resolves `Zero`.
    pub credited: bool,
    /// The current slot.
    pub current_slot: Slot,
}

/// What the callback did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShieldSettlement {
    /// `Applied`: the new state is stored and the Pending Shield is `Credited`.
    Credited {
        /// The credited asset.
        asset: ShieldAsset,
        /// The credited amount.
        amount: u64,
    },
    /// The attempt ended with this status; the Pending Shield stays
    /// `Deposited` (retryable or refundable) and nothing was stored.
    Failed(IntentStatus),
    /// The Intent was already terminal; nothing changed.
    Ignored,
}

/// Verifies and applies a `shield_credit` callback (Task 3.3 helpers).
///
/// Unauthenticated output and commitment, offset, or binding mismatches are
/// errors, so the transaction reverts and the Intent stays `Queued`. A
/// terminal Intent is `Ignored`. Otherwise the decision releases the Lock and
/// [`settle_shield_attempt`] records the outcome; only `Applied` stores the
/// payload (and bumps the generation). Atomic on error.
#[allow(clippy::too_many_arguments)]
pub fn settle_shield_credit(
    market: &Market,
    market_key: Pubkey,
    position: &mut Position,
    position_key: Pubkey,
    shield: &mut PendingShield,
    shield_key: Pubkey,
    intent: &mut OperationIntent,
    callback: &ShieldCallback,
) -> Result<ShieldSettlement, ErrorCode> {
    let bindings = shield_bindings(
        position.owner,
        market,
        market_key,
        position_key,
        intent.position_generation,
        shield_key,
        shield.source_token_account,
        shield.asset,
    );
    // The stored state is the Locked Position's own. The output-nonce input is
    // the one the output was sealed under (output nonce - 1); the commitment
    // binds it to the value derived at queue time, so an output sealed under
    // any other (key, nonce) is rejected.
    let inputs = shield_inputs(
        callback.encryption_key,
        position.viewing_key_epoch,
        &position.encrypted_state,
        shield_key,
        shield.attempt_sequence,
        shield.asset,
        shield.amount,
        callback.output_nonce.wrapping_sub(1),
    );
    let context = CallbackContext {
        output_authenticated: callback.output_authenticated,
        computation_offset: callback.computation_offset,
        intent_key: callback.intent_key,
        kind: OperationKind::Shield,
        accounts: &bindings,
        input_fields: &inputs,
        current: CurrentDependencies {
            position_generation: Some(position.coordination.generation),
            ..CurrentDependencies::captured_by(intent)
        },
        current_slot: callback.current_slot,
        report: CallbackReport::Success {
            nonzero: callback.credited,
        },
    };
    let decision = classify_callback(intent, &context)?;
    match decision {
        CallbackDecision::LeaveQueued => return Err(ErrorCode::CallbackOutputNotAuthenticated),
        CallbackDecision::Ignore => return Ok(ShieldSettlement::Ignored),
        _ => {}
    }
    if position.status != PositionStatus::Active {
        return Err(PendingShieldError::PositionNotActive.into());
    }
    if !intent.lock_owner().holds(&position.coordination.lock) {
        return Err(ShieldError::IntentMismatch.into());
    }
    if !shield.is_current_attempt(&intent.key, intent.computation_offset) {
        return Err(ShieldError::AttemptNotCurrent.into());
    }

    let mut next_intent = *intent;
    let mut coordination = position.coordination;
    apply_decision_to_slots(
        &mut next_intent,
        &mut [CoordinationSlot::new(position_key, &mut coordination)],
        decision,
        callback.current_slot,
    )?;
    let mut next_shield = *shield;
    let outcome = settle_shield_attempt(&mut next_shield, &next_intent)?;
    let settlement = match outcome {
        ShieldAttemptOutcome::Credited { asset, amount } => {
            ShieldSettlement::Credited { asset, amount }
        }
        ShieldAttemptOutcome::Retryable(status) => ShieldSettlement::Failed(status),
        ShieldAttemptOutcome::Ignored => return Err(ShieldError::AttemptNotCurrent.into()),
    };

    *intent = next_intent;
    position.coordination = coordination;
    *shield = next_shield;
    if let ShieldSettlement::Credited { .. } = settlement {
        position.encrypted_state = EncryptedPositionPayload {
            nonce: callback.output_nonce,
            ciphertexts: callback.ciphertexts,
        };
    }
    Ok(settlement)
}

/// `refund_shield`: checks the token accounts, then marks the Pending Shield
/// `Refunded` (owner-only, active attempt terminal and not `Applied`). The
/// handler must transfer exactly [`ShieldRefund::amount`] from the vault to
/// `destination` in the same instruction. Not pausable. Atomic.
pub fn refund_shield(
    market: &Market,
    shield: &mut PendingShield,
    signer: &Pubkey,
    active: &OperationIntent,
    mint: &Pubkey,
    vault: &Pubkey,
    destination: &TokenAccountView,
) -> Result<ShieldRefund, ErrorCode> {
    if *signer != shield.owner {
        return Err(PendingShieldError::NotOwner.into());
    }
    check_refund_accounts(market, shield, mint, vault, destination)?;
    let mut refund = refund_pending_shield(shield, signer, active)?;
    refund.destination = destination.key;
    Ok(refund)
}

/// `abandon_shield`: permissionless timeout recovery of a queued shield
/// attempt after Lock expiry. Bumps the Position generation, releases the
/// Lock, and marks the Intent `Abandoned`. The Pending Shield stays
/// `Deposited` and its tokens stay in the vault, so the owner can retry or
/// refund. Atomic. Returns the number of Locks cleared.
pub fn abandon_shield(
    position: &mut Position,
    position_key: Pubkey,
    shield: &PendingShield,
    intent: &mut OperationIntent,
    current_slot: Slot,
) -> Result<usize, ErrorCode> {
    if intent.kind != OperationKind::Shield || intent.position != Some(position_key) {
        return Err(ShieldError::IntentMismatch.into());
    }
    if intent.status.is_terminal() {
        return Err(ErrorCode::IntentNotQueued);
    }
    if shield.position != position_key
        || !shield.is_current_attempt(&intent.key, intent.computation_offset)
    {
        return Err(ShieldError::AttemptNotCurrent.into());
    }
    if !intent.lock_owner().holds(&position.coordination.lock) {
        return Err(ShieldError::IntentMismatch.into());
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
mod tests {
    use super::*;
    use crate::market::pause_flags;
    use crate::state::nonce::output_nonce;
    use crate::state::pending_shield::PendingShieldStatus;

    const OWNER: Pubkey = Pubkey::new_from_array([0x11; 32]);
    const STRANGER: Pubkey = Pubkey::new_from_array([0x12; 32]);
    const MARKET_KEY: Pubkey = Pubkey::new_from_array([0x22; 32]);
    const POSITION_KEY: Pubkey = Pubkey::new_from_array([0x23; 32]);
    const SOURCE: Pubkey = Pubkey::new_from_array([0x24; 32]);
    const VIEWING: [u8; 32] = [0x33; 32];

    fn key(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    fn market() -> Market {
        Market {
            layout_version: 1,
            status: MarketStatus::Active,
            bump: 0xFE,
            vault_authority_bump: 0xFD,
            admin: key(1),
            pending_admin: Pubkey::default(),
            upgrade_authority: key(3),
            risk_authority: key(4),
            treasury_authority: key(5),
            guardian_authority: key(6),
            asset_state: key(0x41),
            pool_state: key(0x42),
            recovery_position: key(0x43),
            vault_authority: key(0x44),
            wsol_mint: key(0x45),
            usdc_mint: key(0x46),
            wsol_vault: key(0x47),
            usdc_vault: key(0x48),
            treasury_usdc_account: Pubkey::default(),
            position_count: 1,
            debt_position_count: 0,
            unsettled_recovery_claim_count: 0,
            position_count_cap: 10,
            debt_position_count_cap: 0,
            initialized_slot: 1,
            debt_position_reservations: 0,
            reserved: [0; 120],
        }
    }

    fn active_position() -> Position {
        let mut position = Position::blank(OWNER, MARKET_KEY, 0xFC);
        position.status = PositionStatus::Active;
        position.viewing_pubkey = VIEWING;
        position.coordination.generation = 1;
        position.encrypted_state = EncryptedPositionPayload {
            nonce: 1_000,
            ciphertexts: [[0xA0; 32]; 5],
        };
        position
    }

    fn shield_key(sequence: u64) -> Pubkey {
        let mut bytes = [0x50; 32];
        bytes[..8].copy_from_slice(&sequence.to_le_bytes());
        Pubkey::new_from_array(bytes)
    }

    fn intent_key(sequence: u64) -> Pubkey {
        let mut bytes = [0x60; 32];
        bytes[..8].copy_from_slice(&sequence.to_le_bytes());
        Pubkey::new_from_array(bytes)
    }

    fn request(sequence: u64, slot: Slot) -> ShieldRequest {
        ShieldRequest {
            intent_key: intent_key(sequence),
            sequence,
            computation_offset: 9_000 + sequence,
            queued_slot: slot,
        }
    }

    fn new_shield(shield_sequence: u64, asset: ShieldAsset, amount: u64) -> NewPendingShield {
        NewPendingShield {
            owner: OWNER,
            position: POSITION_KEY,
            source_token_account: SOURCE,
            asset,
            amount,
            sequence: shield_sequence,
            bump: 0xFB,
        }
    }

    fn source(mint: Pubkey) -> TokenAccountView {
        TokenAccountView {
            key: SOURCE,
            mint,
            owner: OWNER,
        }
    }

    struct World {
        market: Market,
        pauses: PauseState,
        position: Position,
        counter: OperationCounter,
    }

    fn world() -> World {
        World {
            market: market(),
            pauses: PauseState::default(),
            position: active_position(),
            counter: OperationCounter::new(OWNER, 0xFA),
        }
    }

    impl World {
        fn queue(
            &mut self,
            shield: &mut PendingShield,
            asset: ShieldAsset,
            amount: u64,
            slot: Slot,
        ) -> Result<OperationIntent, ErrorCode> {
            let sequence = self.counter.next_sequence;
            let shield_sequence = self.position.next_shield_seq;
            queue_shield(
                &self.market,
                MARKET_KEY,
                &self.pauses,
                &mut self.position,
                POSITION_KEY,
                &mut self.counter,
                shield,
                shield_key(shield_sequence),
                &new_shield(shield_sequence, asset, amount),
                &request(sequence, slot),
            )
        }

        fn retry(
            &mut self,
            shield: &mut PendingShield,
            prior: &OperationIntent,
            slot: Slot,
        ) -> Result<(OperationIntent, u64), ErrorCode> {
            let sequence = self.counter.next_sequence;
            retry_shield(
                &self.market,
                MARKET_KEY,
                &mut self.position,
                POSITION_KEY,
                &mut self.counter,
                shield,
                shield_key(shield.sequence),
                &OWNER,
                prior,
                &request(sequence, slot),
            )
        }

        fn honest(&self, intent: &OperationIntent, credited: bool, slot: Slot) -> ShieldCallback {
            ShieldCallback {
                output_authenticated: true,
                computation_offset: intent.computation_offset,
                intent_key: intent.key,
                encryption_key: self.position.viewing_pubkey,
                output_nonce: output_nonce(&POSITION_KEY, intent.sequence),
                ciphertexts: [[intent.sequence as u8 ^ 0x5A; 32]; 5],
                credited,
                current_slot: slot,
            }
        }

        fn settle(
            &mut self,
            shield: &mut PendingShield,
            intent: &mut OperationIntent,
            callback: &ShieldCallback,
        ) -> Result<ShieldSettlement, ErrorCode> {
            settle_shield_credit(
                &self.market,
                MARKET_KEY,
                &mut self.position,
                POSITION_KEY,
                shield,
                shield_key(shield.sequence),
                intent,
                callback,
            )
        }
    }

    fn code(error: ErrorCode) -> u32 {
        u32::from(error)
    }

    fn c<T>(result: Result<T, ErrorCode>) -> Result<T, u32> {
        result.map_err(u32::from)
    }

    #[test]
    fn token_account_rules_accept_only_the_market_accounts_of_the_asset() {
        let m = market();
        for (asset, mint, vault) in [
            (ShieldAsset::Sol, m.wsol_mint, m.wsol_vault),
            (ShieldAsset::Usdc, m.usdc_mint, m.usdc_vault),
        ] {
            assert_eq!(
                check_shield_accounts(&m, asset, &OWNER, &mint, &vault, &source(mint)),
                Ok(())
            );
        }
        let (mint, vault) = (m.usdc_mint, m.usdc_vault);
        let usdc = ShieldAsset::Usdc;
        let cases = [
            (
                m.wsol_mint,
                vault,
                source(m.wsol_mint),
                ShieldError::MintMismatch,
            ),
            (mint, m.wsol_vault, source(mint), ShieldError::VaultMismatch),
            (
                mint,
                vault,
                source(m.wsol_mint),
                ShieldError::SourceAccountInvalid,
            ),
            (
                mint,
                vault,
                TokenAccountView {
                    owner: STRANGER,
                    ..source(mint)
                },
                ShieldError::SourceAccountInvalid,
            ),
            (
                mint,
                vault,
                TokenAccountView {
                    key: vault,
                    owner: OWNER,
                    mint,
                },
                ShieldError::SourceAccountInvalid,
            ),
        ];
        for (mint, vault, source, expected) in cases {
            assert_eq!(
                check_shield_accounts(&m, usdc, &OWNER, &mint, &vault, &source),
                Err(expected)
            );
        }
    }

    #[test]
    fn queue_opens_the_shield_binds_the_intent_and_locks() {
        let mut w = world();
        let mut shield = PendingShield::default();
        let intent = w.queue(&mut shield, ShieldAsset::Usdc, 250, 10).unwrap();

        assert_eq!(shield.status, PendingShieldStatus::Deposited);
        assert_eq!(shield.amount, 250);
        assert_eq!(shield.attempt_sequence, 1);
        assert_eq!(shield.active_intent, intent.key);
        assert_eq!(shield.captured_position_generation, 1);
        assert_eq!(w.position.next_shield_seq, 1);
        assert_eq!(w.counter.next_sequence, 1);
        assert!(intent.lock_owner().holds(&w.position.coordination.lock));
        assert_eq!(intent.kind, OperationKind::Shield);
        assert_eq!(intent.decision_expiry_slot, 10 + SHIELD_DECISION_SLOTS);
        assert_eq!(
            intent.lock_expiry_slot,
            10 + SHIELD_DECISION_SLOTS + SHIELD_LOCK_GRACE_SLOTS
        );

        // Every committed input changes the commitment.
        let state = w.position.encrypted_state;
        let nonce_in = derive_output_nonce_input(&POSITION_KEY, 0);
        let base = |state: &EncryptedPositionPayload, key: Pubkey, attempt, asset, amount| {
            input_commitment(
                OperationKind::Shield,
                &shield_inputs(VIEWING, 0, state, key, attempt, asset, amount, nonce_in),
            )
        };
        let k = shield_key(0);
        assert_eq!(
            intent.inputs_commitment,
            base(&state, k, 1, ShieldAsset::Usdc, 250)
        );
        let mut other_state = state;
        other_state.ciphertexts[4][0] ^= 1;
        let mut other_nonce = state;
        other_nonce.nonce += 1;
        for changed in [
            base(&state, shield_key(1), 1, ShieldAsset::Usdc, 250),
            base(&state, k, 2, ShieldAsset::Usdc, 250),
            base(&state, k, 1, ShieldAsset::Sol, 250),
            base(&state, k, 1, ShieldAsset::Usdc, 251),
            base(&other_state, k, 1, ShieldAsset::Usdc, 250),
            base(&other_nonce, k, 1, ShieldAsset::Usdc, 250),
        ] {
            assert_ne!(intent.inputs_commitment, changed);
        }
        // The account commitment binds the Pending Shield, vault, and source.
        let accounts = |shield: Pubkey, source: Pubkey, asset| {
            account_commitment(&shield_bindings(
                OWNER,
                &w.market,
                MARKET_KEY,
                POSITION_KEY,
                Some(1),
                shield,
                source,
                asset,
            ))
        };
        assert_eq!(
            intent.accounts_commitment,
            accounts(k, SOURCE, ShieldAsset::Usdc)
        );
        assert_ne!(
            intent.accounts_commitment,
            accounts(shield_key(1), SOURCE, ShieldAsset::Usdc)
        );
        assert_ne!(
            intent.accounts_commitment,
            accounts(k, STRANGER, ShieldAsset::Usdc)
        );
        assert_ne!(
            intent.accounts_commitment,
            accounts(k, SOURCE, ShieldAsset::Sol)
        );
    }

    #[test]
    fn queue_fails_atomically() {
        // Paused shield, inactive Market, locked Position, zero amount, wrong
        // shield sequence, non-active Position.
        let mut cases: Vec<(World, u64, u64, ErrorCode)> = Vec::new();
        let mut paused = world();
        paused.pauses.flags = pause_flags::SHIELD;
        cases.push((paused, 0, 10, ErrorCode::ActionPaused));
        let mut inactive = world();
        inactive.market.status = MarketStatus::Uninitialized;
        cases.push((inactive, 0, 10, ErrorCode::MarketNotActive));
        cases.push((world(), 0, 0, ErrorCode::PendingShieldZeroAmount));
        cases.push((world(), 1, 10, ErrorCode::PendingShieldSequenceMismatch));
        let mut initializing = world();
        initializing.position.status = PositionStatus::Initializing;
        cases.push((
            initializing,
            0,
            10,
            ErrorCode::PendingShieldPositionNotActive,
        ));
        for (mut w, shield_sequence, amount, expected) in cases {
            let before_counter = w.counter;
            let before_seq = w.position.next_shield_seq;
            let before_lock = w.position.coordination;
            let mut shield = PendingShield::default();
            let result = queue_shield(
                &w.market,
                MARKET_KEY,
                &w.pauses,
                &mut w.position,
                POSITION_KEY,
                &mut w.counter,
                &mut shield,
                shield_key(shield_sequence),
                &new_shield(shield_sequence, ShieldAsset::Sol, amount),
                &request(0, 5),
            );
            assert_eq!(c(result.map(|_| ())), Err(code(expected)));
            assert_eq!(shield, PendingShield::default());
            assert_eq!(w.counter, before_counter);
            assert_eq!(w.position.next_shield_seq, before_seq);
            assert_eq!(w.position.coordination, before_lock);
        }

        // A Position locked by another attempt rejects a second shield.
        let mut w = world();
        let mut first = PendingShield::default();
        w.queue(&mut first, ShieldAsset::Sol, 5, 1).unwrap();
        let counter = w.counter;
        let mut second = PendingShield::default();
        assert_eq!(
            c(w.queue(&mut second, ShieldAsset::Sol, 5, 2).map(|_| ())),
            Err(code(ErrorCode::LockHeld))
        );
        assert_eq!(second, PendingShield::default());
        assert_eq!(w.counter, counter);
        assert_eq!(w.position.next_shield_seq, 1);
    }

    #[test]
    fn applied_callback_stores_the_state_and_credits_once() {
        let mut w = world();
        let mut shield = PendingShield::default();
        let mut intent = w.queue(&mut shield, ShieldAsset::Sol, 7, 10).unwrap();
        let callback = w.honest(&intent, true, 11);
        let result = w.settle(&mut shield, &mut intent, &callback).unwrap();
        assert_eq!(
            result,
            ShieldSettlement::Credited {
                asset: ShieldAsset::Sol,
                amount: 7
            }
        );
        assert_eq!(shield.status, PendingShieldStatus::Credited);
        assert_eq!(intent.status, IntentStatus::Applied);
        assert_eq!(w.position.coordination.generation, 2);
        assert!(!w.position.coordination.lock.is_active());
        assert_eq!(
            w.position.encrypted_state.nonce,
            output_nonce(&POSITION_KEY, intent.sequence)
        );
        assert_eq!(w.position.encrypted_state.ciphertexts, callback.ciphertexts);

        // A duplicate callback is a no-op; refund and retry are refused.
        let snapshot = (shield, intent, w.position.encrypted_state);
        assert_eq!(
            c(w.settle(&mut shield, &mut intent, &callback)),
            Ok(ShieldSettlement::Ignored)
        );
        assert_eq!(snapshot, (shield, intent, w.position.encrypted_state));
        let m = w.market.clone();
        let refund = refund_shield(
            &m,
            &mut shield,
            &OWNER,
            &intent,
            &m.wsol_mint,
            &m.wsol_vault,
            &source(m.wsol_mint),
        );
        assert_eq!(c(refund), Err(code(ErrorCode::PendingShieldNotDeposited)));
        assert_eq!(
            c(w.retry(&mut shield, &intent, 12).map(|_| ())),
            Err(code(ErrorCode::PendingShieldNotDeposited))
        );
    }

    #[test]
    fn overflowing_or_expired_credit_stays_deposited_and_refunds_exactly() {
        for (credited, slot, status) in [
            (false, 11, IntentStatus::Zero),
            (true, 10 + SHIELD_DECISION_SLOTS + 1, IntentStatus::Expired),
        ] {
            let mut w = world();
            let mut shield = PendingShield::default();
            let mut intent = w.queue(&mut shield, ShieldAsset::Usdc, 9, 10).unwrap();
            let stored = w.position.encrypted_state;
            let callback = w.honest(&intent, credited, slot);
            assert_eq!(
                c(w.settle(&mut shield, &mut intent, &callback)),
                Ok(ShieldSettlement::Failed(status))
            );
            assert_eq!(shield.status, PendingShieldStatus::Deposited);
            assert_eq!(w.position.encrypted_state, stored);
            assert_eq!(w.position.coordination.generation, 1);
            assert!(!w.position.coordination.lock.is_active());

            let m = w.market.clone();
            // Wrong destination owner is refused without change.
            let foreign = TokenAccountView {
                owner: STRANGER,
                ..source(m.usdc_mint)
            };
            assert_eq!(
                c(refund_shield(
                    &m,
                    &mut shield,
                    &OWNER,
                    &intent,
                    &m.usdc_mint,
                    &m.usdc_vault,
                    &foreign
                )),
                Err(code(ErrorCode::ShieldRefundDestinationInvalid))
            );
            assert_eq!(
                c(refund_shield(
                    &m,
                    &mut shield,
                    &STRANGER,
                    &intent,
                    &m.usdc_mint,
                    &m.usdc_vault,
                    &source(m.usdc_mint)
                )),
                Err(code(ErrorCode::PendingShieldNotOwner))
            );
            assert_eq!(shield.status, PendingShieldStatus::Deposited);
            let refund = refund_shield(
                &m,
                &mut shield,
                &OWNER,
                &intent,
                &m.usdc_mint,
                &m.usdc_vault,
                &source(m.usdc_mint),
            )
            .unwrap();
            assert_eq!(
                refund,
                ShieldRefund {
                    asset: ShieldAsset::Usdc,
                    amount: 9,
                    destination: SOURCE
                }
            );
            assert_eq!(shield.status, PendingShieldStatus::Refunded);
            assert_eq!(
                c(refund_shield(
                    &m,
                    &mut shield,
                    &OWNER,
                    &intent,
                    &m.usdc_mint,
                    &m.usdc_vault,
                    &source(m.usdc_mint)
                )),
                Err(code(ErrorCode::PendingShieldNotDeposited))
            );
        }
    }

    #[test]
    fn invalid_callbacks_revert_and_leave_the_attempt_queued() {
        let mut w = world();
        let mut shield = PendingShield::default();
        let mut intent = w.queue(&mut shield, ShieldAsset::Sol, 3, 10).unwrap();
        let honest = w.honest(&intent, true, 11);
        let wrong_key = ShieldCallback {
            encryption_key: [0x99; 32],
            ..honest
        };
        let wrong_nonce = ShieldCallback {
            output_nonce: honest.output_nonce + 1,
            ..honest
        };
        let cases = [
            (
                ShieldCallback {
                    output_authenticated: false,
                    ..honest
                },
                ErrorCode::CallbackOutputNotAuthenticated,
            ),
            (
                ShieldCallback {
                    computation_offset: honest.computation_offset + 1,
                    ..honest
                },
                ErrorCode::CallbackOffsetMismatch,
            ),
            (wrong_key, ErrorCode::CallbackInputsCommitmentMismatch),
            (wrong_nonce, ErrorCode::CallbackInputsCommitmentMismatch),
        ];
        for (callback, expected) in cases {
            let snapshot = (shield, intent, w.position.coordination);
            assert_eq!(
                c(w.settle(&mut shield, &mut intent, &callback).map(|_| ())),
                Err(code(expected))
            );
            assert_eq!(snapshot, (shield, intent, w.position.coordination));
        }
        // A foreign Pending Shield (other key) fails the account commitment.
        let foreign_key = shield_key(7);
        let mut foreign = shield;
        let m = w.market.clone();
        assert_eq!(
            c(settle_shield_credit(
                &m,
                MARKET_KEY,
                &mut w.position,
                POSITION_KEY,
                &mut foreign,
                foreign_key,
                &mut intent,
                &honest,
            )
            .map(|_| ())),
            Err(code(ErrorCode::CallbackAccountsCommitmentMismatch))
        );
        assert_eq!(intent.status, IntentStatus::Queued);
        // The honest callback still applies afterwards.
        assert!(matches!(
            w.settle(&mut shield, &mut intent, &honest),
            Ok(ShieldSettlement::Credited { .. })
        ));
    }

    #[test]
    fn abandon_then_retry_credits_and_the_old_callback_is_a_no_op() {
        let mut w = world();
        let mut shield = PendingShield::default();
        let mut first = w.queue(&mut shield, ShieldAsset::Usdc, 40, 10).unwrap();
        let late = w.honest(&first, true, 11);

        // Not before Lock expiry; not by a non-shield Intent.
        assert_eq!(
            c(abandon_shield(
                &mut w.position,
                POSITION_KEY,
                &shield,
                &mut first,
                11
            )),
            Err(code(ErrorCode::LockNotExpired))
        );
        let mut init_intent = first;
        init_intent.kind = OperationKind::PositionInit;
        assert_eq!(
            c(abandon_shield(
                &mut w.position,
                POSITION_KEY,
                &shield,
                &mut init_intent,
                99_999
            )),
            Err(code(ErrorCode::ShieldIntentMismatch))
        );
        let after = first.lock_expiry_slot + 1;
        assert_eq!(
            c(abandon_shield(
                &mut w.position,
                POSITION_KEY,
                &shield,
                &mut first,
                after
            )),
            Ok(1)
        );
        assert_eq!(first.status, IntentStatus::Abandoned);
        assert_eq!(shield.status, PendingShieldStatus::Deposited);
        assert_eq!(w.position.coordination.generation, 2);
        assert!(!w.position.coordination.lock.is_active());

        // The late callback of the abandoned attempt changes nothing.
        let stored = w.position.encrypted_state;
        assert_eq!(
            c(w.settle(&mut shield, &mut first, &late)),
            Ok(ShieldSettlement::Ignored)
        );
        assert_eq!(w.position.encrypted_state, stored);

        // Retry by a stranger is refused; by the owner installs attempt 2.
        let counter = w.counter;
        assert_eq!(
            c(retry_shield(
                &w.market,
                MARKET_KEY,
                &mut w.position,
                POSITION_KEY,
                &mut w.counter,
                &mut shield,
                shield_key(0),
                &STRANGER,
                &first,
                &request(counter.next_sequence, after),
            )
            .map(|_| ())),
            Err(code(ErrorCode::PendingShieldNotOwner))
        );
        assert_eq!(w.counter, counter);
        let (mut second, attempt) = w.retry(&mut shield, &first, after).unwrap();
        assert_eq!(attempt, 2);
        assert_eq!(shield.active_intent, second.key);
        assert_eq!(shield.captured_position_generation, 2);
        assert_eq!(second.position_generation, Some(2));

        // Refund while attempt 2 is queued is refused.
        let m = w.market.clone();
        assert_eq!(
            c(refund_shield(
                &m,
                &mut shield,
                &OWNER,
                &second,
                &m.usdc_mint,
                &m.usdc_vault,
                &source(m.usdc_mint)
            )),
            Err(code(ErrorCode::PendingShieldAttemptQueued))
        );
        let callback = w.honest(&second, true, after + 1);
        assert_eq!(
            c(w.settle(&mut shield, &mut second, &callback)),
            Ok(ShieldSettlement::Credited {
                asset: ShieldAsset::Usdc,
                amount: 40
            })
        );
        assert_eq!(w.position.coordination.generation, 3);
    }

    // ---- Property test --------------------------------------------------

    struct Lcg(u64);

    impl Lcg {
        fn sample(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0 >> 33
        }

        fn below(&mut self, n: u64) -> u64 {
            self.sample() % n
        }
    }

    struct Shielded {
        shield: PendingShield,
        intents: Vec<OperationIntent>,
        ever_credited: bool,
        ever_refunded: bool,
    }

    /// Random shield / credit / zero / expire / unauthenticated / abandon /
    /// retry / refund / stray-callback sequences over both assets, driven
    /// through the real rules. After every step: each vault equals the sum of
    /// `Credited` plus `Deposited` amounts of its asset; the modelled free
    /// balance equals the `Credited` sum; no Pending Shield is both credited
    /// and refunded; the Position Lock is held exactly by the one queued
    /// attempt, if any.
    ///
    /// **Validates: Requirements 5.4, 5.5, 5.6, 4.8**
    #[test]
    fn prop_shield_value_is_conserved_and_credit_refund_are_exclusive() {
        let mut rng = Lcg(0x5EED_5A3D);
        for _ in 0..400 {
            let mut w = world();
            let mut slot: Slot = 10;
            let mut shields: Vec<Shielded> = Vec::new();
            let mut vault = [0u128; 2];
            let mut free = [0u128; 2];
            for _ in 0..40 {
                let queued = shields
                    .iter()
                    .position(|s| s.intents.last().unwrap().status == IntentStatus::Queued);
                let action = rng.below(9);
                match (action, queued) {
                    (0 | 1, _) => {
                        let asset = if rng.below(2) == 0 {
                            ShieldAsset::Sol
                        } else {
                            ShieldAsset::Usdc
                        };
                        let amount = rng.below(1_000);
                        let mut shield = PendingShield::default();
                        // Transfer first; the instruction reverts it on error.
                        let result = w.queue(&mut shield, asset, amount, slot);
                        let ok = amount > 0 && queued.is_none();
                        assert_eq!(result.is_ok(), ok, "queue amount {amount}");
                        if let Ok(intent) = result {
                            vault[asset_tag(asset) as usize] += u128::from(amount);
                            shields.push(Shielded {
                                shield,
                                intents: vec![intent],
                                ever_credited: false,
                                ever_refunded: false,
                            });
                        }
                    }
                    (2 | 3, Some(i)) => {
                        let credited = action == 2;
                        slot += 1;
                        let s = &mut shields[i];
                        let mut intent = *s.intents.last().unwrap();
                        let callback = w.honest(&intent, credited, slot);
                        let result = w.settle(&mut s.shield, &mut intent, &callback).unwrap();
                        *s.intents.last_mut().unwrap() = intent;
                        if let ShieldSettlement::Credited { asset, amount } = result {
                            assert!(credited);
                            free[asset_tag(asset) as usize] += u128::from(amount);
                            s.ever_credited = true;
                        } else {
                            assert_eq!(result, ShieldSettlement::Failed(IntentStatus::Zero));
                        }
                    }
                    (4, Some(i)) => {
                        // Late (expired) or unauthenticated delivery.
                        let s = &mut shields[i];
                        let mut intent = *s.intents.last().unwrap();
                        if rng.below(2) == 0 {
                            slot = slot.max(intent.decision_expiry_slot + 1);
                            let callback = w.honest(&intent, true, slot);
                            assert_eq!(
                                c(w.settle(&mut s.shield, &mut intent, &callback)),
                                Ok(ShieldSettlement::Failed(IntentStatus::Expired))
                            );
                            *s.intents.last_mut().unwrap() = intent;
                        } else {
                            let callback = ShieldCallback {
                                output_authenticated: false,
                                ..w.honest(&intent, true, slot)
                            };
                            assert!(w.settle(&mut s.shield, &mut intent, &callback).is_err());
                        }
                    }
                    (5, Some(i)) => {
                        let s = &mut shields[i];
                        let mut intent = *s.intents.last().unwrap();
                        slot = slot.max(intent.lock_expiry_slot + 1);
                        abandon_shield(&mut w.position, POSITION_KEY, &s.shield, &mut intent, slot)
                            .unwrap();
                        *s.intents.last_mut().unwrap() = intent;
                    }
                    (6, _) if !shields.is_empty() => {
                        let i = rng.below(shields.len() as u64) as usize;
                        let prior = *shields[i].intents.last().unwrap();
                        let mut shield = shields[i].shield;
                        let result = w.retry(&mut shield, &prior, slot);
                        let ok = shield.status == PendingShieldStatus::Deposited
                            && prior.status.is_terminal()
                            && prior.status != IntentStatus::Applied
                            && queued.is_none();
                        assert_eq!(result.is_ok(), ok);
                        shields[i].shield = shield;
                        if let Ok((intent, _)) = result {
                            shields[i].intents.push(intent);
                        }
                    }
                    (7, _) if !shields.is_empty() => {
                        let i = rng.below(shields.len() as u64) as usize;
                        let s = &mut shields[i];
                        let active = *s.intents.last().unwrap();
                        let asset = s.shield.asset;
                        let m = w.market.clone();
                        let mint = asset_mint(&m, asset);
                        let result = refund_shield(
                            &m,
                            &mut s.shield,
                            &OWNER,
                            &active,
                            &mint,
                            &asset_vault(&m, asset),
                            &source(mint),
                        );
                        if let Ok(refund) = result {
                            assert_eq!(refund.amount, s.shield.amount);
                            vault[asset_tag(asset) as usize] -= u128::from(refund.amount);
                            s.ever_refunded = true;
                        }
                    }
                    (8, _) if !shields.is_empty() => {
                        // Stray callback for an older attempt: always a no-op.
                        let i = rng.below(shields.len() as u64) as usize;
                        let s = &mut shields[i];
                        if s.intents.len() > 1 {
                            let j = rng.below(s.intents.len() as u64 - 1) as usize;
                            let mut old = s.intents[j];
                            let before = (s.shield, w.position.encrypted_state);
                            let callback = w.honest(&old, true, slot);
                            assert_eq!(
                                c(w.settle(&mut s.shield, &mut old, &callback)),
                                Ok(ShieldSettlement::Ignored)
                            );
                            assert_eq!(before, (s.shield, w.position.encrypted_state));
                        }
                    }
                    _ => {}
                }

                // Invariants.
                let mut expected_vault = [0u128; 2];
                let mut expected_free = [0u128; 2];
                for s in &shields {
                    assert!(!(s.ever_credited && s.ever_refunded));
                    let i = asset_tag(s.shield.asset) as usize;
                    match s.shield.status {
                        PendingShieldStatus::Credited => {
                            assert!(s.ever_credited);
                            expected_vault[i] += u128::from(s.shield.amount);
                            expected_free[i] += u128::from(s.shield.amount);
                        }
                        PendingShieldStatus::Deposited => {
                            assert!(!s.ever_credited && !s.ever_refunded);
                            expected_vault[i] += u128::from(s.shield.amount);
                        }
                        PendingShieldStatus::Refunded => assert!(s.ever_refunded),
                        PendingShieldStatus::Uninitialized => unreachable!(),
                    }
                }
                assert_eq!(vault, expected_vault);
                assert_eq!(free, expected_free);
                let holders: Vec<_> = shields
                    .iter()
                    .filter(|s| s.intents.last().unwrap().status == IntentStatus::Queued)
                    .collect();
                assert!(holders.len() <= 1);
                match holders.first() {
                    Some(s) => assert!(s
                        .intents
                        .last()
                        .unwrap()
                        .lock_owner()
                        .holds(&w.position.coordination.lock)),
                    None => assert!(!w.position.coordination.lock.is_active()),
                }
            }
        }
    }
}
