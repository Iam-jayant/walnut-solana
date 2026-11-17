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
