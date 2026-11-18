//! walnut-lend `lend_op` (ops 0 to 7): queue, callback and abandon rules.
//!
//! One circuit (`lend_op`) mutates the encrypted Position for add or withdraw
//! collateral (0, 1), borrow (2), repay (3), supply (4), withdraw supply (5),
//! health check (6) and liquidate (7). Shaped like `state::position_op`;
//! `math::lend::lend_op_rules` is the plain-Rust mirror of the circuit.
//!
//! # Public work done here (the circuit never divides)
//!
//! * pause category per op ([`lend_pause_category`]): supply, borrow, add
//!   collateral and liquidation can be paused; repay, withdraw supply,
//!   withdraw collateral and health checks never are;
//! * caps ([`check_lend_caps`]) and liquidity ([`plan_owner_op`]);
//! * share maths (`math::lend`) and interest accrual (`LendingPool::accrue`);
//! * the oracle price ([`pool_price`]): low bound, pool-configured max age.
//!
//! # Intent binding
//!
//! `accounts_commitment` covers the payer, Market, Asset State, LendingPool,
//! Position (captured generation) and, for a liquidation, the vaults, vault
//! authority, token program and the Liquidator's token accounts.
//! `inputs_commitment` covers the Viewing key, epoch, stored nonce, `op`,
//! `amount`, `shares`, the five stored ciphertexts, the output-nonce input and
//! the public circuit arguments (index, price, LTV, liquidation threshold,
//! position debt cap). The circuit echoes `op`, `amount` and `shares`; the
//! other arguments are stored in otherwise unused Intent fields so the
//! callback can rebuild the commitment:
//!
//! | Intent field | Holds |
//! |---|---|
//! | `asset_generation` | `op` |
//! | `oracle_generation` | `ltv_bps \| liq_threshold_bps << 32` |
//! | `rate_epoch_generation` | liquidation repay amount (USDC), else 0 |
//! | `accounting_borrow_index` | borrow index (1e9 scale) |
//! | `risk_borrow_index_at_expiry` | `price \| position_debt_cap << 64` |
//!
//! These fields are only compared with themselves by `CurrentDependencies::
//! captured_by`, so reusing them cannot cause a stale result.
//!
//! # Token movement
//!
//! Ops 0 to 6 move no tokens: balances are encrypted inside the Position, the
//! vaults only move on shield and unshield. A liquidation takes the repay USDC
//! from the Liquidator at queue time and, in the callback, either sends the
//! seized wSOL to the Liquidator (applied) or refunds the USDC (anything else).

use anchor_lang::prelude::Pubkey;

use super::accounts::{EncryptedPositionPayload, Position, PositionStatus};
use super::coordination::{acquire_locks, CoordinationSlot};
use super::lending_pool::LendingPool;
use super::nonce::{derive_output_nonce_input, INPUT_OUTPUT_NONCE};
use super::position::{INPUT_NONCE, INPUT_VIEWING_KEY_EPOCH, INPUT_VIEWING_PUBKEY};
use super::shield::{INPUT_AMOUNT, INPUT_ASSET, INPUT_STATE_CIPHERTEXT_FIRST};
use super::Slot;
use crate::errors::ErrorCode;
use crate::market::caps::{check_supply_cap, check_total_debt_cap};
use crate::market::{ensure_not_paused, Caps, Market, MarketStatus, PauseCategory, PauseState};
use crate::market::{AssetState, CapCheck};
use crate::math::lend::{
    borrow_mint, clamp_price, liquidation_burn, liquidation_seize, repay_burn, supply_shares_for,
    withdraw_supply_burn, MAX_COLLATERAL_LAMPORTS, OP_ADD_COLLATERAL, OP_BORROW, OP_HEALTH_CHECK,
    OP_LIQUIDATE, OP_REPAY, OP_SUPPLY, OP_WITHDRAW_SUPPLY,
};
use crate::operation::callback::{
    apply_decision_to_slots, classify_callback, CallbackContext, CallbackDecision, CallbackReport,
    CurrentDependencies,
};
use crate::operation::commitment::{
    account_commitment, input_commitment, AccountBinding, AccountRole, InputField, InputValue,
};
use crate::operation::seeds::reserve_intent;
use crate::operation::{IntentStatus, OperationCounter, OperationIntent, OperationKind};
use crate::oracle::{validate_price_with_max_age, PriceUse, PythPriceView, DEFAULT_DEVIATION_BPS};

/// Decision Expiry distance, in slots.
pub const LEND_OP_DECISION_SLOTS: u64 = 1_500;
/// Slots between Decision Expiry and Lock expiry.
pub const LEND_OP_LOCK_GRACE_SLOTS: u64 = 300;
/// Largest per-position debt cap handed to the circuit (USDC base units).
pub const MAX_POSITION_DEBT_CAP: u64 = 1_000_000_000_000;

/// Input id: public share amount.
pub const INPUT_SHARES: u8 = 20;
/// Input id: borrow index (1e9 scale).
pub const INPUT_BORROW_INDEX: u8 = 21;
/// Input id: price (USDC base units per SOL).
pub const INPUT_PRICE: u8 = 22;
/// Input id: loan-to-value (bps).
pub const INPUT_LTV: u8 = 23;
/// Input id: liquidation threshold (bps).
pub const INPUT_LIQ_THRESHOLD: u8 = 24;
/// Input id: position debt cap.
pub const INPUT_DEBT_CAP: u8 = 25;

/// The SPL Token program account is owned by this loader.
const BPF_LOADER_PROGRAM: Pubkey =
    Pubkey::from_str_const("BPFLoader2111111111111111111111111111111111");

/// The public arguments of one `lend_op` computation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LendParams {
    /// Operation tag (0 to 7).
    pub op: u64,
    /// Public amount (op 7: seize lamports).
    pub amount: u64,
    /// Public share amount (op 7: debt shares burned).
    pub shares: u64,
    /// Borrow index, 1e9 scale.
    pub borrow_index_1e9: u64,
    /// USDC base units per 1 SOL (0 for ops that do not read a price).
    pub price: u64,
    /// Loan-to-value (bps).
    pub ltv_bps: u64,
    /// Liquidation threshold (bps).
    pub liq_threshold_bps: u64,
    /// Per-position debt cap (USDC base units).
    pub position_debt_cap: u64,
}

impl LendParams {
    /// Writes the callback-only arguments into the unused Intent fields.
    pub fn write_to_intent(&self, intent: &mut OperationIntent, repay_amount: u64) {
        intent.asset_generation = self.op;
        intent.oracle_generation = self.ltv_bps | (self.liq_threshold_bps << 32);
        intent.rate_epoch_generation = repay_amount;
        intent.accounting_borrow_index = u128::from(self.borrow_index_1e9);
        intent.risk_borrow_index_at_expiry =
            u128::from(self.price) | (u128::from(self.position_debt_cap) << 64);
    }

    /// Rebuilds the parameters from a stored Intent and the circuit echoes.
    pub fn from_intent(
        intent: &OperationIntent,
        op: u64,
        amount: u64,
        shares: u64,
    ) -> Result<Self, ErrorCode> {
        Ok(Self {
            op,
            amount,
            shares,
            borrow_index_1e9: u64::try_from(intent.accounting_borrow_index)
                .map_err(|_| ErrorCode::LendOpInvalid)?,
            price: (intent.risk_borrow_index_at_expiry & u128::from(u64::MAX)) as u64,
            ltv_bps: intent.oracle_generation & 0xFFFF_FFFF,
            liq_threshold_bps: intent.oracle_generation >> 32,
            position_debt_cap: (intent.risk_borrow_index_at_expiry >> 64) as u64,
        })
    }
}

/// The accounts a liquidation binds besides the common ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiquidationKeys {
    /// USDC vault.
    pub usdc_vault: Pubkey,
    /// wSOL vault.
    pub wsol_vault: Pubkey,
    /// Vault-authority PDA that signs vault transfers.
    pub vault_authority: Pubkey,
    /// SPL Token program.
    pub token_program: Pubkey,
    /// The Liquidator's USDC account (repay source, refund destination).
    pub liquidator_usdc: Pubkey,
    /// The Liquidator's wSOL account (seize destination).
    pub liquidator_wsol: Pubkey,
}

impl LiquidationKeys {
    /// Requires the vaults, vault authority and token program to be the
    /// Market's.
    pub fn check_market(&self, market: &Market) -> Result<(), ErrorCode> {
        if self.usdc_vault != market.usdc_vault
            || self.wsol_vault != market.wsol_vault
            || self.vault_authority != market.vault_authority
            || self.token_program != anchor_spl::token::ID
        {
            return Err(ErrorCode::LiquidationAccountsInvalid);
        }
        Ok(())
    }
}

/// The accounts a `lend_op` Intent binds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LendKeys {
    /// The signer that pays (owner, Liquidator or health-check caller).
    pub payer: Pubkey,
    /// The Market.
    pub market: Pubkey,
    /// The Asset State.
    pub asset_state: Pubkey,
    /// The LendingPool.
    pub lending_pool: Pubkey,
    /// The target Position.
    pub position: Pubkey,
    /// The target Position's owner.
    pub position_owner: Pubkey,
    /// Captured generation of the Position.
    pub position_generation: Option<u64>,
    /// Liquidation accounts (op 7 only).
    pub liquidation: Option<LiquidationKeys>,
}

/// The ordered accounts bound by a `lend_op` Intent.
pub fn lend_bindings(keys: &LendKeys) -> Vec<AccountBinding> {
    let program = crate::ID;
    let token_program = anchor_spl::token::ID;
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
        plain(
            keys.position,
            AccountRole::Position,
            program,
            true,
            Some(keys.position_owner),
            keys.position_generation,
        ),
    ];
    if let Some(l) = keys.liquidation {
        accounts.push(plain(
            l.usdc_vault,
            AccountRole::Vault,
            token_program,
            true,
            Some(l.vault_authority),
            None,
        ));
        accounts.push(plain(
            l.wsol_vault,
            AccountRole::Vault,
            token_program,
            true,
            Some(l.vault_authority),
            None,
        ));
        accounts.push(plain(
            l.vault_authority,
            AccountRole::Other,
            program,
            false,
            None,
            None,
        ));
        accounts.push(plain(
            l.liquidator_usdc,
            AccountRole::TokenAccount,
            token_program,
            true,
            Some(keys.payer),
            None,
        ));
        accounts.push(plain(
            l.liquidator_wsol,
            AccountRole::TokenAccount,
            token_program,
            true,
            Some(keys.payer),
            None,
        ));
        accounts.push(plain(
            l.token_program,
            AccountRole::Other,
            BPF_LOADER_PROGRAM,
            false,
            None,
            None,
        ));
    }
    accounts
}

/// The ordered inputs bound by a `lend_op` Intent.
pub fn lend_inputs(
    viewing_pubkey: [u8; 32],
    viewing_key_epoch: u64,
    state: &EncryptedPositionPayload,
    params: &LendParams,
    output_nonce_input: u128,
) -> [InputField<'static>; 17] {
    let c = &state.ciphertexts;
    let first = INPUT_STATE_CIPHERTEXT_FIRST;
    [
        InputField::new(INPUT_VIEWING_PUBKEY, InputValue::Bytes32(viewing_pubkey)),
        InputField::new(INPUT_VIEWING_KEY_EPOCH, InputValue::U64(viewing_key_epoch)),
        InputField::new(INPUT_NONCE, InputValue::U128(state.nonce)),
        InputField::new(INPUT_ASSET, InputValue::U64(params.op)),
        InputField::new(INPUT_AMOUNT, InputValue::U64(params.amount)),
        InputField::new(first, InputValue::Bytes32(c[0])),
        InputField::new(first + 1, InputValue::Bytes32(c[1])),
        InputField::new(first + 2, InputValue::Bytes32(c[2])),
        InputField::new(first + 3, InputValue::Bytes32(c[3])),
        InputField::new(first + 4, InputValue::Bytes32(c[4])),
        InputField::new(INPUT_OUTPUT_NONCE, InputValue::U128(output_nonce_input)),
        InputField::new(INPUT_SHARES, InputValue::U64(params.shares)),
        InputField::new(INPUT_BORROW_INDEX, InputValue::U64(params.borrow_index_1e9)),
        InputField::new(INPUT_PRICE, InputValue::U64(params.price)),
        InputField::new(INPUT_LTV, InputValue::U64(params.ltv_bps)),
        InputField::new(
            INPUT_LIQ_THRESHOLD,
            InputValue::U64(params.liq_threshold_bps),
        ),
        InputField::new(INPUT_DEBT_CAP, InputValue::U64(params.position_debt_cap)),
    ]
}

// ---- Public queue-time rules ---------------------------------------------

/// The pause category that blocks `op`, or `None` when it is never paused.
/// Add collateral has no pause bit of its own (the rescue bit never blocks),
/// so it follows the Shield category, which also adds value at risk.
pub fn lend_pause_category(op: u64) -> Option<PauseCategory> {
    match op {
        OP_SUPPLY => Some(PauseCategory::Supply),
        OP_BORROW => Some(PauseCategory::Borrow),
        OP_ADD_COLLATERAL => Some(PauseCategory::Shield),
        OP_LIQUIDATE => Some(PauseCategory::LiquidationComplete),
        _ => None,
    }
}

/// Fails with `ActionPaused` when `op` is blocked at `slot`.
pub fn ensure_lend_op_allowed(pauses: &PauseState, op: u64, slot: Slot) -> Result<(), ErrorCode> {
    match lend_pause_category(op) {
        Some(category) => Ok(ensure_not_paused(pauses, category, slot)?),
        None => Ok(()),
    }
}

/// Supply cap, total-debt cap and per-asset collateral cap of `op`.
pub fn check_lend_caps(
    op: u64,
    amount: u64,
    pool: &LendingPool,
    caps: &Caps,
) -> Result<(), ErrorCode> {
    match op {
        OP_SUPPLY => {
            let after = pool
                .total_supply_value()?
                .checked_add(u128::from(amount))
                .ok_or(ErrorCode::LendPoolTotalsOverflow)?;
            if check_supply_cap(caps, after) == CapCheck::Exceeded {
                return Err(ErrorCode::LendSupplyCapExceeded);
            }
        }
        OP_BORROW => {
            let after = pool
                .total_debt_value()?
                .checked_add(u128::from(amount))
                .ok_or(ErrorCode::LendPoolTotalsOverflow)?;
            if check_total_debt_cap(caps, after) == CapCheck::Exceeded {
                return Err(ErrorCode::LendDebtCapExceeded);
            }
        }
        OP_ADD_COLLATERAL if u128::from(amount) > MAX_COLLATERAL_LAMPORTS => {
            return Err(ErrorCode::LendAssetCapExceeded);
        }
        _ => {}
    }
    Ok(())
}

/// Validates the price account against the pool and returns the conservative
/// price (USDC base units per SOL, clamped) and its publish time. The low bound
/// (`price - conf`) is used; the pool's `max_price_age_seconds` replaces the
/// governed age.
pub fn pool_price(
    pool: &LendingPool,
    max_confidence_bps: u16,
    view: &PythPriceView,
    now_ts: i64,
) -> Result<(u64, i64), ErrorCode> {
    if view.account != pool.oracle {
        return Err(ErrorCode::OracleAccountMismatch);
    }
    let valid = validate_price_with_max_age(
        view,
        &pool.feed_id,
        pool.max_price_age_seconds,
        max_confidence_bps,
        DEFAULT_DEVIATION_BPS,
        now_ts,
        PriceUse::Collateral,
    )?;
    // The normalized price has 8 decimals; USDC base units have 6.
    let price = clamp_price(valid.normalized / 100)?;
    Ok((price, valid.publish_time))
}

/// Pause, accrual, caps, shares and liquidity of an owner op (0 to 5).
/// Returns the public share amount. Mutates `pool` (accrual) only after the
/// pause check; every later error leaves the accrual in the in-memory pool,
/// which the instruction discards by returning the error.
#[allow(clippy::too_many_arguments)]
pub fn plan_owner_op(
    pool: &mut LendingPool,
    asset: &AssetState,
    op: u64,
    amount: u64,
    now_ts: i64,
    slot: Slot,
    vault_usdc_balance: u64,
) -> Result<u64, ErrorCode> {
    if op > OP_WITHDRAW_SUPPLY {
        return Err(ErrorCode::LendOpWrongEntry);
    }
    if amount == 0 {
        return Err(ErrorCode::LendOpZeroAmount);
    }
    ensure_lend_op_allowed(&asset.pauses, op, slot)?;
    pool.accrue_auto(now_ts, asset)?;
    check_lend_caps(op, amount, pool, &asset.caps)?;
    let shares = match op {
        OP_BORROW => borrow_mint(amount, pool.borrow_index_1e9)?,
        OP_REPAY => repay_burn(amount, pool.borrow_index_1e9)?,
        OP_SUPPLY => supply_shares_for(amount, pool.supply_index_1e9)?,
        OP_WITHDRAW_SUPPLY => withdraw_supply_burn(amount, pool.supply_index_1e9)?,
        _ => 0,
    };
    if matches!(op, OP_REPAY | OP_SUPPLY | OP_BORROW | OP_WITHDRAW_SUPPLY) && shares == 0 {
        return Err(ErrorCode::LendOpZeroShares);
    }
    if matches!(op, OP_BORROW | OP_WITHDRAW_SUPPLY)
        && u128::from(amount) > u128::from(vault_usdc_balance.saturating_sub(pool.reserves_usdc))
    {
        return Err(ErrorCode::LendInsufficientLiquidity);
    }
    if op == OP_WITHDRAW_SUPPLY && u128::from(shares) > pool.total_supply_shares {
        return Err(ErrorCode::LendPoolTotalsUnderflow);
    }
    Ok(shares)
}

/// Pause and accrual of a liquidation. Returns `(debt shares burned, seized
/// lamports)`.
pub fn plan_liquidation(
    pool: &mut LendingPool,
    asset: &AssetState,
    repay_amount: u64,
    price: u64,
    now_ts: i64,
    slot: Slot,
    wsol_vault_balance: u64,
) -> Result<(u64, u64), ErrorCode> {
    if repay_amount == 0 {
        return Err(ErrorCode::LendOpZeroAmount);
    }
    ensure_lend_op_allowed(&asset.pauses, OP_LIQUIDATE, slot)?;
    pool.accrue_auto(now_ts, asset)?;
    let burn = liquidation_burn(repay_amount, pool.borrow_index_1e9)?;
    let seize = liquidation_seize(repay_amount, asset.risk.liquidation_bonus_bps, price)?;
    if burn == 0
        || seize == 0
        || u128::from(burn) > pool.total_debt_shares
        || seize > wsol_vault_balance
    {
        return Err(ErrorCode::LiquidationAmountInvalid);
    }
    Ok((burn, seize))
}

/// Applies an applied op to the public pool totals.
pub fn apply_pool_totals(pool: &mut LendingPool, op: u64, shares: u64) -> Result<(), ErrorCode> {
    let shares = u128::from(shares);
    let add = |total: u128| {
        total
            .checked_add(shares)
            .ok_or(ErrorCode::LendPoolTotalsOverflow)
    };
    let sub = |total: u128| {
        total
            .checked_sub(shares)
            .ok_or(ErrorCode::LendPoolTotalsUnderflow)
    };
    match op {
        OP_BORROW => pool.total_debt_shares = add(pool.total_debt_shares)?,
        OP_REPAY | OP_LIQUIDATE => pool.total_debt_shares = sub(pool.total_debt_shares)?,
        OP_SUPPLY => pool.total_supply_shares = add(pool.total_supply_shares)?,
        OP_WITHDRAW_SUPPLY => pool.total_supply_shares = sub(pool.total_supply_shares)?,
        _ => {}
    }
    Ok(())
}

// ---- Queue ---------------------------------------------------------------

/// One `lend_op` queue request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LendRequest {
    /// The Operation Intent PDA address.
    pub intent_key: Pubkey,
    /// The signer's next Intent sequence.
    pub sequence: u64,
    /// The Arcium computation offset.
    pub computation_offset: u64,
    /// The current slot.
    pub queued_slot: Slot,
    /// The circuit arguments.
    pub params: LendParams,
    /// Liquidation repay amount (USDC), else 0.
    pub repay_amount: u64,
    /// Oracle publish time of the price used, if any.
    pub publish_time: Option<i64>,
    /// The pool accrual timestamp the index belongs to.
    pub accrual_ts: i64,
}

/// Builds the Intent of `request` against `position`.
pub fn build_lend_intent(
    signer: &Pubkey,
    keys: &LendKeys,
    position: &Position,
    request: &LendRequest,
) -> Result<OperationIntent, ErrorCode> {
    let decision_expiry_slot = request
        .queued_slot
        .checked_add(LEND_OP_DECISION_SLOTS)
        .ok_or(ErrorCode::InvalidIntentTiming)?;
    let lock_expiry_slot = decision_expiry_slot
        .checked_add(LEND_OP_LOCK_GRACE_SLOTS)
        .ok_or(ErrorCode::InvalidIntentTiming)?;
    let bindings = lend_bindings(keys);
    let inputs = lend_inputs(
        position.viewing_pubkey,
        position.viewing_key_epoch,
        &position.encrypted_state,
        &request.params,
        derive_output_nonce_input(&keys.position, request.sequence),
    );
    let mut intent = OperationIntent {
        key: request.intent_key,
        owner: *signer,
        payer: *signer,
        reclaim_authority: *signer,
        kind: OperationKind::LendOp,
        status: IntentStatus::Queued,
        sequence: request.sequence,
        computation_offset: request.computation_offset,
        accounts_commitment: account_commitment(&bindings),
        inputs_commitment: input_commitment(OperationKind::LendOp, &inputs),
        queued_slot: request.queued_slot,
        decision_expiry_slot,
        lock_expiry_slot,
        position: Some(keys.position),
        position_generation: keys.position_generation,
        oracle_publish_time_sol: request.publish_time,
        accounting_index_timestamp: request.accrual_ts,
        ..OperationIntent::default()
    };
    request
        .params
        .write_to_intent(&mut intent, request.repay_amount);
    Ok(intent)
}

/// Checks the Market and the Position, builds and reserves the Intent, and
/// Locks the Position (not for a health check). Owner ops (0 to 5) need the
/// signer to be the Position owner; health checks and liquidations are
/// permissionless. Atomic: on error the Position and the counter are
/// unchanged.
#[allow(clippy::too_many_arguments)]
pub fn queue_lend_op(
    market: &Market,
    market_key: Pubkey,
    lending_pool_key: Pubkey,
    position: &mut Position,
    position_key: Pubkey,
    counter: &mut OperationCounter,
    signer: &Pubkey,
    liquidation: Option<LiquidationKeys>,
    request: &LendRequest,
) -> Result<OperationIntent, ErrorCode> {
    let op = request.params.op;
    if market.status != MarketStatus::Active {
        return Err(ErrorCode::MarketNotActive);
    }
    if op > OP_LIQUIDATE {
        return Err(ErrorCode::LendOpInvalid);
    }
    if op <= OP_WITHDRAW_SUPPLY && *signer != position.owner {
        return Err(ErrorCode::PositionOpNotOwner);
    }
    if position.status != PositionStatus::Active {
        return Err(ErrorCode::PositionOpPositionNotActive);
    }
    if (op == OP_LIQUIDATE) != liquidation.is_some() {
        return Err(ErrorCode::LiquidationAccountsInvalid);
    }
    if let Some(l) = &liquidation {
        l.check_market(market)?;
    }
    let keys = LendKeys {
        payer: *signer,
        market: market_key,
        asset_state: market.asset_state,
        lending_pool: lending_pool_key,
        position: position_key,
        position_owner: position.owner,
        position_generation: Some(position.coordination.generation),
        liquidation,
    };
    queue_with_keys(position, counter, signer, &keys, request)
}

/// [`queue_lend_op`] with the pool key supplied by the caller (the Anchor
/// handler passes the loaded account's key, which tests also use).
pub fn queue_with_keys(
    position: &mut Position,
    counter: &mut OperationCounter,
    signer: &Pubkey,
    keys: &LendKeys,
    request: &LendRequest,
) -> Result<OperationIntent, ErrorCode> {
    let intent = build_lend_intent(signer, keys, position, request)?;
    let mut next_counter = *counter;
    reserve_intent(&mut next_counter, &intent)?;
    if request.params.op != OP_HEALTH_CHECK {
        acquire_locks(
            intent.lock_owner(),
            intent.lock_expiry_slot,
            &mut [CoordinationSlot::new(
                keys.position,
                &mut position.coordination,
            )],
        )?;
    }
    *counter = next_counter;
    Ok(intent)
}

// ---- Callback ------------------------------------------------------------

/// What the `lend_op` callback observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LendReport {
    /// Whether Arcium `verify_output` succeeded.
    pub output_authenticated: bool,
    /// The computation offset the callback's computation account belongs to.
    pub computation_offset: u64,
    /// The Operation Intent account the callback received.
    pub intent_key: Pubkey,
    /// The output's encryption key.
    pub encryption_key: [u8; 32],
    /// The output nonce (output-nonce input + 1).
    pub output_nonce: u128,
    /// The output ciphertexts in `PositionSecret` order.
    pub ciphertexts: [[u8; 32]; 5],
    /// The revealed applied flag (op 6: unhealthy).
    pub applied: bool,
    /// The revealed op echo.
    pub op: u64,
    /// The revealed amount echo.
    pub amount: u64,
    /// The revealed shares echo.
    pub shares: u64,
    /// The current slot.
    pub current_slot: Slot,
}

/// What the callback did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LendSettlement {
    /// The new state is stored (ops 0 to 5 and 7).
    Applied {
        /// The op.
        op: u64,
        /// The amount echo.
        amount: u64,
        /// The shares echo.
        shares: u64,
    },
    /// The circuit refused the op: nothing stored, Lock released.
    Rejected {
        /// The op.
        op: u64,
    },
    /// Health check result; the Position is unchanged.
    Health {
        /// Whether the Position is past its liquidation threshold.
        unhealthy: bool,
    },
    /// The attempt ended with this status; nothing stored.
    Failed(IntentStatus),
    /// The Intent was already terminal.
    Ignored,
}

/// Verifies and applies a `lend_op` callback. Errors revert (the Intent stays
/// `Queued`). Atomic on error.
#[allow(clippy::too_many_arguments)]
pub fn settle_lend_op(
    market: &Market,
    market_key: Pubkey,
    lending_pool_key: Pubkey,
    position: &mut Position,
    position_key: Pubkey,
    intent: &mut OperationIntent,
    liquidation: Option<LiquidationKeys>,
    report: &LendReport,
) -> Result<LendSettlement, ErrorCode> {
    let keys = LendKeys {
        payer: intent.payer,
        market: market_key,
        asset_state: market.asset_state,
        lending_pool: lending_pool_key,
        position: position_key,
        position_owner: position.owner,
        position_generation: intent.position_generation,
        liquidation,
    };
    let params = LendParams::from_intent(intent, report.op, report.amount, report.shares)?;
    let bindings = lend_bindings(&keys);
    let inputs = lend_inputs(
        report.encryption_key,
        position.viewing_key_epoch,
        &position.encrypted_state,
        &params,
        report.output_nonce.wrapping_sub(1),
    );
    let context = CallbackContext {
        output_authenticated: report.output_authenticated,
        computation_offset: report.computation_offset,
        intent_key: report.intent_key,
        kind: OperationKind::LendOp,
        accounts: &bindings,
        input_fields: &inputs,
        current: CurrentDependencies {
            position_generation: Some(position.coordination.generation),
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
        CallbackDecision::Ignore => return Ok(LendSettlement::Ignored),
        _ => {}
    }
    if intent.asset_generation != report.op {
        return Err(ErrorCode::PositionOpIntentMismatch);
    }
    if position.status != PositionStatus::Active {
        return Err(ErrorCode::PositionOpPositionNotActive);
    }
    let locking = report.op != OP_HEALTH_CHECK;
    if locking && !intent.lock_owner().holds(&position.coordination.lock) {
        return Err(ErrorCode::PositionOpIntentMismatch);
    }
    let mut next_intent = *intent;
    let mut coordination = position.coordination;
    let result = if locking {
        apply_decision_to_slots(
            &mut next_intent,
            &mut [CoordinationSlot::new(position_key, &mut coordination)],
            decision,
            report.current_slot,
        )?
    } else {
        apply_decision_to_slots(&mut next_intent, &mut [], decision, report.current_slot)?
    };
    let settlement = match (result.status, locking) {
        (IntentStatus::Applied, true) => LendSettlement::Applied {
            op: report.op,
            amount: report.amount,
            shares: report.shares,
        },
        (IntentStatus::Zero, true) => LendSettlement::Rejected { op: report.op },
        (IntentStatus::Applied, false) => LendSettlement::Health { unhealthy: true },
        (IntentStatus::Zero, false) => LendSettlement::Health { unhealthy: false },
        (status, _) => LendSettlement::Failed(status),
    };
    *intent = next_intent;
    if locking {
        position.coordination = coordination;
    }
    if matches!(settlement, LendSettlement::Applied { .. }) {
        position.encrypted_state = EncryptedPositionPayload {
            nonce: report.output_nonce,
            ciphertexts: report.ciphertexts,
        };
    }
    Ok(settlement)
}

/// `abandon_lend_op`: permissionless timeout recovery after Lock expiry. A
/// health check holds no Lock, so only the Intent is closed.
pub fn abandon_lend_op(
    position: &mut Position,
    position_key: Pubkey,
    intent: &mut OperationIntent,
    current_slot: Slot,
) -> Result<usize, ErrorCode> {
    if intent.kind != OperationKind::LendOp || intent.position != Some(position_key) {
        return Err(ErrorCode::PositionOpIntentMismatch);
    }
    if intent.status.is_terminal() {
        return Err(ErrorCode::IntentNotQueued);
    }
    if intent.asset_generation == OP_HEALTH_CHECK {
        return Ok(crate::operation::abandon_intent(
            intent,
            current_slot,
            &mut [],
        )?);
    }
    if !intent.lock_owner().holds(&position.coordination.lock) {
        return Err(ErrorCode::PositionOpIntentMismatch);
    }
    Ok(crate::operation::abandon_intent(
        intent,
        current_slot,
        &mut [CoordinationSlot::new(
            position_key,
            &mut position.coordination,
        )],
    )?)
}

/// Whether an Intent is a liquidation (its refund path moves USDC).
pub fn is_liquidation(intent: &OperationIntent) -> bool {
    intent.kind == OperationKind::LendOp && intent.asset_generation == OP_LIQUIDATE
}

/// The USDC a queued liquidation took from the Liquidator.
pub fn liquidation_repay_amount(intent: &OperationIntent) -> u64 {
    intent.rate_epoch_generation
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::pause_flags;
    use crate::state::test_fixtures::{asset, key, market, position, MARKET};

    const POOL_KEY: u8 = 0x70;

    fn pool() -> LendingPool {
        let mut p = LendingPool::new(key(MARKET), 1, key(0x71), [9; 32], 86_400, 0);
        p.total_supply_shares = 10_000_000_000;
        p.total_debt_shares = 1_000_000_000;
        p
    }

    fn params(op: u64, amount: u64, shares: u64) -> LendParams {
        LendParams {
            op,
            amount,
            shares,
            borrow_index_1e9: 1_000_000_000,
            price: 150_000_000,
            ltv_bps: 7_500,
            liq_threshold_bps: 8_000,
            position_debt_cap: 1_000_000_000_000,
        }
    }

    fn keys(position_generation: u64, liquidation: Option<LiquidationKeys>) -> LendKeys {
        LendKeys {
            payer: key(0x11),
            market: key(MARKET),
            asset_state: market().asset_state,
            lending_pool: key(POOL_KEY),
            position: key(0x23),
            position_owner: key(0x11),
            position_generation: Some(position_generation),
            liquidation,
        }
    }

    fn request(p: LendParams, sequence: u64) -> LendRequest {
        LendRequest {
            intent_key: key(0x80),
            sequence,
            computation_offset: 77,
            queued_slot: 100,
            params: p,
            repay_amount: 0,
            publish_time: Some(1_000),
            accrual_ts: 5,
        }
    }

    fn report_for(
        pos: &Position,
        intent: &OperationIntent,
        p: &LendParams,
        applied: bool,
    ) -> LendReport {
        LendReport {
            output_authenticated: true,
            computation_offset: intent.computation_offset,
            intent_key: intent.key,
            encryption_key: pos.viewing_pubkey,
            output_nonce: derive_output_nonce_input(&key(0x23), intent.sequence) + 1,
            ciphertexts: [[0x61; 32]; 5],
            applied,
            op: p.op,
            amount: p.amount,
            shares: p.shares,
            current_slot: 120,
        }
    }

    /// Feature: walnut-lend, Property 7: Pause blocks exactly the risk-increasing ops.
    /// **Validates: Requirements 2.1, 2.2**
    #[test]
    fn pause_matrix_blocks_exactly_supply_borrow_add_collateral_liquidation() {
        let all_paused = PauseState {
            flags: pause_flags::PAUSABLE,
            guardian_flags: 0,
            guardian_expiry_slot: 0,
        };
        for op in 0..=OP_LIQUIDATE {
            let blocked = ensure_lend_op_allowed(&all_paused, op, 10).is_err();
            let expected = matches!(op, OP_SUPPLY | OP_BORROW | OP_ADD_COLLATERAL | OP_LIQUIDATE);
            assert_eq!(blocked, expected, "op {op}");
            assert!(ensure_lend_op_allowed(&PauseState::default(), op, 10).is_ok());
        }
        // Pausing one category blocks only its own op.
        let only_borrow = PauseState {
            flags: pause_flags::BORROW,
            ..PauseState::default()
        };
        assert!(ensure_lend_op_allowed(&only_borrow, OP_BORROW, 10).is_err());
        assert!(ensure_lend_op_allowed(&only_borrow, OP_SUPPLY, 10).is_ok());
        assert!(ensure_lend_op_allowed(&only_borrow, OP_REPAY, 10).is_ok());
    }

    /// Feature: walnut-lend, Property 8: Caps bound public totals.
    /// **Validates: Requirements 2.3, 2.4, 2.5**
    #[test]
    fn caps_bound_supply_borrow_and_collateral() {
        let p = pool(); // supply value 10_000e6, debt value 1_000e6 at index 1.0
        let caps = Caps {
            supply_cap_usdc: 10_000_000_000 + 5,
            total_debt_cap_usdc: 1_000_000_000 + 7,
            position_debt_cap_usdc: 0,
        };
        assert!(check_lend_caps(OP_SUPPLY, 5, &p, &caps).is_ok());
        assert_eq!(
            check_lend_caps(OP_SUPPLY, 6, &p, &caps),
            Err(ErrorCode::LendSupplyCapExceeded)
        );
        assert!(check_lend_caps(OP_BORROW, 7, &p, &caps).is_ok());
        assert_eq!(
            check_lend_caps(OP_BORROW, 8, &p, &caps),
            Err(ErrorCode::LendDebtCapExceeded)
        );
        assert!(check_lend_caps(OP_ADD_COLLATERAL, 1_000_000_000_000_000, &p, &caps).is_ok());
        assert_eq!(
            check_lend_caps(OP_ADD_COLLATERAL, 1_000_000_000_000_001, &p, &caps),
            Err(ErrorCode::LendAssetCapExceeded)
        );
        // Repay and withdrawals are never capped.
        assert!(check_lend_caps(OP_REPAY, u64::MAX, &p, &caps).is_ok());
        assert!(check_lend_caps(OP_WITHDRAW_SUPPLY, u64::MAX, &p, &caps).is_ok());
    }

    fn view(account: Pubkey, owner: Pubkey, publish_time: i64) -> PythPriceView {
        PythPriceView {
            account,
            owner,
            feed_id: [9; 32],
            price: 15_000_000_000, // 150.00 at exponent -8
            conf: 10_000_000,      // 0.10
            exponent: -8,
            publish_time,
            ema_price: 15_000_000_000,
            ema_conf: 10_000_000,
        }
    }

    #[test]
    fn price_rejection_paths_and_low_bound() {
        let p = pool();
        let ok = view(p.oracle, crate::oracle::PYTH_RECEIVER_PROGRAM_ID, 1_000);
        // price - conf = 149.90 USD => 149_900_000 USDC base units per SOL
        assert_eq!(
            pool_price(&p, 200, &ok, 1_100).unwrap(),
            (149_900_000, 1_000)
        );
        // wrong account
        let wrong_account = view(key(0x99), crate::oracle::PYTH_RECEIVER_PROGRAM_ID, 1_000);
        assert_eq!(
            pool_price(&p, 200, &wrong_account, 1_100),
            Err(ErrorCode::OracleAccountMismatch)
        );
        // wrong owner program
        let wrong_owner = view(p.oracle, key(0x98), 1_000);
        assert!(pool_price(&p, 200, &wrong_owner, 1_100).is_err());
        // stale beyond the pool's 24 h window, fresh just inside it
        assert!(pool_price(&p, 200, &ok, 1_000 + 86_401).is_err());
        assert!(pool_price(&p, 200, &ok, 1_000 + 86_400).is_ok());
        // feed id mismatch
        let mut wrong_feed = ok;
        wrong_feed.feed_id = [1; 32];
        assert!(pool_price(&p, 200, &wrong_feed, 1_100).is_err());
        // confidence too wide (2%)
        let mut wide = ok;
        wide.conf = 400_000_000;
        assert!(pool_price(&p, 200, &wide, 1_100).is_err());
    }

    #[test]
    fn price_view_parses_a_price_update_v2_account() {
        let mut data = vec![0u8; 134];
        data[..8].copy_from_slice(&crate::oracle::PRICE_UPDATE_V2_DISCRIMINATOR);
        data[40] = 1; // Full verification
        data[41..73].copy_from_slice(&[9; 32]);
        data[73..81].copy_from_slice(&15_000_000_000i64.to_le_bytes());
        data[81..89].copy_from_slice(&10_000_000u64.to_le_bytes());
        data[89..93].copy_from_slice(&(-8i32).to_le_bytes());
        data[93..101].copy_from_slice(&1_000i64.to_le_bytes());
        data[109..117].copy_from_slice(&15_000_000_000i64.to_le_bytes());
        let v = crate::oracle::price_view_from_account(key(1), key(2), &data).unwrap();
        assert_eq!(
            (v.price, v.conf, v.exponent, v.publish_time),
            (15_000_000_000, 10_000_000, -8, 1_000)
        );
        assert_eq!(v.ema_price, 15_000_000_000);
        assert_eq!(v.feed_id, [9; 32]);
        data[40] = 0; // Partial verification is rejected
        assert!(crate::oracle::price_view_from_account(key(1), key(2), &data).is_none());
    }

    #[test]
    fn plan_owner_op_computes_shares_and_checks_liquidity() {
        let mut a = asset();
        a.caps.total_debt_cap_usdc = u64::MAX;
        let mut p = pool();
        p.borrow_index_1e9 = 1_500_000_000;
        // borrow rounds shares up; vault holds 100 and 0 is reserved
        assert_eq!(
            plan_owner_op(&mut p, &a, OP_BORROW, 10, 0, 1, 100).unwrap(),
            7
        );
        assert_eq!(
            plan_owner_op(&mut p, &a, OP_BORROW, 101, 0, 1, 100),
            Err(ErrorCode::LendInsufficientLiquidity)
        );
        assert_eq!(
            plan_owner_op(&mut p, &a, OP_REPAY, 0, 0, 1, 100),
            Err(ErrorCode::LendOpZeroAmount)
        );
        assert_eq!(
            plan_owner_op(&mut p, &a, OP_HEALTH_CHECK, 1, 0, 1, 100),
            Err(ErrorCode::LendOpWrongEntry)
        );
        // supply shares at index 1.0
        let mut q = pool();
        assert_eq!(
            plan_owner_op(&mut q, &a, OP_SUPPLY, 40, 0, 1, 0).unwrap(),
            40
        );
    }

    #[test]
    fn plan_liquidation_checks_bounds() {
        let a = asset();
        let mut p = pool();
        // 100 USDC repay, 5% bonus (launch default may differ): seize > 0, burn 100e6
        let (burn, seize) =
            plan_liquidation(&mut p, &a, 100_000_000, 150_000_000, 0, 1, u64::MAX).unwrap();
        assert_eq!(burn, 100_000_000);
        assert!(seize > 0);
        assert_eq!(
            plan_liquidation(&mut p, &a, 100_000_000, 150_000_000, 0, 1, seize - 1),
            Err(ErrorCode::LiquidationAmountInvalid)
        );
        assert_eq!(
            plan_liquidation(&mut p, &a, 2_000_000_000, 150_000_000, 0, 1, u64::MAX),
            Err(ErrorCode::LiquidationAmountInvalid)
        );
    }

    /// Feature: walnut-lend, Property 11: Pool totals match applied operations.
    /// **Validates: Requirements 3.5, 4.6**
    #[test]
    fn pool_totals_follow_applied_ops() {
        let mut p = pool();
        let (s0, d0) = (p.total_supply_shares, p.total_debt_shares);
        apply_pool_totals(&mut p, OP_BORROW, 30).unwrap();
        apply_pool_totals(&mut p, OP_REPAY, 10).unwrap();
        apply_pool_totals(&mut p, OP_LIQUIDATE, 5).unwrap();
        apply_pool_totals(&mut p, OP_SUPPLY, 7).unwrap();
        apply_pool_totals(&mut p, OP_WITHDRAW_SUPPLY, 2).unwrap();
        apply_pool_totals(&mut p, OP_ADD_COLLATERAL, 99).unwrap();
        assert_eq!(p.total_debt_shares, d0 + 30 - 10 - 5);
        assert_eq!(p.total_supply_shares, s0 + 7 - 2);
        assert_eq!(
            apply_pool_totals(&mut p, OP_REPAY, u64::MAX),
            Err(ErrorCode::LendPoolTotalsUnderflow)
        );
    }

    #[test]
    fn owner_op_queue_then_callback_applies_and_releases_lock() {
        let m = market();
        let mut pos = position();
        let owner = pos.owner;
        let mut counter = OperationCounter::new(owner, 1);
        let p = params(OP_BORROW, 10, 7);
        let k = keys(pos.coordination.generation, None);
        let mut intent =
            queue_with_keys(&mut pos, &mut counter, &owner, &k, &request(p, 0)).unwrap();
        assert!(intent.lock_owner().holds(&pos.coordination.lock));
        // round trip of the stored parameters
        assert_eq!(
            LendParams::from_intent(&intent, p.op, p.amount, p.shares).unwrap(),
            p
        );
        let report = report_for(&pos, &intent, &p, true);
        let s = settle_lend_op(
            &m,
            key(MARKET),
            key(POOL_KEY),
            &mut pos,
            key(0x23),
            &mut intent,
            None,
            &report,
        )
        .unwrap();
        assert_eq!(
            s,
            LendSettlement::Applied {
                op: OP_BORROW,
                amount: 10,
                shares: 7
            }
        );
        assert_eq!(pos.encrypted_state.ciphertexts, [[0x61; 32]; 5]);
        assert!(!intent.lock_owner().holds(&pos.coordination.lock));
        assert_eq!(intent.status, IntentStatus::Applied);
    }

    #[test]
    fn refusal_stores_nothing_and_a_tampered_echo_reverts() {
        let m = market();
        let mut pos = position();
        let before = pos.encrypted_state;
        let owner = pos.owner;
        let mut counter = OperationCounter::new(owner, 1);
        let p = params(OP_REPAY, 10, 10);
        let k = keys(pos.coordination.generation, None);
        let mut intent =
            queue_with_keys(&mut pos, &mut counter, &owner, &k, &request(p, 0)).unwrap();
        // a changed shares echo no longer matches the committed inputs
        let mut bad = report_for(&pos, &intent, &p, true);
        bad.shares = 11;
        let err = settle_lend_op(
            &m,
            key(MARKET),
            key(POOL_KEY),
            &mut pos,
            key(0x23),
            &mut intent,
            None,
            &bad,
        );
        assert!(err.is_err());
        assert_eq!(intent.status, IntentStatus::Queued);
        let refusal = report_for(&pos, &intent, &p, false);
        let s = settle_lend_op(
            &m,
            key(MARKET),
            key(POOL_KEY),
            &mut pos,
            key(0x23),
            &mut intent,
            None,
            &refusal,
        )
        .unwrap();
        assert_eq!(s, LendSettlement::Rejected { op: OP_REPAY });
        assert_eq!(pos.encrypted_state, before);
        assert!(!intent.lock_owner().holds(&pos.coordination.lock));
    }

    #[test]
    fn health_check_is_permissionless_and_takes_no_lock() {
        let m = market();
        let mut pos = position();
        let mut counter = OperationCounter::new(key(0x55), 1);
        let p = params(OP_HEALTH_CHECK, 0, 0);
        let k = LendKeys {
            payer: key(0x55),
            ..keys(pos.coordination.generation, None)
        };
        let mut intent =
            queue_with_keys(&mut pos, &mut counter, &key(0x55), &k, &request(p, 0)).unwrap();
        assert!(!intent.lock_owner().holds(&pos.coordination.lock));
        let report = report_for(&pos, &intent, &p, true);
        let before = pos.encrypted_state;
        let s = settle_lend_op(
            &m,
            key(MARKET),
            key(POOL_KEY),
            &mut pos,
            key(0x23),
            &mut intent,
            None,
            &report,
        )
        .unwrap();
        assert_eq!(s, LendSettlement::Health { unhealthy: true });
        assert_eq!(pos.encrypted_state, before);
    }

    #[test]
    fn owner_ops_reject_other_signers_and_liquidation_needs_accounts() {
        let m = market();
        let mut pos = position();
        let mut counter = OperationCounter::new(key(0x55), 1);
        let p = params(OP_SUPPLY, 5, 5);
        let r = request(p, 0);
        assert_eq!(
            queue_lend_op(
                &m,
                key(MARKET),
                key(POOL_KEY),
                &mut pos,
                key(0x23),
                &mut counter,
                &key(0x55),
                None,
                &r
            ),
            Err(ErrorCode::PositionOpNotOwner)
        );
        let liq = request(params(OP_LIQUIDATE, 5, 5), 0);
        assert_eq!(
            queue_lend_op(
                &m,
                key(MARKET),
                key(POOL_KEY),
                &mut pos,
                key(0x23),
                &mut counter,
                &key(0x55),
                None,
                &liq
            ),
            Err(ErrorCode::LiquidationAccountsInvalid)
        );
    }

    #[test]
    fn liquidation_bindings_cover_liquidator_accounts() {
        let m = market();
        let lk = LiquidationKeys {
            usdc_vault: m.usdc_vault,
            wsol_vault: m.wsol_vault,
            vault_authority: m.vault_authority,
            token_program: anchor_spl::token::ID,
            liquidator_usdc: key(0x91),
            liquidator_wsol: key(0x92),
        };
        assert!(lk.check_market(&m).is_ok());
        let base = account_commitment(&lend_bindings(&keys(3, None)));
        let with = account_commitment(&lend_bindings(&keys(3, Some(lk))));
        let swapped = LiquidationKeys {
            liquidator_usdc: key(0x93),
            ..lk
        };
        assert_ne!(base, with);
        assert_ne!(
            with,
            account_commitment(&lend_bindings(&keys(3, Some(swapped))))
        );
        let bad = LiquidationKeys {
            wsol_vault: key(0x94),
            ..lk
        };
        assert_eq!(
            bad.check_market(&m),
            Err(ErrorCode::LiquidationAccountsInvalid)
        );
    }

    #[test]
    fn abandon_releases_the_lock_after_expiry_only() {
        let mut pos = position();
        let owner = pos.owner;
        let mut counter = OperationCounter::new(owner, 1);
        let p = params(OP_ADD_COLLATERAL, 5, 0);
        let k = keys(pos.coordination.generation, None);
        let mut intent =
            queue_with_keys(&mut pos, &mut counter, &owner, &k, &request(p, 0)).unwrap();
        assert!(abandon_lend_op(&mut pos, key(0x23), &mut intent, 200).is_err());
        let expiry = intent.lock_expiry_slot + 1;
        abandon_lend_op(&mut pos, key(0x23), &mut intent, expiry).unwrap();
        assert_eq!(intent.status, IntentStatus::Abandoned);
        assert!(!is_liquidation(&intent));
    }
}
