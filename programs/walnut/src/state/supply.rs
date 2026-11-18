//! USDC supply and withdraw supply (Task 8; design: "Supply", "Withdraw supply").
//!
//! One Operation family, one circuit (`supply_flow`), two Intent kinds
//! (`Supply`, `WithdrawSupply`). The owner moves value between the Position's
//! encrypted Free USDC and Supply Shares while the Pool's encrypted Cash and
//! Supply Shares move by the same amount. No SPL token moves: the vault already
//! holds the USDC from `shield`, and `unshield` moves it out later.
//!
//! # Flow
//!
//! 1. Queue ([`queue_supply_flow`]): owner-signed. The Market is `Active`, the
//!    `SUPPLY` or `WITHDRAW_SUPPLY` pause is off, the Position is `Active`, the
//!    request is nonzero, and (supply only) the utilization band is fresh.
//!    [`begin_pool_mutation`] accrues the accounting
//!    index, captures dependencies, and Locks the Pool AND the Position
//!    atomically (all or none).
//! 2. The circuit checkpoints interest FIRST, then runs the all-or-zero rules
//!    of `math::supply`, including the first-supplier canonical-empty check
//!    (`unsettled_recovery_claim_count == 0` is public and passed in) and the
//!    on-chain launch supply cap.
//! 3. Callback ([`settle_supply_flow`]): `Applied` stores both payloads,
//!    advances the accounted index, bumps both generations, and releases both
//!    Locks. `Zero`, `Stale`, `Expired`, and failed outcomes release both Locks
//!    and store nothing. A terminal Intent is a no-op. Unauthenticated output
//!    and every binding mismatch revert, so the Intent stays `Queued` until
//!    [`abandon_supply_flow`].
//!
//! # Intent binding
//!
//! `accounts_commitment`: owner, Market, Asset State, Position (captured
//! generation), Pool (captured generation). `inputs_commitment`: the Pool
//! payload, both indexes, the reserve factor, the Position key, epoch, payload,
//! output-nonce input, the direction flag, the request, the supply cap, and the
//! claims-clear flag.

use anchor_lang::prelude::Pubkey;

use super::accounts::{EncryptedPoolState, PoolState, PositionStatus};
use super::band::{INPUT_ACCOUNTING_INDEX, INPUT_POOL_CIPHERTEXT_FIRST, INPUT_POOL_NONCE};
use super::coordination::CoordinationSlot;
use super::nonce::{derive_output_nonce_input, INPUT_OUTPUT_NONCE};
use super::pool_mutation::{
    abandon_pool_mutation, begin_pool_mutation, pool_dependencies, settle_pool_decision,
    IntentCommitments, PoolCapture, PoolMutationRequest,
};
use super::position::{INPUT_NONCE, INPUT_VIEWING_KEY_EPOCH, INPUT_VIEWING_PUBKEY};
use super::reserve_withdrawal::{INPUT_ACCOUNTED_INDEX, INPUT_RESERVE_FACTOR_BPS};
use super::shield::INPUT_STATE_CIPHERTEXT_FIRST;
use super::{Position, Slot};
use crate::errors::ErrorCode;
use crate::market::{ensure_action_allowed, Action, AssetState, Market};
use crate::math::rate::ensure_band_fresh;
use crate::operation::callback::{
    classify_callback, CallbackContext, CallbackDecision, CallbackReport, CurrentDependencies,
};
use crate::operation::commitment::{
    account_commitment, input_commitment, AccountBinding, AccountRole, InputField, InputValue,
};
use crate::operation::{IntentStatus, OperationCounter, OperationIntent, OperationKind};

/// Input field id: `true` for withdraw-supply.
pub const INPUT_IS_WITHDRAW: u8 = 26;
/// Input field id: the request (`U128`): USDC units or shares.
pub const INPUT_SUPPLY_REQUEST: u8 = 27;
/// Input field id: the public supply cap (`U64`).
pub const INPUT_SUPPLY_CAP: u8 = 28;
/// Input field id: the public recovery-claims-clear flag (`Bool`).
pub const INPUT_CLAIMS_CLEAR: u8 = 29;

/// Decision Expiry distance of a supply or withdrawal, in slots.
pub const SUPPLY_DECISION_SLOTS: u64 = 1_500;
/// Slots between Decision Expiry and Lock expiry.
pub const SUPPLY_LOCK_GRACE_SLOTS: u64 = 300;

/// The Intent kind of a direction flag.
pub fn supply_kind(is_withdraw: bool) -> OperationKind {
    if is_withdraw {
        OperationKind::WithdrawSupply
    } else {
        OperationKind::Supply
    }
}

/// The ordered accounts bound by a supply or withdrawal Intent.
pub fn supply_bindings(
    owner: Pubkey,
    market_key: Pubkey,
    asset_state: Pubkey,
    position: Pubkey,
    position_generation: Option<u64>,
    pool: Pubkey,
    pool_generation: Option<u64>,
) -> [AccountBinding; 5] {
    let program = crate::ID;
    let plain = |key, role, expected_owner, writable, authority, generation| AccountBinding {
        key,
        role,
        expected_owner,
        mint: None,
        authority,
        writable,
        generation,
    };
    [
        plain(
            owner,
            AccountRole::Owner,
            anchor_lang::system_program::ID,
            true,
            None,
            None,
        ),
        plain(market_key, AccountRole::Market, program, false, None, None),
        plain(
            asset_state,
            AccountRole::AssetState,
            program,
            false,
            None,
            None,
        ),
        plain(
            position,
            AccountRole::Position,
            program,
            true,
            Some(owner),
            position_generation,
        ),
        plain(
            pool,
            AccountRole::Pool,
            program,
            true,
            None,
            pool_generation,
        ),
    ]
}

/// The public values a supply or withdrawal Intent binds besides the stored
/// payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupplyPublicInputs {
    /// The Pool's accounted index.
    pub accounted_index: u128,
    /// The accounting index captured by the Intent.
    pub accounting_index: u128,
    /// The reserve factor in bps.
    pub reserve_factor_bps: u16,
    /// Direction.
    pub is_withdraw: bool,
    /// The request.
    pub requested: u128,
    /// The launch supply cap.
    pub supply_cap: u64,
    /// `unsettled_recovery_claim_count == 0`.
    pub claims_clear: bool,
    /// The output-nonce input of the Position output.
    pub output_nonce_input: u128,
}

/// The ordered inputs bound by a supply or withdrawal Intent.
pub fn supply_inputs(
    pool: &PoolState,
    position: &Position,
    viewing_pubkey: [u8; 32],
    public: &SupplyPublicInputs,
) -> Vec<InputField<'static>> {
    let pc = &pool.encrypted.ciphertexts;
    let sc = &position.encrypted_state.ciphertexts;
    let pool_first = INPUT_POOL_CIPHERTEXT_FIRST;
    let state_first = INPUT_STATE_CIPHERTEXT_FIRST;
    let mut fields = vec![
        InputField::new(INPUT_POOL_NONCE, InputValue::U128(pool.encrypted.nonce)),
        InputField::new(INPUT_VIEWING_PUBKEY, InputValue::Bytes32(viewing_pubkey)),
        InputField::new(
            INPUT_VIEWING_KEY_EPOCH,
            InputValue::U64(position.viewing_key_epoch),
        ),
        InputField::new(
            INPUT_NONCE,
            InputValue::U128(position.encrypted_state.nonce),
        ),
        InputField::new(
            INPUT_ACCOUNTING_INDEX,
            InputValue::U128(public.accounting_index),
        ),
        InputField::new(
            INPUT_ACCOUNTED_INDEX,
            InputValue::U128(public.accounted_index),
        ),
        InputField::new(
            INPUT_RESERVE_FACTOR_BPS,
            InputValue::U64(public.reserve_factor_bps as u64),
        ),
        InputField::new(
            INPUT_OUTPUT_NONCE,
            InputValue::U128(public.output_nonce_input),
        ),
        InputField::new(INPUT_IS_WITHDRAW, InputValue::Bool(public.is_withdraw)),
        InputField::new(INPUT_SUPPLY_REQUEST, InputValue::U128(public.requested)),
        InputField::new(INPUT_SUPPLY_CAP, InputValue::U64(public.supply_cap)),
        InputField::new(INPUT_CLAIMS_CLEAR, InputValue::Bool(public.claims_clear)),
    ];
    for (i, c) in pc.iter().enumerate() {
        fields.push(InputField::new(
            pool_first + i as u8,
            InputValue::Bytes32(*c),
        ));
    }
    for (i, c) in sc.iter().enumerate() {
        fields.push(InputField::new(
            state_first + i as u8,
            InputValue::Bytes32(*c),
        ));
    }
    fields
}

/// One supply or withdrawal queue request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupplyFlowRequest {
    /// The Operation Intent PDA address.
    pub intent_key: Pubkey,
    /// The owner's next Intent sequence.
    pub sequence: u64,
    /// The Arcium computation offset.
    pub computation_offset: u64,
    /// The current slot.
    pub queued_slot: Slot,
    /// The current Unix time.
    pub now_ts: i64,
    /// `true` for withdraw-supply.
    pub is_withdraw: bool,
    /// USDC units (supply) or shares (withdraw).
    pub requested: u128,
}

/// Queues a supply or withdrawal: authorizes, checks the pause, then runs the
/// shared Pool mutation queue with the Position as a second Lock. Atomic.
#[allow(clippy::too_many_arguments)]
pub fn queue_supply_flow(
    market: &Market,
    market_key: Pubkey,
    asset_state_key: Pubkey,
    asset: &mut AssetState,
    pool: &mut PoolState,
    pool_key: Pubkey,
    position: &mut Position,
    position_key: Pubkey,
    counter: &mut OperationCounter,
    signer: &Pubkey,
    request: &SupplyFlowRequest,
) -> Result<OperationIntent, ErrorCode> {
    if *signer != position.owner {
        return Err(ErrorCode::SupplyNotOwner);
    }
    if position.status != PositionStatus::Active {
        return Err(ErrorCode::SupplyPositionNotActive);
    }
    if request.requested == 0 {
        return Err(ErrorCode::SupplyZeroRequest);
    }
    let action = if request.is_withdraw {
        Action::WithdrawSupply
    } else {
        Action::Supply
    };
    ensure_action_allowed(&asset.pauses, action, request.queued_slot)?;
    // Supply is risk-increasing: a stale utilization band blocks it before any
    // Lock is taken (Requirement 7.8). Withdrawal stays available.
    if !request.is_withdraw {
        ensure_band_fresh(
            asset.utilization_observation_slot,
            request.queued_slot,
            asset.rates.band_max_age_slots,
        )?;
    }

    let owner = position.owner;
    let position_generation = position.coordination.generation;
    let accounted = pool.accounted_borrow_index;
    let reserve_factor_bps = asset.risk.reserve_factor_bps;
    let supply_cap = asset.caps.supply_cap_usdc;
    let claims_clear = market.unsettled_recovery_claim_count == 0;
    let output_nonce_input = derive_output_nonce_input(&position_key, request.sequence);
    let pool_snapshot = pool.clone();
    let position_snapshot = position.clone();
    let mutation = PoolMutationRequest {
        intent_key: request.intent_key,
        owner,
        payer: owner,
        kind: supply_kind(request.is_withdraw),
        sequence: request.sequence,
        computation_offset: request.computation_offset,
        queued_slot: request.queued_slot,
        now_ts: request.now_ts,
        decision_slots: SUPPLY_DECISION_SLOTS,
        lock_grace_slots: SUPPLY_LOCK_GRACE_SLOTS,
    };
    let is_withdraw = request.is_withdraw;
    let requested = request.requested;
    let mut intent = begin_pool_mutation(
        market.status,
        asset,
        pool,
        pool_key,
        &mut [CoordinationSlot::new(
            position_key,
            &mut position.coordination,
        )],
        counter,
        &mutation,
        |capture: &PoolCapture| {
            let bindings = supply_bindings(
                owner,
                market_key,
                asset_state_key,
                position_key,
                Some(position_generation),
                pool_key,
                Some(capture.pool_generation),
            );
            let inputs = supply_inputs(
                &pool_snapshot,
                &position_snapshot,
                position_snapshot.viewing_pubkey,
                &SupplyPublicInputs {
                    accounted_index: accounted,
                    accounting_index: capture.accounting_borrow_index,
                    reserve_factor_bps,
                    is_withdraw,
                    requested,
                    supply_cap,
                    claims_clear,
                    output_nonce_input,
                },
            );
            IntentCommitments {
                accounts: account_commitment(&bindings),
                inputs: input_commitment(supply_kind(is_withdraw), &inputs),
            }
        },
    )?;
    intent.position = Some(position_key);
    intent.position_generation = Some(position_generation);
    Ok(intent)
}

/// What the `supply_flow` callback observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupplyFlowReport {
    /// Whether Arcium `verify_output` succeeded.
    pub output_authenticated: bool,
    /// The computation offset the callback's computation account belongs to.
    pub computation_offset: u64,
    /// The Operation Intent account the callback received.
    pub intent_key: Pubkey,
    /// The output Pool payload.
    pub pool_output: EncryptedPoolState,
    /// The Position output's encryption key (must be the Viewing Key).
    pub encryption_key: [u8; 32],
    /// The Position output nonce (output-nonce input + 1).
    pub position_nonce: u128,
    /// The Position output ciphertexts in `PositionSecret` order.
    pub position_ciphertexts: [[u8; 32]; 5],
    /// The revealed applied flag.
    pub applied: bool,
    /// The revealed request echo.
    pub requested: u128,
    /// The current slot.
    pub current_slot: Slot,
}

/// What the callback did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupplyFlowSettlement {
    /// `Applied`: both payloads are stored and both Locks released.
    Applied {
        /// `true` for withdraw-supply.
        is_withdraw: bool,
    },
    /// The circuit refused: nothing stored, nothing moved, Locks released.
    Zero,
    /// `Expired`, `Stale`, or `Failed`: nothing stored, Locks released.
    Failed(IntentStatus),
    /// The Intent was already terminal; nothing changed.
    Ignored,
}

/// Verifies and applies a `supply_flow` callback. Unauthenticated output and
/// commitment, offset, or binding mismatches are errors (the Intent stays
/// `Queued`). Atomic on error.
#[allow(clippy::too_many_arguments)]
pub fn settle_supply_flow(
    market: &Market,
    market_key: Pubkey,
    asset: &AssetState,
    pool: &mut PoolState,
    pool_key: Pubkey,
    position: &mut Position,
    position_key: Pubkey,
    intent: &mut OperationIntent,
    report: &SupplyFlowReport,
) -> Result<SupplyFlowSettlement, ErrorCode> {
    let kind = intent.kind;
    if !matches!(kind, OperationKind::Supply | OperationKind::WithdrawSupply)
        || intent.position != Some(position_key)
    {
        return Err(ErrorCode::SupplyIntentMismatch);
    }
    let is_withdraw = kind == OperationKind::WithdrawSupply;
    let bindings = supply_bindings(
        position.owner,
        market_key,
        market.asset_state,
        position_key,
        intent.position_generation,
        pool_key,
        intent.pool_generation,
    );
    let inputs = supply_inputs(
        pool,
        position,
        report.encryption_key,
        &SupplyPublicInputs {
            accounted_index: pool.accounted_borrow_index,
            accounting_index: intent.accounting_borrow_index,
            reserve_factor_bps: asset.risk.reserve_factor_bps,
            is_withdraw,
            requested: report.requested,
            supply_cap: asset.caps.supply_cap_usdc,
            claims_clear: market.unsettled_recovery_claim_count == 0,
            output_nonce_input: report.position_nonce.wrapping_sub(1),
        },
    );
    let context = CallbackContext {
        output_authenticated: report.output_authenticated,
        computation_offset: report.computation_offset,
        intent_key: report.intent_key,
        kind,
        accounts: &bindings,
        input_fields: &inputs,
        current: CurrentDependencies {
            position_generation: Some(position.coordination.generation),
            ..pool_dependencies(pool, asset)
        },
        current_slot: report.current_slot,
        report: CallbackReport::Success {
            nonzero: report.applied,
        },
    };
    let decision = classify_callback(intent, &context)?;
    match decision {
        CallbackDecision::LeaveQueued => return Err(ErrorCode::CallbackOutputNotAuthenticated),
        CallbackDecision::Ignore => return Ok(SupplyFlowSettlement::Ignored),
        _ => {}
    }
    if position.status != PositionStatus::Active {
        return Err(ErrorCode::SupplyPositionNotActive);
    }
    let result = settle_pool_decision(
        pool,
        pool_key,
        &mut [CoordinationSlot::new(
            position_key,
            &mut position.coordination,
        )],
        intent,
        decision,
        &report.pool_output,
        report.current_slot,
    )?;
    Ok(match result.status {
        IntentStatus::Applied => {
            position.encrypted_state = super::accounts::EncryptedPositionPayload {
                nonce: report.position_nonce,
                ciphertexts: report.position_ciphertexts,
            };
            SupplyFlowSettlement::Applied { is_withdraw }
        }
        IntentStatus::Zero => SupplyFlowSettlement::Zero,
        status => SupplyFlowSettlement::Failed(status),
    })
}

/// `abandon_supply_flow`: permissionless timeout recovery after Lock expiry.
/// Bumps both generations, releases both Locks, marks the Intent `Abandoned`.
/// Nothing moved, so nothing to refund. Returns the Locks cleared.
pub fn abandon_supply_flow(
    pool: &mut PoolState,
    pool_key: Pubkey,
    position: &mut Position,
    position_key: Pubkey,
    intent: &mut OperationIntent,
    current_slot: Slot,
) -> Result<usize, ErrorCode> {
    if intent.position != Some(position_key)
        || !matches!(
            intent.kind,
            OperationKind::Supply | OperationKind::WithdrawSupply
        )
    {
        return Err(ErrorCode::SupplyIntentMismatch);
    }
    let kind = intent.kind;
    abandon_pool_mutation(
        pool,
        pool_key,
        &mut [CoordinationSlot::new(
            position_key,
            &mut position.coordination,
        )],
        intent,
        kind,
        current_slot,
    )
}

#[cfg(test)]
mod tests;
