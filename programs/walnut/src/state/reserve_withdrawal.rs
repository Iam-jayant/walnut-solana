//! Reserve withdrawal (Task 7.5; Requirement 7.15; design: "Reserve
//! withdrawal").
//!
//! Treasury-authority-signed, public all-or-zero request. Pure rules behind
//! `reserve_withdrawal`, its callback, `abandon_reserve_withdrawal`, and the
//! admin's `set_treasury_usdc_account`. The Anchor handlers in `lib.rs` and
//! `instructions/reserve_withdrawal.rs` load accounts, queue the computation,
//! and perform the SPL transfer.
//!
//! # Flow
//!
//! 1. `reserve_withdrawal(amount)`: signer must be `Market.treasury_authority`
//!    (`Role::Treasury`; the admin is not an implicit override). The Market is
//!    `Active`, the `WITHDRAW_SUPPLY` pause is off (the `ReserveWithdrawal`
//!    action maps to it), `amount > 0`, the Market's treasury token account is
//!    configured, holds the USDC mint, is owned by the treasury authority, and
//!    is not the vault. [`begin_pool_mutation`] then accrues the accounting
//!    index, captures the dependencies, and Locks the Pool.
//! 2. The circuit checkpoints interest FIRST, then approves the full amount
//!    only if it is no greater than Cash and Reserves after the checkpoint
//!    (all or zero). Cash and Reserves both decrease by exactly the amount, so
//!    supplier equity is unchanged.
//! 3. Callback ([`settle_reserve_withdrawal`]): `Applied` stores the new Pool
//!    payload, sets the accounted index to the Intent's accounting index,
//!    bumps the Pool generation, releases the Lock, and returns
//!    [`ReserveWithdrawalSettlement::Approved`]; the handler then transfers
//!    exactly that amount from the USDC vault to the committed treasury token
//!    account, signed by the vault-authority PDA. `Zero`, `Stale`, `Expired`,
//!    and failed outcomes release the Lock, store nothing, and move nothing.
//!    A terminal Intent is a no-op. Unauthenticated output and every binding
//!    mismatch revert, so the Intent stays `Queued` until
//!    [`abandon_reserve_withdrawal`].
//!
//! # Zero commits nothing
//!
//! A refused request (`Zero`) does not store the checkpointed payload and does
//! not advance `accounted_borrow_index`. Interest is not lost: the next Pool
//! mutation checkpoints from the same accounted index over the same interval,
//! exactly once. (The design states the effect only for approval; this keeps a
//! refused request from changing any encrypted or accounted state.)
//!
//! # Intent binding
//!
//! `accounts_commitment` covers, in order: the signer (treasury authority),
//! Market, Asset State, Pool (captured generation), the USDC vault (mint, vault
//! authority), the USDC mint, the treasury token account (mint, token owner),
//! the vault-authority PDA, and the SPL Token program.
//! `inputs_commitment` covers the Pool payload nonce and four ciphertexts, the
//! accounting and accounted indexes, the reserve factor, the requested amount,
//! and the destination key. The callback rebuilds both from the accounts it
//! received and the Intent's own payer, so substituting the destination, vault,
//! mint, vault authority, or token program, changing the treasury token owner,
//! or echoing another amount fails the commitment check.

use anchor_lang::prelude::Pubkey;

use super::accounts::{EncryptedPoolState, PoolState};
use super::coordination::CoordinationSlot;
use super::pool_mutation::{
    abandon_pool_mutation, begin_pool_mutation, pool_dependencies, settle_pool_decision,
    IntentCommitments, PoolCapture, PoolMutationRequest,
};
use super::shield::TokenAccountView;
use super::Slot;
use crate::errors::ErrorCode;
use crate::market::{authorize, ensure_action_allowed, Action, AssetState, Market, Role};
use crate::operation::callback::{
    classify_callback, CallbackContext, CallbackDecision, CallbackReport,
};
use crate::operation::commitment::{
    account_commitment, input_commitment, AccountBinding, AccountRole, InputField, InputValue,
};
use crate::operation::{IntentStatus, OperationCounter, OperationIntent, OperationKind};

use super::band::{INPUT_ACCOUNTING_INDEX, INPUT_POOL_CIPHERTEXT_FIRST, INPUT_POOL_NONCE};

/// The upgradeable-free BPF loader that owns the SPL Token program account.
const BPF_LOADER_PROGRAM: Pubkey =
    Pubkey::from_str_const("BPFLoader2111111111111111111111111111111111");

/// Input field id: the Pool's accounted index (`U128`).
pub const INPUT_ACCOUNTED_INDEX: u8 = 22;
/// Input field id: the reserve factor in bps (`U64`).
pub const INPUT_RESERVE_FACTOR_BPS: u8 = 23;
/// Input field id: the requested amount (`U64`).
pub const INPUT_REQUESTED_AMOUNT: u8 = 24;
/// Input field id: the treasury destination (`Key`).
pub const INPUT_DESTINATION: u8 = 25;

/// Decision Expiry distance of a reserve withdrawal, in slots.
pub const RESERVE_DECISION_SLOTS: u64 = 300;
/// Slots between Decision Expiry and Lock expiry.
pub const RESERVE_LOCK_GRACE_SLOTS: u64 = 150;

/// The accounts a reserve withdrawal binds besides the signer, Market, Asset
/// State, and Pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReserveKeys {
    /// The USDC vault.
    pub vault: Pubkey,
    /// The USDC mint.
    pub mint: Pubkey,
    /// The treasury token account.
    pub destination: Pubkey,
    /// The token owner of the treasury token account.
    pub destination_owner: Pubkey,
    /// The vault-authority PDA that signs the transfer.
    pub vault_authority: Pubkey,
    /// The SPL Token program.
    pub token_program: Pubkey,
}

/// The ordered accounts bound by a reserve withdrawal Intent.
pub fn reserve_bindings(
    authority: Pubkey,
    market_key: Pubkey,
    asset_state: Pubkey,
    pool: Pubkey,
    pool_generation: Option<u64>,
    keys: &ReserveKeys,
) -> [AccountBinding; 9] {
    let program = crate::ID;
    let token_program = anchor_spl::token::ID;
    let plain = |key, role, expected_owner, writable, generation| AccountBinding {
        key,
        role,
        expected_owner,
        mint: None,
        authority: None,
        writable,
        generation,
    };
    [
        plain(
            authority,
            AccountRole::Owner,
            anchor_lang::system_program::ID,
            true,
            None,
        ),
        plain(market_key, AccountRole::Market, program, false, None),
        plain(asset_state, AccountRole::AssetState, program, false, None),
        plain(pool, AccountRole::Pool, program, true, pool_generation),
        AccountBinding {
            key: keys.vault,
            role: AccountRole::Vault,
            expected_owner: token_program,
            mint: Some(keys.mint),
            authority: Some(keys.vault_authority),
            writable: true,
            generation: None,
        },
        plain(keys.mint, AccountRole::Mint, token_program, false, None),
        AccountBinding {
            key: keys.destination,
            role: AccountRole::TokenAccount,
            expected_owner: token_program,
            mint: Some(keys.mint),
            authority: Some(keys.destination_owner),
            writable: true,
            generation: None,
        },
        plain(
            keys.vault_authority,
            AccountRole::Other,
            program,
            false,
            None,
        ),
        plain(
            keys.token_program,
            AccountRole::Other,
            BPF_LOADER_PROGRAM,
            false,
            None,
        ),
    ]
}

/// The ordered inputs bound by a reserve withdrawal Intent.
pub fn reserve_inputs(
    pool: &PoolState,
    accounted_index: u128,
    accounting_index: u128,
    amount: u64,
    destination: Pubkey,
) -> [InputField<'static>; 9] {
    let c = &pool.encrypted.ciphertexts;
    let first = INPUT_POOL_CIPHERTEXT_FIRST;
    [
        InputField::new(INPUT_POOL_NONCE, InputValue::U128(pool.encrypted.nonce)),
        InputField::new(first, InputValue::Bytes32(c[0])),
        InputField::new(first + 1, InputValue::Bytes32(c[1])),
        InputField::new(first + 2, InputValue::Bytes32(c[2])),
        InputField::new(first + 3, InputValue::Bytes32(c[3])),
        InputField::new(INPUT_ACCOUNTING_INDEX, InputValue::U128(accounting_index)),
        InputField::new(INPUT_ACCOUNTED_INDEX, InputValue::U128(accounted_index)),
        InputField::new(INPUT_REQUESTED_AMOUNT, InputValue::U64(amount)),
        InputField::new(INPUT_DESTINATION, InputValue::Key(destination)),
    ]
}

/// The keys a reserve withdrawal binds, from the Market and the treasury token
/// account.
pub fn reserve_keys(market: &Market, destination: &TokenAccountView) -> ReserveKeys {
    ReserveKeys {
        vault: market.usdc_vault,
        mint: market.usdc_mint,
        destination: destination.key,
        destination_owner: destination.owner,
        vault_authority: market.vault_authority,
        token_program: anchor_spl::token::ID,
    }
}

/// `true` when `destination` is the Market's configured treasury USDC token
/// account, holds the USDC mint, is owned by the current treasury authority,
/// and is not the vault.
pub fn check_treasury_destination(
    market: &Market,
    destination: &TokenAccountView,
) -> Result<(), ErrorCode> {
    if market.treasury_usdc_account == Pubkey::default() {
        return Err(ErrorCode::TreasuryAccountNotSet);
    }
    if destination.key != market.treasury_usdc_account
        || destination.mint != market.usdc_mint
        || destination.owner != market.treasury_authority
        || destination.key == market.usdc_vault
    {
        return Err(ErrorCode::TreasuryAccountInvalid);
    }
    Ok(())
}

/// `set_treasury_usdc_account` (admin): records the single treasury USDC token
/// account. It must hold the USDC mint, be owned by the current treasury
/// authority, and not be the vault or the zero key. Returns the previous
/// account. Rotating the treasury authority later invalidates the account
/// until the admin sets it again (reserve withdrawal then refuses).
pub fn set_treasury_usdc_account(
    market: &mut Market,
    signer: &Pubkey,
    account: &TokenAccountView,
) -> Result<Pubkey, ErrorCode> {
    authorize(market, Role::Admin, signer)?;
    if account.key == Pubkey::default()
        || account.mint != market.usdc_mint
        || account.owner != market.treasury_authority
        || account.key == market.usdc_vault
    {
        return Err(ErrorCode::TreasuryAccountInvalid);
    }
    let old = market.treasury_usdc_account;
    market.treasury_usdc_account = account.key;
    Ok(old)
}

/// One `reserve_withdrawal` queue request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReserveWithdrawalRequest {
    /// The Operation Intent PDA address.
    pub intent_key: Pubkey,
    /// The Pool counter's next Intent sequence.
    pub sequence: u64,
    /// The Arcium computation offset.
    pub computation_offset: u64,
    /// The current slot.
    pub queued_slot: Slot,
    /// The current Unix time.
    pub now_ts: i64,
    /// The public requested USDC amount.
    pub amount: u64,
}

/// `reserve_withdrawal`: authorizes, checks the pause and the destination,
/// then runs the shared Pool mutation queue ([`begin_pool_mutation`]). Atomic.
#[allow(clippy::too_many_arguments)]
pub fn queue_reserve_withdrawal(
    market: &Market,
    market_key: Pubkey,
    asset_state_key: Pubkey,
    asset: &mut AssetState,
    pool: &mut PoolState,
    pool_key: Pubkey,
    counter: &mut OperationCounter,
    signer: &Pubkey,
    destination: &TokenAccountView,
    request: &ReserveWithdrawalRequest,
) -> Result<OperationIntent, ErrorCode> {
    authorize(market, Role::Treasury, signer)?;
    ensure_action_allowed(
        &asset.pauses,
        Action::ReserveWithdrawal,
        request.queued_slot,
    )?;
    if request.amount == 0 {
        return Err(ErrorCode::ReserveWithdrawalZeroAmount);
    }
    check_treasury_destination(market, destination)?;
    let keys = reserve_keys(market, destination);
    let accounted = pool.accounted_borrow_index;
    let amount = request.amount;
    let signer = *signer;
    let pool_snapshot = pool.clone();
    let mutation = PoolMutationRequest {
        intent_key: request.intent_key,
        owner: pool_key,
        payer: signer,
        kind: OperationKind::ReserveWithdrawal,
        sequence: request.sequence,
        computation_offset: request.computation_offset,
        queued_slot: request.queued_slot,
        now_ts: request.now_ts,
        decision_slots: RESERVE_DECISION_SLOTS,
        lock_grace_slots: RESERVE_LOCK_GRACE_SLOTS,
    };
    begin_pool_mutation(
        market.status,
        asset,
        pool,
        pool_key,
        &mut [],
        counter,
        &mutation,
        |capture: &PoolCapture| {
            let bindings = reserve_bindings(
                signer,
                market_key,
                asset_state_key,
                pool_key,
                Some(capture.pool_generation),
                &keys,
            );
            let inputs = reserve_inputs(
                &pool_snapshot,
                accounted,
                capture.accounting_borrow_index,
                amount,
                keys.destination,
            );
            IntentCommitments {
                accounts: account_commitment(&bindings),
                inputs: input_commitment(OperationKind::ReserveWithdrawal, &inputs),
            }
        },
    )
}

/// What the `reserve_withdrawal` callback observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReserveWithdrawalReport {
    /// Whether Arcium `verify_output` succeeded.
    pub output_authenticated: bool,
    /// The computation offset the callback's computation account belongs to.
    pub computation_offset: u64,
    /// The Operation Intent account the callback received.
    pub intent_key: Pubkey,
    /// The output Pool payload.
    pub output: EncryptedPoolState,
    /// The revealed approved amount: the request or zero.
    pub approved_amount: u64,
    /// The revealed request echo.
    pub requested_amount: u64,
    /// The treasury token account the callback received.
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
pub enum ReserveWithdrawalSettlement {
    /// `Applied`: the new Pool payload is stored. The handler must transfer
    /// exactly `amount` from the USDC vault to `destination`, signed by the
    /// vault authority.
    Approved {
        /// The exact amount to transfer.
        amount: u64,
        /// The committed treasury token account.
        destination: Pubkey,
    },
    /// The circuit approved zero: nothing stored, nothing moves.
    Zero,
    /// `Expired`, `Stale`, or `Failed`: nothing stored, nothing moves.
    Failed(IntentStatus),
    /// The Intent was already terminal; nothing changed.
    Ignored,
}

/// Verifies and applies a `reserve_withdrawal` callback. Unauthenticated
/// output and commitment, offset, or binding mismatches are errors (the Intent
/// stays `Queued`). Atomic on error.
pub fn settle_reserve_withdrawal(
    market: &Market,
    market_key: Pubkey,
    asset: &AssetState,
    pool: &mut PoolState,
    pool_key: Pubkey,
    intent: &mut OperationIntent,
    report: &ReserveWithdrawalReport,
) -> Result<ReserveWithdrawalSettlement, ErrorCode> {
    let keys = ReserveKeys {
        vault: report.vault,
        mint: report.mint,
        destination: report.destination.key,
        destination_owner: report.destination.owner,
        vault_authority: report.vault_authority,
        token_program: report.token_program,
    };
    let bindings = reserve_bindings(
        intent.payer,
        market_key,
        market.asset_state,
        pool_key,
        intent.pool_generation,
        &keys,
    );
    // The reserve factor is a governed value whose change bumps the Asset
    // State generation (`Stale` below), so the current value is the captured
    // one whenever the decision reaches `Applied`.
    let inputs = reserve_inputs(
        pool,
        pool.accounted_borrow_index,
        intent.accounting_borrow_index,
        report.requested_amount,
        report.destination.key,
    );
    let context = CallbackContext {
        output_authenticated: report.output_authenticated,
        computation_offset: report.computation_offset,
        intent_key: report.intent_key,
        kind: OperationKind::ReserveWithdrawal,
        accounts: &bindings,
        input_fields: &inputs,
        current: pool_dependencies(pool, asset),
        current_slot: report.current_slot,
        report: CallbackReport::Success {
            nonzero: report.approved_amount != 0,
        },
    };
    let decision = classify_callback(intent, &context)?;
    match decision {
        CallbackDecision::LeaveQueued => return Err(ErrorCode::CallbackOutputNotAuthenticated),
        CallbackDecision::Ignore => return Ok(ReserveWithdrawalSettlement::Ignored),
        CallbackDecision::Applied => {
            // The circuit approves the request or zero, never a partial amount.
            if report.approved_amount == 0 || report.approved_amount != report.requested_amount {
                return Err(ErrorCode::ReserveWithdrawalApprovalInvalid);
            }
            // The mint, vault, and token program are the Market's, and the
            // destination is still the configured treasury account.
            if report.mint != market.usdc_mint
                || report.vault != market.usdc_vault
                || report.vault_authority != market.vault_authority
                || report.token_program != anchor_spl::token::ID
            {
                return Err(ErrorCode::ReserveWithdrawalAccountMismatch);
            }
            check_treasury_destination(market, &report.destination)?;
        }
        _ => {}
    }

    let result = settle_pool_decision(
        pool,
        pool_key,
        &mut [],
        intent,
        decision,
        &report.output,
        report.current_slot,
    )?;
    Ok(match result.status {
        IntentStatus::Applied => ReserveWithdrawalSettlement::Approved {
            amount: report.approved_amount,
            destination: report.destination.key,
        },
        IntentStatus::Zero => ReserveWithdrawalSettlement::Zero,
        status => ReserveWithdrawalSettlement::Failed(status),
    })
}

/// `abandon_reserve_withdrawal`: permissionless timeout recovery after Lock
/// expiry. Bumps the Pool generation, releases the Lock, marks the Intent
/// `Abandoned`. Nothing moved, so nothing to refund. Returns the number of
/// Locks cleared.
pub fn abandon_reserve_withdrawal(
    pool: &mut PoolState,
    pool_key: Pubkey,
    intent: &mut OperationIntent,
    current_slot: Slot,
) -> Result<usize, ErrorCode> {
    let others: &mut [CoordinationSlot<'_>] = &mut [];
    abandon_pool_mutation(
        pool,
        pool_key,
        others,
        intent,
        OperationKind::ReserveWithdrawal,
        current_slot,
    )
}

#[cfg(test)]
mod tests;
