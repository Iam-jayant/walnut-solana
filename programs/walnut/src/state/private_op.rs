//! Private operations with encrypted amounts (ADR-0004, track P).
//!
//! Three circuits share these rules:
//!
//! * `pool_op` ([`OperationKind::PoolOp`]): supply, withdraw supply, borrow
//!   and repay. The encrypted Pool (`PoolState`, `Enc<Mxe>`) and the owner's
//!   Position change together. Locks the Pool and the Position.
//! * `position_private` ([`OperationKind::PositionPrivate`]): add and withdraw
//!   collateral. Locks the Position.
//! * `liquidate_pool` ([`OperationKind::LiquidatePool`]): the liquidator pays
//!   from its own encrypted free USDC and receives the seized lamports into its
//!   own encrypted free SOL. Locks the Pool, the target and the liquidator's
//!   Position.
//!
//! # Amounts
//!
//! The owner encrypts `(amount, shares)` (liquidation: `(amount, burn_shares,
//! seize)`) to the MXE. No instruction argument, Intent field or event carries
//! an amount. The circuits verify the share conversion against the public
//! indexes with multiplications and comparisons only.
//!
//! # Indexes
//!
//! The public `LendingPool` indexes are the conversion authority. The client
//! chooses `accrue_to_ts` (not in the future, at most
//! [`MAX_ACCRUAL_LAG_SECONDS`] old, not before the last accrual) and projects
//! the indexes to it off chain; the queue accrues to exactly that time, so both
//! sides use identical indexes. A mismatch only makes the circuit refuse.
//!
//! # Intent binding
//!
//! `accounts_commitment`: payer, Market, Asset State, LendingPool, Pool (when
//! locked, with its generation), Position and second Position (with their
//! generations). `inputs_commitment`: the stored Pool and Position payloads,
//! the Viewing keys and epochs, the output-nonce inputs and the public circuit
//! parameters. The client ciphertext is not bound: it is immutable inside the
//! queued computation, which `classify_callback` already ties to the Intent
//! through the computation offset.

use anchor_lang::prelude::Pubkey;

use super::accounts::{EncryptedPoolState, EncryptedPositionPayload, PoolState, PoolStatus};
use super::band::{INPUT_POOL_CIPHERTEXT_FIRST, INPUT_POOL_NONCE};
use super::coordination::{acquire_locks, CoordinationSlot};
use super::lend_op::ensure_lend_op_allowed;
use super::lending_pool::LendingPool;
use super::nonce::{derive_output_nonce_input, INPUT_OUTPUT_NONCE};
use super::pool_mutation::{abandon_pool_mutation, settle_pool_decision};
use super::position::{INPUT_NONCE, INPUT_VIEWING_KEY_EPOCH, INPUT_VIEWING_PUBKEY};
use super::shield::INPUT_STATE_CIPHERTEXT_FIRST;
use super::{Position, PositionStatus, Slot};
use crate::errors::ErrorCode;
use crate::market::{AssetState, Market, MarketStatus};
use crate::math::lend::{
    OP_ADD_COLLATERAL, OP_BORROW, OP_LIQUIDATE, OP_REPAY, OP_SUPPLY, OP_WITHDRAW_COLLATERAL,
    OP_WITHDRAW_SUPPLY,
};
use crate::operation::callback::{
    apply_decision_to_slots, classify_callback, CallbackContext, CallbackDecision, CallbackReport,
    CurrentDependencies,
};
use crate::operation::commitment::{
    account_commitment, input_commitment, AccountBinding, AccountRole, InputField, InputValue,
};
use crate::operation::seeds::reserve_intent;
use crate::operation::{
    abandon_intent, IntentStatus, OperationCounter, OperationIntent, OperationKind,
};

/// `pool_op` codes.
pub const POOL_SUPPLY: u64 = 0;
/// Withdraw supply.
pub const POOL_WITHDRAW_SUPPLY: u64 = 1;
/// Borrow.
pub const POOL_BORROW: u64 = 2;
/// Repay.
pub const POOL_REPAY: u64 = 3;
/// `position_private` codes: add collateral.
pub const COLLATERAL_ADD: u64 = 0;
/// Withdraw collateral.
pub const COLLATERAL_WITHDRAW: u64 = 1;

/// Oldest accrual timestamp a queue accepts, relative to the clock.
pub const MAX_ACCRUAL_LAG_SECONDS: i64 = 120;
/// Decision Expiry distance of a private operation, in slots.
pub const PRIVATE_DECISION_SLOTS: u64 = 1_500;
/// Slots between Decision Expiry and Lock expiry.
pub const PRIVATE_LOCK_GRACE_SLOTS: u64 = 300;

/// Input field ids (append-only; unique within each kind).
pub const INPUT_PRIVATE_OP: u8 = 40;
/// Supply index (1e9).
pub const INPUT_PRIVATE_SUPPLY_INDEX: u8 = 41;
/// Borrow index (1e9).
pub const INPUT_PRIVATE_BORROW_INDEX: u8 = 42;
/// Price (USDC base units per SOL).
pub const INPUT_PRIVATE_PRICE: u8 = 43;
/// LTV in bps.
pub const INPUT_PRIVATE_LTV: u8 = 44;
/// Liquidation threshold in bps.
pub const INPUT_PRIVATE_LIQ_THRESHOLD: u8 = 45;
/// Liquidation bonus in bps.
pub const INPUT_PRIVATE_BONUS: u8 = 46;
/// Position debt cap (USDC).
pub const INPUT_PRIVATE_DEBT_CAP: u8 = 47;
/// Supply cap (USDC).
pub const INPUT_PRIVATE_SUPPLY_CAP: u8 = 48;
/// Second Position Viewing key.
pub const INPUT_SECOND_VIEWING_PUBKEY: u8 = 50;
/// Second Position Viewing key epoch.
pub const INPUT_SECOND_VIEWING_KEY_EPOCH: u8 = 51;
/// Second Position stored nonce.
pub const INPUT_SECOND_NONCE: u8 = 52;
/// Second Position output-nonce input.
pub const INPUT_SECOND_OUTPUT_NONCE: u8 = 53;
/// First of the five second-Position stored ciphertexts.
pub const INPUT_SECOND_CIPHERTEXT_FIRST: u8 = 54;

/// The lend-op code whose pause category gates a private operation.
pub fn pause_code(kind: OperationKind, op: u64) -> Result<u64, ErrorCode> {
    Ok(match (kind, op) {
        (OperationKind::PoolOp, POOL_SUPPLY) => OP_SUPPLY,
        (OperationKind::PoolOp, POOL_WITHDRAW_SUPPLY) => OP_WITHDRAW_SUPPLY,
        (OperationKind::PoolOp, POOL_BORROW) => OP_BORROW,
        (OperationKind::PoolOp, POOL_REPAY) => OP_REPAY,
        (OperationKind::PositionPrivate, COLLATERAL_ADD) => OP_ADD_COLLATERAL,
        (OperationKind::PositionPrivate, COLLATERAL_WITHDRAW) => OP_WITHDRAW_COLLATERAL,
        (OperationKind::LiquidatePool, 0) => OP_LIQUIDATE,
        _ => return Err(ErrorCode::PrivateOpInvalid),
    })
}

/// Pause gate shared by the three queues.
pub fn ensure_private_allowed(
    asset: &AssetState,
    kind: OperationKind,
    op: u64,
    slot: Slot,
) -> Result<(), ErrorCode> {
    ensure_lend_op_allowed(&asset.pauses, pause_code(kind, op)?, slot)
}

/// Whether the op reads the Pyth price.
pub fn needs_price(kind: OperationKind, op: u64) -> bool {
    matches!(
        (kind, op),
        (OperationKind::PoolOp, POOL_BORROW)
            | (OperationKind::PositionPrivate, COLLATERAL_WITHDRAW)
            | (OperationKind::LiquidatePool, _)
    )
}

/// In band mode a borrow (risk-increasing) needs a fresh utilization band, so
/// borrowers never accrue at a band that is older than `band_max_age_slots`.
pub fn ensure_rate_fresh(
    pool: &LendingPool,
    asset: &AssetState,
    kind: OperationKind,
    op: u64,
    slot: Slot,
) -> Result<(), ErrorCode> {
    if pool.rate_source == super::lending_pool::RATE_SOURCE_BAND
        && kind == OperationKind::PoolOp
        && op == POOL_BORROW
    {
        crate::math::rate::ensure_band_fresh(
            asset.utilization_observation_slot,
            slot,
            asset.rates.band_max_age_slots,
        )?;
    }
    Ok(())
}

/// Accrues `pool` to the client-chosen `accrue_to_ts` (see the module docs).
pub fn accrue_to(
    pool: &mut LendingPool,
    asset: &AssetState,
    accrue_to_ts: i64,
    now_ts: i64,
) -> Result<(), ErrorCode> {
    if accrue_to_ts > now_ts
        || now_ts.saturating_sub(accrue_to_ts) > MAX_ACCRUAL_LAG_SECONDS
        || accrue_to_ts < pool.last_accrual_ts
    {
        return Err(ErrorCode::PrivateAccrualTimeInvalid);
    }
    pool.accrue_auto(accrue_to_ts, asset)?;
    Ok(())
}

/// The public circuit parameters of a private operation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PrivateParams {
    /// The operation code.
    pub op: u64,
    /// Supply index (1e9).
    pub supply_index_1e9: u64,
    /// Borrow index (1e9).
    pub borrow_index_1e9: u64,
    /// Price, 0 when unused.
    pub price: u64,
    /// LTV in bps.
    pub ltv_bps: u64,
    /// Liquidation threshold in bps.
    pub liq_threshold_bps: u64,
    /// Liquidation bonus in bps.
    pub bonus_bps: u64,
    /// Position debt cap (USDC).
    pub position_debt_cap: u64,
    /// Supply cap (USDC).
    pub supply_cap: u64,
}

impl PrivateParams {
    /// Stores the parameters in Intent fields the private kinds do not use
    /// otherwise, so the callback can rebuild the input commitment.
    /// `CurrentDependencies::captured_by` compares these fields with
    /// themselves, so they never make a callback stale.
    pub fn write_to_intent(&self, intent: &mut OperationIntent) -> Result<(), ErrorCode> {
        let bps = |v: u64| {
            if v > u64::from(u16::MAX) {
                Err(ErrorCode::PrivateOpInvalid)
            } else {
                Ok(v)
            }
        };
        intent.asset_generation = self.op;
        intent.oracle_generation = bps(self.ltv_bps)?
            | (bps(self.liq_threshold_bps)? << 16)
            | (bps(self.bonus_bps)? << 32);
        intent.rate_epoch_generation = self.supply_cap;
        intent.accounting_borrow_index =
            u128::from(self.supply_index_1e9) | (u128::from(self.borrow_index_1e9) << 64);
        intent.risk_borrow_index_at_expiry =
            u128::from(self.price) | (u128::from(self.position_debt_cap) << 64);
        Ok(())
    }

    /// The inverse of [`PrivateParams::write_to_intent`].
    pub fn from_intent(intent: &OperationIntent) -> Self {
        let mask16 = u64::from(u16::MAX);
        let low = |v: u128| (v & u128::from(u64::MAX)) as u64;
        Self {
            op: intent.asset_generation,
            ltv_bps: intent.oracle_generation & mask16,
            liq_threshold_bps: (intent.oracle_generation >> 16) & mask16,
            bonus_bps: (intent.oracle_generation >> 32) & mask16,
            supply_cap: intent.rate_epoch_generation,
            supply_index_1e9: low(intent.accounting_borrow_index),
            borrow_index_1e9: low(intent.accounting_borrow_index >> 64),
            price: low(intent.risk_borrow_index_at_expiry),
            position_debt_cap: low(intent.risk_borrow_index_at_expiry >> 64),
        }
    }
}

/// The accounts a private Intent binds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrivateKeys {
    /// The signer (owner or liquidator).
    pub payer: Pubkey,
    /// The Market.
    pub market: Pubkey,
    /// The Asset State.
    pub asset_state: Pubkey,
    /// The LendingPool (index authority).
    pub lending_pool: Pubkey,
    /// The Pool State and its captured generation, when locked.
    pub pool: Option<(Pubkey, Option<u64>)>,
    /// The primary (owner or target) Position, its owner and generation.
    pub position: (Pubkey, Pubkey, Option<u64>),
    /// The second (liquidator) Position, its owner and generation.
    pub second: Option<(Pubkey, Pubkey, Option<u64>)>,
}

/// The ordered accounts bound by a private Intent.
pub fn private_bindings(keys: &PrivateKeys) -> Vec<AccountBinding> {
    let program = crate::ID;
    let plain = |key, role, owner, writable, authority, generation| AccountBinding {
        key,
        role,
        expected_owner: owner,
        mint: None,
        authority,
        writable,
        generation,
    };
    let mut accounts = vec![
        plain(
            keys.payer,
            AccountRole::Owner,
            anchor_lang::system_program::ID,
            true,
            None,
            None,
        ),
        plain(keys.market, AccountRole::Market, program, false, None, None),
        plain(
            keys.asset_state,
            AccountRole::AssetState,
            program,
            false,
            None,
            None,
        ),
        plain(
            keys.lending_pool,
            AccountRole::Other,
            program,
            true,
            None,
            None,
        ),
    ];
    if let Some((pool, generation)) = keys.pool {
        accounts.push(plain(
            pool,
            AccountRole::Pool,
            program,
            true,
            None,
            generation,
        ));
    }
    let (position, owner, generation) = keys.position;
    accounts.push(plain(
        position,
        AccountRole::Position,
        program,
        true,
        Some(owner),
        generation,
    ));
    if let Some((second, owner, generation)) = keys.second {
        accounts.push(plain(
            second,
            AccountRole::SecondPosition,
            program,
            true,
            Some(owner),
            generation,
        ));
    }
    accounts
}

/// One Position's sealing inputs: the key its output is sealed to, its epoch,
/// its stored payload and the output-nonce input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SealInputs {
    /// The output encryption key (the Position's Viewing key).
    pub viewing_pubkey: [u8; 32],
    /// The Viewing key epoch.
    pub viewing_key_epoch: u64,
    /// The stored payload the circuit reads.
    pub state: EncryptedPositionPayload,
    /// The output-nonce input.
    pub output_nonce_input: u128,
}

impl SealInputs {
    /// The queue-time sealing inputs of `position` for `sequence`.
    pub fn at_queue(position: &Position, position_key: &Pubkey, sequence: u64) -> Self {
        Self {
            viewing_pubkey: position.viewing_pubkey,
            viewing_key_epoch: position.viewing_key_epoch,
            state: position.encrypted_state,
            output_nonce_input: derive_output_nonce_input(position_key, sequence),
        }
    }
}

/// The ordered inputs bound by a private Intent.
pub fn private_inputs(
    pool: Option<&EncryptedPoolState>,
    primary: &SealInputs,
    second: Option<&SealInputs>,
    params: &PrivateParams,
) -> Vec<InputField<'static>> {
    let mut fields = Vec::with_capacity(32);
    if let Some(pool) = pool {
        fields.push(InputField::new(
            INPUT_POOL_NONCE,
            InputValue::U128(pool.nonce),
        ));
        for (i, c) in pool.ciphertexts.iter().enumerate() {
            fields.push(InputField::new(
                INPUT_POOL_CIPHERTEXT_FIRST + i as u8,
                InputValue::Bytes32(*c),
            ));
        }
    }
    fields.push(InputField::new(
        INPUT_VIEWING_PUBKEY,
        InputValue::Bytes32(primary.viewing_pubkey),
    ));
    fields.push(InputField::new(
        INPUT_VIEWING_KEY_EPOCH,
        InputValue::U64(primary.viewing_key_epoch),
    ));
    fields.push(InputField::new(
        INPUT_NONCE,
        InputValue::U128(primary.state.nonce),
    ));
    for (i, c) in primary.state.ciphertexts.iter().enumerate() {
        fields.push(InputField::new(
            INPUT_STATE_CIPHERTEXT_FIRST + i as u8,
            InputValue::Bytes32(*c),
        ));
    }
    fields.push(InputField::new(
        INPUT_OUTPUT_NONCE,
        InputValue::U128(primary.output_nonce_input),
    ));
    if let Some(second) = second {
        fields.push(InputField::new(
            INPUT_SECOND_VIEWING_PUBKEY,
            InputValue::Bytes32(second.viewing_pubkey),
        ));
        fields.push(InputField::new(
            INPUT_SECOND_VIEWING_KEY_EPOCH,
            InputValue::U64(second.viewing_key_epoch),
        ));
        fields.push(InputField::new(
            INPUT_SECOND_NONCE,
            InputValue::U128(second.state.nonce),
        ));
        fields.push(InputField::new(
            INPUT_SECOND_OUTPUT_NONCE,
            InputValue::U128(second.output_nonce_input),
        ));
        for (i, c) in second.state.ciphertexts.iter().enumerate() {
            fields.push(InputField::new(
                INPUT_SECOND_CIPHERTEXT_FIRST + i as u8,
                InputValue::Bytes32(*c),
            ));
        }
    }
    let p = params;
    for (id, value) in [
        (INPUT_PRIVATE_OP, p.op),
        (INPUT_PRIVATE_SUPPLY_INDEX, p.supply_index_1e9),
        (INPUT_PRIVATE_BORROW_INDEX, p.borrow_index_1e9),
        (INPUT_PRIVATE_PRICE, p.price),
        (INPUT_PRIVATE_LTV, p.ltv_bps),
        (INPUT_PRIVATE_LIQ_THRESHOLD, p.liq_threshold_bps),
        (INPUT_PRIVATE_BONUS, p.bonus_bps),
        (INPUT_PRIVATE_DEBT_CAP, p.position_debt_cap),
        (INPUT_PRIVATE_SUPPLY_CAP, p.supply_cap),
    ] {
        fields.push(InputField::new(id, InputValue::U64(value)));
    }
    fields
}

/// One private queue request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrivateRequest {
    /// The Intent kind.
    pub kind: OperationKind,
    /// The Operation Intent PDA address.
    pub intent_key: Pubkey,
    /// The signer's next Intent sequence.
    pub sequence: u64,
    /// The Arcium computation offset.
    pub computation_offset: u64,
    /// The current slot.
    pub queued_slot: Slot,
    /// The circuit parameters.
    pub params: PrivateParams,
    /// Oracle publish time of the price used, if any.
    pub publish_time: Option<i64>,
}

/// Checks the Market and the accounts, builds and reserves the Intent and
/// takes every Lock (Pool, Position, second Position), all or none.
/// `pool` and `second` follow the kind: `PoolOp` has a Pool, `LiquidatePool`
/// has a Pool and a second Position, `PositionPrivate` has neither. Owner ops
/// need the signer to own the Position; a liquidation needs the signer to own
/// the second Position and the two Positions to differ. Atomic.
#[allow(clippy::too_many_arguments)]
pub fn queue_private(
    market: &Market,
    keys: &PrivateKeys,
    pool: Option<&mut PoolState>,
    position: &mut Position,
    second: Option<&mut Position>,
    counter: &mut OperationCounter,
    request: &PrivateRequest,
) -> Result<OperationIntent, ErrorCode> {
    let kind = request.kind;
    let signer = keys.payer;
    if market.status != MarketStatus::Active {
        return Err(ErrorCode::MarketNotActive);
    }
    pause_code(kind, request.params.op)?;
    let expects_pool = matches!(kind, OperationKind::PoolOp | OperationKind::LiquidatePool);
    let expects_second = kind == OperationKind::LiquidatePool;
    if pool.is_some() != expects_pool
        || keys.pool.is_some() != expects_pool
        || second.is_some() != expects_second
        || keys.second.is_some() != expects_second
    {
        return Err(ErrorCode::PrivateOpIntentMismatch);
    }
    if position.status != PositionStatus::Active {
        return Err(ErrorCode::PositionOpPositionNotActive);
    }
    let (position_key, _, _) = keys.position;
    match (&second, keys.second) {
        (Some(liquidator), Some((liquidator_key, _, _))) => {
            if liquidator.owner != signer {
                return Err(ErrorCode::PositionOpNotOwner);
            }
            if liquidator_key == position_key || position.owner == signer {
                return Err(ErrorCode::PrivateLiquidationSelf);
            }
            if liquidator.status != PositionStatus::Active {
                return Err(ErrorCode::PositionOpPositionNotActive);
            }
        }
        _ => {
            if position.owner != signer {
                return Err(ErrorCode::PositionOpNotOwner);
            }
        }
    }
    if let Some(pool) = &pool {
        if pool.status != PoolStatus::Active {
            return Err(ErrorCode::PoolMutationPoolNotActive);
        }
    }

    let decision_expiry_slot = request
        .queued_slot
        .checked_add(PRIVATE_DECISION_SLOTS)
        .ok_or(ErrorCode::InvalidIntentTiming)?;
    let lock_expiry_slot = decision_expiry_slot
        .checked_add(PRIVATE_LOCK_GRACE_SLOTS)
        .ok_or(ErrorCode::InvalidIntentTiming)?;
    let primary = SealInputs::at_queue(position, &position_key, request.sequence);
    let second_seal = match (&second, keys.second) {
        (Some(p), Some((k, _, _))) => Some(SealInputs::at_queue(p, &k, request.sequence)),
        _ => None,
    };
    let inputs = private_inputs(
        pool.as_ref().map(|p| &p.encrypted),
        &primary,
        second_seal.as_ref(),
        &request.params,
    );
    let mut intent = OperationIntent {
        key: request.intent_key,
        owner: signer,
        payer: signer,
        reclaim_authority: signer,
        kind,
        status: IntentStatus::Queued,
        sequence: request.sequence,
        computation_offset: request.computation_offset,
        accounts_commitment: account_commitment(&private_bindings(keys)),
        inputs_commitment: input_commitment(kind, &inputs),
        queued_slot: request.queued_slot,
        decision_expiry_slot,
        lock_expiry_slot,
        position: Some(position_key),
        position_generation: keys.position.2,
        second_position: keys.second.map(|s| s.0),
        second_position_generation: keys.second.and_then(|s| s.2),
        pool_generation: keys.pool.and_then(|p| p.1),
        oracle_publish_time_sol: request.publish_time,
        ..OperationIntent::default()
    };
    request.params.write_to_intent(&mut intent)?;

    let mut next_counter = *counter;
    reserve_intent(&mut next_counter, &intent)?;
    let mut pool_coordination = pool.as_ref().map(|p| p.coordination);
    let mut position_coordination = position.coordination;
    let mut second_coordination = second.as_ref().map(|p| p.coordination);
    {
        let mut slots: Vec<CoordinationSlot<'_>> = Vec::with_capacity(3);
        if let (Some(c), Some((k, _))) = (pool_coordination.as_mut(), keys.pool) {
            slots.push(CoordinationSlot::new(k, c));
        }
        slots.push(CoordinationSlot::new(
            position_key,
            &mut position_coordination,
        ));
        if let (Some(c), Some((k, _, _))) = (second_coordination.as_mut(), keys.second) {
            slots.push(CoordinationSlot::new(k, c));
        }
        acquire_locks(intent.lock_owner(), intent.lock_expiry_slot, &mut slots)?;
    }
    *counter = next_counter;
    if let (Some(pool), Some(c)) = (pool, pool_coordination) {
        pool.coordination = c;
    }
    position.coordination = position_coordination;
    if let (Some(second), Some(c)) = (second, second_coordination) {
        second.coordination = c;
    }
    Ok(intent)
}

/// One sealed Position output of a callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SealedPosition {
    /// The output encryption key.
    pub encryption_key: [u8; 32],
    /// The output nonce (output-nonce input + 1).
    pub nonce: u128,
    /// The ciphertexts in `PositionSecret` order.
    pub ciphertexts: [[u8; 32]; 5],
}

/// What a private callback observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrivateReport {
    /// Whether Arcium `verify_output` succeeded.
    pub output_authenticated: bool,
    /// The computation offset the callback's computation account belongs to.
    pub computation_offset: u64,
    /// The Operation Intent account the callback received.
    pub intent_key: Pubkey,
    /// The output Pool payload, when the kind has a Pool.
    pub pool_output: Option<EncryptedPoolState>,
    /// The primary Position output.
    pub primary: SealedPosition,
    /// The second Position output, for a liquidation.
    pub second: Option<SealedPosition>,
    /// The one revealed bit.
    pub applied: bool,
    /// The current slot.
    pub current_slot: Slot,
}

/// Verifies and applies a private callback. Unauthenticated output and any
/// binding mismatch are errors (the Intent stays `Queued`). `Applied` stores
/// every payload and bumps every generation; anything else stores nothing and
/// releases every Lock. Returns the Intent status, or `None` when the Intent
/// was already terminal. Atomic on error.
#[allow(clippy::too_many_arguments)]
pub fn settle_private(
    keys: &PrivateKeys,
    pool: Option<&mut PoolState>,
    position: &mut Position,
    second: Option<&mut Position>,
    intent: &mut OperationIntent,
    kind: OperationKind,
    report: &PrivateReport,
) -> Result<Option<IntentStatus>, ErrorCode> {
    let (position_key, _, _) = keys.position;
    if intent.kind != kind
        || intent.position != Some(position_key)
        || intent.second_position != keys.second.map(|s| s.0)
        || pool.is_some() != keys.pool.is_some()
        || second.is_some() != keys.second.is_some()
        || report.pool_output.is_some() != pool.is_some()
        || report.second.is_some() != second.is_some()
    {
        return Err(ErrorCode::PrivateOpIntentMismatch);
    }
    let params = PrivateParams::from_intent(intent);
    let bound = PrivateKeys {
        pool: keys.pool.map(|(k, _)| (k, intent.pool_generation)),
        position: (keys.position.0, keys.position.1, intent.position_generation),
        second: keys
            .second
            .map(|(k, o, _)| (k, o, intent.second_position_generation)),
        ..*keys
    };
    let seal = |p: &Position, out: &SealedPosition| SealInputs {
        viewing_pubkey: out.encryption_key,
        viewing_key_epoch: p.viewing_key_epoch,
        state: p.encrypted_state,
        output_nonce_input: out.nonce.wrapping_sub(1),
    };
    let primary = seal(position, &report.primary);
    let second_seal = match (&second, &report.second) {
        (Some(p), Some(out)) => Some(seal(p, out)),
        _ => None,
    };
    let inputs = private_inputs(
        pool.as_ref().map(|p| &p.encrypted),
        &primary,
        second_seal.as_ref(),
        &params,
    );
    let bindings = private_bindings(&bound);
    let context = CallbackContext {
        output_authenticated: report.output_authenticated,
        computation_offset: report.computation_offset,
        intent_key: report.intent_key,
        kind,
        accounts: &bindings,
        input_fields: &inputs,
        current: CurrentDependencies {
            position_generation: Some(position.coordination.generation),
            second_position_generation: second.as_ref().map(|p| p.coordination.generation),
            pool_generation: pool.as_ref().map(|p| p.coordination.generation),
            ..CurrentDependencies::captured_by(intent)
        },
        current_slot: report.current_slot,
        report: CallbackReport::Success {
            nonzero: report.applied,
        },
    };
    let decision = classify_callback(intent, &context)?;
    match decision {
        CallbackDecision::LeaveQueued => return Err(ErrorCode::CallbackOutputNotAuthenticated),
        CallbackDecision::Ignore => return Ok(None),
        _ => {}
    }
    if position.status != PositionStatus::Active
        || second
            .as_ref()
            .is_some_and(|p| p.status != PositionStatus::Active)
    {
        return Err(ErrorCode::PositionOpPositionNotActive);
    }
    let status = match pool {
        Some(pool) => {
            let pool_key = keys.pool.map(|p| p.0).unwrap_or_default();
            let pool_output = report.pool_output.unwrap_or_default();
            let mut slots: Vec<CoordinationSlot<'_>> = Vec::with_capacity(2);
            slots.push(CoordinationSlot::new(
                position_key,
                &mut position.coordination,
            ));
            let second_key = keys.second.map(|s| s.0).unwrap_or_default();
            let mut second_ref = second;
            if let Some(s) = second_ref.as_deref_mut() {
                slots.push(CoordinationSlot::new(second_key, &mut s.coordination));
            }
            let status = settle_pool_decision(
                pool,
                pool_key,
                &mut slots,
                intent,
                decision,
                &pool_output,
                report.current_slot,
            )?
            .status;
            drop(slots);
            if status == IntentStatus::Applied {
                store(position, &report.primary);
                if let (Some(s), Some(out)) = (second_ref, report.second.as_ref()) {
                    store(s, out);
                }
            }
            status
        }
        None => {
            if !intent.lock_owner().holds(&position.coordination.lock) {
                return Err(ErrorCode::PrivateOpIntentMismatch);
            }
            let mut next_intent = *intent;
            let mut coordination = position.coordination;
            let result = apply_decision_to_slots(
                &mut next_intent,
                &mut [CoordinationSlot::new(position_key, &mut coordination)],
                decision,
                report.current_slot,
            )?;
            *intent = next_intent;
            position.coordination = coordination;
            if result.status == IntentStatus::Applied {
                store(position, &report.primary);
            }
            result.status
        }
    };
    Ok(Some(status))
}

fn store(position: &mut Position, out: &SealedPosition) {
    position.encrypted_state = EncryptedPositionPayload {
        nonce: out.nonce,
        ciphertexts: out.ciphertexts,
    };
}

/// Permissionless timeout recovery of a private operation after Lock expiry.
/// Bumps every generation, releases every Lock and marks the Intent
/// `Abandoned`. Nothing moved, so nothing is refunded.
pub fn abandon_private(
    keys: &PrivateKeys,
    pool: Option<&mut PoolState>,
    position: &mut Position,
    second: Option<&mut Position>,
    intent: &mut OperationIntent,
    current_slot: Slot,
) -> Result<usize, ErrorCode> {
    let kind = intent.kind;
    let (position_key, _, _) = keys.position;
    if !matches!(
        kind,
        OperationKind::PoolOp | OperationKind::PositionPrivate | OperationKind::LiquidatePool
    ) || intent.position != Some(position_key)
        || intent.second_position != keys.second.map(|s| s.0)
        || second.is_some() != keys.second.is_some()
    {
        return Err(ErrorCode::PrivateOpIntentMismatch);
    }
    let mut slots: Vec<CoordinationSlot<'_>> = Vec::with_capacity(2);
    slots.push(CoordinationSlot::new(
        position_key,
        &mut position.coordination,
    ));
    let second_key = keys.second.map(|s| s.0).unwrap_or_default();
    if let Some(s) = second {
        slots.push(CoordinationSlot::new(second_key, &mut s.coordination));
    }
    match (pool, keys.pool) {
        (Some(pool), Some((pool_key, _))) => {
            abandon_pool_mutation(pool, pool_key, &mut slots, intent, kind, current_slot)
        }
        (None, None) if kind == OperationKind::PositionPrivate => {
            Ok(abandon_intent(intent, current_slot, &mut slots)?)
        }
        _ => Err(ErrorCode::PrivateOpIntentMismatch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_round_trip_through_the_intent() {
        let p = PrivateParams {
            op: 2,
            supply_index_1e9: 1_000_000_123,
            borrow_index_1e9: 1_000_004_567,
            price: 72_406_137,
            ltv_bps: 5_000,
            liq_threshold_bps: 6_500,
            bonus_bps: 500,
            position_debt_cap: 1_000_000_000_000,
            supply_cap: 50_000_000_000,
        };
        let mut intent = OperationIntent::default();
        p.write_to_intent(&mut intent).unwrap();
        assert_eq!(PrivateParams::from_intent(&intent), p);
    }

    #[test]
    fn bps_above_u16_are_rejected() {
        let p = PrivateParams {
            ltv_bps: 70_000,
            ..PrivateParams::default()
        };
        let mut intent = OperationIntent::default();
        assert_eq!(
            p.write_to_intent(&mut intent),
            Err(ErrorCode::PrivateOpInvalid)
        );
    }

    #[test]
    fn pause_codes_cover_every_op_and_reject_unknown_ones() {
        assert_eq!(
            pause_code(OperationKind::PoolOp, POOL_SUPPLY),
            Ok(OP_SUPPLY)
        );
        assert_eq!(pause_code(OperationKind::PoolOp, POOL_REPAY), Ok(OP_REPAY));
        assert_eq!(
            pause_code(OperationKind::PositionPrivate, COLLATERAL_WITHDRAW),
            Ok(OP_WITHDRAW_COLLATERAL)
        );
        assert_eq!(
            pause_code(OperationKind::LiquidatePool, 0),
            Ok(OP_LIQUIDATE)
        );
        assert_eq!(
            pause_code(OperationKind::PoolOp, 4),
            Err(ErrorCode::PrivateOpInvalid)
        );
        assert_eq!(
            pause_code(OperationKind::LendOp, 0),
            Err(ErrorCode::PrivateOpInvalid)
        );
    }

    #[test]
    fn only_borrow_collateral_withdrawal_and_liquidation_read_a_price() {
        assert!(needs_price(OperationKind::PoolOp, POOL_BORROW));
        assert!(!needs_price(OperationKind::PoolOp, POOL_SUPPLY));
        assert!(needs_price(
            OperationKind::PositionPrivate,
            COLLATERAL_WITHDRAW
        ));
        assert!(!needs_price(OperationKind::PositionPrivate, COLLATERAL_ADD));
        assert!(needs_price(OperationKind::LiquidatePool, 0));
    }

    #[test]
    fn inputs_contain_no_amount_field() {
        let seal = SealInputs {
            viewing_pubkey: [1; 32],
            viewing_key_epoch: 0,
            state: EncryptedPositionPayload::default(),
            output_nonce_input: 7,
        };
        let fields = private_inputs(None, &seal, None, &PrivateParams::default());
        assert!(fields
            .iter()
            .all(|f| f.id != super::super::shield::INPUT_AMOUNT));
    }

    // ---- Lifecycle (task P.8): queue, callback and abandonment ------------

    use crate::state::nonce::output_nonce;
    use crate::state::test_fixtures::{key, market, pool, position, MARKET, OWNER, POOL, POSITION};

    const LIQ_OWNER: u8 = 0x12;
    const LIQ_POSITION: u8 = 0x24;
    const LENDING: u8 = 0x49;

    struct World {
        market: Market,
        pool: PoolState,
        position: Position,
        liquidator: Position,
        counter: OperationCounter,
    }

    fn world(kind: OperationKind) -> World {
        let mut liquidator = position();
        liquidator.owner = key(LIQ_OWNER);
        liquidator.viewing_pubkey = [0x44; 32];
        liquidator.encrypted_state.nonce = 900;
        liquidator.coordination.generation = 11;
        let payer = if kind == OperationKind::LiquidatePool {
            LIQ_OWNER
        } else {
            OWNER
        };
        World {
            market: market(),
            pool: pool(),
            position: position(),
            liquidator,
            counter: OperationCounter::new(key(payer), 255),
        }
    }

    fn keys(w: &World, kind: OperationKind, captured: bool) -> PrivateKeys {
        let g = |v: u64| if captured { Some(v) } else { None };
        let liquidation = kind == OperationKind::LiquidatePool;
        PrivateKeys {
            payer: key(if liquidation { LIQ_OWNER } else { OWNER }),
            market: key(MARKET),
            asset_state: w.market.asset_state,
            lending_pool: key(LENDING),
            pool: (kind != OperationKind::PositionPrivate)
                .then(|| (key(POOL), g(w.pool.coordination.generation))),
            position: (
                key(POSITION),
                w.position.owner,
                g(w.position.coordination.generation),
            ),
            second: liquidation.then(|| {
                (
                    key(LIQ_POSITION),
                    w.liquidator.owner,
                    g(w.liquidator.coordination.generation),
                )
            }),
        }
    }

    fn params(op: u64) -> PrivateParams {
        PrivateParams {
            op,
            supply_index_1e9: 1_000_000_000,
            borrow_index_1e9: 1_000_000_000,
            price: 100_000_000,
            ltv_bps: 5_000,
            liq_threshold_bps: 6_500,
            bonus_bps: 500,
            position_debt_cap: 1_000_000,
            supply_cap: u64::MAX,
        }
    }

    fn queue(w: &mut World, kind: OperationKind, op: u64) -> Result<OperationIntent, ErrorCode> {
        let k = keys(w, kind, true);
        let request = PrivateRequest {
            kind,
            intent_key: key(0x70),
            sequence: 0,
            computation_offset: 4_242,
            queued_slot: 100,
            params: params(op),
            publish_time: None,
        };
        let has_pool = kind != OperationKind::PositionPrivate;
        let liquidation = kind == OperationKind::LiquidatePool;
        queue_private(
            &w.market,
            &k,
            has_pool.then_some(&mut w.pool),
            &mut w.position,
            liquidation.then_some(&mut w.liquidator),
            &mut w.counter,
            &request,
        )
    }

    fn sealed(viewing: [u8; 32], position_key: u8, sequence: u64) -> SealedPosition {
        SealedPosition {
            encryption_key: viewing,
            nonce: output_nonce(&key(position_key), sequence),
            ciphertexts: [[position_key; 32]; 5],
        }
    }

    fn report(w: &World, intent: &OperationIntent, applied: bool) -> PrivateReport {
        let kind = intent.kind;
        PrivateReport {
            output_authenticated: true,
            computation_offset: intent.computation_offset,
            intent_key: intent.key,
            pool_output: (kind != OperationKind::PositionPrivate).then_some(EncryptedPoolState {
                nonce: 999,
                ciphertexts: [[9; 32]; 4],
            }),
            primary: sealed(w.position.viewing_pubkey, POSITION, intent.sequence),
            second: (kind == OperationKind::LiquidatePool)
                .then(|| sealed(w.liquidator.viewing_pubkey, LIQ_POSITION, intent.sequence)),
            applied,
            current_slot: intent.queued_slot + 10,
        }
    }

    fn settle(
        w: &mut World,
        intent: &mut OperationIntent,
        r: &PrivateReport,
    ) -> Result<Option<IntentStatus>, ErrorCode> {
        let kind = intent.kind;
        let k = keys(w, kind, false);
        let has_pool = kind != OperationKind::PositionPrivate;
        let liquidation = kind == OperationKind::LiquidatePool;
        settle_private(
            &k,
            has_pool.then_some(&mut w.pool),
            &mut w.position,
            liquidation.then_some(&mut w.liquidator),
            intent,
            kind,
            r,
        )
    }

    fn abandon(
        w: &mut World,
        intent: &mut OperationIntent,
        slot: Slot,
    ) -> Result<usize, ErrorCode> {
        let kind = intent.kind;
        let k = keys(w, kind, false);
        let has_pool = kind != OperationKind::PositionPrivate;
        let liquidation = kind == OperationKind::LiquidatePool;
        abandon_private(
            &k,
            has_pool.then_some(&mut w.pool),
            &mut w.position,
            liquidation.then_some(&mut w.liquidator),
            intent,
            slot,
        )
    }

    const KINDS: [(OperationKind, u64); 3] = [
        (OperationKind::PoolOp, POOL_BORROW),
        (OperationKind::PositionPrivate, COLLATERAL_WITHDRAW),
        (OperationKind::LiquidatePool, 0),
    ];

    #[test]
    fn queue_locks_every_protected_account_and_binds_the_params() {
        for (kind, op) in KINDS {
            let mut w = world(kind);
            let intent = queue(&mut w, kind, op).unwrap();
            assert_eq!(intent.kind, kind);
            assert_eq!(intent.position, Some(key(POSITION)));
            assert_eq!(PrivateParams::from_intent(&intent), params(op));
            assert_eq!(w.counter.next_sequence, 1);
            assert!(intent.lock_owner().holds(&w.position.coordination.lock));
            let pool_locked = intent.lock_owner().holds(&w.pool.coordination.lock);
            assert_eq!(pool_locked, kind != OperationKind::PositionPrivate);
            let second_locked = intent.lock_owner().holds(&w.liquidator.coordination.lock);
            assert_eq!(second_locked, kind == OperationKind::LiquidatePool);
            assert_eq!(
                intent.second_position,
                (kind == OperationKind::LiquidatePool).then(|| key(LIQ_POSITION))
            );
        }
    }

    #[test]
    fn queue_rejections_change_nothing() {
        // A second operation while the first holds the Locks.
        let mut w = world(OperationKind::PoolOp);
        queue(&mut w, OperationKind::PoolOp, POOL_SUPPLY).unwrap();
        let (pool, position, counter) = (w.pool.coordination, w.position.coordination, w.counter);
        assert!(queue(&mut w, OperationKind::PoolOp, POOL_SUPPLY).is_err());
        assert_eq!(
            (
                w.pool.coordination,
                w.position.coordination,
                w.counter.next_sequence
            ),
            (pool, position, counter.next_sequence)
        );

        // Not the owner.
        let mut w = world(OperationKind::PoolOp);
        w.position.owner = key(0x99);
        assert_eq!(
            queue(&mut w, OperationKind::PoolOp, POOL_SUPPLY),
            Err(ErrorCode::PositionOpNotOwner)
        );
        assert!(!w.pool.coordination.lock.is_active());

        // Unknown op, inactive Position, paused Market.
        let mut w = world(OperationKind::PoolOp);
        assert_eq!(
            queue(&mut w, OperationKind::PoolOp, 4),
            Err(ErrorCode::PrivateOpInvalid)
        );
        w.position.status = PositionStatus::Initializing;
        assert!(queue(&mut w, OperationKind::PoolOp, POOL_SUPPLY).is_err());
        let mut w = world(OperationKind::PoolOp);
        w.market.status = MarketStatus::Uninitialized;
        assert_eq!(
            queue(&mut w, OperationKind::PoolOp, POOL_SUPPLY),
            Err(ErrorCode::MarketNotActive)
        );
        assert_eq!(w.counter.next_sequence, 0);

        // A liquidator cannot target its own Position.
        let mut w = world(OperationKind::LiquidatePool);
        w.position.owner = key(LIQ_OWNER);
        assert_eq!(
            queue(&mut w, OperationKind::LiquidatePool, 0),
            Err(ErrorCode::PrivateLiquidationSelf)
        );
        assert!(!w.liquidator.coordination.lock.is_active());
    }

    #[test]
    fn applied_stores_every_payload_bumps_every_generation_and_releases_locks() {
        for (kind, op) in KINDS {
            let mut w = world(kind);
            let mut intent = queue(&mut w, kind, op).unwrap();
            let gens = (
                w.pool.coordination.generation,
                w.position.coordination.generation,
                w.liquidator.coordination.generation,
            );
            let r = report(&w, &intent, true);
            assert_eq!(
                settle(&mut w, &mut intent, &r),
                Ok(Some(IntentStatus::Applied))
            );
            assert_eq!(w.position.encrypted_state.nonce, r.primary.nonce);
            assert_eq!(
                w.position.encrypted_state.ciphertexts,
                r.primary.ciphertexts
            );
            assert_eq!(w.position.coordination.generation, gens.1 + 1);
            assert!(!w.position.coordination.lock.is_active());
            if kind != OperationKind::PositionPrivate {
                assert_eq!(w.pool.encrypted, r.pool_output.unwrap());
                assert_eq!(w.pool.coordination.generation, gens.0 + 1);
                assert!(!w.pool.coordination.lock.is_active());
            } else {
                assert_eq!(w.pool.coordination.generation, gens.0);
            }
            if kind == OperationKind::LiquidatePool {
                let second = r.second.unwrap();
                assert_eq!(w.liquidator.encrypted_state.nonce, second.nonce);
                assert_eq!(w.liquidator.coordination.generation, gens.2 + 1);
                assert!(!w.liquidator.coordination.lock.is_active());
            }
            // A late repeat of the callback is ignored.
            assert_eq!(settle(&mut w, &mut intent, &r), Ok(None));
        }
    }

    #[test]
    fn refused_stores_nothing_and_releases_locks() {
        for (kind, op) in KINDS {
            let mut w = world(kind);
            let before = (w.position.encrypted_state, w.pool.encrypted);
            let mut intent = queue(&mut w, kind, op).unwrap();
            let r = report(&w, &intent, false);
            assert_eq!(
                settle(&mut w, &mut intent, &r),
                Ok(Some(IntentStatus::Zero))
            );
            assert_eq!((w.position.encrypted_state, w.pool.encrypted), before);
            assert!(!w.position.coordination.lock.is_active());
            assert!(!w.pool.coordination.lock.is_active());
            assert!(!w.liquidator.coordination.lock.is_active());
        }
    }

    #[test]
    fn binding_mismatches_revert_and_leave_the_intent_queued() {
        for (kind, op) in KINDS {
            // Sealed to a key other than the Position's Viewing key.
            let mut w = world(kind);
            let mut intent = queue(&mut w, kind, op).unwrap();
            let mut r = report(&w, &intent, true);
            r.primary.encryption_key = [0xEE; 32];
            assert!(settle(&mut w, &mut intent, &r).is_err());
            assert_eq!(intent.status, IntentStatus::Queued);
            assert!(intent.lock_owner().holds(&w.position.coordination.lock));

            // A wrong output nonce.
            let mut r = report(&w, &intent, true);
            r.primary.nonce += 1;
            assert!(settle(&mut w, &mut intent, &r).is_err());

            // Unauthenticated output.
            let mut r = report(&w, &intent, true);
            r.output_authenticated = false;
            assert_eq!(
                settle(&mut w, &mut intent, &r),
                Err(ErrorCode::CallbackOutputNotAuthenticated)
            );

            // A missing second output for a liquidation, or an extra one.
            let mut r = report(&w, &intent, true);
            r.second = match kind {
                OperationKind::LiquidatePool => None,
                _ => Some(sealed([1; 32], LIQ_POSITION, 0)),
            };
            assert_eq!(
                settle(&mut w, &mut intent, &r),
                Err(ErrorCode::PrivateOpIntentMismatch)
            );
            assert_eq!(intent.status, IntentStatus::Queued);
        }
    }

    #[test]
    fn expired_callbacks_store_nothing() {
        for (kind, op) in KINDS {
            let mut w = world(kind);
            let before = w.position.encrypted_state;
            let mut intent = queue(&mut w, kind, op).unwrap();
            let mut r = report(&w, &intent, true);
            r.current_slot = intent.decision_expiry_slot + 1;
            assert_eq!(
                settle(&mut w, &mut intent, &r),
                Ok(Some(IntentStatus::Expired))
            );
            assert_eq!(w.position.encrypted_state, before);
            assert!(!w.position.coordination.lock.is_active());
        }
    }

    #[test]
    fn abandonment_waits_for_lock_expiry_then_bumps_generations() {
        for (kind, op) in KINDS {
            let mut w = world(kind);
            let mut intent = queue(&mut w, kind, op).unwrap();
            let gen = w.position.coordination.generation;
            let expiry = intent.lock_expiry_slot;
            assert!(abandon(&mut w, &mut intent, expiry).is_err());
            assert_eq!(intent.status, IntentStatus::Queued);
            let cleared = abandon(&mut w, &mut intent, expiry + 1).unwrap();
            let expected = match kind {
                OperationKind::PositionPrivate => 1,
                OperationKind::PoolOp => 2,
                _ => 3,
            };
            assert_eq!(cleared, expected);
            assert_eq!(intent.status, IntentStatus::Abandoned);
            assert_eq!(w.position.coordination.generation, gen + 1);
            assert!(!w.position.coordination.lock.is_active());
            assert!(!w.pool.coordination.lock.is_active());
            assert!(!w.liquidator.coordination.lock.is_active());
            // A late callback after abandonment is ignored.
            let r = report(&w, &intent, true);
            assert_eq!(settle(&mut w, &mut intent, &r), Ok(None));
        }
    }

    #[test]
    fn accrual_window_is_enforced() {
        let mut lp = crate::state::lending_pool::LendingPool::new(
            Pubkey::default(),
            1,
            Pubkey::new_unique(),
            [0; 32],
            60,
            1_000,
        );
        let a = crate::state::test_fixtures::asset();
        let bad = Err(ErrorCode::PrivateAccrualTimeInvalid);
        assert_eq!(accrue_to(&mut lp, &a, 2_001, 2_000), bad, "future");
        assert_eq!(accrue_to(&mut lp, &a, 1_879, 2_000), bad, "too old");
        assert_eq!(
            accrue_to(&mut lp, &a, 999, 1_050),
            bad,
            "before last accrual"
        );
        assert_eq!(accrue_to(&mut lp, &a, 1_900, 2_000), Ok(()));
        assert_eq!(lp.last_accrual_ts, 1_900);
    }

    #[test]
    fn band_mode_borrow_needs_a_fresh_band() {
        let mut lp = crate::state::lending_pool::LendingPool::new(
            Pubkey::default(),
            1,
            Pubkey::new_unique(),
            [0; 32],
            60,
            0,
        );
        let a = crate::state::test_fixtures::asset(); // band observed at slot 90
        let age = a.rates.band_max_age_slots;
        let late = 90 + age + 1;
        // Totals mode never checks the band.
        assert!(ensure_rate_fresh(&lp, &a, OperationKind::PoolOp, POOL_BORROW, late).is_ok());
        lp.rate_source = crate::state::lending_pool::RATE_SOURCE_BAND;
        assert!(ensure_rate_fresh(&lp, &a, OperationKind::PoolOp, POOL_BORROW, late).is_err());
        assert!(ensure_rate_fresh(&lp, &a, OperationKind::PoolOp, POOL_BORROW, 95).is_ok());
        // Repay and withdrawals never wait on the band.
        assert!(ensure_rate_fresh(&lp, &a, OperationKind::PoolOp, POOL_REPAY, late).is_ok());
        assert!(ensure_rate_fresh(
            &lp,
            &a,
            OperationKind::PositionPrivate,
            COLLATERAL_WITHDRAW,
            late
        )
        .is_ok());
    }
}
