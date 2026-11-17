use anchor_lang::prelude::*;

/// Emitted by the scaffold `add_together` callback. The sum stays encrypted
/// to the caller's shared key; only the ciphertext and nonce are public.
#[event]
pub struct SumEvent {
    pub sum: [u8; 32],
    pub nonce: [u8; 16],
}

/// Emitted by the Task 2.2 Position callbacks when a new encrypted revision is
/// persisted. The payload stays encrypted in the account; only the Position
/// key, the new revision counter, and the output nonce are public. The nonce
/// lets a client locate and decrypt the stored ciphertext locally.
#[event]
pub struct PositionStateStored {
    pub position: Pubkey,
    pub revision: u64,
    pub nonce: [u8; 16],
}

/// Emitted by the Task 2.3 `liquidation_receipts` callback. Proves one
/// computation sealed two encrypted Receipts to two different owners. Each
/// Receipt stays encrypted; only its owner's effective encryption key, the
/// output nonce, and the ciphertexts are public. The design emits borrower and
/// liquidator Receipts as authenticated callback events rather than creating
/// Receipt accounts, so the test decrypts each owner's ciphertexts locally and
/// records the per-owner (key, nonce) identities for the no-repeat assertion.
#[event]
pub struct LiquidationReceiptsEmitted {
    pub borrower_key: [u8; 32],
    pub borrower_nonce: [u8; 16],
    pub borrower_ciphertexts: [[u8; 32]; 2],
    pub liquidator_key: [u8; 32],
    pub liquidator_nonce: [u8; 16],
    pub liquidator_ciphertexts: [[u8; 32]; 2],
}

/// Emitted by the Task 2.4 `oversized_output` success callback. In practice the
/// computation fails with `OutputTooLarge` before this callback runs, so the
/// event is the negative control: the localnet test asserts it is NOT observed.
/// Only the output nonce is carried; the oversized ciphertexts are ignored.
#[event]
pub struct OversizedOutputEmitted {
    pub nonce: [u8; 16],
}

/// Emitted by the Task 2.3 `rotate_viewing_key` callback when a Position is
/// re-encrypted to a new Viewing Key. Commits the old and new key epochs and
/// public keys (Requirement 5.11). The re-encrypted payload stays in the
/// account; only the epoch/public-key transition and the new output nonce are
/// public.
#[event]
pub struct ViewingKeyRotated {
    pub position: Pubkey,
    pub old_epoch: u64,
    pub new_epoch: u64,
    pub old_pubkey: [u8; 32],
    pub new_pubkey: [u8; 32],
    pub new_nonce: [u8; 16],
}

/// Emitted by `initialize_market` (Task 4.1). Every value is public
/// configuration; no amount is involved.
#[event]
pub struct MarketInitialized {
    pub market: Pubkey,
    pub asset_state: Pubkey,
    pub pool_state: Pubkey,
    pub recovery_position: Pubkey,
    pub vault_authority: Pubkey,
    pub wsol_vault: Pubkey,
    pub usdc_vault: Pubkey,
    pub usdc_mint: Pubkey,
    pub admin: Pubkey,
    pub risk_authority: Pubkey,
    pub treasury_authority: Pubkey,
    pub guardian_authority: Pubkey,
}

/// Emitted by the `init_pool_state` callback when the encrypted zero Pool
/// payload is stored and the Pool becomes `Active`. Only the generation and
/// the MXE output nonce are public.
#[event]
pub struct PoolStateActivated {
    pub pool_state: Pubkey,
    pub generation: u64,
    pub nonce: [u8; 16],
}

/// Emitted by `propose_admin` (Task 4.2). `old_pending_admin` is all zero when
/// no transfer was pending; a re-proposal overwrites it.
#[event]
pub struct AdminTransferProposed {
    pub market: Pubkey,
    pub admin: Pubkey,
    pub old_pending_admin: Pubkey,
    pub new_pending_admin: Pubkey,
    pub slot: u64,
}

/// Emitted by `accept_admin` (Task 4.2) when the pending admin becomes admin.
#[event]
pub struct AdminTransferAccepted {
    pub market: Pubkey,
    pub old_admin: Pubkey,
    pub new_admin: Pubkey,
    pub slot: u64,
}

/// Emitted by `cancel_admin_transfer` (Task 4.2). `new_pending_admin` is
/// always all zero.
#[event]
pub struct AdminTransferCancelled {
    pub market: Pubkey,
    pub admin: Pubkey,
    pub old_pending_admin: Pubkey,
    pub new_pending_admin: Pubkey,
    pub slot: u64,
}

/// Emitted by every separated-role setter (Task 4.2): risk, treasury,
/// guardian, and the recorded upgrade authority. `changed_by` is the signer
/// that authorized the change.
#[event]
pub struct RoleAuthorityChanged {
    pub market: Pubkey,
    pub role: crate::market::RoleKind,
    pub old_authority: Pubkey,
    pub new_authority: Pubkey,
    pub changed_by: Pubkey,
    pub slot: u64,
}

/// Emitted by every pause instruction (Task 4.3). `flags` is the mask the
/// instruction named (empty for a guardian clear or renewal); the `old_*` and
/// `new_*` fields are the whole pause state before and after. `actor` is the
/// guardian or risk authority that signed.
#[event]
pub struct PauseUpdated {
    pub market: Pubkey,
    pub asset_state: Pubkey,
    pub change: crate::market::PauseChange,
    pub actor: Pubkey,
    pub flags: u16,
    pub old_flags: u16,
    pub new_flags: u16,
    pub old_guardian_flags: u16,
    pub new_guardian_flags: u16,
    pub old_guardian_expiry_slot: u64,
    pub new_guardian_expiry_slot: u64,
    pub slot: u64,
}

/// Emitted by `propose_config_change` (Task 4.4). `old_value` is the group's
/// current value and `new_value` the proposed one.
#[event]
pub struct ConfigChangeProposed {
    pub market: Pubkey,
    pub asset_state: Pubkey,
    pub proposal: Pubkey,
    pub proposal_id: u64,
    pub group: crate::market::ConfigGroup,
    pub proposer: Pubkey,
    pub old_value: crate::market::ConfigValue,
    pub new_value: crate::market::ConfigValue,
    pub proposed_at: i64,
    pub earliest_execute_at: i64,
    pub expires_at: i64,
    pub proposal_generation: u64,
    pub slot: u64,
}

/// Emitted by `execute_config_change` (Task 4.4) with the old and new group
/// values and the Asset State generation before and after (Requirement 4.6).
#[event]
pub struct ConfigChangeExecuted {
    pub market: Pubkey,
    pub asset_state: Pubkey,
    pub proposal: Pubkey,
    pub proposal_id: u64,
    pub group: crate::market::ConfigGroup,
    pub executor: Pubkey,
    pub old_value: crate::market::ConfigValue,
    pub new_value: crate::market::ConfigValue,
    pub old_generation: u64,
    pub new_generation: u64,
    pub executed_at: i64,
    pub slot: u64,
}

/// Emitted by `cancel_config_change` (Task 4.4).
#[event]
pub struct ConfigChangeCancelled {
    pub market: Pubkey,
    pub proposal: Pubkey,
    pub proposal_id: u64,
    pub group: crate::market::ConfigGroup,
    pub cancelled_by: Pubkey,
    pub cancelled_at: i64,
    pub slot: u64,
}

/// Emitted by `decrease_caps` (Task 4.5): an immediate cap decrease by the
/// risk authority, with old and new values and the Asset State generation
/// before and after (Requirement 4.6). Cap raises use the timelock and emit
/// `ConfigChangeExecuted`.
#[event]
pub struct CapsDecreased {
    pub market: Pubkey,
    pub asset_state: Pubkey,
    pub group: crate::market::ConfigGroup,
    pub authority: Pubkey,
    pub old_value: crate::market::ConfigValue,
    pub new_value: crate::market::ConfigValue,
    pub old_generation: u64,
    pub new_generation: u64,
    pub slot: u64,
}

/// Emitted when a Position PDA is created and `position_count` increments
/// (Task 4.5 rule `market::on_position_initialized`; emitted by the Task 5
/// Position initialization instruction).
#[event]
pub struct PositionCountChanged {
    pub market: Pubkey,
    pub position: Pubkey,
    pub old_count: u64,
    pub new_count: u64,
    pub cap: u64,
    pub slot: u64,
}

/// Emitted when an authenticated zero/nonzero debt transition changes a
/// Position's public `has_debt` and `debt_position_count` (Task 4.5 rule
/// `market::settle_debt_intent`; emitted by the borrow, repay, liquidation,
/// and Absorb callbacks). Reveals only the transition, never an amount.
#[event]
pub struct DebtPositionCountChanged {
    pub market: Pubkey,
    pub position: Pubkey,
    pub has_debt: bool,
    pub old_count: u64,
    pub new_count: u64,
    pub reservations: u64,
    pub cap: u64,
    pub slot: u64,
}

// ---- Task 5.1: Position initialization ---------------------------------------

/// Emitted when a Position initialization attempt is queued: by
/// `initialize_position` (`retry == false`) and `retry_position_init`
/// (`retry == true`).
#[event]
pub struct PositionInitQueued {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub operation_intent: Pubkey,
    pub sequence: u64,
    pub computation_offset: u64,
    pub viewing_pubkey: [u8; 32],
    pub viewing_key_epoch: u64,
    pub position_generation: u64,
    pub decision_expiry_slot: u64,
    pub lock_expiry_slot: u64,
    pub retry: bool,
    pub slot: u64,
}

/// Emitted by the `init_position_state` callback when the encrypted zero
/// state is stored and the Position becomes `Active`. `nonce` is the stored
/// output nonce (little-endian); the client decrypts locally.
#[event]
pub struct PositionActivated {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub operation_intent: Pubkey,
    pub generation: u64,
    pub nonce: [u8; 16],
    pub slot: u64,
}

/// Emitted when an initialization attempt ends without storing state
/// (callback `Stale`, `Expired`, `Failed`, or `Zero`, or permissionless
/// abandonment). The Position is `InitFailed`: the owner may retry or close.
/// `intent_status` is the Intent's terminal status.
#[event]
pub struct PositionInitFailed {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub operation_intent: Pubkey,
    pub intent_status: crate::operation::IntentStatus,
    pub generation: u64,
    pub slot: u64,
}

/// Emitted by `close_position`: the `InitFailed` Position was closed and its
/// rent returned to the owner. `PositionCountChanged` is emitted with it.
#[event]
pub struct PositionClosed {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub slot: u64,
}

// ---- Task 5.3: shield ---------------------------------------------------------

/// Emitted by `shield` after the exact public `amount` moved from
/// `source_token_account` into `vault` and the first credit attempt was
/// queued. `asset` and `amount` are public by design (Requirement 5.5).
#[event]
pub struct ShieldQueued {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub pending_shield: Pubkey,
    pub operation_intent: Pubkey,
    pub asset: crate::state::ShieldAsset,
    pub amount: u64,
    pub shield_sequence: u64,
    pub attempt_sequence: u64,
    pub computation_offset: u64,
    pub source_token_account: Pubkey,
    pub vault: Pubkey,
    pub position_generation: u64,
    pub decision_expiry_slot: u64,
    pub lock_expiry_slot: u64,
    pub slot: u64,
}

/// Emitted by `retry_shield` when a new credit attempt is queued for a
/// `Deposited` Pending Shield whose previous attempt ended without a credit.
#[event]
pub struct ShieldRetried {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub pending_shield: Pubkey,
    pub operation_intent: Pubkey,
    pub previous_intent: Pubkey,
    pub attempt_sequence: u64,
    pub computation_offset: u64,
    pub position_generation: u64,
    pub decision_expiry_slot: u64,
    pub lock_expiry_slot: u64,
    pub slot: u64,
}

/// Emitted by the `shield_credit` callback when the credit is stored and the
/// Pending Shield is `Credited`. `nonce` is the stored output nonce
/// (little-endian); the owner decrypts locally.
#[event]
pub struct ShieldCredited {
    pub position: Pubkey,
    pub owner: Pubkey,
    pub pending_shield: Pubkey,
    pub operation_intent: Pubkey,
    pub asset: crate::state::ShieldAsset,
    pub amount: u64,
    pub generation: u64,
    pub nonce: [u8; 16],
    pub slot: u64,
}

/// Emitted by the `shield_credit` callback when the attempt ended without a
/// credit (`Zero`, `Stale`, or `Expired`). The Pending Shield stays
/// `Deposited`: the owner may retry or refund.
#[event]
pub struct ShieldCreditFailed {
    pub position: Pubkey,
    pub owner: Pubkey,
    pub pending_shield: Pubkey,
    pub operation_intent: Pubkey,
    pub intent_status: crate::operation::IntentStatus,
    pub slot: u64,
}

/// Emitted by `abandon_shield` (any signer, after Lock expiry). The Pending
/// Shield stays `Deposited` and its tokens stay in the vault.
#[event]
pub struct ShieldAbandoned {
    pub position: Pubkey,
    pub owner: Pubkey,
    pub pending_shield: Pubkey,
    pub operation_intent: Pubkey,
    pub generation: u64,
    pub signer: Pubkey,
    pub slot: u64,
}

/// Emitted by `refund_shield`: exactly `amount` moved from `vault` back to
/// `destination` and the Pending Shield is `Refunded`.
#[event]
pub struct ShieldRefunded {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub pending_shield: Pubkey,
    pub asset: crate::state::ShieldAsset,
    pub amount: u64,
    pub vault: Pubkey,
    pub destination: Pubkey,
    pub slot: u64,
}

// ---- Task 5.4: unshield -------------------------------------------------------

/// Emitted by `unshield` when the request was queued. `asset`, `amount`, and
/// `destination` are public by design (Requirement 5.7): the outgoing SPL
/// transfer is public. Nothing has moved yet.
#[event]
pub struct UnshieldQueued {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub operation_intent: Pubkey,
    pub asset: crate::state::ShieldAsset,
    pub amount: u64,
    pub destination: Pubkey,
    pub vault: Pubkey,
    pub computation_offset: u64,
    pub position_generation: u64,
    pub decision_expiry_slot: u64,
    pub lock_expiry_slot: u64,
    pub slot: u64,
}

/// Emitted by the `unshield` callback when the circuit approved the request:
/// exactly `amount` moved from `vault` to `destination` and the new state is
/// stored. `nonce` is the stored output nonce (little-endian).
#[event]
pub struct UnshieldApproved {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub operation_intent: Pubkey,
    pub asset: crate::state::ShieldAsset,
    pub amount: u64,
    pub vault: Pubkey,
    pub destination: Pubkey,
    pub generation: u64,
    pub nonce: [u8; 16],
    pub slot: u64,
}

/// Emitted by the `unshield` callback when the circuit approved zero
/// (insufficient free balance). Nothing moved and nothing was stored.
#[event]
pub struct UnshieldZero {
    pub position: Pubkey,
    pub owner: Pubkey,
    pub operation_intent: Pubkey,
    pub asset: crate::state::ShieldAsset,
    pub requested_amount: u64,
    pub slot: u64,
}

/// Emitted by the `unshield` callback when the attempt ended without a
/// transfer (`Expired`, `Stale`, or `Failed`).
#[event]
pub struct UnshieldFailed {
    pub position: Pubkey,
    pub owner: Pubkey,
    pub operation_intent: Pubkey,
    pub intent_status: crate::operation::IntentStatus,
    pub slot: u64,
}

/// Emitted by `abandon_unshield` (any signer, after Lock expiry).
#[event]
pub struct UnshieldAbandoned {
    pub position: Pubkey,
    pub owner: Pubkey,
    pub operation_intent: Pubkey,
    pub generation: u64,
    pub signer: Pubkey,
    pub slot: u64,
}

// ---- Task 5.5: Viewing Key rotation ---------------------------------------------

/// Emitted by `rotate_position_key` when the rotation was queued. The old and
/// new epochs and public keys are public by design (Requirement 5.11); no
/// balance is. Nothing has changed yet: the old key stays valid until the
/// callback applies.
#[event]
pub struct ViewingKeyRotationQueued {
    pub position: Pubkey,
    pub owner: Pubkey,
    pub operation_intent: Pubkey,
    pub sequence: u64,
    pub old_epoch: u64,
    pub new_epoch: u64,
    pub old_pubkey: [u8; 32],
    pub new_pubkey: [u8; 32],
    pub computation_offset: u64,
    pub position_generation: u64,
    pub decision_expiry_slot: u64,
    pub lock_expiry_slot: u64,
    pub slot: u64,
}

/// Emitted by the `rotate_position_key` callback when the payload, public key,
/// and epoch changed together. `nonce` is the stored output nonce
/// (little-endian). This is the production counterpart of the spike's
/// `ViewingKeyRotated`.
#[event]
pub struct ViewingKeyRotationApplied {
    pub position: Pubkey,
    pub owner: Pubkey,
    pub operation_intent: Pubkey,
    pub old_epoch: u64,
    pub new_epoch: u64,
    pub old_pubkey: [u8; 32],
    pub new_pubkey: [u8; 32],
    pub generation: u64,
    pub nonce: [u8; 16],
    pub slot: u64,
}

/// Emitted by the `rotate_position_key` callback when the attempt ended
/// without a rotation (`Expired`, `Stale`, or `Failed`). The old key, epoch,
/// and payload are unchanged.
#[event]
pub struct ViewingKeyRotationFailed {
    pub position: Pubkey,
    pub owner: Pubkey,
    pub operation_intent: Pubkey,
    pub intent_status: crate::operation::IntentStatus,
    pub slot: u64,
}

/// Emitted by `abandon_rotation` (any signer, after Lock expiry).
#[event]
pub struct ViewingKeyRotationAbandoned {
    pub position: Pubkey,
    pub owner: Pubkey,
    pub operation_intent: Pubkey,
    pub generation: u64,
    pub signer: Pubkey,
    pub slot: u64,
}

/// Emitted when `update_utilization_band` queues a computation (Task 7.2).
/// Public values only: the band computation reads encrypted Cash and debt but
/// reveals only the band index.
#[event]
pub struct UtilizationBandQueued {
    pub market: Pubkey,
    pub pool_state: Pubkey,
    pub operation_intent: Pubkey,
    pub computation_offset: u64,
    pub pool_generation: u64,
    pub rate_epoch_generation: u64,
    pub accounting_borrow_index: u128,
    pub decision_expiry_slot: u64,
    pub slot: u64,
}

/// Emitted by the `update_utilization_band` callback when the revealed band
/// was current and stored (`written`) or superseded by a newer observation.
#[event]
pub struct UtilizationBandUpdated {
    pub asset_state: Pubkey,
    pub operation_intent: Pubkey,
    pub band: u8,
    pub observation_slot: u64,
    pub written: bool,
    pub slot: u64,
}

/// Emitted by the `update_utilization_band` callback when nothing was stored
/// (`Stale`, `Expired`, or `Failed`). The status says which.
#[event]
pub struct UtilizationBandNotApplied {
    pub operation_intent: Pubkey,
    pub intent_status: crate::operation::IntentStatus,
    pub slot: u64,
}

/// Emitted by `reserve_withdrawal` when it queues (Task 7.5). The requested
/// amount and the treasury account are public by design.
#[event]
pub struct ReserveWithdrawalQueued {
    pub market: Pubkey,
    pub pool_state: Pubkey,
    pub operation_intent: Pubkey,
    pub authority: Pubkey,
    pub amount: u64,
    pub destination: Pubkey,
    pub computation_offset: u64,
    pub pool_generation: u64,
    pub accounting_borrow_index: u128,
    pub risk_borrow_index_at_expiry: u128,
    pub decision_expiry_slot: u64,
    pub lock_expiry_slot: u64,
    pub slot: u64,
}

/// Emitted by the `reserve_withdrawal` callback when the request was approved,
/// the new Pool state stored, and exactly `amount` transferred.
#[event]
pub struct ReserveWithdrawalApproved {
    pub market: Pubkey,
    pub pool_state: Pubkey,
    pub operation_intent: Pubkey,
    pub amount: u64,
    pub vault: Pubkey,
    pub destination: Pubkey,
    pub generation: u64,
    pub accounted_borrow_index: u128,
    pub nonce: [u8; 16],
    pub slot: u64,
}

/// Emitted by the `reserve_withdrawal` callback when nothing moved: the
/// circuit approved zero (`Zero`), or the attempt was `Stale`, `Expired`, or
/// `Failed`. The Pool state is unchanged and the Lock is released.
#[event]
pub struct ReserveWithdrawalNotApplied {
    pub pool_state: Pubkey,
    pub operation_intent: Pubkey,
    pub intent_status: crate::operation::IntentStatus,
    pub requested_amount: u64,
    pub slot: u64,
}

/// Emitted by `abandon_reserve_withdrawal` (any signer, after Lock expiry).
#[event]
pub struct ReserveWithdrawalAbandoned {
    pub pool_state: Pubkey,
    pub operation_intent: Pubkey,
    pub generation: u64,
    pub signer: Pubkey,
    pub slot: u64,
}

/// Emitted by `set_treasury_usdc_account` (admin).
#[event]
pub struct TreasuryUsdcAccountChanged {
    pub market: Pubkey,
    pub old_account: Pubkey,
    pub new_account: Pubkey,
    pub changed_by: Pubkey,
    pub slot: u64,
}

/// Emitted by `supply_flow` when it queues a supply or withdrawal (Task 8).
/// The request is a public argument of the computation in v1 (see ADR-0002).
#[event]
pub struct SupplyFlowQueued {
    pub market: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub pool_state: Pubkey,
    pub operation_intent: Pubkey,
    pub is_withdraw: bool,
    pub requested: u128,
    pub computation_offset: u64,
    pub position_generation: u64,
    pub pool_generation: u64,
    pub accounting_borrow_index: u128,
    pub decision_expiry_slot: u64,
    pub lock_expiry_slot: u64,
    pub slot: u64,
}

/// Emitted by the `supply_flow` callback when both payloads were stored.
/// Carries no balance, share, or claim amount.
#[event]
pub struct SupplyFlowApplied {
    pub position: Pubkey,
    pub pool_state: Pubkey,
    pub operation_intent: Pubkey,
    pub is_withdraw: bool,
    pub position_generation: u64,
    pub pool_generation: u64,
    pub accounted_borrow_index: u128,
    pub slot: u64,
}

/// Emitted when the request resolved `Zero`, `Stale`, `Expired`, or `Failed`.
/// Nothing was stored; both Locks were released.
#[event]
pub struct SupplyFlowNotApplied {
    pub position: Pubkey,
    pub pool_state: Pubkey,
    pub operation_intent: Pubkey,
    pub intent_status: crate::operation::IntentStatus,
    pub slot: u64,
}

/// Emitted by `abandon_supply_flow` (any signer, after Lock expiry).
#[event]
pub struct SupplyFlowAbandoned {
    pub position: Pubkey,
    pub pool_state: Pubkey,
    pub operation_intent: Pubkey,
    pub signer: Pubkey,
    pub slot: u64,
}