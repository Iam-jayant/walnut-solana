//! Walnut-Solana program.
//!
//! This is the Arcium/Anchor workspace scaffold plus the Task 2 feasibility
//! gate. It contains the standard `add_together` hello-world computation used
//! to verify the toolchain (Task 1.5) and the Task 2.2 persistent encrypted
//! Position state spike (`init_position` / `apply_position_delta`). No Walnut
//! financial logic is added until the Arcium feasibility gate (Task 2) passes.
//!
//! Task 4.1 adds the first production instructions: `initialize_market` and
//! the `init_pool_state` computation that seals the zero Pool payload. Task
//! 4.2 adds the role instructions (two-step admin transfer and the separated
//! upgrade, risk, treasury, and guardian authorities).
//!
//! Module directories reserved by the design: `state/`, `instructions/`,
//! `math/`, `oracle/`, `operation/`, `lock/`.

// Clippy's `diverging_sub_expression` is a false-positive in the code generated
// by the Arcium `#[arcium_program]` macro. The lint is reported at the module
// declaration (crate-root scope), so it must be allowed crate-wide here rather
// than with an attribute on the module.
#![allow(clippy::diverging_sub_expression)]

use anchor_lang::prelude::*;
use arcium_anchor::prelude::*;
use arcium_client::idl::arcium::types::CallbackAccount;

pub mod errors;
pub mod events;

// Coordination scaffolding proven by the Task 2.1 Arcium feasibility gate.
// These modules carry no financial state; they only exercise the generation /
// Lock / Operation Intent lifecycle. The design reserves the directories
// `state/`, `lock/`, `operation/`, `instructions/`, `math/`, and `oracle/`.
pub mod lock;
pub mod operation;
pub mod state;

// Task 4: Market, Asset State, vaults, and governance. Account layouts and
// pure validation live in `market/`; instruction account structs in
// `instructions/` (glob re-exported so the program macro finds them).
pub mod instructions;
pub mod market;

// Task 6: the committed arithmetic range table (`math/`) and Pyth oracle
// validation plus anomaly controls (`oracle/`). Both are pure, host-tested
// logic the price-sensitive instructions (Task 9+) build on.
pub mod math;
pub mod oracle;

pub use instructions::caps::*;
pub use instructions::config::*;
pub use instructions::disclose::*;
pub use instructions::initialize_market::*;
pub use instructions::lend_op::*;
pub use instructions::lending_pool::*;
pub use instructions::pause::*;
pub use instructions::position::*;
pub use instructions::position_op::*;
pub use instructions::private_op::*;
pub use instructions::recovery::*;
pub use instructions::reserve_withdrawal::*;
pub use instructions::roles::*;
pub use instructions::rotation::*;
pub use instructions::shield::*;
pub use instructions::supply::*;
pub use instructions::unshield::*;

// Task 2.2 Arcium feasibility gate: persistent encrypted Position state. This
// account-backed module proves the complete encrypted Position payload can be
// stored, reread by a later computation, updated, and locally decrypted. It is
// feasibility scaffolding, not the production Position (Task 5).
pub mod position;

use anchor_spl::token::{self as spl, Mint, Token, TokenAccount, Transfer, TransferChecked};
use errors::ErrorCode;
use events::{
    DisclosureNotStored, DisclosureQueued, HealthChecked, LendOpApplied, LendOpQueued,
    LendOpRejected, LiquidationApplied, PositionDisclosed,
};
use events::{EmptyPoolRecovered, EmptyPoolRecoveryNotApplied, EmptyPoolRecoveryQueued};
use events::{
    LiquidationReceiptsEmitted, OversizedOutputEmitted, PoolStateActivated, PositionActivated,
    PositionCountChanged, PositionInitFailed, PositionInitQueued, PositionStateStored,
    ShieldCreditFailed, ShieldCredited, ShieldQueued, ShieldRetried, SumEvent, UnshieldApproved,
    UnshieldFailed, UnshieldQueued, UnshieldZero, ViewingKeyRotated,
};
use events::{PositionOpApplied, PositionOpQueued, PositionOpRejected};
use events::{PrivateOpQueued, PrivateOpSettled};
use events::{ReserveWithdrawalApproved, ReserveWithdrawalNotApplied, ReserveWithdrawalQueued};
use events::{SupplyFlowApplied, SupplyFlowNotApplied, SupplyFlowQueued};
use events::{UtilizationBandNotApplied, UtilizationBandQueued, UtilizationBandUpdated};
use events::{ViewingKeyRotationApplied, ViewingKeyRotationFailed, ViewingKeyRotationQueued};
use market::{
    check_pool_initializing, AssetState, Market, VaultAuthority, MARKET_SEED, POOL_STATE_SEED,
    VAULT_AUTHORITY_SEED,
};
use math::lend::{
    OP_BORROW, OP_HEALTH_CHECK, OP_LIQUIDATE, OP_WITHDRAW_COLLATERAL, OP_WITHDRAW_SUPPLY,
};
use operation::{OperationCounter, OperationIntent, OPERATION_COUNTER_SEED, OPERATION_INTENT_SEED};
use position::{EncryptedPositionState, SpikePosition, POSITION_SECRET_FIELDS};
use state::band::{BandKeys, BandReport, BandRequest, BandSettlement};
use state::disclosure::{
    DiscloseReport, DiscloseRequest, DiscloseSettlement, Disclosure, DISCLOSURE_SEED,
};
use state::lend_op::{
    LendParams, LendReport, LendRequest, LendSettlement, LiquidationKeys, MAX_POSITION_DEBT_CAP,
};
use state::lending_pool::{LendingPool, LENDING_POOL_SEED};
use state::pending_shield::{NewPendingShield, PENDING_SHIELD_SEED};
use state::position::{
    NewPosition, PositionInitCallback, PositionInitRequest, PositionInitSettlement,
    POSITION_ACCOUNT_SEED,
};
use state::position_op::{PositionOpReport, PositionOpRequest, PositionOpSettlement};
use state::private_op::{
    PrivateKeys, PrivateParams, PrivateReport, PrivateRequest, SealedPosition,
};
use state::recovery::{
    RecoverEmptyPoolReport, RecoverEmptyPoolRequest, RecoverEmptyPoolSettlement,
};
use state::reserve_withdrawal::{
    ReserveWithdrawalReport, ReserveWithdrawalRequest, ReserveWithdrawalSettlement,
};
use state::rotation::{RotationReport, RotationRequest, RotationSettlement};
use state::shield::{ShieldCallback, ShieldRequest, ShieldSettlement, TokenAccountView};
use state::supply::{SupplyFlowReport, SupplyFlowRequest, SupplyFlowSettlement};
use state::unshield::{UnshieldReport, UnshieldRequest, UnshieldSettlement};
use state::{EncryptedPoolState, PendingShield, PoolState, Position, ShieldAsset};

const COMP_DEF_OFFSET_SHIELD_CREDIT: u32 = comp_def_offset("shield_credit");
const COMP_DEF_OFFSET_UNSHIELD: u32 = comp_def_offset("unshield");
const COMP_DEF_OFFSET_POSITION_OP: u32 = comp_def_offset("position_op");
const COMP_DEF_OFFSET_ROTATE_POSITION_KEY: u32 = comp_def_offset("rotate_position_key");
const COMP_DEF_OFFSET_ADD_TOGETHER: u32 = comp_def_offset("add_together");
const COMP_DEF_OFFSET_INIT_POSITION: u32 = comp_def_offset("init_position");
const COMP_DEF_OFFSET_APPLY_POSITION_DELTA: u32 = comp_def_offset("apply_position_delta");
const COMP_DEF_OFFSET_LIQUIDATION_RECEIPTS: u32 = comp_def_offset("liquidation_receipts");
const COMP_DEF_OFFSET_ROTATE_VIEWING_KEY: u32 = comp_def_offset("rotate_viewing_key");
const COMP_DEF_OFFSET_OVERSIZED_OUTPUT: u32 = comp_def_offset("oversized_output");
const COMP_DEF_OFFSET_INIT_POOL_STATE: u32 = comp_def_offset("init_pool_state");
const COMP_DEF_OFFSET_INIT_POSITION_STATE: u32 = comp_def_offset("init_position_state");
const COMP_DEF_OFFSET_UPDATE_UTILIZATION_BAND: u32 = comp_def_offset("update_utilization_band");
const COMP_DEF_OFFSET_RESERVE_WITHDRAWAL: u32 = comp_def_offset("reserve_withdrawal");
const COMP_DEF_OFFSET_SUPPLY_FLOW: u32 = comp_def_offset("supply_flow");
const COMP_DEF_OFFSET_RECOVER_EMPTY_POOL: u32 = comp_def_offset("recover_empty_pool");
const COMP_DEF_OFFSET_LEND_OP: u32 = comp_def_offset("lend_op");
const COMP_DEF_OFFSET_DISCLOSE_POSITION: u32 = comp_def_offset("disclose_position");
const COMP_DEF_OFFSET_POOL_OP: u32 = comp_def_offset("pool_op");
const COMP_DEF_OFFSET_POSITION_PRIVATE: u32 = comp_def_offset("position_private");
const COMP_DEF_OFFSET_LIQUIDATE_POOL: u32 = comp_def_offset("liquidate_pool");
const COMP_DEF_OFFSET_POOL_BAND: u32 = comp_def_offset("pool_band");

/// PDA seed for the Task 2.2 `SpikePosition` account: one per owner.
const POSITION_SEED: &[u8] = b"spike_position";

declare_id!("5Qyybr5DfcVezNa7tQwUx7sEwNdArqAKv9R3fy9o6vqc");

#[arcium_program]
pub mod walnut {
    use super::*;

    /// Registers the `add_together` computation definition with the MXE.
    pub fn init_add_together_comp_def(ctx: Context<InitAddTogetherCompDef>) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Queues `add_together` with two encrypted `u8` inputs.
    pub fn add_together(
        ctx: Context<AddTogether>,
        computation_offset: u64,
        ciphertext_0: [u8; 32],
        ciphertext_1: [u8; 32],
        pub_key: [u8; 32],
        nonce: u128,
    ) -> Result<()> {
        // Argument order must match the circuit signature Enc<Shared, InputValues>:
        // pubkey, nonce, then ciphertexts.
        let args = ArgBuilder::new()
            .x25519_pubkey(pub_key)
            .plaintext_u128(nonce)
            .encrypted_u8(ciphertext_0)
            .encrypted_u8(ciphertext_1)
            .build();

        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;

        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![AddTogetherCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &[],
            )?],
            1,
            0, // cu_price_micro: priority fee in microlamports
            0, // callback_cu_limit: 0 derives the limit from simulation
        )?;
        Ok(())
    }

    /// Verifies the MPC output and emits the encrypted sum.
    #[arcium_callback(encrypted_ix = "add_together")]
    pub fn add_together_callback(
        ctx: Context<AddTogetherCallback>,
        output: SignedComputationOutputs<AddTogetherOutput>,
    ) -> Result<()> {
        let o = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(AddTogetherOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::AbortedComputation.into()),
        };

        emit!(SumEvent {
            sum: o.ciphertexts[0],
            nonce: o.nonce.to_le_bytes(),
        });
        Ok(())
    }

    // ---- Task 2.4: callback compute-unit exhaustion -----------------------

    /// Queues `add_together` (reusing its circuit, accounts, and callback) but
    /// with a caller-supplied `callback_cu_limit`. A deliberately tiny non-zero
    /// limit makes the callback run out of compute units and the callback
    /// transaction abort, so nothing is committed (Requirement 2.4,
    /// compute-unit exhaustion). A `callback_cu_limit` of `0` would instead let
    /// Arcium derive the limit from simulation and retry on exhaustion, so the
    /// test must pass a small fixed value such as `1`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_together_bounded_cu(
        ctx: Context<AddTogether>,
        computation_offset: u64,
        ciphertext_0: [u8; 32],
        ciphertext_1: [u8; 32],
        pub_key: [u8; 32],
        nonce: u128,
        callback_cu_limit: u64,
    ) -> Result<()> {
        let args = ArgBuilder::new()
            .x25519_pubkey(pub_key)
            .plaintext_u128(nonce)
            .encrypted_u8(ciphertext_0)
            .encrypted_u8(ciphertext_1)
            .build();

        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;

        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![AddTogetherCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &[],
            )?],
            1,
            0,
            // A fixed, non-zero limit: Arcium does not retry it on exhaustion.
            callback_cu_limit as u32,
        )?;
        Ok(())
    }

    // ---- Task 2.4: OutputTooLarge -----------------------------------------

    /// Registers the `oversized_output` computation definition with the MXE.
    pub fn init_oversized_output_comp_def(ctx: Context<InitOversizedOutputCompDef>) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Queues `oversized_output`: one encrypted `u64` input, a 48-field output.
    /// The sealed output is too large for the node to fit in a single callback
    /// transaction, so the computation fails with `OutputTooLarge` and the
    /// success callback below never runs (Requirement 2.4).
    pub fn oversized_output(
        ctx: Context<OversizedOutput>,
        computation_offset: u64,
        ciphertext: [u8; 32],
        pub_key: [u8; 32],
        nonce: u128,
    ) -> Result<()> {
        // Argument order must match `oversized_output(input: Enc<Shared, u64>)`:
        // pubkey, nonce, then the single ciphertext.
        let args = ArgBuilder::new()
            .x25519_pubkey(pub_key)
            .plaintext_u128(nonce)
            .encrypted_u64(ciphertext)
            .build();

        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;

        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![OversizedOutputCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &[],
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Success callback for `oversized_output`. It is expected never to run
    /// because the computation fails with `OutputTooLarge`; the localnet test
    /// asserts `OversizedOutputEmitted` is not observed. The oversized
    /// ciphertexts are ignored; only the nonce is relayed.
    #[arcium_callback(encrypted_ix = "oversized_output")]
    pub fn oversized_output_callback(
        ctx: Context<OversizedOutputCallback>,
        output: SignedComputationOutputs<OversizedOutputOutput>,
    ) -> Result<()> {
        let o = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(OversizedOutputOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::AbortedComputation.into()),
        };

        emit!(OversizedOutputEmitted {
            nonce: o.nonce.to_le_bytes(),
        });
        Ok(())
    }

    // ---- Task 2.2: persistent encrypted Position state --------------------

    /// Registers the `init_position` computation definition with the MXE.
    pub fn init_position_comp_def(ctx: Context<InitPositionCompDef>) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Registers the `apply_position_delta` computation definition with the MXE.
    pub fn apply_position_delta_comp_def(ctx: Context<ApplyPositionDeltaCompDef>) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Queues `init_position`: encrypts the complete Position payload once and
    /// persists the resulting ciphertext into a freshly created Position
    /// account via the callback. The five ciphertexts are the client-encrypted
    /// `PositionSecret` scalars in canonical order.
    pub fn init_position(
        ctx: Context<InitPosition>,
        computation_offset: u64,
        ciphertexts: [[u8; 32]; 5],
        pub_key: [u8; 32],
        nonce: u128,
    ) -> Result<()> {
        // Argument order must match `init_position(initial: Enc<Shared,
        // PositionSecret>)`: pubkey, nonce, then one ciphertext per scalar in
        // PositionSecret field order (u64, u64, u64, u128, u128).
        let args = ArgBuilder::new()
            .x25519_pubkey(pub_key)
            .plaintext_u128(nonce)
            .encrypted_u64(ciphertexts[0])
            .encrypted_u64(ciphertexts[1])
            .encrypted_u64(ciphertexts[2])
            .encrypted_u128(ciphertexts[3])
            .encrypted_u128(ciphertexts[4])
            .build();

        let position_key = ctx.accounts.position.key();
        ctx.accounts.position.owner = ctx.accounts.payer.key();
        ctx.accounts.position.viewing_pubkey = pub_key;
        ctx.accounts.position.viewing_key_epoch = 0;
        ctx.accounts.position.revision = 0;
        ctx.accounts.position.state = EncryptedPositionState {
            nonce: 0,
            ciphertexts: [[0u8; 32]; POSITION_SECRET_FIELDS],
        };
        ctx.accounts.position.bump = ctx.bumps.position;
        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;

        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![InitPositionCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &[CallbackAccount {
                    pubkey: position_key,
                    is_writable: true,
                }],
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Persists the output of `init_position` as the account's first encrypted
    /// Position revision.
    #[arcium_callback(encrypted_ix = "init_position")]
    pub fn init_position_callback(
        ctx: Context<InitPositionCallback>,
        output: SignedComputationOutputs<InitPositionOutput>,
    ) -> Result<()> {
        let o = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(InitPositionOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::AbortedComputation.into()),
        };

        let position_key = ctx.accounts.position.key();
        ctx.accounts.position.viewing_pubkey = o.encryption_key;
        ctx.accounts.position.state = EncryptedPositionState {
            nonce: o.nonce,
            ciphertexts: o.ciphertexts,
        };
        let revision = ctx
            .accounts
            .position
            .revision
            .checked_add(1)
            .ok_or(ErrorCode::AbortedComputation)?;
        ctx.accounts.position.revision = revision;

        emit!(PositionStateStored {
            position: position_key,
            revision,
            nonce: o.nonce.to_le_bytes(),
        });
        Ok(())
    }

    /// Queues `apply_position_delta`: reads the Position payload already stored
    /// in the account (the account-backed encrypted input), applies a
    /// caller-supplied per-field delta inside MPC, and persists the updated
    /// ciphertext via the callback. Running this repeatedly proves the
    /// encrypted state persists and is consumable across computations.
    ///
    /// Transaction-envelope note: here the stored state ciphertext is re-fed
    /// through `ArgBuilder` instruction data (five 32-byte chunks), which is
    /// small enough for the gate. The size-optimized alternative that keeps the
    /// stored ciphertext out of the transaction is `ArgBuilder::packed_account`
    /// (v0.16.0); Task 2.5 must measure and select between them.
    pub fn apply_position_delta(
        ctx: Context<ApplyPositionDelta>,
        computation_offset: u64,
        delta_ciphertexts: [[u8; 32]; 5],
        delta_pub_key: [u8; 32],
        delta_nonce: u128,
    ) -> Result<()> {
        // Copy the stored payload out of the account so no borrow of
        // `ctx.accounts` is held while queueing.
        let position_key = ctx.accounts.position.key();
        let state_pubkey = ctx.accounts.position.viewing_pubkey;
        let state_nonce = ctx.accounts.position.state.nonce;
        let state_ciphertexts = ctx.accounts.position.state.ciphertexts;

        // Argument order must match `apply_position_delta(state: Enc<Shared,
        // PositionSecret>, delta: Enc<Shared, PositionDelta>)`: the stored state
        // (pubkey, nonce, five ciphertexts) first, then the delta (pubkey,
        // nonce, five ciphertexts), each in PositionSecret field order.
        let args = ArgBuilder::new()
            .x25519_pubkey(state_pubkey)
            .plaintext_u128(state_nonce)
            .encrypted_u64(state_ciphertexts[0])
            .encrypted_u64(state_ciphertexts[1])
            .encrypted_u64(state_ciphertexts[2])
            .encrypted_u128(state_ciphertexts[3])
            .encrypted_u128(state_ciphertexts[4])
            .x25519_pubkey(delta_pub_key)
            .plaintext_u128(delta_nonce)
            .encrypted_u64(delta_ciphertexts[0])
            .encrypted_u64(delta_ciphertexts[1])
            .encrypted_u64(delta_ciphertexts[2])
            .encrypted_u128(delta_ciphertexts[3])
            .encrypted_u128(delta_ciphertexts[4])
            .build();

        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;

        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![ApplyPositionDeltaCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &[CallbackAccount {
                    pubkey: position_key,
                    is_writable: true,
                }],
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Persists the updated Position payload produced by
    /// `apply_position_delta`, replacing the previous encrypted revision.
    #[arcium_callback(encrypted_ix = "apply_position_delta")]
    pub fn apply_position_delta_callback(
        ctx: Context<ApplyPositionDeltaCallback>,
        output: SignedComputationOutputs<ApplyPositionDeltaOutput>,
    ) -> Result<()> {
        let o = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(ApplyPositionDeltaOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::AbortedComputation.into()),
        };

        let position_key = ctx.accounts.position.key();
        ctx.accounts.position.viewing_pubkey = o.encryption_key;
        ctx.accounts.position.state = EncryptedPositionState {
            nonce: o.nonce,
            ciphertexts: o.ciphertexts,
        };
        let revision = ctx
            .accounts
            .position
            .revision
            .checked_add(1)
            .ok_or(ErrorCode::AbortedComputation)?;
        ctx.accounts.position.revision = revision;

        emit!(PositionStateStored {
            position: position_key,
            revision,
            nonce: o.nonce.to_le_bytes(),
        });
        Ok(())
    }

    // ---- Task 2.3: two-owner output and Viewing Key rotation --------------

    /// Registers the `liquidation_receipts` computation definition with the MXE.
    pub fn init_liquidation_receipts_comp_def(
        ctx: Context<InitLiquidationReceiptsCompDef>,
    ) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Queues `liquidation_receipts`: one computation that reads a borrower and
    /// a liquidator encrypted input and seals an independent encrypted Receipt
    /// to each owner's Viewing Key. The callback emits both ciphertext groups so
    /// each owner decrypts only their own Receipt (Requirement 2.3).
    // Arcium queue instructions take ciphertexts + pubkey + nonce per owner; two
    // owners exceed clippy's 7-arg default. Grouping would fight the ArgBuilder shape.
    #[allow(clippy::too_many_arguments)]
    pub fn liquidation_receipts(
        ctx: Context<LiquidationReceipts>,
        computation_offset: u64,
        borrower_ciphertexts: [[u8; 32]; 2],
        borrower_pub_key: [u8; 32],
        borrower_nonce: u128,
        liquidator_ciphertexts: [[u8; 32]; 2],
        liquidator_pub_key: [u8; 32],
        liquidator_nonce: u128,
    ) -> Result<()> {
        // Argument order must match `liquidation_receipts(borrower: Enc<Shared,
        // ReceiptInput>, liquidator: Enc<Shared, ReceiptInput>)`: each owner's
        // pubkey, nonce, then its two ciphertexts (amount, aux) in order.
        let args = ArgBuilder::new()
            .x25519_pubkey(borrower_pub_key)
            .plaintext_u128(borrower_nonce)
            .encrypted_u64(borrower_ciphertexts[0])
            .encrypted_u64(borrower_ciphertexts[1])
            .x25519_pubkey(liquidator_pub_key)
            .plaintext_u128(liquidator_nonce)
            .encrypted_u64(liquidator_ciphertexts[0])
            .encrypted_u64(liquidator_ciphertexts[1])
            .build();

        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;

        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![LiquidationReceiptsCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &[],
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Verifies the two-owner output and emits each owner's encrypted Receipt.
    /// The generated output wraps the returned tuple as `field_0`, whose
    /// `field_0` is the borrower's `SharedEncryptedStruct<2>` and `field_1` is
    /// the liquidator's, each carrying its own effective key and nonce.
    #[arcium_callback(encrypted_ix = "liquidation_receipts")]
    pub fn liquidation_receipts_callback(
        ctx: Context<LiquidationReceiptsCallback>,
        output: SignedComputationOutputs<LiquidationReceiptsOutput>,
    ) -> Result<()> {
        let o = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(LiquidationReceiptsOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::AbortedComputation.into()),
        };

        emit!(LiquidationReceiptsEmitted {
            borrower_key: o.field_0.encryption_key,
            borrower_nonce: o.field_0.nonce.to_le_bytes(),
            borrower_ciphertexts: o.field_0.ciphertexts,
            liquidator_key: o.field_1.encryption_key,
            liquidator_nonce: o.field_1.nonce.to_le_bytes(),
            liquidator_ciphertexts: o.field_1.ciphertexts,
        });
        Ok(())
    }

    /// Registers the `rotate_viewing_key` computation definition with the MXE.
    pub fn init_rotate_viewing_key_comp_def(
        ctx: Context<InitRotateViewingKeyCompDef>,
    ) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Queues `rotate_viewing_key`: reads the Position payload stored under the
    /// current Viewing Key and re-encrypts it to `new_pub_key`. The Position PDA
    /// is seeded by the owner, so only the owner's signature can rotate the key
    /// (Requirement 5.11). `probe_ciphertext` is a throwaway value encrypted
    /// under the new key that only carries the new key into the circuit.
    pub fn rotate_viewing_key(
        ctx: Context<RotateViewingKey>,
        computation_offset: u64,
        new_pub_key: [u8; 32],
        probe_nonce: u128,
        probe_ciphertext: [u8; 32],
    ) -> Result<()> {
        // Copy the stored payload out before queueing so no borrow is held.
        let position_key = ctx.accounts.position.key();
        let state_pubkey = ctx.accounts.position.viewing_pubkey;
        let state_nonce = ctx.accounts.position.state.nonce;
        let state_ciphertexts = ctx.accounts.position.state.ciphertexts;

        // Argument order must match `rotate_viewing_key(state: Enc<Shared,
        // PositionSecret>, new_key: Enc<Shared, u64>)`: the old-key state
        // (pubkey, nonce, five ciphertexts) first, then the new-key probe
        // (pubkey, nonce, one ciphertext).
        let args = ArgBuilder::new()
            .x25519_pubkey(state_pubkey)
            .plaintext_u128(state_nonce)
            .encrypted_u64(state_ciphertexts[0])
            .encrypted_u64(state_ciphertexts[1])
            .encrypted_u64(state_ciphertexts[2])
            .encrypted_u128(state_ciphertexts[3])
            .encrypted_u128(state_ciphertexts[4])
            .x25519_pubkey(new_pub_key)
            .plaintext_u128(probe_nonce)
            .encrypted_u64(probe_ciphertext)
            .build();

        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;

        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![RotateViewingKeyCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &[CallbackAccount {
                    pubkey: position_key,
                    is_writable: true,
                }],
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Atomically re-encrypts the Position to the new Viewing Key, writes the
    /// new public key, and increments `viewing_key_epoch` (Requirement 5.11).
    /// After this callback the stored payload decrypts under the new key only.
    #[arcium_callback(encrypted_ix = "rotate_viewing_key")]
    pub fn rotate_viewing_key_callback(
        ctx: Context<RotateViewingKeyCallback>,
        output: SignedComputationOutputs<RotateViewingKeyOutput>,
    ) -> Result<()> {
        let o = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(RotateViewingKeyOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::AbortedComputation.into()),
        };

        let position_key = ctx.accounts.position.key();
        let old_pubkey = ctx.accounts.position.viewing_pubkey;
        let old_epoch = ctx.accounts.position.viewing_key_epoch;
        let new_epoch = old_epoch
            .checked_add(1)
            .ok_or(ErrorCode::AbortedComputation)?;

        ctx.accounts.position.viewing_pubkey = o.encryption_key;
        ctx.accounts.position.viewing_key_epoch = new_epoch;
        ctx.accounts.position.state = EncryptedPositionState {
            nonce: o.nonce,
            ciphertexts: o.ciphertexts,
        };
        let revision = ctx
            .accounts
            .position
            .revision
            .checked_add(1)
            .ok_or(ErrorCode::AbortedComputation)?;
        ctx.accounts.position.revision = revision;

        emit!(ViewingKeyRotated {
            position: position_key,
            old_epoch,
            new_epoch,
            old_pubkey,
            new_pubkey: o.encryption_key,
            new_nonce: o.nonce.to_le_bytes(),
        });
        Ok(())
    }

    // ---- Task 4.1: market initialization ----------------------------------

    /// Creates the Market, Asset State, Pool State (`Initializing`), vault
    /// authority, wSOL and USDC vaults, and Recovery Position. Only the
    /// program upgrade authority may call it, and only once.
    pub fn initialize_market(
        ctx: Context<InitializeMarket>,
        args: InitializeMarketArgs,
    ) -> Result<()> {
        initialize_market_handler(ctx, args)
    }

    // ---- Task 4.2: roles ------------------------------------------------------

    /// Admin-signed: names `new_admin` as pending admin (overwrites any
    /// earlier proposal).
    pub fn propose_admin(ctx: Context<ProposeAdmin>, new_admin: Pubkey) -> Result<()> {
        propose_admin_handler(ctx, new_admin)
    }

    /// Pending-admin-signed: becomes admin and clears the pending admin.
    pub fn accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
        accept_admin_handler(ctx)
    }

    /// Admin-signed: clears the pending admin.
    pub fn cancel_admin_transfer(ctx: Context<CancelAdminTransfer>) -> Result<()> {
        cancel_admin_transfer_handler(ctx)
    }

    /// Admin-signed: sets the risk authority.
    pub fn set_risk_authority(ctx: Context<SetRoleAuthority>, new_authority: Pubkey) -> Result<()> {
        set_role_authority_handler(ctx, market::RoleKind::Risk, new_authority)
    }

    /// Admin-signed: sets the treasury authority.
    pub fn set_treasury_authority(
        ctx: Context<SetRoleAuthority>,
        new_authority: Pubkey,
    ) -> Result<()> {
        set_role_authority_handler(ctx, market::RoleKind::Treasury, new_authority)
    }

    /// Admin-signed: sets the guardian authority.
    pub fn set_guardian_authority(
        ctx: Context<SetRoleAuthority>,
        new_authority: Pubkey,
    ) -> Result<()> {
        set_role_authority_handler(ctx, market::RoleKind::Guardian, new_authority)
    }

    /// Admin-signed: records the program upgrade authority after it was
    /// rotated in the loader. The program-data account must name it.
    pub fn set_upgrade_authority(
        ctx: Context<SetUpgradeAuthority>,
        new_authority: Pubkey,
    ) -> Result<()> {
        set_upgrade_authority_handler(ctx, new_authority)
    }

    /// Guardian-signed: pauses the `market::pause_flags` bits in `flags` for
    /// `duration_slots` (at most `GUARDIAN_MAX_PAUSE_SLOTS`). Repay and
    /// collateral addition cannot be paused.
    pub fn guardian_pause(
        ctx: Context<UpdatePause>,
        flags: u16,
        duration_slots: u64,
    ) -> Result<()> {
        guardian_pause_handler(ctx, flags, duration_slots)
    }

    /// Risk-authority-signed: sets (`paused`) or clears durable pause bits.
    pub fn set_pause(ctx: Context<UpdatePause>, flags: u16, paused: bool) -> Result<()> {
        set_pause_handler(ctx, flags, paused)
    }

    /// Risk-authority-signed: extends the active guardian pause.
    pub fn renew_guardian_pause(ctx: Context<UpdatePause>, new_expiry_slot: u64) -> Result<()> {
        renew_guardian_pause_handler(ctx, new_expiry_slot)
    }

    /// Risk-authority-signed: clears every guardian pause.
    pub fn clear_guardian_pause(ctx: Context<UpdatePause>) -> Result<()> {
        clear_guardian_pause_handler(ctx)
    }

    // ---- Task 4.4: timelocked configuration ----------------------------------

    /// Risk-authority-signed: proposes one configuration group change.
    /// `proposal_id` must be `asset_state.timelock.next_proposal_id`.
    pub fn propose_config_change(
        ctx: Context<ProposeConfigChange>,
        proposal_id: u64,
        change: market::ConfigValue,
    ) -> Result<()> {
        propose_config_change_handler(ctx, proposal_id, change)
    }

    /// Permissionless: executes a pending proposal after the timelock delay
    /// and before its grace window ends; bumps the Asset State generation.
    pub fn execute_config_change(
        ctx: Context<ExecuteConfigChange>,
        proposal_id: u64,
    ) -> Result<()> {
        execute_config_change_handler(ctx, proposal_id)
    }

    /// Risk-authority- or guardian-signed: cancels a pending proposal.
    pub fn cancel_config_change(ctx: Context<CancelConfigChange>, proposal_id: u64) -> Result<()> {
        cancel_config_change_handler(ctx, proposal_id)
    }

    /// Risk-authority-signed (Task 4.5): lowers the USDC caps or the count
    /// caps immediately and bumps the Asset State generation. Raises need
    /// `propose_config_change`.
    pub fn decrease_caps(ctx: Context<DecreaseCaps>, change: market::ConfigValue) -> Result<()> {
        decrease_caps_handler(ctx, change)
    }

    /// Registers the `init_pool_state` computation definition with the MXE.
    pub fn init_pool_state_comp_def(ctx: Context<InitPoolStateCompDef>) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Queues `init_pool_state`, which seals the all-zero Pool payload to the
    /// MXE. Admin-only and only while the Pool is `Initializing`. Re-queueing
    /// after a failed computation is allowed; the first authenticated callback
    /// activates the Pool and later ones are no-ops.
    pub fn queue_pool_state_init(
        ctx: Context<QueuePoolStateInit>,
        computation_offset: u64,
    ) -> Result<()> {
        check_pool_initializing(ctx.accounts.pool_state.status)?;
        let pool_key = ctx.accounts.pool_state.key();
        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;

        // `init_pool_state()` takes no arguments.
        let args = ArgBuilder::new().build();
        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![InitPoolStateCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &[CallbackAccount {
                    pubkey: pool_key,
                    is_writable: true,
                }],
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Stores the encrypted zero Pool payload and activates the Pool. A
    /// callback for an already active Pool returns without writing.
    #[arcium_callback(encrypted_ix = "init_pool_state")]
    pub fn init_pool_state_callback(
        ctx: Context<InitPoolStateCallback>,
        output: SignedComputationOutputs<InitPoolStateOutput>,
    ) -> Result<()> {
        let o = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(InitPoolStateOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::AbortedComputation.into()),
        };

        let pool_key = ctx.accounts.pool_state.key();
        let activated = ctx.accounts.pool_state.activate(EncryptedPoolState {
            nonce: o.nonce,
            ciphertexts: o.ciphertexts,
        })?;
        if activated {
            emit!(PoolStateActivated {
                pool_state: pool_key,
                generation: ctx.accounts.pool_state.coordination.generation,
                nonce: o.nonce.to_le_bytes(),
            });
        }
        Ok(())
    }

    // ---- Task 5.1: production Position initialization ---------------------

    /// Registers the `init_position_state` computation definition with the MXE.
    pub fn init_position_state_comp_def(ctx: Context<InitPositionStateCompDef>) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Owner-signed: creates the owner's Position in `Initializing` with the
    /// Viewing Key registered at epoch zero, enforces the Position count cap
    /// (incrementing `position_count`), creates the Operation Intent at
    /// `sequence`, Locks the Position, and queues `init_position_state`, which
    /// seals the zero `PositionSecret` to `viewing_pubkey`. `nonce` is a fresh
    /// random client nonce for the `Shared` input; no ciphertext is sent.
    pub fn initialize_position(
        ctx: Context<InitializePosition>,
        computation_offset: u64,
        sequence: u64,
        viewing_pubkey: [u8; 32],
        nonce: u128,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let owner = ctx.accounts.payer.key();
        let market_key = ctx.accounts.market.key();
        let position_key = ctx.accounts.position.key();
        let intent_key = ctx.accounts.operation_intent.key();
        if ctx.accounts.operation_counter.owner == Pubkey::default() {
            ctx.accounts
                .operation_counter
                .set_inner(OperationCounter::new(owner, ctx.bumps.operation_counter));
        }
        let request = PositionInitRequest {
            intent_key,
            position_key,
            sequence,
            computation_offset,
            nonce,
            queued_slot: clock.slot,
        };
        let new = NewPosition {
            owner,
            market: market_key,
            bump: ctx.bumps.position,
            viewing_pubkey,
        };
        let accounts = &mut *ctx.accounts;
        let done = state::position::initialize_position(
            &mut accounts.market,
            &mut accounts.position,
            &mut accounts.operation_counter,
            &new,
            &request,
        )?;
        let mut intent = done.intent;
        intent.bump = ctx.bumps.operation_intent;
        accounts.operation_intent.set_inner(intent);

        emit!(PositionCountChanged {
            market: market_key,
            position: position_key,
            old_count: done.count.old_count,
            new_count: done.count.new_count,
            cap: done.count.cap,
            slot: clock.slot,
        });
        emit!(PositionInitQueued {
            market: market_key,
            position: position_key,
            owner,
            operation_intent: intent_key,
            sequence,
            computation_offset,
            viewing_pubkey,
            viewing_key_epoch: 0,
            position_generation: accounts.position.coordination.generation,
            decision_expiry_slot: intent.decision_expiry_slot,
            lock_expiry_slot: intent.lock_expiry_slot,
            retry: false,
            slot: clock.slot,
        });

        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;
        // `init_position_state(owner: Shared)`: the Viewing public key and the
        // input nonce, no ciphertext.
        let args = ArgBuilder::new()
            .x25519_pubkey(viewing_pubkey)
            .plaintext_u128(nonce)
            .build();
        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![InitPositionStateCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &[
                    CallbackAccount {
                        pubkey: position_key,
                        is_writable: true,
                    },
                    CallbackAccount {
                        pubkey: intent_key,
                        is_writable: true,
                    },
                ],
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Owner-signed: retries a failed initialization (`InitFailed`, Lock free)
    /// with a new Operation Intent at `sequence` and a fresh computation
    /// offset and nonce. The registered Viewing Key and epoch are reused.
    pub fn retry_position_init(
        ctx: Context<RetryPositionInit>,
        computation_offset: u64,
        sequence: u64,
        nonce: u128,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let position_key = ctx.accounts.position.key();
        let intent_key = ctx.accounts.operation_intent.key();
        let request = PositionInitRequest {
            intent_key,
            position_key,
            sequence,
            computation_offset,
            nonce,
            queued_slot: clock.slot,
        };
        let accounts = &mut *ctx.accounts;
        let mut intent = state::position::retry_position_init(
            &mut accounts.position,
            &mut accounts.operation_counter,
            &request,
        )?;
        intent.bump = ctx.bumps.operation_intent;
        accounts.operation_intent.set_inner(intent);
        let viewing_pubkey = accounts.position.viewing_pubkey;

        emit!(PositionInitQueued {
            market: accounts.position.market,
            position: position_key,
            owner: accounts.position.owner,
            operation_intent: intent_key,
            sequence,
            computation_offset,
            viewing_pubkey,
            viewing_key_epoch: accounts.position.viewing_key_epoch,
            position_generation: accounts.position.coordination.generation,
            decision_expiry_slot: intent.decision_expiry_slot,
            lock_expiry_slot: intent.lock_expiry_slot,
            retry: true,
            slot: clock.slot,
        });

        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;
        let args = ArgBuilder::new()
            .x25519_pubkey(viewing_pubkey)
            .plaintext_u128(nonce)
            .build();
        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![InitPositionStateCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &[
                    CallbackAccount {
                        pubkey: position_key,
                        is_writable: true,
                    },
                    CallbackAccount {
                        pubkey: intent_key,
                        is_writable: true,
                    },
                ],
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Verifies the `init_position_state` output with the Task 3.3 helpers.
    /// `Applied` stores the zero state, bumps the generation, releases the
    /// Lock, and activates the Position. `Stale`/`Expired` release the Lock
    /// and leave the Position `InitFailed`. A terminal Intent is a no-op.
    /// Unauthenticated output (including Arcium's unsigned `Failure`) and any
    /// binding mismatch revert, so the Intent stays `Queued` for
    /// `abandon_position_init`.
    #[arcium_callback(encrypted_ix = "init_position_state")]
    pub fn init_position_state_callback(
        ctx: Context<InitPositionStateCallback>,
        output: SignedComputationOutputs<InitPositionStateOutput>,
    ) -> Result<()> {
        let o = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(InitPositionStateOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::CallbackOutputNotAuthenticated.into()),
        };

        // The computation account address encodes the offset; an account for
        // any other offset yields a different value, so `classify_callback`
        // rejects it as an offset mismatch.
        let intent_offset = ctx.accounts.operation_intent.computation_offset;
        let computation_offset = if ctx.accounts.computation_account.key()
            == derive_comp_pda!(intent_offset, ctx.accounts.mxe_account)
        {
            intent_offset
        } else {
            intent_offset.wrapping_add(1)
        };

        let clock = Clock::get()?;
        let position_key = ctx.accounts.position.key();
        let intent_key = ctx.accounts.operation_intent.key();
        let callback = PositionInitCallback {
            output_authenticated: true,
            computation_offset,
            intent_key,
            encryption_key: o.encryption_key,
            output_nonce: o.nonce,
            ciphertexts: o.ciphertexts,
            current_slot: clock.slot,
        };
        let accounts = &mut *ctx.accounts;
        let settlement = state::position::settle_position_init(
            &mut accounts.position,
            position_key,
            &mut accounts.operation_intent,
            &callback,
        )?;
        let position = &accounts.position;
        match settlement {
            PositionInitSettlement::Activated => emit!(PositionActivated {
                market: position.market,
                position: position_key,
                owner: position.owner,
                operation_intent: intent_key,
                generation: position.coordination.generation,
                nonce: o.nonce.to_le_bytes(),
                slot: clock.slot,
            }),
            PositionInitSettlement::Failed(intent_status) => emit!(PositionInitFailed {
                market: position.market,
                position: position_key,
                owner: position.owner,
                operation_intent: intent_key,
                intent_status,
                generation: position.coordination.generation,
                slot: clock.slot,
            }),
            PositionInitSettlement::Ignored => {}
        }
        Ok(())
    }

    /// Permissionless: abandons a queued initialization after Lock expiry;
    /// the Position becomes `InitFailed` (retryable or closable).
    pub fn abandon_position_init(ctx: Context<AbandonPositionInit>) -> Result<()> {
        abandon_position_init_handler(ctx)
    }

    /// Owner-signed: closes an `InitFailed` Position, returns its rent to the
    /// owner, and decrements `position_count`. `Active` Positions cannot
    /// close yet (needs a zero-state proof computation).
    pub fn close_position(ctx: Context<ClosePosition>) -> Result<()> {
        close_position_handler(ctx)
    }

    // ---- Task 5.3: shield --------------------------------------------------

    /// Registers the `shield_credit` computation definition with the MXE.
    pub fn init_shield_credit_comp_def(ctx: Context<InitShieldCreditCompDef>) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Owner-signed: moves exactly `amount` of `asset` from the owner's
    /// source token account into the Market vault (`transfer_checked`), then
    /// opens the Pending Shield at `shield_sequence` (`Deposited`, attempt 1),
    /// creates the Shield Intent at `sequence`, Locks the Position, and
    /// queues `shield_credit`. Pausable (`SHIELD`).
    ///
    /// SOL: the source is the owner's wSOL token account. The client wraps in
    /// the same transaction, before this instruction, with a System Program
    /// transfer into that account followed by SPL Token `SyncNative`.
    pub fn shield(
        ctx: Context<ShieldDeposit>,
        computation_offset: u64,
        sequence: u64,
        shield_sequence: u64,
        asset: ShieldAsset,
        amount: u64,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let owner = ctx.accounts.payer.key();
        let market_key = ctx.accounts.market.key();
        let position_key = ctx.accounts.position.key();
        let intent_key = ctx.accounts.operation_intent.key();
        let shield_key = ctx.accounts.pending_shield.key();
        let vault_key = ctx.accounts.vault.key();
        let source = TokenAccountView {
            key: ctx.accounts.source_token_account.key(),
            mint: ctx.accounts.source_token_account.mint,
            owner: ctx.accounts.source_token_account.owner,
        };
        state::shield::check_shield_accounts(
            &ctx.accounts.market,
            asset,
            &owner,
            &ctx.accounts.mint.key(),
            &vault_key,
            &source,
        )?;

        // 1. The exact public amount moves into the vault first.
        spl::transfer_checked(
            CpiContext::new(
                ctx.accounts.token_program.key(),
                TransferChecked {
                    from: ctx.accounts.source_token_account.to_account_info(),
                    mint: ctx.accounts.mint.to_account_info(),
                    to: ctx.accounts.vault.to_account_info(),
                    authority: ctx.accounts.payer.to_account_info(),
                },
            ),
            amount,
            ctx.accounts.mint.decimals,
        )?;

        // 2. Pending Shield, Intent, Lock.
        let new = NewPendingShield {
            owner,
            position: position_key,
            source_token_account: source.key,
            asset,
            amount,
            sequence: shield_sequence,
            bump: ctx.bumps.pending_shield,
        };
        let request = ShieldRequest {
            intent_key,
            sequence,
            computation_offset,
            queued_slot: clock.slot,
        };
        let accounts = &mut *ctx.accounts;
        let mut intent = state::shield::queue_shield(
            &accounts.market,
            market_key,
            &accounts.asset_state.pauses,
            &mut accounts.position,
            position_key,
            &mut accounts.operation_counter,
            &mut accounts.pending_shield,
            shield_key,
            &new,
            &request,
        )?;
        intent.bump = ctx.bumps.operation_intent;
        accounts.operation_intent.set_inner(intent);

        emit!(ShieldQueued {
            market: market_key,
            position: position_key,
            owner,
            pending_shield: shield_key,
            operation_intent: intent_key,
            asset,
            amount,
            shield_sequence,
            attempt_sequence: 1,
            computation_offset,
            source_token_account: source.key,
            vault: vault_key,
            position_generation: accounts.position.coordination.generation,
            decision_expiry_slot: intent.decision_expiry_slot,
            lock_expiry_slot: intent.lock_expiry_slot,
            slot: clock.slot,
        });

        // 3. Queue the encrypted credit.
        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;
        let output_nonce_input = state::nonce::derive_output_nonce_input(&position_key, sequence);
        let args =
            shield_credit_args(&ctx.accounts.position, output_nonce_input, asset, amount).build();
        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![ShieldCreditCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &shield_callback_accounts(market_key, position_key, shield_key, intent_key),
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Owner-signed and not pausable: queues a new credit attempt for a
    /// `Deposited` Pending Shield whose active attempt (`prior_intent`) ended
    /// without a credit. New Intent at `sequence` and new computation offset.
    pub fn retry_shield(
        ctx: Context<RetryShield>,
        computation_offset: u64,
        sequence: u64,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let owner = ctx.accounts.payer.key();
        let market_key = ctx.accounts.market.key();
        let position_key = ctx.accounts.position.key();
        let intent_key = ctx.accounts.operation_intent.key();
        let shield_key = ctx.accounts.pending_shield.key();
        let prior_key = ctx.accounts.prior_intent.key();
        let request = ShieldRequest {
            intent_key,
            sequence,
            computation_offset,
            queued_slot: clock.slot,
        };
        let accounts = &mut *ctx.accounts;
        let (mut intent, attempt_sequence) = state::shield::retry_shield(
            &accounts.market,
            market_key,
            &mut accounts.position,
            position_key,
            &mut accounts.operation_counter,
            &mut accounts.pending_shield,
            shield_key,
            &owner,
            &accounts.prior_intent,
            &request,
        )?;
        intent.bump = ctx.bumps.operation_intent;
        accounts.operation_intent.set_inner(intent);
        let asset = accounts.pending_shield.asset;
        let amount = accounts.pending_shield.amount;

        emit!(ShieldRetried {
            market: market_key,
            position: position_key,
            owner,
            pending_shield: shield_key,
            operation_intent: intent_key,
            previous_intent: prior_key,
            attempt_sequence,
            computation_offset,
            position_generation: accounts.position.coordination.generation,
            decision_expiry_slot: intent.decision_expiry_slot,
            lock_expiry_slot: intent.lock_expiry_slot,
            slot: clock.slot,
        });

        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;
        let output_nonce_input = state::nonce::derive_output_nonce_input(&position_key, sequence);
        let args =
            shield_credit_args(&ctx.accounts.position, output_nonce_input, asset, amount).build();
        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![ShieldCreditCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &shield_callback_accounts(market_key, position_key, shield_key, intent_key),
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Verifies the `shield_credit` output with the Task 3.3 helpers.
    /// `Applied` stores the credited state, bumps the generation, releases the
    /// Lock, and marks the Pending Shield `Credited`. `Zero` (overflow),
    /// `Stale`, and `Expired` release the Lock and leave it `Deposited`. A
    /// terminal Intent or an old attempt is a no-op. Unauthenticated output
    /// and any binding mismatch revert, so the Intent stays `Queued` for
    /// `abandon_shield`.
    #[arcium_callback(encrypted_ix = "shield_credit")]
    pub fn shield_credit_callback(
        ctx: Context<ShieldCreditCallback>,
        output: SignedComputationOutputs<ShieldCreditOutput>,
    ) -> Result<()> {
        let o = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(ShieldCreditOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::CallbackOutputNotAuthenticated.into()),
        };
        let sealed = o.field_0;
        let credited = o.field_1;

        let intent_offset = ctx.accounts.operation_intent.computation_offset;
        let computation_offset = if ctx.accounts.computation_account.key()
            == derive_comp_pda!(intent_offset, ctx.accounts.mxe_account)
        {
            intent_offset
        } else {
            intent_offset.wrapping_add(1)
        };

        let clock = Clock::get()?;
        let market_key = ctx.accounts.market.key();
        let position_key = ctx.accounts.position.key();
        let shield_key = ctx.accounts.pending_shield.key();
        let intent_key = ctx.accounts.operation_intent.key();
        let callback = ShieldCallback {
            output_authenticated: true,
            computation_offset,
            intent_key,
            encryption_key: sealed.encryption_key,
            output_nonce: sealed.nonce,
            ciphertexts: sealed.ciphertexts,
            credited,
            current_slot: clock.slot,
        };
        let accounts = &mut *ctx.accounts;
        let settlement = state::shield::settle_shield_credit(
            &accounts.market,
            market_key,
            &mut accounts.position,
            position_key,
            &mut accounts.pending_shield,
            shield_key,
            &mut accounts.operation_intent,
            &callback,
        )?;
        let owner = accounts.position.owner;
        match settlement {
            ShieldSettlement::Credited { asset, amount } => emit!(ShieldCredited {
                position: position_key,
                owner,
                pending_shield: shield_key,
                operation_intent: intent_key,
                asset,
                amount,
                generation: accounts.position.coordination.generation,
                nonce: sealed.nonce.to_le_bytes(),
                slot: clock.slot,
            }),
            ShieldSettlement::Failed(intent_status) => emit!(ShieldCreditFailed {
                position: position_key,
                owner,
                pending_shield: shield_key,
                operation_intent: intent_key,
                intent_status,
                slot: clock.slot,
            }),
            ShieldSettlement::Ignored => {}
        }
        Ok(())
    }

    /// Owner-signed and not pausable: refunds a `Deposited` Pending Shield
    /// whose active attempt ended without a credit; exactly the recorded
    /// amount leaves the vault, signed by the vault-authority PDA.
    pub fn refund_shield(ctx: Context<RefundShield>) -> Result<()> {
        refund_shield_handler(ctx)
    }

    /// Permissionless: abandons a queued shield attempt after Lock expiry.
    /// The Pending Shield stays `Deposited` (retryable or refundable).
    pub fn abandon_shield(ctx: Context<AbandonShield>) -> Result<()> {
        abandon_shield_handler(ctx)
    }

    // ---- Task 5.4: unshield ------------------------------------------------

    /// Registers the `unshield` computation definition with the MXE.
    pub fn init_unshield_comp_def(ctx: Context<InitUnshieldCompDef>) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Owner-signed: requests a public `amount` of `asset` back to
    /// `destination`, a token account of the asset's mint owned by the owner.
    /// Requires an `Active` Position with a free Lock and the `UNSHIELD`
    /// pause off. Creates the Unshield Intent at `sequence`, Locks the
    /// Position, and queues `unshield`. No token moves until the callback
    /// approves the amount.
    ///
    /// SOL: the destination is the owner's wSOL token account; the callback
    /// delivers wSOL, and native SOL unwrap is a separate owner-signed
    /// transaction (`buildUnwrapWsolTransaction` in the SDK).
    pub fn unshield(
        ctx: Context<UnshieldQueue>,
        computation_offset: u64,
        sequence: u64,
        asset: ShieldAsset,
        amount: u64,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let owner = ctx.accounts.payer.key();
        let market_key = ctx.accounts.market.key();
        let position_key = ctx.accounts.position.key();
        let intent_key = ctx.accounts.operation_intent.key();
        let destination = TokenAccountView {
            key: ctx.accounts.destination.key(),
            mint: ctx.accounts.destination.mint,
            owner: ctx.accounts.destination.owner,
        };
        let vault_key = state::shield::asset_vault(&ctx.accounts.market, asset);
        let request = UnshieldRequest {
            intent_key,
            sequence,
            computation_offset,
            queued_slot: clock.slot,
            asset,
            amount,
            destination: destination.key,
        };
        let accounts = &mut *ctx.accounts;
        let mut intent = state::unshield::queue_unshield(
            &accounts.market,
            market_key,
            &accounts.asset_state.pauses,
            &mut accounts.position,
            position_key,
            &mut accounts.operation_counter,
            &owner,
            &destination,
            &request,
        )?;
        intent.bump = ctx.bumps.operation_intent;
        accounts.operation_intent.set_inner(intent);

        emit!(UnshieldQueued {
            market: market_key,
            position: position_key,
            owner,
            operation_intent: intent_key,
            asset,
            amount,
            destination: destination.key,
            vault: vault_key,
            computation_offset,
            position_generation: accounts.position.coordination.generation,
            decision_expiry_slot: intent.decision_expiry_slot,
            lock_expiry_slot: intent.lock_expiry_slot,
            slot: clock.slot,
        });

        // `unshield(state, owner: Shared, is_usdc, amount)`.
        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;
        let output_nonce_input = state::nonce::derive_output_nonce_input(&position_key, sequence);
        let args = unshield_args(&ctx.accounts.position, output_nonce_input, asset, amount).build();
        let callback_accounts = unshield_callback_accounts(
            &ctx.accounts.market,
            asset,
            market_key,
            position_key,
            intent_key,
            destination.key,
        );
        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![UnshieldCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &callback_accounts,
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Verifies the `unshield` output with the Task 3.3 helpers.
    /// `Applied` (approved) stores the new state, bumps the generation,
    /// releases the Lock, and transfers exactly the approved amount from the
    /// vault to the committed destination, signed by the vault-authority PDA.
    /// `Zero` (insufficient balance), `Stale`, and `Expired` release the Lock
    /// and move nothing. A terminal Intent is a no-op. Unauthenticated output
    /// and any binding mismatch (including a substituted destination) revert,
    /// so the Intent stays `Queued` for `abandon_unshield`.
    #[arcium_callback(encrypted_ix = "unshield")]
    pub fn unshield_callback(
        ctx: Context<UnshieldCallback>,
        output: SignedComputationOutputs<UnshieldOutput>,
    ) -> Result<()> {
        let o = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(UnshieldOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::CallbackOutputNotAuthenticated.into()),
        };
        let sealed = o.field_0;

        let intent_offset = ctx.accounts.operation_intent.computation_offset;
        let computation_offset = if ctx.accounts.computation_account.key()
            == derive_comp_pda!(intent_offset, ctx.accounts.mxe_account)
        {
            intent_offset
        } else {
            intent_offset.wrapping_add(1)
        };

        let clock = Clock::get()?;
        let market_key = ctx.accounts.market.key();
        let position_key = ctx.accounts.position.key();
        let intent_key = ctx.accounts.operation_intent.key();
        let vault_key = ctx.accounts.vault.key();
        let report = UnshieldReport {
            output_authenticated: true,
            computation_offset,
            intent_key,
            encryption_key: sealed.encryption_key,
            output_nonce: sealed.nonce,
            ciphertexts: sealed.ciphertexts,
            approved_amount: o.field_1,
            requested_amount: o.field_2,
            destination: TokenAccountView {
                key: ctx.accounts.destination.key(),
                mint: ctx.accounts.destination.mint,
                owner: ctx.accounts.destination.owner,
            },
            mint: ctx.accounts.mint.key(),
            vault: vault_key,
            vault_authority: ctx.accounts.vault_authority.key(),
            token_program: ctx.accounts.token_program.key(),
            current_slot: clock.slot,
        };
        let accounts = &mut *ctx.accounts;
        let settlement = state::unshield::settle_unshield(
            &accounts.market,
            market_key,
            &mut accounts.position,
            position_key,
            &mut accounts.operation_intent,
            &report,
        )?;
        let owner = accounts.position.owner;
        match settlement {
            UnshieldSettlement::Approved {
                asset,
                amount,
                destination,
            } => {
                let bump = [accounts.market.vault_authority_bump];
                let seeds: &[&[u8]] = &[VAULT_AUTHORITY_SEED, market_key.as_ref(), &bump];
                let signer = &[seeds];
                spl::transfer_checked(
                    CpiContext::new_with_signer(
                        accounts.token_program.key(),
                        TransferChecked {
                            from: accounts.vault.to_account_info(),
                            mint: accounts.mint.to_account_info(),
                            to: accounts.destination.to_account_info(),
                            authority: accounts.vault_authority.to_account_info(),
                        },
                        signer,
                    ),
                    amount,
                    accounts.mint.decimals,
                )?;
                emit!(UnshieldApproved {
                    market: market_key,
                    position: position_key,
                    owner,
                    operation_intent: intent_key,
                    asset,
                    amount,
                    vault: vault_key,
                    destination,
                    generation: accounts.position.coordination.generation,
                    nonce: sealed.nonce.to_le_bytes(),
                    slot: clock.slot,
                });
            }
            UnshieldSettlement::Zero { asset } => emit!(UnshieldZero {
                position: position_key,
                owner,
                operation_intent: intent_key,
                asset,
                requested_amount: o.field_2,
                slot: clock.slot,
            }),
            UnshieldSettlement::Failed(intent_status) => emit!(UnshieldFailed {
                position: position_key,
                owner,
                operation_intent: intent_key,
                intent_status,
                slot: clock.slot,
            }),
            UnshieldSettlement::Ignored => {}
        }
        Ok(())
    }

    /// Permissionless: abandons a queued unshield after Lock expiry. Nothing
    /// moved, so there is nothing to refund.
    pub fn abandon_unshield(ctx: Context<AbandonUnshield>) -> Result<()> {
        abandon_unshield_handler(ctx)
    }

    // ---- walnut-diffn: position_op -------------------------------------------

    /// Registers the `position_op` computation definition with the MXE.
    pub fn init_position_op_comp_def(ctx: Context<InitPositionOpCompDef>) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Owner-signed demo operation on the encrypted Position: `op` 0 add
    /// collateral, 1 withdraw collateral, 2 borrow, 3 repay. `price` (USDC base
    /// units per SOL) is caller supplied (demo shortcut); the LTV comes from
    /// the Asset State. Pure encrypted state update, no token moves.
    #[allow(clippy::too_many_arguments)]
    pub fn position_op(
        ctx: Context<PositionOpQueue>,
        computation_offset: u64,
        sequence: u64,
        op: u8,
        amount: u64,
        price: u64,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let owner = ctx.accounts.payer.key();
        let market_key = ctx.accounts.market.key();
        let position_key = ctx.accounts.position.key();
        let intent_key = ctx.accounts.operation_intent.key();
        let ltv_bps = u64::from(ctx.accounts.asset_state.risk.ltv_bps);
        let request = PositionOpRequest {
            intent_key,
            sequence,
            computation_offset,
            queued_slot: clock.slot,
            op,
            amount,
        };
        let accounts = &mut *ctx.accounts;
        let mut intent = state::position_op::queue_position_op(
            &accounts.market,
            market_key,
            &mut accounts.position,
            position_key,
            &mut accounts.operation_counter,
            &owner,
            &request,
        )?;
        intent.bump = ctx.bumps.operation_intent;
        accounts.operation_intent.set_inner(intent);

        emit!(PositionOpQueued {
            market: market_key,
            position: position_key,
            owner,
            operation_intent: intent_key,
            op,
            amount,
            price,
            ltv_bps,
            computation_offset,
            position_generation: accounts.position.coordination.generation,
            decision_expiry_slot: intent.decision_expiry_slot,
            lock_expiry_slot: intent.lock_expiry_slot,
            slot: clock.slot,
        });

        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;
        let output_nonce_input = state::nonce::derive_output_nonce_input(&position_key, sequence);
        let args = position_op_args(
            &ctx.accounts.position,
            output_nonce_input,
            op,
            amount,
            price,
            ltv_bps,
        )
        .build();
        let callback_accounts = [
            CallbackAccount {
                pubkey: market_key,
                is_writable: false,
            },
            CallbackAccount {
                pubkey: position_key,
                is_writable: true,
            },
            CallbackAccount {
                pubkey: intent_key,
                is_writable: true,
            },
        ];
        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![PositionOpCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &callback_accounts,
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Verifies the `position_op` output. `Applied` stores the new state,
    /// bumps the generation and releases the Lock. A refusal (`Zero`),
    /// `Stale`, and `Expired` release the Lock and store nothing.
    /// Unauthenticated output and binding mismatches revert.
    #[arcium_callback(encrypted_ix = "position_op")]
    pub fn position_op_callback(
        ctx: Context<PositionOpCallback>,
        output: SignedComputationOutputs<PositionOpOutput>,
    ) -> Result<()> {
        let o = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(PositionOpOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::CallbackOutputNotAuthenticated.into()),
        };
        let sealed = o.field_0;

        let intent_offset = ctx.accounts.operation_intent.computation_offset;
        let computation_offset = if ctx.accounts.computation_account.key()
            == derive_comp_pda!(intent_offset, ctx.accounts.mxe_account)
        {
            intent_offset
        } else {
            intent_offset.wrapping_add(1)
        };

        let clock = Clock::get()?;
        let market_key = ctx.accounts.market.key();
        let position_key = ctx.accounts.position.key();
        let intent_key = ctx.accounts.operation_intent.key();
        let report = PositionOpReport {
            output_authenticated: true,
            computation_offset,
            intent_key,
            encryption_key: sealed.encryption_key,
            output_nonce: sealed.nonce,
            ciphertexts: sealed.ciphertexts,
            applied: o.field_1,
            op: o.field_2,
            amount: o.field_3,
            current_slot: clock.slot,
        };
        let accounts = &mut *ctx.accounts;
        let settlement = state::position_op::settle_position_op(
            &accounts.market,
            market_key,
            &mut accounts.position,
            position_key,
            &mut accounts.operation_intent,
            &report,
        )?;
        let owner = accounts.position.owner;
        match settlement {
            PositionOpSettlement::Applied => emit!(PositionOpApplied {
                position: position_key,
                owner,
                operation_intent: intent_key,
                op: o.field_2,
                amount: o.field_3,
                generation: accounts.position.coordination.generation,
                nonce: sealed.nonce.to_le_bytes(),
                slot: clock.slot,
            }),
            PositionOpSettlement::Rejected => emit!(PositionOpRejected {
                position: position_key,
                owner,
                operation_intent: intent_key,
                op: o.field_2,
                amount: o.field_3,
                intent_status: crate::operation::IntentStatus::Zero,
                slot: clock.slot,
            }),
            PositionOpSettlement::Failed(intent_status) => emit!(PositionOpRejected {
                position: position_key,
                owner,
                operation_intent: intent_key,
                op: o.field_2,
                amount: o.field_3,
                intent_status,
                slot: clock.slot,
            }),
            PositionOpSettlement::Ignored => {}
        }
        Ok(())
    }

    /// Permissionless: abandons a queued position operation after Lock expiry.
    pub fn abandon_position_op(ctx: Context<AbandonPositionOp>) -> Result<()> {
        abandon_position_op_handler(ctx)
    }

    // ---- Task 5.5: production Viewing Key rotation -------------------------

    /// Registers the `rotate_position_key` computation definition with the MXE.
    pub fn init_rotate_position_key_comp_def(
        ctx: Context<InitRotatePositionKeyCompDef>,
    ) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Owner-signed and not pausable: queues the re-encryption of the stored
    /// Position payload from the current Viewing Key to `new_viewing_pubkey`.
    /// The owner states the epochs and the current key it expects
    /// (`expected_new_epoch` must be `expected_old_epoch + 1`); the Intent
    /// commits them with the new key, the stored payload, and the sequence.
    /// Requires an `Active` Position with a free Lock. Nothing changes until
    /// the callback applies, so the old key stays valid meanwhile.
    #[allow(clippy::too_many_arguments)]
    pub fn rotate_position_key(
        ctx: Context<RotatePositionKey>,
        computation_offset: u64,
        sequence: u64,
        expected_old_epoch: u64,
        expected_new_epoch: u64,
        expected_old_pubkey: [u8; 32],
        new_viewing_pubkey: [u8; 32],
    ) -> Result<()> {
        let clock = Clock::get()?;
        let owner = ctx.accounts.payer.key();
        let position_key = ctx.accounts.position.key();
        let intent_key = ctx.accounts.operation_intent.key();
        let request = RotationRequest {
            intent_key,
            sequence,
            computation_offset,
            queued_slot: clock.slot,
            expected_old_epoch,
            expected_new_epoch,
            expected_old_pubkey,
            new_viewing_pubkey,
        };
        let accounts = &mut *ctx.accounts;
        let mut intent = state::rotation::queue_rotation(
            &mut accounts.position,
            position_key,
            &mut accounts.operation_counter,
            &owner,
            &request,
        )?;
        intent.bump = ctx.bumps.operation_intent;
        accounts.operation_intent.set_inner(intent);

        emit!(ViewingKeyRotationQueued {
            position: position_key,
            owner,
            operation_intent: intent_key,
            sequence,
            old_epoch: expected_old_epoch,
            new_epoch: expected_new_epoch,
            old_pubkey: expected_old_pubkey,
            new_pubkey: new_viewing_pubkey,
            computation_offset,
            position_generation: accounts.position.coordination.generation,
            decision_expiry_slot: intent.decision_expiry_slot,
            lock_expiry_slot: intent.lock_expiry_slot,
            slot: clock.slot,
        });

        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;
        let output_nonce_input = state::nonce::derive_output_nonce_input(&position_key, sequence);
        let args = rotate_position_key_args(
            &ctx.accounts.position,
            new_viewing_pubkey,
            output_nonce_input,
        )
        .build();
        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![RotatePositionKeyCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &[
                    CallbackAccount {
                        pubkey: position_key,
                        is_writable: true,
                    },
                    CallbackAccount {
                        pubkey: intent_key,
                        is_writable: true,
                    },
                ],
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Verifies the `rotate_position_key` output with the Task 3.3 helpers.
    /// `Applied` stores the re-sealed payload, the new Viewing public key, and
    /// the incremented epoch together, bumps the generation, and releases the
    /// Lock. `Stale` and `Expired` release the Lock and change nothing (the
    /// old key stays valid). A terminal Intent is a no-op. Unauthenticated
    /// output and any binding mismatch (including an output sealed to a key
    /// other than the committed one) revert, so the Intent stays `Queued` for
    /// `abandon_rotation`.
    #[arcium_callback(encrypted_ix = "rotate_position_key")]
    pub fn rotate_position_key_callback(
        ctx: Context<RotatePositionKeyCallback>,
        output: SignedComputationOutputs<RotatePositionKeyOutput>,
    ) -> Result<()> {
        let o = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(RotatePositionKeyOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::CallbackOutputNotAuthenticated.into()),
        };

        let intent_offset = ctx.accounts.operation_intent.computation_offset;
        let computation_offset = if ctx.accounts.computation_account.key()
            == derive_comp_pda!(intent_offset, ctx.accounts.mxe_account)
        {
            intent_offset
        } else {
            intent_offset.wrapping_add(1)
        };

        let clock = Clock::get()?;
        let position_key = ctx.accounts.position.key();
        let intent_key = ctx.accounts.operation_intent.key();
        let report = RotationReport {
            output_authenticated: true,
            computation_offset,
            intent_key,
            encryption_key: o.encryption_key,
            output_nonce: o.nonce,
            ciphertexts: o.ciphertexts,
            current_slot: clock.slot,
        };
        let accounts = &mut *ctx.accounts;
        let settlement = state::rotation::settle_rotation(
            &mut accounts.position,
            position_key,
            &mut accounts.operation_intent,
            &report,
        )?;
        let owner = accounts.position.owner;
        match settlement {
            RotationSettlement::Rotated {
                old_epoch,
                new_epoch,
                old_pubkey,
                new_pubkey,
            } => emit!(ViewingKeyRotationApplied {
                position: position_key,
                owner,
                operation_intent: intent_key,
                old_epoch,
                new_epoch,
                old_pubkey,
                new_pubkey,
                generation: accounts.position.coordination.generation,
                nonce: o.nonce.to_le_bytes(),
                slot: clock.slot,
            }),
            RotationSettlement::Failed(intent_status) => emit!(ViewingKeyRotationFailed {
                position: position_key,
                owner,
                operation_intent: intent_key,
                intent_status,
                slot: clock.slot,
            }),
            RotationSettlement::Ignored => {}
        }
        Ok(())
    }

    /// Permissionless: abandons a queued rotation after Lock expiry. The key,
    /// epoch, and payload are untouched.
    pub fn abandon_rotation(ctx: Context<AbandonRotation>) -> Result<()> {
        abandon_rotation_handler(ctx)
    }

    // ---- walnut-lend ---------------------------------------------------------

    /// Admin, idempotent: creates the `LendingPool` (seeding the supply totals
    /// from the USDC vault balance) and records the oracle account, feed id and
    /// maximum price age; a repeat call updates only that configuration.
    pub fn init_lending_pool(
        ctx: Context<InitLendingPool>,
        max_price_age_seconds: u32,
        oracle: Pubkey,
        feed_id: [u8; 32],
    ) -> Result<()> {
        init_lending_pool_handler(ctx, max_price_age_seconds, oracle, feed_id)
    }

    /// Registers the `lend_op` computation definition with the MXE.
    pub fn init_lend_op_comp_def(ctx: Context<InitLendOpCompDef>) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Registers the `disclose_position` computation definition with the MXE.
    pub fn init_disclose_position_comp_def(
        ctx: Context<InitDisclosePositionCompDef>,
    ) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Owner-signed lending operation on the encrypted Position: `op` 0 add
    /// collateral, 1 withdraw collateral, 2 borrow, 3 repay, 4 supply, 5
    /// withdraw supply. `amount` is a public USDC amount (lamports for ops 0
    /// and 1). The Program accrues interest, checks pause, caps and liquidity,
    /// computes the share amount publicly, reads the Pyth price from
    /// `price_update` for ops 1 and 2 (any account otherwise), locks the
    /// Position and queues `lend_op`. No caller-supplied price.
    pub fn lend_op(
        ctx: Context<LendOpQueue>,
        computation_offset: u64,
        sequence: u64,
        op: u8,
        amount: u64,
    ) -> Result<()> {
        let op = u64::from(op);
        if op > OP_WITHDRAW_SUPPLY {
            return Err(ErrorCode::LendOpWrongEntry.into());
        }
        let clock = Clock::get()?;
        let owner = ctx.accounts.payer.key();
        let market_key = ctx.accounts.market.key();
        let pool_key = ctx.accounts.lending_pool.key();
        let position_key = ctx.accounts.position.key();
        let intent_key = ctx.accounts.operation_intent.key();
        let (price, publish_time) = if matches!(op, OP_WITHDRAW_COLLATERAL | OP_BORROW) {
            let (price, time) = read_price(
                &ctx.accounts.lending_pool,
                &ctx.accounts.asset_state,
                &ctx.accounts.price_update,
                clock.unix_timestamp,
            )?;
            (price, Some(time))
        } else {
            (0, None)
        };
        let accounts = &mut *ctx.accounts;
        let shares = state::lend_op::plan_owner_op(
            &mut accounts.lending_pool,
            &accounts.asset_state,
            op,
            amount,
            clock.unix_timestamp,
            clock.slot,
            accounts.usdc_vault.amount,
        )?;
        let params = lend_params(
            &accounts.lending_pool,
            &accounts.asset_state,
            op,
            amount,
            shares,
            price,
        )?;
        let request = LendRequest {
            intent_key,
            sequence,
            computation_offset,
            queued_slot: clock.slot,
            params,
            repay_amount: 0,
            publish_time,
            accrual_ts: accounts.lending_pool.last_accrual_ts,
        };
        let mut intent = state::lend_op::queue_lend_op(
            &accounts.market,
            market_key,
            pool_key,
            &mut accounts.position,
            position_key,
            &mut accounts.operation_counter,
            &owner,
            None,
            &request,
        )?;
        intent.bump = ctx.bumps.operation_intent;
        accounts.operation_intent.set_inner(intent);
        emit!(LendOpQueued {
            market: market_key,
            lending_pool: pool_key,
            position: position_key,
            payer: owner,
            operation_intent: intent_key,
            op,
            amount,
            shares,
            borrow_index_1e9: params.borrow_index_1e9,
            supply_index_1e9: accounts.lending_pool.supply_index_1e9,
            price,
            computation_offset,
            position_generation: accounts.position.coordination.generation,
            decision_expiry_slot: intent.decision_expiry_slot,
            lock_expiry_slot: intent.lock_expiry_slot,
            slot: clock.slot,
        });

        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;
        let output_nonce_input = state::nonce::derive_output_nonce_input(&position_key, sequence);
        let args = lend_op_args(&ctx.accounts.position, output_nonce_input, &params).build();
        let callback_accounts =
            lend_callback_accounts(market_key, pool_key, position_key, intent_key);
        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![LendOpCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &callback_accounts,
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Permissionless health check: queues op 6 with the Pyth low-bound price
    /// and the accrued Borrow Index. The Position is not locked and never
    /// changes; the callback emits `HealthChecked { position, unhealthy }`.
    pub fn check_health(
        ctx: Context<CheckHealthQueue>,
        computation_offset: u64,
        sequence: u64,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let payer = ctx.accounts.payer.key();
        let market_key = ctx.accounts.market.key();
        let pool_key = ctx.accounts.lending_pool.key();
        let position_key = ctx.accounts.position.key();
        let intent_key = ctx.accounts.operation_intent.key();
        if ctx.accounts.operation_counter.owner == Pubkey::default() {
            ctx.accounts
                .operation_counter
                .set_inner(OperationCounter::new(payer, ctx.bumps.operation_counter));
        }
        let (price, publish_time) = read_price(
            &ctx.accounts.lending_pool,
            &ctx.accounts.asset_state,
            &ctx.accounts.price_update,
            clock.unix_timestamp,
        )?;
        let accounts = &mut *ctx.accounts;
        accounts
            .lending_pool
            .accrue_auto(clock.unix_timestamp, &accounts.asset_state)?;
        let params = lend_params(
            &accounts.lending_pool,
            &accounts.asset_state,
            OP_HEALTH_CHECK,
            0,
            0,
            price,
        )?;
        let request = LendRequest {
            intent_key,
            sequence,
            computation_offset,
            queued_slot: clock.slot,
            params,
            repay_amount: 0,
            publish_time: Some(publish_time),
            accrual_ts: accounts.lending_pool.last_accrual_ts,
        };
        let mut intent = state::lend_op::queue_lend_op(
            &accounts.market,
            market_key,
            pool_key,
            &mut accounts.position,
            position_key,
            &mut accounts.operation_counter,
            &payer,
            None,
            &request,
        )?;
        intent.bump = ctx.bumps.operation_intent;
        accounts.operation_intent.set_inner(intent);
        emit!(LendOpQueued {
            market: market_key,
            lending_pool: pool_key,
            position: position_key,
            payer,
            operation_intent: intent_key,
            op: params.op,
            amount: 0,
            shares: 0,
            borrow_index_1e9: params.borrow_index_1e9,
            supply_index_1e9: accounts.lending_pool.supply_index_1e9,
            price,
            computation_offset,
            position_generation: accounts.position.coordination.generation,
            decision_expiry_slot: intent.decision_expiry_slot,
            lock_expiry_slot: intent.lock_expiry_slot,
            slot: clock.slot,
        });

        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;
        let output_nonce_input = state::nonce::derive_output_nonce_input(&position_key, sequence);
        let args = lend_op_args(&ctx.accounts.position, output_nonce_input, &params).build();
        let callback_accounts =
            lend_callback_accounts(market_key, pool_key, position_key, intent_key);
        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![LendOpCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &callback_accounts,
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Permissionless liquidation. The Liquidator pays `repay_amount` USDC into
    /// the vault now; the Program computes the debt burn and the wSOL seize
    /// publicly from the accrued index, the Pyth low-bound price and the
    /// liquidation bonus, locks the target Position and queues `lend_op` op 7.
    /// `remaining_accounts` must be `[liquidator_usdc, liquidator_wsol]`.
    /// On `applied` the callback sends the seized wSOL to the Liquidator; on
    /// any other outcome (healthy, close factor, seize above collateral,
    /// stale, expired) it refunds the USDC. `abandon_lend_op` refunds after
    /// Lock expiry.
    pub fn liquidate<'info>(
        ctx: Context<'info, LiquidateQueue<'info>>,
        computation_offset: u64,
        sequence: u64,
        repay_amount: u64,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let payer = ctx.accounts.payer.key();
        let market_key = ctx.accounts.market.key();
        let pool_key = ctx.accounts.lending_pool.key();
        let position_key = ctx.accounts.position.key();
        let intent_key = ctx.accounts.operation_intent.key();
        if ctx.accounts.operation_counter.owner == Pubkey::default() {
            ctx.accounts
                .operation_counter
                .set_inner(OperationCounter::new(payer, ctx.bumps.operation_counter));
        }
        let remaining = ctx.remaining_accounts;
        if remaining.len() != 4
            || remaining[2].key() != anchor_spl::token::ID
            || remaining[3].key() != ctx.accounts.market.wsol_vault
        {
            return Err(ErrorCode::LiquidationAccountsInvalid.into());
        }
        let wsol_vault = Account::<TokenAccount>::try_from(&remaining[3])?;
        let liquidator_usdc = Account::<TokenAccount>::try_from(&remaining[0])?;
        let liquidator_wsol = Account::<TokenAccount>::try_from(&remaining[1])?;
        if liquidator_usdc.mint != ctx.accounts.market.usdc_mint
            || liquidator_usdc.owner != payer
            || liquidator_wsol.mint != ctx.accounts.market.wsol_mint
            || liquidator_wsol.owner != payer
        {
            return Err(ErrorCode::LiquidationAccountsInvalid.into());
        }
        let (price, publish_time) = read_price(
            &ctx.accounts.lending_pool,
            &ctx.accounts.asset_state,
            &ctx.accounts.price_update,
            clock.unix_timestamp,
        )?;
        let liquidation = LiquidationKeys {
            usdc_vault: ctx.accounts.market.usdc_vault,
            wsol_vault: ctx.accounts.market.wsol_vault,
            vault_authority: ctx.accounts.market.vault_authority,
            token_program: anchor_spl::token::ID,
            liquidator_usdc: remaining[0].key(),
            liquidator_wsol: remaining[1].key(),
        };
        let accounts = &mut *ctx.accounts;
        let (burn, seize) = state::lend_op::plan_liquidation(
            &mut accounts.lending_pool,
            &accounts.asset_state,
            repay_amount,
            price,
            clock.unix_timestamp,
            clock.slot,
            wsol_vault.amount,
        )?;
        let params = lend_params(
            &accounts.lending_pool,
            &accounts.asset_state,
            OP_LIQUIDATE,
            seize,
            burn,
            price,
        )?;
        let request = LendRequest {
            intent_key,
            sequence,
            computation_offset,
            queued_slot: clock.slot,
            params,
            repay_amount,
            publish_time: Some(publish_time),
            accrual_ts: accounts.lending_pool.last_accrual_ts,
        };
        let mut intent = state::lend_op::queue_lend_op(
            &accounts.market,
            market_key,
            pool_key,
            &mut accounts.position,
            position_key,
            &mut accounts.operation_counter,
            &payer,
            Some(liquidation),
            &request,
        )?;
        intent.bump = ctx.bumps.operation_intent;
        accounts.operation_intent.set_inner(intent);

        // The Liquidator pays the repay amount into the vault.
        spl::transfer(
            CpiContext::new(
                remaining[2].key(),
                Transfer {
                    from: remaining[0].clone(),
                    to: accounts.usdc_vault.to_account_info(),
                    authority: accounts.payer.to_account_info(),
                },
            ),
            repay_amount,
        )?;
        emit!(LendOpQueued {
            market: market_key,
            lending_pool: pool_key,
            position: position_key,
            payer,
            operation_intent: intent_key,
            op: params.op,
            amount: seize,
            shares: burn,
            borrow_index_1e9: params.borrow_index_1e9,
            supply_index_1e9: accounts.lending_pool.supply_index_1e9,
            price,
            computation_offset,
            position_generation: accounts.position.coordination.generation,
            decision_expiry_slot: intent.decision_expiry_slot,
            lock_expiry_slot: intent.lock_expiry_slot,
            slot: clock.slot,
        });

        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;
        let output_nonce_input = state::nonce::derive_output_nonce_input(&position_key, sequence);
        let args = lend_op_args(&ctx.accounts.position, output_nonce_input, &params).build();
        let market = &ctx.accounts.market;
        let callback_accounts = [
            CallbackAccount {
                pubkey: market_key,
                is_writable: false,
            },
            CallbackAccount {
                pubkey: pool_key,
                is_writable: true,
            },
            CallbackAccount {
                pubkey: position_key,
                is_writable: true,
            },
            CallbackAccount {
                pubkey: intent_key,
                is_writable: true,
            },
            CallbackAccount {
                pubkey: market.usdc_vault,
                is_writable: true,
            },
            CallbackAccount {
                pubkey: market.wsol_vault,
                is_writable: true,
            },
            CallbackAccount {
                pubkey: market.vault_authority,
                is_writable: false,
            },
            CallbackAccount {
                pubkey: anchor_spl::token::ID,
                is_writable: false,
            },
            CallbackAccount {
                pubkey: liquidation.liquidator_usdc,
                is_writable: true,
            },
            CallbackAccount {
                pubkey: liquidation.liquidator_wsol,
                is_writable: true,
            },
        ];
        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![LendOpCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &callback_accounts,
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Verifies the `lend_op` output. `Applied` stores the new state, bumps the
    /// generation, releases the Lock, updates the pool totals and (for a
    /// liquidation) sends the seized wSOL to the Liquidator. A refusal or an
    /// unapplied attempt releases the Lock, stores nothing and (for a
    /// liquidation) refunds the Liquidator's USDC. A health check only emits
    /// `HealthChecked`. A failing vault transfer reverts the callback, so the
    /// Intent stays queued for `abandon_lend_op`. Unauthenticated output and
    /// binding mismatches revert.
    #[arcium_callback(encrypted_ix = "lend_op")]
    pub fn lend_op_callback(
        ctx: Context<LendOpCallback>,
        output: SignedComputationOutputs<LendOpOutput>,
    ) -> Result<()> {
        let o = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(LendOpOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::CallbackOutputNotAuthenticated.into()),
        };
        let sealed = o.field_0;

        let intent_offset = ctx.accounts.operation_intent.computation_offset;
        let computation_offset = if ctx.accounts.computation_account.key()
            == derive_comp_pda!(intent_offset, ctx.accounts.mxe_account)
        {
            intent_offset
        } else {
            intent_offset.wrapping_add(1)
        };

        let clock = Clock::get()?;
        let market_key = ctx.accounts.market.key();
        let pool_key = ctx.accounts.lending_pool.key();
        let position_key = ctx.accounts.position.key();
        let intent_key = ctx.accounts.operation_intent.key();
        let remaining = ctx.remaining_accounts;
        let liquidation = if state::lend_op::is_liquidation(&ctx.accounts.operation_intent) {
            let keys = liquidation_keys_from_remaining(remaining)?;
            keys.check_market(&ctx.accounts.market)?;
            Some(keys)
        } else {
            None
        };
        let report = LendReport {
            output_authenticated: true,
            computation_offset,
            intent_key,
            encryption_key: sealed.encryption_key,
            output_nonce: sealed.nonce,
            ciphertexts: sealed.ciphertexts,
            applied: o.field_1,
            op: o.field_2,
            amount: o.field_3,
            shares: o.field_4,
            current_slot: clock.slot,
        };
        let accounts = &mut *ctx.accounts;
        let settlement = state::lend_op::settle_lend_op(
            &accounts.market,
            market_key,
            pool_key,
            &mut accounts.position,
            position_key,
            &mut accounts.operation_intent,
            liquidation,
            &report,
        )?;
        let liquidator = accounts.operation_intent.payer;
        let repay = state::lend_op::liquidation_repay_amount(&accounts.operation_intent);
        let refund = liquidation.is_some();
        match settlement {
            LendSettlement::Applied { op, amount, shares } => {
                state::lend_op::apply_pool_totals(&mut accounts.lending_pool, op, shares)?;
                if op == OP_LIQUIDATE {
                    transfer_from_vault(
                        &accounts.market,
                        &market_key,
                        remaining,
                        REMAINING_WSOL_VAULT,
                        REMAINING_LIQUIDATOR_WSOL,
                        amount,
                    )?;
                    emit!(LiquidationApplied {
                        position: position_key,
                        liquidator,
                        operation_intent: intent_key,
                        repay_amount: repay,
                        debt_shares_burned: shares,
                        seized_lamports: amount,
                        slot: clock.slot,
                    });
                }
                emit!(LendOpApplied {
                    position: position_key,
                    operation_intent: intent_key,
                    op,
                    amount,
                    shares,
                    total_supply_shares: accounts.lending_pool.total_supply_shares,
                    total_debt_shares: accounts.lending_pool.total_debt_shares,
                    generation: accounts.position.coordination.generation,
                    nonce: sealed.nonce.to_le_bytes(),
                    slot: clock.slot,
                });
            }
            LendSettlement::Rejected { op } => {
                if refund {
                    transfer_from_vault(
                        &accounts.market,
                        &market_key,
                        remaining,
                        REMAINING_USDC_VAULT,
                        REMAINING_LIQUIDATOR_USDC,
                        repay,
                    )?;
                }
                emit!(LendOpRejected {
                    position: position_key,
                    operation_intent: intent_key,
                    op,
                    amount: o.field_3,
                    intent_status: crate::operation::IntentStatus::Zero,
                    refunded_usdc: if refund { repay } else { 0 },
                    slot: clock.slot,
                });
            }
            LendSettlement::Health { unhealthy } => emit!(HealthChecked {
                position: position_key,
                operation_intent: intent_key,
                unhealthy,
                slot: clock.slot,
            }),
            LendSettlement::Failed(intent_status) => {
                if refund {
                    transfer_from_vault(
                        &accounts.market,
                        &market_key,
                        remaining,
                        REMAINING_USDC_VAULT,
                        REMAINING_LIQUIDATOR_USDC,
                        repay,
                    )?;
                }
                emit!(LendOpRejected {
                    position: position_key,
                    operation_intent: intent_key,
                    op: o.field_2,
                    amount: o.field_3,
                    intent_status,
                    refunded_usdc: if refund { repay } else { 0 },
                    slot: clock.slot,
                });
            }
            LendSettlement::Ignored => {}
        }
        Ok(())
    }

    /// Permissionless: abandons a queued lend operation after Lock expiry and
    /// refunds a queued liquidation's USDC to the Liquidator (six liquidation
    /// accounts as `remaining_accounts`, see `instructions::lend_op`).
    pub fn abandon_lend_op(ctx: Context<AbandonLendOp>) -> Result<()> {
        abandon_lend_op_handler(ctx)
    }

    /// Owner-signed: queues `disclose_position`, which re-seals a field-masked
    /// copy of the Position to `auditor_pubkey`. `mask` bit 0 free SOL, 1 free
    /// USDC, 2 collateral, 3 supply shares, 4 debt shares. The Position is not
    /// locked or changed. The callback writes the `Disclosure` PDA
    /// `["disclosure", position, auditor]`, overwriting an earlier one.
    pub fn disclose_position(
        ctx: Context<DisclosePositionQueue>,
        computation_offset: u64,
        auditor_pubkey: [u8; 32],
        mask: u64,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let owner = ctx.accounts.payer.key();
        let position_key = ctx.accounts.position.key();
        let intent_key = ctx.accounts.operation_intent.key();
        let disclosure_key = ctx.accounts.disclosure.key();
        let sequence = ctx.accounts.operation_counter.next_sequence;
        if ctx.accounts.disclosure.position == Pubkey::default() {
            ctx.accounts.disclosure.set_inner(Disclosure::blank(
                position_key,
                owner,
                auditor_pubkey,
                ctx.bumps.disclosure,
            ));
        }
        let request = DiscloseRequest {
            intent_key,
            sequence,
            computation_offset,
            queued_slot: clock.slot,
            disclosure: disclosure_key,
            auditor: auditor_pubkey,
            mask,
        };
        let accounts = &mut *ctx.accounts;
        let mut intent = state::disclosure::queue_disclosure(
            &accounts.position,
            position_key,
            &mut accounts.operation_counter,
            &owner,
            &request,
        )?;
        intent.bump = ctx.bumps.operation_intent;
        accounts.operation_intent.set_inner(intent);
        emit!(DisclosureQueued {
            position: position_key,
            disclosure: disclosure_key,
            owner,
            auditor: auditor_pubkey,
            mask,
            operation_intent: intent_key,
            computation_offset,
            position_generation: accounts.position.coordination.generation,
            slot: clock.slot,
        });

        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;
        let output_nonce_input = state::nonce::derive_output_nonce_input(&position_key, sequence);
        let args = disclose_position_args(
            &ctx.accounts.position,
            auditor_pubkey,
            output_nonce_input,
            mask,
        )
        .build();
        let callback_accounts = [
            CallbackAccount {
                pubkey: position_key,
                is_writable: false,
            },
            CallbackAccount {
                pubkey: disclosure_key,
                is_writable: true,
            },
            CallbackAccount {
                pubkey: intent_key,
                is_writable: true,
            },
        ];
        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![DisclosePositionCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &callback_accounts,
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Verifies the `disclose_position` output (sealed to the committed
    /// auditor key) and writes the Disclosure only on `Applied`. `Stale`,
    /// `Expired` and failed outcomes leave the previous Disclosure in place.
    #[arcium_callback(encrypted_ix = "disclose_position")]
    pub fn disclose_position_callback(
        ctx: Context<DisclosePositionCallback>,
        output: SignedComputationOutputs<DisclosePositionOutput>,
    ) -> Result<()> {
        let sealed = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(DisclosePositionOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::CallbackOutputNotAuthenticated.into()),
        };

        let intent_offset = ctx.accounts.operation_intent.computation_offset;
        let computation_offset = if ctx.accounts.computation_account.key()
            == derive_comp_pda!(intent_offset, ctx.accounts.mxe_account)
        {
            intent_offset
        } else {
            intent_offset.wrapping_add(1)
        };

        let clock = Clock::get()?;
        let position_key = ctx.accounts.position.key();
        let disclosure_key = ctx.accounts.disclosure.key();
        let intent_key = ctx.accounts.operation_intent.key();
        let report = DiscloseReport {
            output_authenticated: true,
            computation_offset,
            intent_key,
            encryption_key: sealed.encryption_key,
            output_nonce: sealed.nonce,
            ciphertexts: sealed.ciphertexts,
            current_slot: clock.slot,
        };
        let accounts = &mut *ctx.accounts;
        let settlement = state::disclosure::settle_disclosure(
            &accounts.position,
            position_key,
            &mut accounts.disclosure,
            disclosure_key,
            &mut accounts.operation_intent,
            &report,
        )?;
        match settlement {
            DiscloseSettlement::Stored => emit!(PositionDisclosed {
                position: position_key,
                disclosure: disclosure_key,
                owner: accounts.disclosure.owner,
                auditor: accounts.disclosure.auditor.to_bytes(),
                mask: accounts.disclosure.mask,
                position_generation: accounts.disclosure.position_generation,
                nonce: sealed.nonce.to_le_bytes(),
                slot: clock.slot,
            }),
            DiscloseSettlement::Failed(intent_status) => emit!(DisclosureNotStored {
                position: position_key,
                operation_intent: intent_key,
                intent_status,
                slot: clock.slot,
            }),
            DiscloseSettlement::Ignored => {}
        }
        Ok(())
    }

    /// Permissionless: closes a queued disclosure whose callback never arrived.
    pub fn abandon_disclosure(ctx: Context<AbandonDisclosure>) -> Result<()> {
        abandon_disclosure_handler(ctx)
    }
    // ---- Task 7.2: utilization band --------------------------------------

    /// Registers the `update_utilization_band` computation definition.
    pub fn init_update_utilization_band_comp_def(
        ctx: Context<InitUpdateUtilizationBandCompDef>,
    ) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Permissionless: queues the read-only utilization-band computation. It
    /// reads the Pool's encrypted Cash and Debt Shares at the public
    /// accounting Borrow Index and reveals only the band index. It neither
    /// checkpoints nor Locks the Pool and changes no generation. The Intent
    /// captures the Pool generation, Asset State generation, rate epoch, and
    /// accounting index; the Pool-owned counter supplies `sequence`.
    pub fn update_utilization_band(
        ctx: Context<UpdateUtilizationBandQueue>,
        computation_offset: u64,
        sequence: u64,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let market_key = ctx.accounts.market.key();
        let pool_key = ctx.accounts.pool_state.key();
        let intent_key = ctx.accounts.operation_intent.key();
        if ctx.accounts.operation_counter.owner == Pubkey::default() {
            ctx.accounts
                .operation_counter
                .set_inner(OperationCounter::new(pool_key, ctx.bumps.operation_counter));
        }
        let keys = BandKeys {
            payer: ctx.accounts.payer.key(),
            market: market_key,
            asset_state: ctx.accounts.asset_state.key(),
            pool: pool_key,
        };
        let request = BandRequest {
            intent_key,
            sequence,
            computation_offset,
            queued_slot: clock.slot,
        };
        let circuit_index = band_circuit_index(
            &ctx.accounts.asset_state,
            ctx.accounts.lending_pool.as_deref().map(|p| &**p),
        )?;
        let accounts = &mut *ctx.accounts;
        let mut intent = state::band::queue_band_update_at(
            accounts.market.status,
            &keys,
            &accounts.asset_state,
            &accounts.pool_state,
            &mut accounts.operation_counter,
            &request,
            circuit_index,
        )?;
        intent.bump = ctx.bumps.operation_intent;
        accounts.operation_intent.set_inner(intent);
        emit!(UtilizationBandQueued {
            market: market_key,
            pool_state: pool_key,
            operation_intent: intent_key,
            computation_offset,
            pool_generation: accounts.pool_state.coordination.generation,
            rate_epoch_generation: intent.rate_epoch_generation,
            accounting_borrow_index: intent.accounting_borrow_index,
            decision_expiry_slot: intent.decision_expiry_slot,
            slot: clock.slot,
        });

        // `update_utilization_band(pool: Enc<Mxe, PoolSecret>, accounting_index: u128)`.
        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;
        let args = update_utilization_band_args(&ctx.accounts.pool_state, circuit_index).build();
        let callback_accounts = [
            CallbackAccount {
                pubkey: market_key,
                is_writable: false,
            },
            CallbackAccount {
                pubkey: keys.asset_state,
                is_writable: true,
            },
            CallbackAccount {
                pubkey: pool_key,
                is_writable: false,
            },
            CallbackAccount {
                pubkey: intent_key,
                is_writable: true,
            },
        ];
        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![UpdateUtilizationBandCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &callback_accounts,
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Verifies the revealed band with the Task 3.3 helpers. `Applied` stores
    /// the band and the observation slot (the queue slot) on the Asset State
    /// only if the Pool generation, Asset State generation, rate epoch, and
    /// accounting index still match; `Stale`, `Expired`, and failed outcomes
    /// store nothing. A terminal Intent is a no-op. Unauthenticated output and
    /// any binding mismatch revert, so the Intent stays `Queued`.
    #[arcium_callback(encrypted_ix = "update_utilization_band")]
    pub fn update_utilization_band_callback(
        ctx: Context<UpdateUtilizationBandCallback>,
        output: SignedComputationOutputs<UpdateUtilizationBandOutput>,
    ) -> Result<()> {
        let band = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(UpdateUtilizationBandOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::CallbackOutputNotAuthenticated.into()),
        };

        let intent_offset = ctx.accounts.operation_intent.computation_offset;
        let computation_offset = if ctx.accounts.computation_account.key()
            == derive_comp_pda!(intent_offset, ctx.accounts.mxe_account)
        {
            intent_offset
        } else {
            intent_offset.wrapping_add(1)
        };

        let clock = Clock::get()?;
        let intent_key = ctx.accounts.operation_intent.key();
        let asset_state_key = ctx.accounts.asset_state.key();
        let keys = BandKeys {
            payer: ctx.accounts.operation_intent.payer,
            market: ctx.accounts.market.key(),
            asset_state: asset_state_key,
            pool: ctx.accounts.pool_state.key(),
        };
        let report = BandReport {
            output_authenticated: true,
            computation_offset,
            intent_key,
            band,
            current_slot: clock.slot,
        };
        let accounts = &mut *ctx.accounts;
        let settlement = state::band::settle_band_update(
            &keys,
            &mut accounts.asset_state,
            &accounts.pool_state,
            &mut accounts.operation_intent,
            &report,
        )?;
        match settlement {
            BandSettlement::Applied {
                band,
                observation_slot,
                written,
            } => emit!(UtilizationBandUpdated {
                asset_state: asset_state_key,
                operation_intent: intent_key,
                band,
                observation_slot,
                written,
                slot: clock.slot,
            }),
            BandSettlement::Stale(_) | BandSettlement::Failed(_) => {
                emit!(UtilizationBandNotApplied {
                    operation_intent: intent_key,
                    intent_status: accounts.operation_intent.status,
                    slot: clock.slot,
                })
            }
            BandSettlement::Ignored => {}
        }
        Ok(())
    }

    // ---- Task 7.5: reserve withdrawal ---------------------------------------

    /// Registers the `reserve_withdrawal` computation definition.
    pub fn init_reserve_withdrawal_comp_def(
        ctx: Context<InitReserveWithdrawalCompDef>,
    ) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Treasury-authority-signed: requests a public all-or-zero USDC `amount`
    /// from the Pool's Reserves to the configured treasury token account.
    /// Requires an `Active` Pool with a free Lock, the `WITHDRAW_SUPPLY` pause
    /// off, and a configured treasury account owned by the treasury
    /// authority. Accrues the accounting Borrow Index, creates the Intent at
    /// `sequence` (Pool-owned counter), Locks the Pool, and queues
    /// `reserve_withdrawal`. No token moves until the callback approves.
    pub fn reserve_withdrawal(
        ctx: Context<ReserveWithdrawalQueue>,
        computation_offset: u64,
        sequence: u64,
        amount: u64,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let signer = ctx.accounts.payer.key();
        let market_key = ctx.accounts.market.key();
        let pool_key = ctx.accounts.pool_state.key();
        let asset_state_key = ctx.accounts.asset_state.key();
        let intent_key = ctx.accounts.operation_intent.key();
        if ctx.accounts.operation_counter.owner == Pubkey::default() {
            ctx.accounts
                .operation_counter
                .set_inner(OperationCounter::new(pool_key, ctx.bumps.operation_counter));
        }
        let destination = TokenAccountView {
            key: ctx.accounts.destination.key(),
            mint: ctx.accounts.destination.mint,
            owner: ctx.accounts.destination.owner,
        };
        let request = ReserveWithdrawalRequest {
            intent_key,
            sequence,
            computation_offset,
            queued_slot: clock.slot,
            now_ts: clock.unix_timestamp,
            amount,
        };
        let accounts = &mut *ctx.accounts;
        let accounted_index = accounts.pool_state.accounted_borrow_index;
        let reserve_factor_bps = accounts.asset_state.risk.reserve_factor_bps;
        let mut intent = state::reserve_withdrawal::queue_reserve_withdrawal(
            &accounts.market,
            market_key,
            asset_state_key,
            &mut accounts.asset_state,
            &mut accounts.pool_state,
            pool_key,
            &mut accounts.operation_counter,
            &signer,
            &destination,
            &request,
        )?;
        intent.bump = ctx.bumps.operation_intent;
        accounts.operation_intent.set_inner(intent);
        emit!(ReserveWithdrawalQueued {
            market: market_key,
            pool_state: pool_key,
            operation_intent: intent_key,
            authority: signer,
            amount,
            destination: destination.key,
            computation_offset,
            pool_generation: accounts.pool_state.coordination.generation,
            accounting_borrow_index: intent.accounting_borrow_index,
            risk_borrow_index_at_expiry: intent.risk_borrow_index_at_expiry,
            decision_expiry_slot: intent.decision_expiry_slot,
            lock_expiry_slot: intent.lock_expiry_slot,
            slot: clock.slot,
        });

        // `reserve_withdrawal(pool, accounted_index, accounting_index,
        // reserve_factor_bps, requested)`.
        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;
        let args = reserve_withdrawal_args(
            &ctx.accounts.pool_state,
            accounted_index,
            intent.accounting_borrow_index,
            reserve_factor_bps,
            amount,
        )
        .build();
        let callback_accounts = reserve_withdrawal_callback_accounts(
            &ctx.accounts.market,
            market_key,
            asset_state_key,
            pool_key,
            intent_key,
            destination.key,
        );
        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![ReserveWithdrawalCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &callback_accounts,
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Verifies the `reserve_withdrawal` output with the Task 3.3 helpers.
    /// `Applied` stores the new Pool payload, sets the accounted index, bumps
    /// the Pool generation, releases the Lock, and transfers exactly the
    /// approved amount from the USDC vault to the committed treasury account,
    /// signed by the vault-authority PDA. `Zero`, `Stale`, and `Expired`
    /// release the Lock and move nothing. A terminal Intent is a no-op.
    /// Unauthenticated output and any binding mismatch revert, so the Intent
    /// stays `Queued` for `abandon_reserve_withdrawal`.
    #[arcium_callback(encrypted_ix = "reserve_withdrawal")]
    pub fn reserve_withdrawal_callback(
        ctx: Context<ReserveWithdrawalCallback>,
        output: SignedComputationOutputs<ReserveWithdrawalOutput>,
    ) -> Result<()> {
        let o = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(ReserveWithdrawalOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::CallbackOutputNotAuthenticated.into()),
        };
        let sealed = o.field_0;

        let intent_offset = ctx.accounts.operation_intent.computation_offset;
        let computation_offset = if ctx.accounts.computation_account.key()
            == derive_comp_pda!(intent_offset, ctx.accounts.mxe_account)
        {
            intent_offset
        } else {
            intent_offset.wrapping_add(1)
        };

        let clock = Clock::get()?;
        let market_key = ctx.accounts.market.key();
        let pool_key = ctx.accounts.pool_state.key();
        let intent_key = ctx.accounts.operation_intent.key();
        let vault_key = ctx.accounts.vault.key();
        let report = ReserveWithdrawalReport {
            output_authenticated: true,
            computation_offset,
            intent_key,
            output: EncryptedPoolState {
                nonce: sealed.nonce,
                ciphertexts: sealed.ciphertexts,
            },
            approved_amount: o.field_1,
            requested_amount: o.field_2,
            destination: TokenAccountView {
                key: ctx.accounts.destination.key(),
                mint: ctx.accounts.destination.mint,
                owner: ctx.accounts.destination.owner,
            },
            mint: ctx.accounts.mint.key(),
            vault: vault_key,
            vault_authority: ctx.accounts.vault_authority.key(),
            token_program: ctx.accounts.token_program.key(),
            current_slot: clock.slot,
        };
        let accounts = &mut *ctx.accounts;
        let settlement = state::reserve_withdrawal::settle_reserve_withdrawal(
            &accounts.market,
            market_key,
            &accounts.asset_state,
            &mut accounts.pool_state,
            pool_key,
            &mut accounts.operation_intent,
            &report,
        )?;
        match settlement {
            ReserveWithdrawalSettlement::Approved {
                amount,
                destination,
            } => {
                let bump = [accounts.market.vault_authority_bump];
                let seeds: &[&[u8]] = &[VAULT_AUTHORITY_SEED, market_key.as_ref(), &bump];
                let signer = &[seeds];
                spl::transfer_checked(
                    CpiContext::new_with_signer(
                        accounts.token_program.key(),
                        TransferChecked {
                            from: accounts.vault.to_account_info(),
                            mint: accounts.mint.to_account_info(),
                            to: accounts.destination.to_account_info(),
                            authority: accounts.vault_authority.to_account_info(),
                        },
                        signer,
                    ),
                    amount,
                    accounts.mint.decimals,
                )?;
                emit!(ReserveWithdrawalApproved {
                    market: market_key,
                    pool_state: pool_key,
                    operation_intent: intent_key,
                    amount,
                    vault: vault_key,
                    destination,
                    generation: accounts.pool_state.coordination.generation,
                    accounted_borrow_index: accounts.pool_state.accounted_borrow_index,
                    nonce: sealed.nonce.to_le_bytes(),
                    slot: clock.slot,
                });
            }
            ReserveWithdrawalSettlement::Zero | ReserveWithdrawalSettlement::Failed(_) => {
                emit!(ReserveWithdrawalNotApplied {
                    pool_state: pool_key,
                    operation_intent: intent_key,
                    intent_status: accounts.operation_intent.status,
                    requested_amount: o.field_2,
                    slot: clock.slot,
                })
            }
            ReserveWithdrawalSettlement::Ignored => {}
        }
        Ok(())
    }

    /// Permissionless: abandons a queued reserve withdrawal after Lock
    /// expiry. Nothing moved, so there is nothing to refund.
    pub fn abandon_reserve_withdrawal(ctx: Context<AbandonReserveWithdrawal>) -> Result<()> {
        abandon_reserve_withdrawal_handler(ctx)
    }

    /// Admin-signed: records the single treasury USDC token account (USDC
    /// mint, owned by the treasury authority).
    pub fn set_treasury_usdc_account(ctx: Context<SetTreasuryUsdcAccount>) -> Result<()> {
        set_treasury_usdc_account_handler(ctx)
    }

    // ---- Task 8: supply and withdraw supply --------------------------------

    /// Registers the `supply_flow` computation definition with the MXE.
    pub fn init_supply_flow_comp_def(ctx: Context<InitSupplyFlowCompDef>) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Owner-signed: supplies `requested` USDC units (`is_withdraw = false`)
    /// or withdraws `requested` supply shares (`is_withdraw = true`). Requires
    /// an `Active` Position and Pool with free Locks and the matching pause
    /// off. Accrues the accounting Borrow Index, creates the Intent at
    /// `sequence`, Locks the Pool and the Position together, and queues
    /// `supply_flow`. No token moves: only encrypted balances change.
    pub fn supply_flow(
        ctx: Context<SupplyFlowQueue>,
        computation_offset: u64,
        sequence: u64,
        is_withdraw: bool,
        requested: u128,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let owner = ctx.accounts.payer.key();
        let market_key = ctx.accounts.market.key();
        let pool_key = ctx.accounts.pool_state.key();
        let position_key = ctx.accounts.position.key();
        let asset_state_key = ctx.accounts.asset_state.key();
        let intent_key = ctx.accounts.operation_intent.key();
        let request = SupplyFlowRequest {
            intent_key,
            sequence,
            computation_offset,
            queued_slot: clock.slot,
            now_ts: clock.unix_timestamp,
            is_withdraw,
            requested,
        };
        let accounts = &mut *ctx.accounts;
        let accounted_index = accounts.pool_state.accounted_borrow_index;
        let reserve_factor_bps = accounts.asset_state.risk.reserve_factor_bps;
        let supply_cap = accounts.asset_state.caps.supply_cap_usdc;
        let claims_clear = accounts.market.unsettled_recovery_claim_count == 0;
        let mut intent = state::supply::queue_supply_flow(
            &accounts.market,
            market_key,
            asset_state_key,
            &mut accounts.asset_state,
            &mut accounts.pool_state,
            pool_key,
            &mut accounts.position,
            position_key,
            &mut accounts.operation_counter,
            &owner,
            &request,
        )?;
        intent.bump = ctx.bumps.operation_intent;
        accounts.operation_intent.set_inner(intent);
        emit!(SupplyFlowQueued {
            market: market_key,
            position: position_key,
            owner,
            pool_state: pool_key,
            operation_intent: intent_key,
            is_withdraw,
            requested,
            computation_offset,
            position_generation: accounts.position.coordination.generation,
            pool_generation: accounts.pool_state.coordination.generation,
            accounting_borrow_index: intent.accounting_borrow_index,
            decision_expiry_slot: intent.decision_expiry_slot,
            lock_expiry_slot: intent.lock_expiry_slot,
            slot: clock.slot,
        });

        // `supply_flow(pool, position, owner: Shared, accounted_index,
        // accounting_index, reserve_factor_bps, is_withdraw, requested,
        // supply_cap, claims_clear)`.
        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;
        let output_nonce_input = state::nonce::derive_output_nonce_input(&position_key, sequence);
        let args = supply_flow_args(
            &ctx.accounts.pool_state,
            &ctx.accounts.position,
            output_nonce_input,
            accounted_index,
            intent.accounting_borrow_index,
            reserve_factor_bps,
            is_withdraw,
            requested,
            supply_cap,
            claims_clear,
        )
        .build();
        let callback_accounts = supply_flow_callback_accounts(
            market_key,
            asset_state_key,
            pool_key,
            position_key,
            intent_key,
        );
        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![SupplyFlowCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &callback_accounts,
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Verifies the `supply_flow` output with the Task 3.3 helpers. `Applied`
    /// stores the Pool and Position payloads, advances the accounted index,
    /// bumps both generations, and releases both Locks. `Zero`, `Stale`, and
    /// `Expired` release both Locks and store nothing. A terminal Intent is a
    /// no-op. Unauthenticated output and any binding mismatch revert, so the
    /// Intent stays `Queued` for `abandon_supply_flow`.
    #[arcium_callback(encrypted_ix = "supply_flow")]
    pub fn supply_flow_callback(
        ctx: Context<SupplyFlowCallback>,
        output: SignedComputationOutputs<SupplyFlowOutput>,
    ) -> Result<()> {
        let o = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(SupplyFlowOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::CallbackOutputNotAuthenticated.into()),
        };
        let pool_sealed = o.field_0;
        let position_sealed = o.field_1;

        let intent_offset = ctx.accounts.operation_intent.computation_offset;
        let computation_offset = if ctx.accounts.computation_account.key()
            == derive_comp_pda!(intent_offset, ctx.accounts.mxe_account)
        {
            intent_offset
        } else {
            intent_offset.wrapping_add(1)
        };

        let clock = Clock::get()?;
        let market_key = ctx.accounts.market.key();
        let pool_key = ctx.accounts.pool_state.key();
        let position_key = ctx.accounts.position.key();
        let intent_key = ctx.accounts.operation_intent.key();
        let report = SupplyFlowReport {
            output_authenticated: true,
            computation_offset,
            intent_key,
            pool_output: EncryptedPoolState {
                nonce: pool_sealed.nonce,
                ciphertexts: pool_sealed.ciphertexts,
            },
            encryption_key: position_sealed.encryption_key,
            position_nonce: position_sealed.nonce,
            position_ciphertexts: position_sealed.ciphertexts,
            applied: o.field_2,
            requested: o.field_3,
            current_slot: clock.slot,
        };
        let accounts = &mut *ctx.accounts;
        let settlement = state::supply::settle_supply_flow(
            &accounts.market,
            market_key,
            &accounts.asset_state,
            &mut accounts.pool_state,
            pool_key,
            &mut accounts.position,
            position_key,
            &mut accounts.operation_intent,
            &report,
        )?;
        match settlement {
            SupplyFlowSettlement::Applied { is_withdraw } => emit!(SupplyFlowApplied {
                position: position_key,
                pool_state: pool_key,
                operation_intent: intent_key,
                is_withdraw,
                position_generation: accounts.position.coordination.generation,
                pool_generation: accounts.pool_state.coordination.generation,
                accounted_borrow_index: accounts.pool_state.accounted_borrow_index,
                slot: clock.slot,
            }),
            SupplyFlowSettlement::Zero | SupplyFlowSettlement::Failed(_) => {
                emit!(SupplyFlowNotApplied {
                    position: position_key,
                    pool_state: pool_key,
                    operation_intent: intent_key,
                    intent_status: accounts.operation_intent.status,
                    slot: clock.slot,
                })
            }
            SupplyFlowSettlement::Ignored => {}
        }
        Ok(())
    }

    /// Permissionless: abandons a queued supply or withdrawal after Lock
    /// expiry. Nothing moved, so there is nothing to refund.
    pub fn abandon_supply_flow(ctx: Context<AbandonSupplyFlow>) -> Result<()> {
        abandon_supply_flow_handler(ctx)
    }

    // ---- walnut-private (ADR-0004): encrypted-amount operations -------------

    /// Registers the `pool_op` computation definition.
    pub fn init_pool_op_comp_def(ctx: Context<InitPoolOpCompDef>) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Registers the `position_private` computation definition.
    pub fn init_position_private_comp_def(ctx: Context<InitPositionPrivateCompDef>) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Registers the `liquidate_pool` computation definition.
    pub fn init_liquidate_pool_comp_def(ctx: Context<InitLiquidatePoolCompDef>) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Owner-signed private Pool operation: `op` 0 supply, 1 withdraw supply,
    /// 2 borrow, 3 repay. The amount and the share count arrive encrypted
    /// (`input_*`); no argument, Intent field or event carries them. Accrues
    /// the LendingPool to `accrue_to_ts`, reads the Pyth price for a borrow,
    /// Locks the Pool and the Position and queues `pool_op`.
    #[allow(clippy::too_many_arguments)]
    pub fn pool_op(
        ctx: Context<PoolOpQueue>,
        computation_offset: u64,
        sequence: u64,
        op: u8,
        accrue_to_ts: i64,
        input_pubkey: [u8; 32],
        input_nonce: u128,
        input_ciphertexts: [[u8; 32]; 2],
    ) -> Result<()> {
        let op = u64::from(op);
        let kind = operation::OperationKind::PoolOp;
        let clock = Clock::get()?;
        if input_pubkey == [0u8; 32] {
            return Err(ErrorCode::PrivateInputKeyInvalid.into());
        }
        state::private_op::ensure_private_allowed(&ctx.accounts.asset_state, kind, op, clock.slot)?;
        state::private_op::ensure_rate_fresh(
            &ctx.accounts.lending_pool,
            &ctx.accounts.asset_state,
            kind,
            op,
            clock.slot,
        )?;
        let (price, publish_time) = private_price(
            kind,
            op,
            &ctx.accounts.lending_pool,
            &ctx.accounts.asset_state,
            &ctx.accounts.price_update,
            clock.unix_timestamp,
        )?;
        let accounts = &mut *ctx.accounts;
        state::private_op::accrue_to(
            &mut accounts.lending_pool,
            &accounts.asset_state,
            accrue_to_ts,
            clock.unix_timestamp,
        )?;
        let params = private_params(&accounts.lending_pool, &accounts.asset_state, op, price)?;
        let keys = PrivateKeys {
            payer: accounts.payer.key(),
            market: accounts.market.key(),
            asset_state: accounts.asset_state.key(),
            lending_pool: accounts.lending_pool.key(),
            pool: Some((
                accounts.pool_state.key(),
                Some(accounts.pool_state.coordination.generation),
            )),
            position: (
                accounts.position.key(),
                accounts.position.owner,
                Some(accounts.position.coordination.generation),
            ),
            second: None,
        };
        let intent_key = accounts.operation_intent.key();
        let request = PrivateRequest {
            kind,
            intent_key,
            sequence,
            computation_offset,
            queued_slot: clock.slot,
            params,
            publish_time,
        };
        let mut intent = state::private_op::queue_private(
            &accounts.market,
            &keys,
            Some(&mut accounts.pool_state),
            &mut accounts.position,
            None,
            &mut accounts.operation_counter,
            &request,
        )?;
        intent.bump = ctx.bumps.operation_intent;
        accounts.operation_intent.set_inner(intent);
        emit!(PrivateOpQueued {
            market: keys.market,
            position: keys.position.0,
            second_position: None,
            payer: keys.payer,
            operation_intent: intent_key,
            kind: kind.tag(),
            op,
            computation_offset,
            decision_expiry_slot: intent.decision_expiry_slot,
            lock_expiry_slot: intent.lock_expiry_slot,
            slot: clock.slot,
        });

        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;
        let output_nonce_input =
            state::nonce::derive_output_nonce_input(&keys.position.0, sequence);
        let args = pool_op_args(
            &ctx.accounts.pool_state,
            &ctx.accounts.position,
            input_pubkey,
            input_nonce,
            &input_ciphertexts,
            output_nonce_input,
            &params,
        )
        .build();
        let callback_accounts = private_callback_accounts(
            keys.market,
            keys.lending_pool,
            Some(ctx.accounts.pool_state.key()),
            keys.position.0,
            None,
            intent_key,
        );
        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![PoolOpCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &callback_accounts,
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Verifies the `pool_op` output. `Applied` stores the Pool and Position
    /// payloads and bumps both generations; anything else stores nothing and
    /// releases both Locks. Only the `applied` bit is public.
    #[arcium_callback(encrypted_ix = "pool_op")]
    pub fn pool_op_callback(
        ctx: Context<PoolOpCallback>,
        output: SignedComputationOutputs<PoolOpOutput>,
    ) -> Result<()> {
        let o = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(PoolOpOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::CallbackOutputNotAuthenticated.into()),
        };
        let computation_offset = callback_offset(
            ctx.accounts.operation_intent.computation_offset,
            ctx.accounts.computation_account.key(),
            &ctx.accounts.mxe_account,
        );
        let clock = Clock::get()?;
        let report = PrivateReport {
            output_authenticated: true,
            computation_offset,
            intent_key: ctx.accounts.operation_intent.key(),
            pool_output: Some(EncryptedPoolState {
                nonce: o.field_0.nonce,
                ciphertexts: o.field_0.ciphertexts,
            }),
            primary: SealedPosition {
                encryption_key: o.field_1.encryption_key,
                nonce: o.field_1.nonce,
                ciphertexts: o.field_1.ciphertexts,
            },
            second: None,
            applied: o.field_2,
            current_slot: clock.slot,
        };
        let accounts = &mut *ctx.accounts;
        let keys = PrivateKeys {
            payer: accounts.operation_intent.payer,
            market: accounts.market.key(),
            asset_state: accounts.market.asset_state,
            lending_pool: accounts.lending_pool.key(),
            pool: Some((accounts.pool_state.key(), None)),
            position: (accounts.position.key(), accounts.position.owner, None),
            second: None,
        };
        let status = state::private_op::settle_private(
            &keys,
            Some(&mut accounts.pool_state),
            &mut accounts.position,
            None,
            &mut accounts.operation_intent,
            operation::OperationKind::PoolOp,
            &report,
        )?;
        if let Some(status) = status {
            emit!(PrivateOpSettled {
                position: keys.position.0,
                second_position: None,
                operation_intent: report.intent_key,
                kind: operation::OperationKind::PoolOp.tag(),
                op: accounts.operation_intent.asset_generation,
                applied: status == operation::IntentStatus::Applied,
                intent_status: status,
                slot: clock.slot,
            });
        }
        Ok(())
    }

    /// Owner-signed private collateral operation: `op` 0 add collateral, 1
    /// withdraw collateral. The lamport amount arrives encrypted. Reads the
    /// Pyth price for a withdrawal, Locks the Position and queues
    /// `position_private`.
    #[allow(clippy::too_many_arguments)]
    pub fn position_private(
        ctx: Context<PositionPrivateQueue>,
        computation_offset: u64,
        sequence: u64,
        op: u8,
        accrue_to_ts: i64,
        input_pubkey: [u8; 32],
        input_nonce: u128,
        input_ciphertexts: [[u8; 32]; 2],
    ) -> Result<()> {
        let op = u64::from(op);
        let kind = operation::OperationKind::PositionPrivate;
        let clock = Clock::get()?;
        if input_pubkey == [0u8; 32] {
            return Err(ErrorCode::PrivateInputKeyInvalid.into());
        }
        state::private_op::ensure_private_allowed(&ctx.accounts.asset_state, kind, op, clock.slot)?;
        let (price, publish_time) = private_price(
            kind,
            op,
            &ctx.accounts.lending_pool,
            &ctx.accounts.asset_state,
            &ctx.accounts.price_update,
            clock.unix_timestamp,
        )?;
        let accounts = &mut *ctx.accounts;
        state::private_op::accrue_to(
            &mut accounts.lending_pool,
            &accounts.asset_state,
            accrue_to_ts,
            clock.unix_timestamp,
        )?;
        let params = private_params(&accounts.lending_pool, &accounts.asset_state, op, price)?;
        let keys = PrivateKeys {
            payer: accounts.payer.key(),
            market: accounts.market.key(),
            asset_state: accounts.asset_state.key(),
            lending_pool: accounts.lending_pool.key(),
            pool: None,
            position: (
                accounts.position.key(),
                accounts.position.owner,
                Some(accounts.position.coordination.generation),
            ),
            second: None,
        };
        let intent_key = accounts.operation_intent.key();
        let request = PrivateRequest {
            kind,
            intent_key,
            sequence,
            computation_offset,
            queued_slot: clock.slot,
            params,
            publish_time,
        };
        let mut intent = state::private_op::queue_private(
            &accounts.market,
            &keys,
            None,
            &mut accounts.position,
            None,
            &mut accounts.operation_counter,
            &request,
        )?;
        intent.bump = ctx.bumps.operation_intent;
        accounts.operation_intent.set_inner(intent);
        emit!(PrivateOpQueued {
            market: keys.market,
            position: keys.position.0,
            second_position: None,
            payer: keys.payer,
            operation_intent: intent_key,
            kind: kind.tag(),
            op,
            computation_offset,
            decision_expiry_slot: intent.decision_expiry_slot,
            lock_expiry_slot: intent.lock_expiry_slot,
            slot: clock.slot,
        });

        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;
        let output_nonce_input =
            state::nonce::derive_output_nonce_input(&keys.position.0, sequence);
        let args = position_private_args(
            &ctx.accounts.position,
            input_pubkey,
            input_nonce,
            &input_ciphertexts,
            output_nonce_input,
            &params,
        )
        .build();
        let callback_accounts = private_callback_accounts(
            keys.market,
            keys.lending_pool,
            None,
            keys.position.0,
            None,
            intent_key,
        );
        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![PositionPrivateCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &callback_accounts,
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Verifies the `position_private` output (see `pool_op_callback`).
    #[arcium_callback(encrypted_ix = "position_private")]
    pub fn position_private_callback(
        ctx: Context<PositionPrivateCallback>,
        output: SignedComputationOutputs<PositionPrivateOutput>,
    ) -> Result<()> {
        let o = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(PositionPrivateOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::CallbackOutputNotAuthenticated.into()),
        };
        let computation_offset = callback_offset(
            ctx.accounts.operation_intent.computation_offset,
            ctx.accounts.computation_account.key(),
            &ctx.accounts.mxe_account,
        );
        let clock = Clock::get()?;
        let report = PrivateReport {
            output_authenticated: true,
            computation_offset,
            intent_key: ctx.accounts.operation_intent.key(),
            pool_output: None,
            primary: SealedPosition {
                encryption_key: o.field_0.encryption_key,
                nonce: o.field_0.nonce,
                ciphertexts: o.field_0.ciphertexts,
            },
            second: None,
            applied: o.field_1,
            current_slot: clock.slot,
        };
        let accounts = &mut *ctx.accounts;
        let keys = PrivateKeys {
            payer: accounts.operation_intent.payer,
            market: accounts.market.key(),
            asset_state: accounts.market.asset_state,
            lending_pool: accounts.lending_pool.key(),
            pool: None,
            position: (accounts.position.key(), accounts.position.owner, None),
            second: None,
        };
        let status = state::private_op::settle_private(
            &keys,
            None,
            &mut accounts.position,
            None,
            &mut accounts.operation_intent,
            operation::OperationKind::PositionPrivate,
            &report,
        )?;
        if let Some(status) = status {
            emit!(PrivateOpSettled {
                position: keys.position.0,
                second_position: None,
                operation_intent: report.intent_key,
                kind: operation::OperationKind::PositionPrivate.tag(),
                op: accounts.operation_intent.asset_generation,
                applied: status == operation::IntentStatus::Applied,
                intent_status: status,
                slot: clock.slot,
            });
        }
        Ok(())
    }

    /// Permissionless private liquidation. The signer's own Position pays the
    /// encrypted repay amount from its free USDC and receives the seized
    /// lamports into its free SOL; the target's health is checked inside the
    /// circuit. No token moves. Locks the Pool, the target and the
    /// liquidator's Position and queues `liquidate_pool`.
    #[allow(clippy::too_many_arguments)]
    pub fn liquidate_pool(
        ctx: Context<LiquidatePoolQueue>,
        computation_offset: u64,
        sequence: u64,
        accrue_to_ts: i64,
        input_pubkey: [u8; 32],
        input_nonce: u128,
        input_ciphertexts: [[u8; 32]; 3],
    ) -> Result<()> {
        let kind = operation::OperationKind::LiquidatePool;
        let clock = Clock::get()?;
        if input_pubkey == [0u8; 32] {
            return Err(ErrorCode::PrivateInputKeyInvalid.into());
        }
        state::private_op::ensure_private_allowed(&ctx.accounts.asset_state, kind, 0, clock.slot)?;
        let (price, publish_time) = private_price(
            kind,
            0,
            &ctx.accounts.lending_pool,
            &ctx.accounts.asset_state,
            &ctx.accounts.price_update,
            clock.unix_timestamp,
        )?;
        let accounts = &mut *ctx.accounts;
        state::private_op::accrue_to(
            &mut accounts.lending_pool,
            &accounts.asset_state,
            accrue_to_ts,
            clock.unix_timestamp,
        )?;
        let params = private_params(&accounts.lending_pool, &accounts.asset_state, 0, price)?;
        let keys = PrivateKeys {
            payer: accounts.payer.key(),
            market: accounts.market.key(),
            asset_state: accounts.asset_state.key(),
            lending_pool: accounts.lending_pool.key(),
            pool: Some((
                accounts.pool_state.key(),
                Some(accounts.pool_state.coordination.generation),
            )),
            position: (
                accounts.position.key(),
                accounts.position.owner,
                Some(accounts.position.coordination.generation),
            ),
            second: Some((
                accounts.liquidator_position.key(),
                accounts.liquidator_position.owner,
                Some(accounts.liquidator_position.coordination.generation),
            )),
        };
        let intent_key = accounts.operation_intent.key();
        let request = PrivateRequest {
            kind,
            intent_key,
            sequence,
            computation_offset,
            queued_slot: clock.slot,
            params,
            publish_time,
        };
        let mut intent = state::private_op::queue_private(
            &accounts.market,
            &keys,
            Some(&mut accounts.pool_state),
            &mut accounts.position,
            Some(&mut accounts.liquidator_position),
            &mut accounts.operation_counter,
            &request,
        )?;
        intent.bump = ctx.bumps.operation_intent;
        accounts.operation_intent.set_inner(intent);
        let liquidator_key = accounts.liquidator_position.key();
        emit!(PrivateOpQueued {
            market: keys.market,
            position: keys.position.0,
            second_position: Some(liquidator_key),
            payer: keys.payer,
            operation_intent: intent_key,
            kind: kind.tag(),
            op: 0,
            computation_offset,
            decision_expiry_slot: intent.decision_expiry_slot,
            lock_expiry_slot: intent.lock_expiry_slot,
            slot: clock.slot,
        });

        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;
        let target_nonce = state::nonce::derive_output_nonce_input(&keys.position.0, sequence);
        let liquidator_nonce = state::nonce::derive_output_nonce_input(&liquidator_key, sequence);
        let args = liquidate_pool_args(
            &ctx.accounts.pool_state,
            &ctx.accounts.position,
            &ctx.accounts.liquidator_position,
            input_pubkey,
            input_nonce,
            &input_ciphertexts,
            target_nonce,
            liquidator_nonce,
            &params,
        )
        .build();
        let callback_accounts = private_callback_accounts(
            keys.market,
            keys.lending_pool,
            Some(ctx.accounts.pool_state.key()),
            keys.position.0,
            Some(liquidator_key),
            intent_key,
        );
        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![LiquidatePoolCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &callback_accounts,
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Verifies the `liquidate_pool` output. `Applied` stores the Pool, the
    /// target and the liquidator payloads and bumps all three generations.
    #[arcium_callback(encrypted_ix = "liquidate_pool")]
    pub fn liquidate_pool_callback(
        ctx: Context<LiquidatePoolCallback>,
        output: SignedComputationOutputs<LiquidatePoolOutput>,
    ) -> Result<()> {
        let o = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(LiquidatePoolOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::CallbackOutputNotAuthenticated.into()),
        };
        let computation_offset = callback_offset(
            ctx.accounts.operation_intent.computation_offset,
            ctx.accounts.computation_account.key(),
            &ctx.accounts.mxe_account,
        );
        let clock = Clock::get()?;
        let report = PrivateReport {
            output_authenticated: true,
            computation_offset,
            intent_key: ctx.accounts.operation_intent.key(),
            pool_output: Some(EncryptedPoolState {
                nonce: o.field_0.nonce,
                ciphertexts: o.field_0.ciphertexts,
            }),
            primary: SealedPosition {
                encryption_key: o.field_1.encryption_key,
                nonce: o.field_1.nonce,
                ciphertexts: o.field_1.ciphertexts,
            },
            second: Some(SealedPosition {
                encryption_key: o.field_2.encryption_key,
                nonce: o.field_2.nonce,
                ciphertexts: o.field_2.ciphertexts,
            }),
            applied: o.field_3,
            current_slot: clock.slot,
        };
        let accounts = &mut *ctx.accounts;
        let keys = PrivateKeys {
            payer: accounts.operation_intent.payer,
            market: accounts.market.key(),
            asset_state: accounts.market.asset_state,
            lending_pool: accounts.lending_pool.key(),
            pool: Some((accounts.pool_state.key(), None)),
            position: (accounts.position.key(), accounts.position.owner, None),
            second: Some((
                accounts.liquidator_position.key(),
                accounts.liquidator_position.owner,
                None,
            )),
        };
        let status = state::private_op::settle_private(
            &keys,
            Some(&mut accounts.pool_state),
            &mut accounts.position,
            Some(&mut accounts.liquidator_position),
            &mut accounts.operation_intent,
            operation::OperationKind::LiquidatePool,
            &report,
        )?;
        if let Some(status) = status {
            emit!(PrivateOpSettled {
                position: keys.position.0,
                second_position: keys.second.map(|s| s.0),
                operation_intent: report.intent_key,
                kind: operation::OperationKind::LiquidatePool.tag(),
                op: 0,
                applied: status == operation::IntentStatus::Applied,
                intent_status: status,
                slot: clock.slot,
            });
        }
        Ok(())
    }

    /// Permissionless: abandons a queued private operation after Lock expiry.
    pub fn abandon_private_op(ctx: Context<AbandonPrivateOp>) -> Result<()> {
        abandon_private_op_handler(ctx)
    }

    /// Registers the `pool_band` computation definition.
    pub fn init_pool_band_comp_def(ctx: Context<InitPoolBandCompDef>) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Permissionless: queues the light `pool_band` circuit, which reveals only
    /// the private Pool's utilization band at the LendingPool borrow index
    /// (ADR-0004). Same Intent rules as `update_utilization_band` (Pool-owned
    /// counter, no Lock); the callback stores the band on the Asset State.
    pub fn update_pool_band(
        ctx: Context<UpdatePoolBandQueue>,
        computation_offset: u64,
        sequence: u64,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let market_key = ctx.accounts.market.key();
        let pool_key = ctx.accounts.pool_state.key();
        let intent_key = ctx.accounts.operation_intent.key();
        if ctx.accounts.operation_counter.owner == Pubkey::default() {
            ctx.accounts
                .operation_counter
                .set_inner(OperationCounter::new(pool_key, ctx.bumps.operation_counter));
        }
        let keys = BandKeys {
            payer: ctx.accounts.payer.key(),
            market: market_key,
            asset_state: ctx.accounts.asset_state.key(),
            pool: pool_key,
        };
        let request = BandRequest {
            intent_key,
            sequence,
            computation_offset,
            queued_slot: clock.slot,
        };
        let borrow_index = u64::try_from(ctx.accounts.lending_pool.borrow_index_1e9)
            .map_err(|_| ErrorCode::LendOpInvalid)?;
        let accounts = &mut *ctx.accounts;
        let mut intent = state::band::queue_band_update_at(
            accounts.market.status,
            &keys,
            &accounts.asset_state,
            &accounts.pool_state,
            &mut accounts.operation_counter,
            &request,
            u128::from(borrow_index),
        )?;
        intent.bump = ctx.bumps.operation_intent;
        accounts.operation_intent.set_inner(intent);
        emit!(UtilizationBandQueued {
            market: market_key,
            pool_state: pool_key,
            operation_intent: intent_key,
            computation_offset,
            pool_generation: accounts.pool_state.coordination.generation,
            rate_epoch_generation: intent.rate_epoch_generation,
            accounting_borrow_index: intent.accounting_borrow_index,
            decision_expiry_slot: intent.decision_expiry_slot,
            slot: clock.slot,
        });
        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;
        let c = &ctx.accounts.pool_state.encrypted.ciphertexts;
        let args = ArgBuilder::new()
            .plaintext_u128(ctx.accounts.pool_state.encrypted.nonce)
            .encrypted_u128(c[0])
            .encrypted_u128(c[1])
            .encrypted_u128(c[2])
            .encrypted_u128(c[3])
            .plaintext_u64(borrow_index)
            .build();
        let callback_accounts = [
            CallbackAccount {
                pubkey: market_key,
                is_writable: false,
            },
            CallbackAccount {
                pubkey: keys.asset_state,
                is_writable: true,
            },
            CallbackAccount {
                pubkey: pool_key,
                is_writable: false,
            },
            CallbackAccount {
                pubkey: intent_key,
                is_writable: true,
            },
        ];
        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![PoolBandCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &callback_accounts,
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Verifies the revealed band and stores it with the same rules as
    /// `update_utilization_band_callback`.
    #[arcium_callback(encrypted_ix = "pool_band")]
    pub fn pool_band_callback(
        ctx: Context<PoolBandCallback>,
        output: SignedComputationOutputs<PoolBandOutput>,
    ) -> Result<()> {
        let band = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(PoolBandOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::CallbackOutputNotAuthenticated.into()),
        };
        let computation_offset = callback_offset(
            ctx.accounts.operation_intent.computation_offset,
            ctx.accounts.computation_account.key(),
            &ctx.accounts.mxe_account,
        );
        let clock = Clock::get()?;
        let intent_key = ctx.accounts.operation_intent.key();
        let asset_state_key = ctx.accounts.asset_state.key();
        let keys = BandKeys {
            payer: ctx.accounts.operation_intent.payer,
            market: ctx.accounts.market.key(),
            asset_state: asset_state_key,
            pool: ctx.accounts.pool_state.key(),
        };
        let report = BandReport {
            output_authenticated: true,
            computation_offset,
            intent_key,
            band,
            current_slot: clock.slot,
        };
        let accounts = &mut *ctx.accounts;
        let settlement = state::band::settle_band_update(
            &keys,
            &mut accounts.asset_state,
            &accounts.pool_state,
            &mut accounts.operation_intent,
            &report,
        )?;
        match settlement {
            BandSettlement::Applied {
                band,
                observation_slot,
                written,
            } => emit!(UtilizationBandUpdated {
                asset_state: asset_state_key,
                operation_intent: intent_key,
                band,
                observation_slot,
                written,
                slot: clock.slot,
            }),
            BandSettlement::Stale(_) | BandSettlement::Failed(_) => {
                emit!(UtilizationBandNotApplied {
                    operation_intent: intent_key,
                    intent_status: accounts.operation_intent.status,
                    slot: clock.slot,
                })
            }
            BandSettlement::Ignored => {}
        }
        Ok(())
    }

    /// Admin: switches the LendingPool rate source (0 public totals, 1
    /// utilization band of the encrypted Pool).
    pub fn set_lending_rate_source(
        ctx: Context<SetLendingRateSource>,
        rate_source: u8,
    ) -> Result<()> {
        set_lending_rate_source_handler(ctx, rate_source)
    }

    // ---- Task 8.1: separately reviewed recovery of an empty Pool -----------

    /// Registers the `recover_empty_pool` computation definition.
    pub fn init_recover_empty_pool_comp_def(
        ctx: Context<InitRecoverEmptyPoolCompDef>,
    ) -> Result<()> {
        init_computation_def(ctx.accounts, None)?;
        Ok(())
    }

    /// Admin-signed: reclassifies an empty Pool's provable dust residue as
    /// Reserves so the treasury can withdraw it and supply can reopen. See
    /// `state::recovery` for the review notes. Locks the Pool, accrues the
    /// accounting Borrow Index, creates the Intent at `sequence` (Pool-owned
    /// counter), and queues `recover_empty_pool`. No token moves.
    pub fn recover_empty_pool(
        ctx: Context<RecoverEmptyPoolQueue>,
        computation_offset: u64,
        sequence: u64,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let signer = ctx.accounts.payer.key();
        let market_key = ctx.accounts.market.key();
        let pool_key = ctx.accounts.pool_state.key();
        let asset_state_key = ctx.accounts.asset_state.key();
        let intent_key = ctx.accounts.operation_intent.key();
        if ctx.accounts.operation_counter.owner == Pubkey::default() {
            ctx.accounts
                .operation_counter
                .set_inner(OperationCounter::new(pool_key, ctx.bumps.operation_counter));
        }
        let request = RecoverEmptyPoolRequest {
            intent_key,
            sequence,
            computation_offset,
            queued_slot: clock.slot,
            now_ts: clock.unix_timestamp,
        };
        let accounts = &mut *ctx.accounts;
        let accounted_index = accounts.pool_state.accounted_borrow_index;
        let reserve_factor_bps = accounts.asset_state.risk.reserve_factor_bps;
        let claims_clear = state::first_supplier::claims_clear(&accounts.market);
        let mut intent = state::recovery::queue_recover_empty_pool(
            &accounts.market,
            market_key,
            asset_state_key,
            &mut accounts.asset_state,
            &mut accounts.pool_state,
            pool_key,
            &mut accounts.operation_counter,
            &signer,
            &request,
        )?;
        intent.bump = ctx.bumps.operation_intent;
        accounts.operation_intent.set_inner(intent);
        emit!(EmptyPoolRecoveryQueued {
            market: market_key,
            pool_state: pool_key,
            operation_intent: intent_key,
            admin: signer,
            computation_offset,
            pool_generation: accounts.pool_state.coordination.generation,
            dust_limit: math::supply::RECOVERY_DUST_LIMIT,
            decision_expiry_slot: intent.decision_expiry_slot,
            lock_expiry_slot: intent.lock_expiry_slot,
            slot: clock.slot,
        });

        // `recover_empty_pool(pool, accounted_index, accounting_index,
        // reserve_factor_bps, dust_limit, claims_clear)`.
        ctx.accounts.sign_pda_account.bump = ctx.bumps.sign_pda_account;
        let c = &ctx.accounts.pool_state.encrypted.ciphertexts;
        let args = ArgBuilder::new()
            .plaintext_u128(ctx.accounts.pool_state.encrypted.nonce)
            .encrypted_u128(c[0])
            .encrypted_u128(c[1])
            .encrypted_u128(c[2])
            .encrypted_u128(c[3])
            .plaintext_u128(accounted_index)
            .plaintext_u128(intent.accounting_borrow_index)
            .plaintext_u128(reserve_factor_bps as u128)
            .plaintext_u128(math::supply::RECOVERY_DUST_LIMIT)
            .plaintext_bool(claims_clear)
            .build();
        let callback_accounts = [
            CallbackAccount {
                pubkey: market_key,
                is_writable: false,
            },
            CallbackAccount {
                pubkey: asset_state_key,
                is_writable: false,
            },
            CallbackAccount {
                pubkey: pool_key,
                is_writable: true,
            },
            CallbackAccount {
                pubkey: intent_key,
                is_writable: true,
            },
        ];
        queue_computation(
            ctx.accounts,
            computation_offset,
            args,
            vec![RecoverEmptyPoolCallback::callback_ix(
                computation_offset,
                &ctx.accounts.mxe_account,
                &callback_accounts,
            )?],
            1,
            0,
            0,
        )?;
        Ok(())
    }

    /// Verifies the `recover_empty_pool` output. `Applied` stores the Pool
    /// payload, advances the accounted index, bumps the generation, and
    /// releases the Lock. `Zero`, `Stale`, and `Expired` release the Lock and
    /// store nothing. Unauthenticated output and binding mismatches revert,
    /// so the Intent stays `Queued` for `abandon_recover_empty_pool`.
    #[arcium_callback(encrypted_ix = "recover_empty_pool")]
    pub fn recover_empty_pool_callback(
        ctx: Context<RecoverEmptyPoolCallback>,
        output: SignedComputationOutputs<RecoverEmptyPoolOutput>,
    ) -> Result<()> {
        let o = match output.verify_output(
            &ctx.accounts.cluster_account,
            &ctx.accounts.computation_account,
        ) {
            Ok(RecoverEmptyPoolOutput { field_0 }) => field_0,
            Err(_) => return Err(ErrorCode::CallbackOutputNotAuthenticated.into()),
        };
        let sealed = o.field_0;

        let intent_offset = ctx.accounts.operation_intent.computation_offset;
        let computation_offset = if ctx.accounts.computation_account.key()
            == derive_comp_pda!(intent_offset, ctx.accounts.mxe_account)
        {
            intent_offset
        } else {
            intent_offset.wrapping_add(1)
        };

        let clock = Clock::get()?;
        let market_key = ctx.accounts.market.key();
        let pool_key = ctx.accounts.pool_state.key();
        let intent_key = ctx.accounts.operation_intent.key();
        let report = RecoverEmptyPoolReport {
            output_authenticated: true,
            computation_offset,
            intent_key,
            output: EncryptedPoolState {
                nonce: sealed.nonce,
                ciphertexts: sealed.ciphertexts,
            },
            applied: o.field_1,
            current_slot: clock.slot,
        };
        let accounts = &mut *ctx.accounts;
        let settlement = state::recovery::settle_recover_empty_pool(
            &accounts.market,
            market_key,
            &accounts.asset_state,
            &mut accounts.pool_state,
            pool_key,
            &mut accounts.operation_intent,
            &report,
        )?;
        match settlement {
            RecoverEmptyPoolSettlement::Recovered => emit!(EmptyPoolRecovered {
                pool_state: pool_key,
                operation_intent: intent_key,
                generation: accounts.pool_state.coordination.generation,
                accounted_borrow_index: accounts.pool_state.accounted_borrow_index,
                slot: clock.slot,
            }),
            RecoverEmptyPoolSettlement::Zero | RecoverEmptyPoolSettlement::Failed(_) => {
                emit!(EmptyPoolRecoveryNotApplied {
                    pool_state: pool_key,
                    operation_intent: intent_key,
                    intent_status: accounts.operation_intent.status,
                    slot: clock.slot,
                })
            }
            RecoverEmptyPoolSettlement::Ignored => {}
        }
        Ok(())
    }

    /// Permissionless: abandons a queued recovery after Lock expiry.
    pub fn abandon_recover_empty_pool(ctx: Context<AbandonRecoverEmptyPool>) -> Result<()> {
        abandon_recover_empty_pool_handler(ctx)
    }
}

// ---- Task 5.3: shield accounts and helpers ----------------------------------

/// `shield_credit(state: Enc<Shared, PositionSecret>, is_usdc: bool, amount:
/// u64)` arguments: the stored state (Viewing public key, nonce, five
/// ciphertexts in `PositionSecret` order) read from the Position account,
/// then the public asset flag and amount.
fn shield_credit_args(
    position: &Position,
    output_nonce_input: u128,
    asset: ShieldAsset,
    amount: u64,
) -> ArgBuilder {
    let c = &position.encrypted_state.ciphertexts;
    ArgBuilder::new()
        .x25519_pubkey(position.viewing_pubkey)
        .plaintext_u128(position.encrypted_state.nonce)
        .encrypted_u64(c[0])
        .encrypted_u64(c[1])
        .encrypted_u64(c[2])
        .encrypted_u128(c[3])
        .encrypted_u128(c[4])
        .x25519_pubkey(position.viewing_pubkey)
        .plaintext_u128(output_nonce_input)
        .plaintext_bool(asset == ShieldAsset::Usdc)
        .plaintext_u64(amount)
}

/// The `shield_credit` callback's custom accounts, in struct order.
fn shield_callback_accounts(
    market: Pubkey,
    position: Pubkey,
    pending_shield: Pubkey,
    operation_intent: Pubkey,
) -> [CallbackAccount; 4] {
    [
        CallbackAccount {
            pubkey: market,
            is_writable: false,
        },
        CallbackAccount {
            pubkey: position,
            is_writable: true,
        },
        CallbackAccount {
            pubkey: pending_shield,
            is_writable: true,
        },
        CallbackAccount {
            pubkey: operation_intent,
            is_writable: true,
        },
    ]
}

#[queue_computation_accounts("shield_credit", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, sequence: u64, shield_sequence: u64)]
pub struct ShieldDeposit<'info> {
    /// The Position owner; signs the token transfer and pays rent and fees.
    #[account(mut)]
    pub payer: Signer<'info>,
    /// Not init_if_needed: a Position is Active only after its
    /// initialization computation was queued, and initialize_position creates
    /// this program-wide PDA. seeds + bump already pin the address, so the
    /// template's extra address = derive_sign_pda!() is dropped. Both changes
    /// keep try_accounts inside the 4,096-byte SBF stack frame.
    #[account(mut, seeds = [&SIGN_PDA_SEED], bump)]
    pub sign_pda_account: Box<Account<'info, ArciumSignerAccount>>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_SHIELD_CREDIT))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Box<Account<'info, FeePool>>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Box<Account<'info, ClockAccount>>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    /// Read for the `SHIELD` pause.
    #[account(address = market.asset_state)]
    pub asset_state: Box<Account<'info, AssetState>>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, market.key().as_ref(), payer.key.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    #[account(
        mut,
        seeds = [OPERATION_COUNTER_SEED, payer.key.as_ref()],
        bump = operation_counter.bump,
    )]
    pub operation_counter: Box<Account<'info, OperationCounter>>,
    #[account(
        init,
        payer = payer,
        space = OperationIntent::ACCOUNT_LEN,
        seeds = [OPERATION_INTENT_SEED, payer.key.as_ref(), &sequence.to_le_bytes()],
        bump,
    )]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
    #[account(
        init,
        payer = payer,
        space = PendingShield::ACCOUNT_LEN,
        seeds = [PENDING_SHIELD_SEED, position.key().as_ref(), &shield_sequence.to_le_bytes()],
        bump,
    )]
    pub pending_shield: Box<Account<'info, PendingShield>>,
    /// The asset mint; `state::shield` checks it is the Market's.
    pub mint: Box<Account<'info, Mint>>,
    /// The asset vault; `state::shield` checks it is the Market's.
    #[account(mut)]
    pub vault: Box<Account<'info, TokenAccount>>,
    /// The owner's token account of the asset (wSOL for SOL).
    #[account(mut)]
    pub source_token_account: Box<Account<'info, TokenAccount>>,
    /// Classic SPL Token only; Token-2022 is rejected.
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[queue_computation_accounts("shield_credit", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, sequence: u64)]
pub struct RetryShield<'info> {
    /// The Pending Shield owner; pays rent and the computation fee.
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init_if_needed,
        space = 9,
        payer = payer,
        seeds = [&SIGN_PDA_SEED],
        bump,
        address = derive_sign_pda!(),
    )]
    pub sign_pda_account: Box<Account<'info, ArciumSignerAccount>>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_SHIELD_CREDIT))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Box<Account<'info, FeePool>>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Box<Account<'info, ClockAccount>>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, market.key().as_ref(), payer.key.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    #[account(
        mut,
        seeds = [OPERATION_COUNTER_SEED, payer.key.as_ref()],
        bump = operation_counter.bump,
    )]
    pub operation_counter: Box<Account<'info, OperationCounter>>,
    #[account(
        init,
        payer = payer,
        space = OperationIntent::ACCOUNT_LEN,
        seeds = [OPERATION_INTENT_SEED, payer.key.as_ref(), &sequence.to_le_bytes()],
        bump,
    )]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
    #[account(
        mut,
        seeds = [
            PENDING_SHIELD_SEED,
            position.key().as_ref(),
            &pending_shield.sequence.to_le_bytes(),
        ],
        bump = pending_shield.bump,
    )]
    pub pending_shield: Box<Account<'info, PendingShield>>,
    /// The Pending Shield's active attempt; it must be terminal.
    #[account(address = pending_shield.active_intent)]
    pub prior_intent: Box<Account<'info, OperationIntent>>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[callback_accounts("shield_credit")]
#[derive(Accounts)]
pub struct ShieldCreditCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_SHIELD_CREDIT))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    #[account(
        mut,
        seeds = [
            PENDING_SHIELD_SEED,
            position.key().as_ref(),
            &pending_shield.sequence.to_le_bytes(),
        ],
        bump = pending_shield.bump,
    )]
    pub pending_shield: Box<Account<'info, PendingShield>>,
    /// The attempt's Operation Intent; `classify_callback` verifies it.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

#[init_computation_definition_accounts("shield_credit", payer)]
#[derive(Accounts)]
pub struct InitShieldCreditCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

// ---- Task 5.4: unshield accounts and helpers --------------------------------

/// `unshield(state: Enc<Shared, PositionSecret>, owner: Shared, is_usdc: bool,
/// amount: u64)` arguments: the stored state (Viewing public key, nonce, five
/// ciphertexts in `PositionSecret` order), the bare `Shared` output owner
/// (Viewing public key and the unique output-nonce input, `state::nonce`),
/// then the public asset flag and requested amount.
fn unshield_args(
    position: &Position,
    output_nonce_input: u128,
    asset: ShieldAsset,
    amount: u64,
) -> ArgBuilder {
    shield_credit_args(position, output_nonce_input, asset, amount)
}

/// The `unshield` callback's custom accounts, in struct order: Market,
/// Position, Operation Intent, mint, vault, destination, vault authority, and
/// the SPL Token program.
fn unshield_callback_accounts(
    market: &Market,
    asset: ShieldAsset,
    market_key: Pubkey,
    position: Pubkey,
    operation_intent: Pubkey,
    destination: Pubkey,
) -> [CallbackAccount; 8] {
    let account = |pubkey, is_writable| CallbackAccount {
        pubkey,
        is_writable,
    };
    [
        account(market_key, false),
        account(position, true),
        account(operation_intent, true),
        account(state::shield::asset_mint(market, asset), false),
        account(state::shield::asset_vault(market, asset), true),
        account(destination, true),
        account(market.vault_authority, false),
        account(anchor_spl::token::ID, false),
    ]
}

#[queue_computation_accounts("unshield", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, sequence: u64)]
pub struct UnshieldQueue<'info> {
    /// The Position owner; signs, pays rent and the computation fee.
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init_if_needed,
        space = 9,
        payer = payer,
        seeds = [&SIGN_PDA_SEED],
        bump,
        address = derive_sign_pda!(),
    )]
    pub sign_pda_account: Account<'info, ArciumSignerAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_UNSHIELD))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Account<'info, FeePool>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Account<'info, ClockAccount>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    /// Read for the `UNSHIELD` pause.
    #[account(address = market.asset_state)]
    pub asset_state: Box<Account<'info, AssetState>>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, market.key().as_ref(), payer.key().as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    #[account(
        mut,
        seeds = [OPERATION_COUNTER_SEED, payer.key().as_ref()],
        bump = operation_counter.bump,
    )]
    pub operation_counter: Box<Account<'info, OperationCounter>>,
    #[account(
        init,
        payer = payer,
        space = OperationIntent::ACCOUNT_LEN,
        seeds = [OPERATION_INTENT_SEED, payer.key().as_ref(), &sequence.to_le_bytes()],
        bump,
    )]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
    /// The destination token account of the asset's mint, owned by the
    /// payer; `state::unshield` checks it. It is bound in the Intent.
    pub destination: Box<Account<'info, TokenAccount>>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[callback_accounts("unshield")]
#[derive(Accounts)]
pub struct UnshieldCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_UNSHIELD))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    /// The attempt's Operation Intent; `classify_callback` verifies it.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
    /// The asset mint (bound in the Intent).
    pub mint: Box<Account<'info, Mint>>,
    /// The asset vault (bound in the Intent).
    #[account(mut)]
    pub vault: Box<Account<'info, TokenAccount>>,
    /// The committed destination token account (bound in the Intent).
    #[account(mut)]
    pub destination: Box<Account<'info, TokenAccount>>,
    /// Signs the vault transfer.
    #[account(
        seeds = [VAULT_AUTHORITY_SEED, market.key().as_ref()],
        bump = market.vault_authority_bump,
        address = market.vault_authority,
    )]
    pub vault_authority: Box<Account<'info, VaultAuthority>>,
    /// Classic SPL Token only; Token-2022 is rejected.
    pub token_program: Program<'info, Token>,
}

#[init_computation_definition_accounts("unshield", payer)]
#[derive(Accounts)]
pub struct InitUnshieldCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

// ---- walnut-diffn: position_op accounts and helpers ---------------------------

/// `position_op(state, owner: Shared, op: u8, amount: u64, price: u64,
/// ltv_bps: u64)` arguments: the stored state, the bare `Shared` output owner
/// with the derived nonce input, then the four public values.
fn position_op_args(
    position: &Position,
    output_nonce_input: u128,
    op: u8,
    amount: u64,
    price: u64,
    ltv_bps: u64,
) -> ArgBuilder {
    let c = &position.encrypted_state.ciphertexts;
    ArgBuilder::new()
        .x25519_pubkey(position.viewing_pubkey)
        .plaintext_u128(position.encrypted_state.nonce)
        .encrypted_u64(c[0])
        .encrypted_u64(c[1])
        .encrypted_u64(c[2])
        .encrypted_u128(c[3])
        .encrypted_u128(c[4])
        .x25519_pubkey(position.viewing_pubkey)
        .plaintext_u128(output_nonce_input)
        .plaintext_u8(op)
        .plaintext_u64(amount)
        .plaintext_u64(price)
        .plaintext_u64(ltv_bps)
}

#[queue_computation_accounts("position_op", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, sequence: u64)]
pub struct PositionOpQueue<'info> {
    /// The Position owner; signs, pays rent and the computation fee.
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, seeds = [&SIGN_PDA_SEED], bump)]
    pub sign_pda_account: Account<'info, ArciumSignerAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_POSITION_OP))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Account<'info, FeePool>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Account<'info, ClockAccount>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    /// Read for the LTV.
    #[account(address = market.asset_state)]
    pub asset_state: Box<Account<'info, AssetState>>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, market.key().as_ref(), payer.key().as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    #[account(
        mut,
        seeds = [OPERATION_COUNTER_SEED, payer.key().as_ref()],
        bump = operation_counter.bump,
    )]
    pub operation_counter: Box<Account<'info, OperationCounter>>,
    #[account(
        init,
        payer = payer,
        space = OperationIntent::ACCOUNT_LEN,
        seeds = [OPERATION_INTENT_SEED, payer.key().as_ref(), &sequence.to_le_bytes()],
        bump,
    )]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[callback_accounts("position_op")]
#[derive(Accounts)]
pub struct PositionOpCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_POSITION_OP))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    /// The attempt's Operation Intent; `classify_callback` verifies it.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

#[init_computation_definition_accounts("position_op", payer)]
#[derive(Accounts)]
pub struct InitPositionOpCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

// ---- Task 5.5: Viewing Key rotation accounts and helpers ---------------------

/// `rotate_position_key(state: Enc<Shared, PositionSecret>, new_owner:
/// Shared)` arguments: the stored state read from the Position account (old
/// Viewing public key, nonce, five ciphertexts), then the bare `Shared` new
/// owner (the NEW Viewing public key and the unique output-nonce input,
/// `state::nonce`). The circuit seals the same five values to `new_owner`.
fn rotate_position_key_args(
    position: &Position,
    new_viewing_pubkey: [u8; 32],
    output_nonce_input: u128,
) -> ArgBuilder {
    let c = &position.encrypted_state.ciphertexts;
    ArgBuilder::new()
        .x25519_pubkey(position.viewing_pubkey)
        .plaintext_u128(position.encrypted_state.nonce)
        .encrypted_u64(c[0])
        .encrypted_u64(c[1])
        .encrypted_u64(c[2])
        .encrypted_u128(c[3])
        .encrypted_u128(c[4])
        .x25519_pubkey(new_viewing_pubkey)
        .plaintext_u128(output_nonce_input)
}

#[queue_computation_accounts("rotate_position_key", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, sequence: u64)]
pub struct RotatePositionKey<'info> {
    /// The Position owner; signs, pays rent and the computation fee.
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init_if_needed,
        space = 9,
        payer = payer,
        seeds = [&SIGN_PDA_SEED],
        bump,
        address = derive_sign_pda!(),
    )]
    pub sign_pda_account: Box<Account<'info, ArciumSignerAccount>>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_ROTATE_POSITION_KEY))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Box<Account<'info, FeePool>>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Box<Account<'info, ClockAccount>>,
    /// The signer's own Position (the seeds name the payer, so only the owner
    /// can rotate). No Market account: rotation moves no value and is not
    /// pausable.
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), payer.key.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    #[account(
        mut,
        seeds = [OPERATION_COUNTER_SEED, payer.key.as_ref()],
        bump = operation_counter.bump,
    )]
    pub operation_counter: Box<Account<'info, OperationCounter>>,
    #[account(
        init,
        payer = payer,
        space = OperationIntent::ACCOUNT_LEN,
        seeds = [OPERATION_INTENT_SEED, payer.key.as_ref(), &sequence.to_le_bytes()],
        bump,
    )]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[callback_accounts("rotate_position_key")]
#[derive(Accounts)]
pub struct RotatePositionKeyCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_ROTATE_POSITION_KEY))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    /// The attempt's Operation Intent; `classify_callback` verifies it.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

#[init_computation_definition_accounts("rotate_position_key", payer)]
#[derive(Accounts)]
pub struct InitRotatePositionKeyCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}
// ---- Task 5.1: Position initialization accounts ----------------------------

#[queue_computation_accounts("init_position_state", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, sequence: u64)]
pub struct InitializePosition<'info> {
    /// The Position owner; pays rent and the computation fee.
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init_if_needed,
        space = 9,
        payer = payer,
        seeds = [&SIGN_PDA_SEED],
        bump,
        address = derive_sign_pda!(),
    )]
    pub sign_pda_account: Account<'info, ArciumSignerAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_INIT_POSITION_STATE))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Account<'info, FeePool>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Account<'info, ClockAccount>,
    #[account(mut, seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(
        init,
        payer = payer,
        space = Position::ACCOUNT_LEN,
        seeds = [POSITION_ACCOUNT_SEED, market.key().as_ref(), payer.key().as_ref()],
        bump,
    )]
    pub position: Box<Account<'info, Position>>,
    #[account(
        init_if_needed,
        payer = payer,
        space = OperationCounter::ACCOUNT_LEN,
        seeds = [OPERATION_COUNTER_SEED, payer.key().as_ref()],
        bump,
    )]
    pub operation_counter: Box<Account<'info, OperationCounter>>,
    #[account(
        init,
        payer = payer,
        space = OperationIntent::ACCOUNT_LEN,
        seeds = [OPERATION_INTENT_SEED, payer.key().as_ref(), &sequence.to_le_bytes()],
        bump,
    )]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[queue_computation_accounts("init_position_state", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, sequence: u64)]
pub struct RetryPositionInit<'info> {
    /// The Position owner; pays rent and the computation fee.
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init_if_needed,
        space = 9,
        payer = payer,
        seeds = [&SIGN_PDA_SEED],
        bump,
        address = derive_sign_pda!(),
    )]
    pub sign_pda_account: Account<'info, ArciumSignerAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_INIT_POSITION_STATE))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Account<'info, FeePool>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Account<'info, ClockAccount>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, market.key().as_ref(), payer.key().as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    #[account(
        mut,
        seeds = [OPERATION_COUNTER_SEED, payer.key().as_ref()],
        bump = operation_counter.bump,
    )]
    pub operation_counter: Box<Account<'info, OperationCounter>>,
    #[account(
        init,
        payer = payer,
        space = OperationIntent::ACCOUNT_LEN,
        seeds = [OPERATION_INTENT_SEED, payer.key().as_ref(), &sequence.to_le_bytes()],
        bump,
    )]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[callback_accounts("init_position_state")]
#[derive(Accounts)]
pub struct InitPositionStateCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_INIT_POSITION_STATE))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    /// The attempt's Operation Intent; `classify_callback` verifies it.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

#[init_computation_definition_accounts("init_position_state", payer)]
#[derive(Accounts)]
pub struct InitPositionStateCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

// ---- Task 4.1: Pool State initialization accounts --------------------------

#[queue_computation_accounts("init_pool_state", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64)]
pub struct QueuePoolStateInit<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init_if_needed,
        space = 9,
        payer = payer,
        seeds = [&SIGN_PDA_SEED],
        bump,
        address = derive_sign_pda!(),
    )]
    pub sign_pda_account: Account<'info, ArciumSignerAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_INIT_POOL_STATE))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Account<'info, FeePool>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Account<'info, ClockAccount>,
    #[account(
        seeds = [MARKET_SEED],
        bump = market.bump,
        constraint = market.admin == payer.key() @ ErrorCode::NotMarketAdmin,
    )]
    pub market: Box<Account<'info, Market>>,
    #[account(
        seeds = [POOL_STATE_SEED, market.key().as_ref()],
        bump = pool_state.bump,
    )]
    pub pool_state: Box<Account<'info, PoolState>>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[callback_accounts("init_pool_state")]
#[derive(Accounts)]
pub struct InitPoolStateCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_INIT_POOL_STATE))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
    #[account(
        mut,
        seeds = [POOL_STATE_SEED, pool_state.market.as_ref()],
        bump = pool_state.bump,
    )]
    pub pool_state: Box<Account<'info, PoolState>>,
}

#[init_computation_definition_accounts("init_pool_state", payer)]
#[derive(Accounts)]
pub struct InitPoolStateCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

#[queue_computation_accounts("add_together", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64)]
pub struct AddTogether<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init_if_needed,
        space = 9,
        payer = payer,
        seeds = [&SIGN_PDA_SEED],
        bump,
        address = derive_sign_pda!(),
    )]
    pub sign_pda_account: Account<'info, ArciumSignerAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_ADD_TOGETHER))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Account<'info, FeePool>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Account<'info, ClockAccount>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[callback_accounts("add_together")]
#[derive(Accounts)]
pub struct AddTogetherCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_ADD_TOGETHER))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
}

#[init_computation_definition_accounts("add_together", payer)]
#[derive(Accounts)]
pub struct InitAddTogetherCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

// ---- Task 2.2: persistent encrypted Position state accounts ---------------

#[queue_computation_accounts("init_position", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64)]
pub struct InitPosition<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init_if_needed,
        space = 9,
        payer = payer,
        seeds = [&SIGN_PDA_SEED],
        bump,
        address = derive_sign_pda!(),
    )]
    pub sign_pda_account: Account<'info, ArciumSignerAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_INIT_POSITION))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Account<'info, FeePool>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Account<'info, ClockAccount>,
    #[account(
        init,
        payer = payer,
        space = SpikePosition::ACCOUNT_LEN,
        seeds = [POSITION_SEED, payer.key().as_ref()],
        bump,
    )]
    pub position: Box<Account<'info, SpikePosition>>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[callback_accounts("init_position")]
#[derive(Accounts)]
pub struct InitPositionCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_INIT_POSITION))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
    #[account(
        mut,
        seeds = [POSITION_SEED, position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, SpikePosition>>,
}

#[queue_computation_accounts("apply_position_delta", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64)]
pub struct ApplyPositionDelta<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init_if_needed,
        space = 9,
        payer = payer,
        seeds = [&SIGN_PDA_SEED],
        bump,
        address = derive_sign_pda!(),
    )]
    pub sign_pda_account: Account<'info, ArciumSignerAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_APPLY_POSITION_DELTA))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Account<'info, FeePool>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Account<'info, ClockAccount>,
    #[account(
        seeds = [POSITION_SEED, payer.key().as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, SpikePosition>>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[callback_accounts("apply_position_delta")]
#[derive(Accounts)]
pub struct ApplyPositionDeltaCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_APPLY_POSITION_DELTA))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
    #[account(
        mut,
        seeds = [POSITION_SEED, position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, SpikePosition>>,
}

#[init_computation_definition_accounts("init_position", payer)]
#[derive(Accounts)]
pub struct InitPositionCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

#[init_computation_definition_accounts("apply_position_delta", payer)]
#[derive(Accounts)]
pub struct ApplyPositionDeltaCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

// ---- Task 2.3: two-owner output accounts ----------------------------------

#[queue_computation_accounts("liquidation_receipts", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64)]
pub struct LiquidationReceipts<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init_if_needed,
        space = 9,
        payer = payer,
        seeds = [&SIGN_PDA_SEED],
        bump,
        address = derive_sign_pda!(),
    )]
    pub sign_pda_account: Account<'info, ArciumSignerAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_LIQUIDATION_RECEIPTS))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Account<'info, FeePool>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Account<'info, ClockAccount>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[callback_accounts("liquidation_receipts")]
#[derive(Accounts)]
pub struct LiquidationReceiptsCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_LIQUIDATION_RECEIPTS))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
}

#[init_computation_definition_accounts("liquidation_receipts", payer)]
#[derive(Accounts)]
pub struct InitLiquidationReceiptsCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

// ---- Task 2.3: Viewing Key rotation accounts ------------------------------

#[queue_computation_accounts("rotate_viewing_key", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64)]
pub struct RotateViewingKey<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init_if_needed,
        space = 9,
        payer = payer,
        seeds = [&SIGN_PDA_SEED],
        bump,
        address = derive_sign_pda!(),
    )]
    pub sign_pda_account: Account<'info, ArciumSignerAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_ROTATE_VIEWING_KEY))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Account<'info, FeePool>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Account<'info, ClockAccount>,
    // Seeded by the payer: only the Position owner can sign a rotation.
    #[account(
        seeds = [POSITION_SEED, payer.key().as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, SpikePosition>>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[callback_accounts("rotate_viewing_key")]
#[derive(Accounts)]
pub struct RotateViewingKeyCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_ROTATE_VIEWING_KEY))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
    #[account(
        mut,
        seeds = [POSITION_SEED, position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, SpikePosition>>,
}

#[init_computation_definition_accounts("rotate_viewing_key", payer)]
#[derive(Accounts)]
pub struct InitRotateViewingKeyCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

// ---- Task 2.4: OutputTooLarge accounts ------------------------------------

#[queue_computation_accounts("oversized_output", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64)]
pub struct OversizedOutput<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init_if_needed,
        space = 9,
        payer = payer,
        seeds = [&SIGN_PDA_SEED],
        bump,
        address = derive_sign_pda!(),
    )]
    pub sign_pda_account: Account<'info, ArciumSignerAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_OVERSIZED_OUTPUT))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Account<'info, FeePool>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Account<'info, ClockAccount>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[callback_accounts("oversized_output")]
#[derive(Accounts)]
pub struct OversizedOutputCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_OVERSIZED_OUTPUT))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
}

#[init_computation_definition_accounts("oversized_output", payer)]
#[derive(Accounts)]
pub struct InitOversizedOutputCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

// ---- Task 7.2: utilization band accounts and helpers --------------------------

/// `update_utilization_band(pool: Enc<Mxe, PoolSecret>, accounting_index:
/// u128)` arguments: the stored Pool payload (nonce and the four ciphertexts
/// in `PoolSecret` order; `Enc<Mxe>` has no public key), then the public
/// accounting Borrow Index.
fn update_utilization_band_args(pool: &PoolState, accounting_index: u128) -> ArgBuilder {
    let c = &pool.encrypted.ciphertexts;
    ArgBuilder::new()
        .plaintext_u128(pool.encrypted.nonce)
        .encrypted_u128(c[0])
        .encrypted_u128(c[1])
        .encrypted_u128(c[2])
        .encrypted_u128(c[3])
        .plaintext_u128(accounting_index)
}

#[queue_computation_accounts("update_utilization_band", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, sequence: u64)]
pub struct UpdateUtilizationBandQueue<'info> {
    /// Anyone; pays rent and the computation fee.
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init_if_needed,
        space = 9,
        payer = payer,
        seeds = [&SIGN_PDA_SEED],
        bump,
        address = derive_sign_pda!(),
    )]
    pub sign_pda_account: Account<'info, ArciumSignerAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_UPDATE_UTILIZATION_BAND))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Box<Account<'info, FeePool>>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Box<Account<'info, ClockAccount>>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(address = market.asset_state)]
    pub asset_state: Box<Account<'info, AssetState>>,
    /// Read for the encrypted payload and its generation; not Locked.
    #[account(
        address = market.pool_state,
        seeds = [POOL_STATE_SEED, market.key().as_ref()],
        bump = pool_state.bump,
    )]
    pub pool_state: Box<Account<'info, PoolState>>,
    /// Optional (ADR-0004): when present and in band mode, the band is measured
    /// at its borrow index instead of the Asset State's accounting index.
    #[account(seeds = [LENDING_POOL_SEED, market.key().as_ref()], bump = lending_pool.bump)]
    pub lending_pool: Option<Box<Account<'info, LendingPool>>>,
    /// The Pool-owned Intent counter (created on first use).
    #[account(
        init_if_needed,
        payer = payer,
        space = OperationCounter::ACCOUNT_LEN,
        seeds = [OPERATION_COUNTER_SEED, pool_state.key().as_ref()],
        bump,
    )]
    pub operation_counter: Box<Account<'info, OperationCounter>>,
    #[account(
        init,
        payer = payer,
        space = OperationIntent::ACCOUNT_LEN,
        seeds = [OPERATION_INTENT_SEED, pool_state.key().as_ref(), &sequence.to_le_bytes()],
        bump,
    )]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[callback_accounts("update_utilization_band")]
#[derive(Accounts)]
pub struct UpdateUtilizationBandCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_UPDATE_UTILIZATION_BAND))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    /// Written only for an `Applied` decision.
    #[account(mut, address = market.asset_state)]
    pub asset_state: Box<Account<'info, AssetState>>,
    #[account(
        address = market.pool_state,
        seeds = [POOL_STATE_SEED, market.key().as_ref()],
        bump = pool_state.bump,
    )]
    pub pool_state: Box<Account<'info, PoolState>>,
    /// The attempt's Operation Intent; `classify_callback` verifies it.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

#[init_computation_definition_accounts("update_utilization_band", payer)]
#[derive(Accounts)]
pub struct InitUpdateUtilizationBandCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

// ---- Task 7.5: reserve withdrawal accounts and helpers --------------------------

/// `reserve_withdrawal(pool: Enc<Mxe, PoolSecret>, accounted_index: u128,
/// accounting_index: u128, reserve_factor_bps: u128, requested: u64)`
/// arguments: the stored Pool payload (nonce and four ciphertexts), then the
/// public indexes, reserve factor, and requested amount.
fn reserve_withdrawal_args(
    pool: &PoolState,
    accounted_index: u128,
    accounting_index: u128,
    reserve_factor_bps: u16,
    amount: u64,
) -> ArgBuilder {
    let c = &pool.encrypted.ciphertexts;
    ArgBuilder::new()
        .plaintext_u128(pool.encrypted.nonce)
        .encrypted_u128(c[0])
        .encrypted_u128(c[1])
        .encrypted_u128(c[2])
        .encrypted_u128(c[3])
        .plaintext_u128(accounted_index)
        .plaintext_u128(accounting_index)
        .plaintext_u128(reserve_factor_bps as u128)
        .plaintext_u64(amount)
}

/// The `reserve_withdrawal` callback's custom accounts, in struct order:
/// Market, Asset State, Pool, Operation Intent, mint, vault, treasury token
/// account, vault authority, and the SPL Token program.
fn reserve_withdrawal_callback_accounts(
    market: &Market,
    market_key: Pubkey,
    asset_state: Pubkey,
    pool: Pubkey,
    operation_intent: Pubkey,
    destination: Pubkey,
) -> [CallbackAccount; 9] {
    let account = |pubkey, is_writable| CallbackAccount {
        pubkey,
        is_writable,
    };
    [
        account(market_key, false),
        account(asset_state, false),
        account(pool, true),
        account(operation_intent, true),
        account(market.usdc_mint, false),
        account(market.usdc_vault, true),
        account(destination, true),
        account(market.vault_authority, false),
        account(anchor_spl::token::ID, false),
    ]
}

#[queue_computation_accounts("reserve_withdrawal", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, sequence: u64)]
pub struct ReserveWithdrawalQueue<'info> {
    /// The treasury authority; signs, pays rent and the computation fee.
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init_if_needed,
        space = 9,
        payer = payer,
        seeds = [&SIGN_PDA_SEED],
        bump,
        address = derive_sign_pda!(),
    )]
    pub sign_pda_account: Box<Account<'info, ArciumSignerAccount>>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_RESERVE_WITHDRAWAL))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Box<Account<'info, FeePool>>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Box<Account<'info, ClockAccount>>,
    /// Read for the roles and the treasury account; the signer must be the
    /// treasury authority (`state::reserve_withdrawal` checks).
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    /// Written: the accounting Borrow Index accrues at queue time.
    #[account(mut, address = market.asset_state)]
    pub asset_state: Box<Account<'info, AssetState>>,
    /// Locked by this operation.
    #[account(
        mut,
        address = market.pool_state,
        seeds = [POOL_STATE_SEED, market.key().as_ref()],
        bump = pool_state.bump,
    )]
    pub pool_state: Box<Account<'info, PoolState>>,
    /// The Pool-owned Intent counter (created on first use).
    #[account(
        init_if_needed,
        payer = payer,
        space = OperationCounter::ACCOUNT_LEN,
        seeds = [OPERATION_COUNTER_SEED, pool_state.key().as_ref()],
        bump,
    )]
    pub operation_counter: Box<Account<'info, OperationCounter>>,
    #[account(
        init,
        payer = payer,
        space = OperationIntent::ACCOUNT_LEN,
        seeds = [OPERATION_INTENT_SEED, pool_state.key().as_ref(), &sequence.to_le_bytes()],
        bump,
    )]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
    /// The treasury token account; `state::reserve_withdrawal` checks it is
    /// the Market's configured one. It is bound in the Intent.
    pub destination: Box<Account<'info, TokenAccount>>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[callback_accounts("reserve_withdrawal")]
#[derive(Accounts)]
pub struct ReserveWithdrawalCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_RESERVE_WITHDRAWAL))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(address = market.asset_state)]
    pub asset_state: Box<Account<'info, AssetState>>,
    #[account(
        mut,
        address = market.pool_state,
        seeds = [POOL_STATE_SEED, market.key().as_ref()],
        bump = pool_state.bump,
    )]
    pub pool_state: Box<Account<'info, PoolState>>,
    /// The attempt's Operation Intent; `classify_callback` verifies it.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
    /// The USDC mint (bound in the Intent).
    pub mint: Box<Account<'info, Mint>>,
    /// The USDC vault (bound in the Intent).
    #[account(mut)]
    pub vault: Box<Account<'info, TokenAccount>>,
    /// The committed treasury token account (bound in the Intent).
    #[account(mut)]
    pub destination: Box<Account<'info, TokenAccount>>,
    /// Signs the vault transfer.
    #[account(
        seeds = [VAULT_AUTHORITY_SEED, market.key().as_ref()],
        bump = market.vault_authority_bump,
        address = market.vault_authority,
    )]
    pub vault_authority: Box<Account<'info, VaultAuthority>>,
    /// Classic SPL Token only; Token-2022 is rejected.
    pub token_program: Program<'info, Token>,
}

#[init_computation_definition_accounts("reserve_withdrawal", payer)]
#[derive(Accounts)]
pub struct InitReserveWithdrawalCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

// ---- Task 8: supply_flow accounts and helpers ---------------------------------

/// `supply_flow(pool: Enc<Mxe, PoolSecret>, position: Enc<Shared,
/// PositionSecret>, owner: Shared, accounted_index: u128, accounting_index:
/// u128, reserve_factor_bps: u128, is_withdraw: bool, requested: u128,
/// supply_cap: u64, claims_clear: bool)` arguments, in that order.
#[allow(clippy::too_many_arguments)]
fn supply_flow_args(
    pool: &PoolState,
    position: &Position,
    output_nonce_input: u128,
    accounted_index: u128,
    accounting_index: u128,
    reserve_factor_bps: u16,
    is_withdraw: bool,
    requested: u128,
    supply_cap: u64,
    claims_clear: bool,
) -> ArgBuilder {
    let pc = &pool.encrypted.ciphertexts;
    let sc = &position.encrypted_state.ciphertexts;
    ArgBuilder::new()
        .plaintext_u128(pool.encrypted.nonce)
        .encrypted_u128(pc[0])
        .encrypted_u128(pc[1])
        .encrypted_u128(pc[2])
        .encrypted_u128(pc[3])
        .x25519_pubkey(position.viewing_pubkey)
        .plaintext_u128(position.encrypted_state.nonce)
        .encrypted_u64(sc[0])
        .encrypted_u64(sc[1])
        .encrypted_u64(sc[2])
        .encrypted_u128(sc[3])
        .encrypted_u128(sc[4])
        .x25519_pubkey(position.viewing_pubkey)
        .plaintext_u128(output_nonce_input)
        .plaintext_u128(accounted_index)
        .plaintext_u128(accounting_index)
        .plaintext_u128(reserve_factor_bps as u128)
        .plaintext_bool(is_withdraw)
        .plaintext_u128(requested)
        .plaintext_u64(supply_cap)
        .plaintext_bool(claims_clear)
}

/// The `supply_flow` callback's custom accounts, in struct order: Market,
/// Asset State, Pool, Position, and Operation Intent.
fn supply_flow_callback_accounts(
    market: Pubkey,
    asset_state: Pubkey,
    pool: Pubkey,
    position: Pubkey,
    operation_intent: Pubkey,
) -> [CallbackAccount; 5] {
    let account = |pubkey, is_writable| CallbackAccount {
        pubkey,
        is_writable,
    };
    [
        account(market, false),
        account(asset_state, false),
        account(pool, true),
        account(position, true),
        account(operation_intent, true),
    ]
}

#[queue_computation_accounts("supply_flow", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, sequence: u64)]
pub struct SupplyFlowQueue<'info> {
    /// The Position owner; signs, pays rent and the computation fee.
    #[account(mut)]
    pub payer: Signer<'info>,
    /// Not init_if_needed: a Position is Active only after initialize_position
    /// created this program-wide PDA; seeds + bump pin the address. This keeps
    /// try_accounts inside the 4,096-byte SBF stack frame.
    #[account(mut, seeds = [&SIGN_PDA_SEED], bump)]
    pub sign_pda_account: Box<Account<'info, ArciumSignerAccount>>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_SUPPLY_FLOW))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Box<Account<'info, FeePool>>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Box<Account<'info, ClockAccount>>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    /// Written: the accounting Borrow Index accrues at queue time.
    #[account(mut, address = market.asset_state)]
    pub asset_state: Box<Account<'info, AssetState>>,
    /// Locked by this operation.
    #[account(
        mut,
        address = market.pool_state,
        seeds = [POOL_STATE_SEED, market.key().as_ref()],
        bump = pool_state.bump,
    )]
    pub pool_state: Box<Account<'info, PoolState>>,
    /// Locked by this operation.
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, market.key().as_ref(), payer.key.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    #[account(
        mut,
        seeds = [OPERATION_COUNTER_SEED, payer.key.as_ref()],
        bump = operation_counter.bump,
    )]
    pub operation_counter: Box<Account<'info, OperationCounter>>,
    #[account(
        init,
        payer = payer,
        space = OperationIntent::ACCOUNT_LEN,
        seeds = [OPERATION_INTENT_SEED, payer.key.as_ref(), &sequence.to_le_bytes()],
        bump,
    )]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[callback_accounts("supply_flow")]
#[derive(Accounts)]
pub struct SupplyFlowCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_SUPPLY_FLOW))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(address = market.asset_state)]
    pub asset_state: Box<Account<'info, AssetState>>,
    #[account(
        mut,
        address = market.pool_state,
        seeds = [POOL_STATE_SEED, market.key().as_ref()],
        bump = pool_state.bump,
    )]
    pub pool_state: Box<Account<'info, PoolState>>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    /// The attempt's Operation Intent; `classify_callback` verifies it.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

#[init_computation_definition_accounts("supply_flow", payer)]
#[derive(Accounts)]
pub struct InitSupplyFlowCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

// ---- Task 8.1: recover_empty_pool accounts -------------------------------------

#[queue_computation_accounts("recover_empty_pool", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, sequence: u64)]
pub struct RecoverEmptyPoolQueue<'info> {
    /// The Market admin; signs, pays rent and the computation fee.
    #[account(mut)]
    pub payer: Signer<'info>,
    /// Not init_if_needed (see `SupplyFlowQueue`).
    #[account(mut, seeds = [&SIGN_PDA_SEED], bump)]
    pub sign_pda_account: Box<Account<'info, ArciumSignerAccount>>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_RECOVER_EMPTY_POOL))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Box<Account<'info, FeePool>>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Box<Account<'info, ClockAccount>>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    /// Written: the accounting Borrow Index accrues at queue time.
    #[account(mut, address = market.asset_state)]
    pub asset_state: Box<Account<'info, AssetState>>,
    /// Locked by this operation.
    #[account(
        mut,
        address = market.pool_state,
        seeds = [POOL_STATE_SEED, market.key().as_ref()],
        bump = pool_state.bump,
    )]
    pub pool_state: Box<Account<'info, PoolState>>,
    /// The Pool-owned Intent counter (created on first use).
    #[account(
        init_if_needed,
        payer = payer,
        space = OperationCounter::ACCOUNT_LEN,
        seeds = [OPERATION_COUNTER_SEED, pool_state.key().as_ref()],
        bump,
    )]
    pub operation_counter: Box<Account<'info, OperationCounter>>,
    #[account(
        init,
        payer = payer,
        space = OperationIntent::ACCOUNT_LEN,
        seeds = [OPERATION_INTENT_SEED, pool_state.key().as_ref(), &sequence.to_le_bytes()],
        bump,
    )]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[callback_accounts("recover_empty_pool")]
#[derive(Accounts)]
pub struct RecoverEmptyPoolCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_RECOVER_EMPTY_POOL))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(address = market.asset_state)]
    pub asset_state: Box<Account<'info, AssetState>>,
    #[account(
        mut,
        address = market.pool_state,
        seeds = [POOL_STATE_SEED, market.key().as_ref()],
        bump = pool_state.bump,
    )]
    pub pool_state: Box<Account<'info, PoolState>>,
    /// The attempt's Operation Intent; `classify_callback` verifies it.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

#[init_computation_definition_accounts("recover_empty_pool", payer)]
#[derive(Accounts)]
pub struct InitRecoverEmptyPoolCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

// ---- walnut-lend: helpers, accounts ---------------------------------------

/// Reads and validates the Pyth `PriceUpdateV2` account against the pool's
/// oracle configuration. Returns the conservative price (USDC base units per
/// SOL) and its publish time. Errors before anything is locked.
fn read_price(
    pool: &LendingPool,
    asset: &AssetState,
    price_update: &UncheckedAccount,
    now: i64,
) -> Result<(u64, i64)> {
    if price_update.key() != pool.oracle {
        return Err(ErrorCode::OracleAccountMismatch.into());
    }
    let data = price_update.try_borrow_data()?;
    let view = oracle::price_view_from_account(price_update.key(), *price_update.owner, &data)
        .ok_or(ErrorCode::OraclePriceAccountInvalid)?;
    Ok(state::lend_op::pool_price(
        pool,
        asset.risk.oracle_max_confidence_bps,
        &view,
        now,
    )?)
}

/// The public `lend_op` arguments for `op`, from the accrued pool and the Asset
/// State. The position debt cap is clamped to the circuit's product bound.
fn lend_params(
    pool: &LendingPool,
    asset: &AssetState,
    op: u64,
    amount: u64,
    shares: u64,
    price: u64,
) -> Result<LendParams> {
    Ok(LendParams {
        op,
        amount,
        shares,
        borrow_index_1e9: u64::try_from(pool.borrow_index_1e9)
            .map_err(|_| ErrorCode::LendOpInvalid)?,
        price,
        ltv_bps: u64::from(asset.risk.ltv_bps),
        liq_threshold_bps: u64::from(asset.risk.liquidation_threshold_bps),
        position_debt_cap: asset.caps.position_debt_cap_usdc.min(MAX_POSITION_DEBT_CAP),
    })
}

/// `lend_op(state, owner: Shared, op, amount, shares, borrow_index_1e9,
/// price, ltv_bps, liq_threshold_bps, position_debt_cap)` arguments: the stored
/// state, the bare `Shared` output owner (the Position's own Viewing key and
/// the derived nonce input), then the eight public values.
fn lend_op_args(position: &Position, output_nonce_input: u128, p: &LendParams) -> ArgBuilder {
    let c = &position.encrypted_state.ciphertexts;
    ArgBuilder::new()
        .x25519_pubkey(position.viewing_pubkey)
        .plaintext_u128(position.encrypted_state.nonce)
        .encrypted_u64(c[0])
        .encrypted_u64(c[1])
        .encrypted_u64(c[2])
        .encrypted_u128(c[3])
        .encrypted_u128(c[4])
        .x25519_pubkey(position.viewing_pubkey)
        .plaintext_u128(output_nonce_input)
        .plaintext_u64(p.op)
        .plaintext_u64(p.amount)
        .plaintext_u64(p.shares)
        .plaintext_u64(p.borrow_index_1e9)
        .plaintext_u64(p.price)
        .plaintext_u64(p.ltv_bps)
        .plaintext_u64(p.liq_threshold_bps)
        .plaintext_u64(p.position_debt_cap)
}

/// The callback accounts of an op with no token movement.
fn lend_callback_accounts(
    market: Pubkey,
    pool: Pubkey,
    position: Pubkey,
    intent: Pubkey,
) -> [CallbackAccount; 4] {
    [
        CallbackAccount {
            pubkey: market,
            is_writable: false,
        },
        CallbackAccount {
            pubkey: pool,
            is_writable: true,
        },
        CallbackAccount {
            pubkey: position,
            is_writable: true,
        },
        CallbackAccount {
            pubkey: intent,
            is_writable: true,
        },
    ]
}

/// `disclose_position(state, auditor: Shared, mask: u64)` arguments.
fn disclose_position_args(
    position: &Position,
    auditor_pubkey: [u8; 32],
    output_nonce_input: u128,
    mask: u64,
) -> ArgBuilder {
    let c = &position.encrypted_state.ciphertexts;
    ArgBuilder::new()
        .x25519_pubkey(position.viewing_pubkey)
        .plaintext_u128(position.encrypted_state.nonce)
        .encrypted_u64(c[0])
        .encrypted_u64(c[1])
        .encrypted_u64(c[2])
        .encrypted_u128(c[3])
        .encrypted_u128(c[4])
        .x25519_pubkey(auditor_pubkey)
        .plaintext_u128(output_nonce_input)
        .plaintext_u64(mask)
}

#[queue_computation_accounts("lend_op", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, sequence: u64)]
pub struct LendOpQueue<'info> {
    /// The Position owner; signs, pays rent and the computation fee.
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, seeds = [&SIGN_PDA_SEED], bump)]
    pub sign_pda_account: Box<Account<'info, ArciumSignerAccount>>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_LEND_OP))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Box<Account<'info, FeePool>>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Box<Account<'info, ClockAccount>>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    /// Risk parameters, caps and pauses.
    #[account(address = market.asset_state)]
    pub asset_state: Box<Account<'info, AssetState>>,
    #[account(
        mut,
        seeds = [LENDING_POOL_SEED, market.key().as_ref()],
        bump = lending_pool.bump,
    )]
    pub lending_pool: Box<Account<'info, LendingPool>>,
    /// Read for the liquidity check of borrow and withdraw supply.
    #[account(address = market.usdc_vault)]
    pub usdc_vault: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, market.key().as_ref(), payer.key().as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    #[account(
        mut,
        seeds = [OPERATION_COUNTER_SEED, payer.key().as_ref()],
        bump = operation_counter.bump,
    )]
    pub operation_counter: Box<Account<'info, OperationCounter>>,
    #[account(
        init,
        payer = payer,
        space = OperationIntent::ACCOUNT_LEN,
        seeds = [OPERATION_INTENT_SEED, payer.key().as_ref(), &sequence.to_le_bytes()],
        bump,
    )]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
    /// The Pyth `PriceUpdateV2` account (`lending_pool.oracle`); read for ops
    /// 1 and 2, any account otherwise.
    /// CHECK: key, owner and contents are validated against the pool.
    pub price_update: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[queue_computation_accounts("lend_op", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, sequence: u64)]
pub struct CheckHealthQueue<'info> {
    /// Anyone; pays rent and the computation fee.
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, seeds = [&SIGN_PDA_SEED], bump)]
    pub sign_pda_account: Box<Account<'info, ArciumSignerAccount>>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_LEND_OP))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Box<Account<'info, FeePool>>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Box<Account<'info, ClockAccount>>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(address = market.asset_state)]
    pub asset_state: Box<Account<'info, AssetState>>,
    #[account(
        mut,
        seeds = [LENDING_POOL_SEED, market.key().as_ref()],
        bump = lending_pool.bump,
    )]
    pub lending_pool: Box<Account<'info, LendingPool>>,
    /// The Position being checked (not locked).
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    /// The caller's sequence counter, created on first use.
    #[account(
        init_if_needed,
        payer = payer,
        space = OperationCounter::ACCOUNT_LEN,
        seeds = [OPERATION_COUNTER_SEED, payer.key().as_ref()],
        bump,
    )]
    pub operation_counter: Box<Account<'info, OperationCounter>>,
    #[account(
        init,
        payer = payer,
        space = OperationIntent::ACCOUNT_LEN,
        seeds = [OPERATION_INTENT_SEED, payer.key().as_ref(), &sequence.to_le_bytes()],
        bump,
    )]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
    /// The Pyth `PriceUpdateV2` account (`lending_pool.oracle`).
    /// CHECK: key, owner and contents are validated against the pool.
    pub price_update: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[queue_computation_accounts("lend_op", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, sequence: u64)]
pub struct LiquidateQueue<'info> {
    /// The Liquidator; signs, pays the repay amount, rent and the fee.
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, seeds = [&SIGN_PDA_SEED], bump)]
    pub sign_pda_account: Box<Account<'info, ArciumSignerAccount>>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_LEND_OP))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Box<Account<'info, FeePool>>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Box<Account<'info, ClockAccount>>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(address = market.asset_state)]
    pub asset_state: Box<Account<'info, AssetState>>,
    #[account(
        mut,
        seeds = [LENDING_POOL_SEED, market.key().as_ref()],
        bump = lending_pool.bump,
    )]
    pub lending_pool: Box<Account<'info, LendingPool>>,
    /// Receives the repay USDC.
    #[account(mut, address = market.usdc_vault)]
    pub usdc_vault: Box<Account<'info, TokenAccount>>,
    /// The target Position (locked until the callback).
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    /// The Liquidator's sequence counter, created on first use.
    #[account(
        init_if_needed,
        payer = payer,
        space = OperationCounter::ACCOUNT_LEN,
        seeds = [OPERATION_COUNTER_SEED, payer.key().as_ref()],
        bump,
    )]
    pub operation_counter: Box<Account<'info, OperationCounter>>,
    #[account(
        init,
        payer = payer,
        space = OperationIntent::ACCOUNT_LEN,
        seeds = [OPERATION_INTENT_SEED, payer.key().as_ref(), &sequence.to_le_bytes()],
        bump,
    )]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
    /// The Pyth `PriceUpdateV2` account (`lending_pool.oracle`).
    /// CHECK: key, owner and contents are validated against the pool.
    pub price_update: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

/// The shared `lend_op` callback. Ops 0 to 6 pass the four accounts below;
/// a liquidation appends six more as `remaining_accounts` (USDC vault, wSOL
/// vault, vault authority, token program, Liquidator USDC, Liquidator wSOL),
/// whose addresses are bound in the Intent's accounts commitment.
#[callback_accounts("lend_op")]
#[derive(Accounts)]
pub struct LendOpCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_LEND_OP))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(
        mut,
        seeds = [LENDING_POOL_SEED, market.key().as_ref()],
        bump = lending_pool.bump,
    )]
    pub lending_pool: Box<Account<'info, LendingPool>>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    /// The attempt's Operation Intent; `classify_callback` verifies it.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

#[init_computation_definition_accounts("lend_op", payer)]
#[derive(Accounts)]
pub struct InitLendOpCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

#[queue_computation_accounts("disclose_position", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, auditor_pubkey: [u8; 32], mask: u64)]
pub struct DisclosePositionQueue<'info> {
    /// The Position owner; signs, pays rent and the computation fee.
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, seeds = [&SIGN_PDA_SEED], bump)]
    pub sign_pda_account: Box<Account<'info, ArciumSignerAccount>>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_DISCLOSE_POSITION))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Box<Account<'info, FeePool>>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Box<Account<'info, ClockAccount>>,
    /// The owner's own Position (read only; not locked).
    #[account(
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), payer.key().as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    #[account(
        mut,
        seeds = [OPERATION_COUNTER_SEED, payer.key().as_ref()],
        bump = operation_counter.bump,
    )]
    pub operation_counter: Box<Account<'info, OperationCounter>>,
    /// The Intent address uses the counter's next sequence.
    #[account(
        init,
        payer = payer,
        space = OperationIntent::ACCOUNT_LEN,
        seeds = [
            OPERATION_INTENT_SEED,
            payer.key().as_ref(),
            &operation_counter.next_sequence.to_le_bytes(),
        ],
        bump,
    )]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
    /// PDA `["disclosure", position, auditor]`; created on first use and
    /// overwritten by a later Disclosure to the same auditor.
    #[account(
        init_if_needed,
        payer = payer,
        space = Disclosure::ACCOUNT_LEN,
        seeds = [DISCLOSURE_SEED, position.key().as_ref(), auditor_pubkey.as_ref()],
        bump,
    )]
    pub disclosure: Box<Account<'info, Disclosure>>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[callback_accounts("disclose_position")]
#[derive(Accounts)]
pub struct DisclosePositionCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_DISCLOSE_POSITION))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
    #[account(
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    #[account(
        mut,
        seeds = [DISCLOSURE_SEED, disclosure.position.as_ref(), disclosure.auditor.as_ref()],
        bump = disclosure.bump,
    )]
    pub disclosure: Box<Account<'info, Disclosure>>,
    /// The attempt's Operation Intent; `classify_callback` verifies it.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

#[init_computation_definition_accounts("disclose_position", payer)]
#[derive(Accounts)]
pub struct InitDisclosePositionCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

// ---- walnut-private (ADR-0004): helpers and account structs --------------------

/// The computation offset a callback belongs to (the Intent's offset, or the
/// retry offset + 1), as in `supply_flow_callback`.
fn callback_offset(
    intent_offset: u64,
    computation_key: Pubkey,
    mxe_account: &Account<MXEAccount>,
) -> u64 {
    if computation_key == derive_comp_pda!(intent_offset, mxe_account) {
        intent_offset
    } else {
        intent_offset.wrapping_add(1)
    }
}

/// The Pyth low-bound price when the op reads one, else `(0, None)`.
fn private_price(
    kind: operation::OperationKind,
    op: u64,
    pool: &LendingPool,
    asset: &AssetState,
    price_update: &UncheckedAccount,
    now: i64,
) -> Result<(u64, Option<i64>)> {
    if state::private_op::needs_price(kind, op) {
        let (price, time) = read_price(pool, asset, price_update, now)?;
        Ok((price, Some(time)))
    } else {
        Ok((0, None))
    }
}

/// The public circuit parameters from the accrued pool and the Asset State.
fn private_params(
    pool: &LendingPool,
    asset: &AssetState,
    op: u64,
    price: u64,
) -> Result<PrivateParams> {
    let index = |v: u128| u64::try_from(v).map_err(|_| ErrorCode::LendOpInvalid);
    Ok(PrivateParams {
        op,
        supply_index_1e9: index(pool.supply_index_1e9)?,
        borrow_index_1e9: index(pool.borrow_index_1e9)?,
        price,
        ltv_bps: u64::from(asset.risk.ltv_bps),
        liq_threshold_bps: u64::from(asset.risk.liquidation_threshold_bps),
        bonus_bps: u64::from(asset.risk.liquidation_bonus_bps),
        position_debt_cap: asset.caps.position_debt_cap_usdc.min(MAX_POSITION_DEBT_CAP),
        supply_cap: asset.caps.supply_cap_usdc,
    })
}

/// Appends a stored Position payload (`Enc<Shared, PositionSecret>`).
fn with_position(builder: ArgBuilder, position: &Position) -> ArgBuilder {
    let c = &position.encrypted_state.ciphertexts;
    builder
        .x25519_pubkey(position.viewing_pubkey)
        .plaintext_u128(position.encrypted_state.nonce)
        .encrypted_u64(c[0])
        .encrypted_u64(c[1])
        .encrypted_u64(c[2])
        .encrypted_u128(c[3])
        .encrypted_u128(c[4])
}

/// Appends the stored Pool payload (`Enc<Mxe, PoolSecret>`).
fn with_pool(builder: ArgBuilder, pool: &PoolState) -> ArgBuilder {
    let c = &pool.encrypted.ciphertexts;
    builder
        .plaintext_u128(pool.encrypted.nonce)
        .encrypted_u128(c[0])
        .encrypted_u128(c[1])
        .encrypted_u128(c[2])
        .encrypted_u128(c[3])
}

/// `pool_op(pool, state, input: Enc<Shared, OpInput>, owner: Shared, op,
/// supply_index_1e9, borrow_index_1e9, price, ltv_bps, position_debt_cap,
/// supply_cap)` arguments, in that order.
fn pool_op_args(
    pool: &PoolState,
    position: &Position,
    input_pubkey: [u8; 32],
    input_nonce: u128,
    input: &[[u8; 32]; 2],
    output_nonce_input: u128,
    p: &PrivateParams,
) -> ArgBuilder {
    with_position(with_pool(ArgBuilder::new(), pool), position)
        .x25519_pubkey(input_pubkey)
        .plaintext_u128(input_nonce)
        .encrypted_u64(input[0])
        .encrypted_u64(input[1])
        .x25519_pubkey(position.viewing_pubkey)
        .plaintext_u128(output_nonce_input)
        .plaintext_u64(p.op)
        .plaintext_u64(p.supply_index_1e9)
        .plaintext_u64(p.borrow_index_1e9)
        .plaintext_u64(p.price)
        .plaintext_u64(p.ltv_bps)
        .plaintext_u64(p.position_debt_cap)
        .plaintext_u64(p.supply_cap)
}

/// `position_private(state, input: Enc<Shared, OpInput>, owner: Shared, op,
/// borrow_index_1e9, price, ltv_bps)` arguments, in that order.
fn position_private_args(
    position: &Position,
    input_pubkey: [u8; 32],
    input_nonce: u128,
    input: &[[u8; 32]; 2],
    output_nonce_input: u128,
    p: &PrivateParams,
) -> ArgBuilder {
    with_position(ArgBuilder::new(), position)
        .x25519_pubkey(input_pubkey)
        .plaintext_u128(input_nonce)
        .encrypted_u64(input[0])
        .encrypted_u64(input[1])
        .x25519_pubkey(position.viewing_pubkey)
        .plaintext_u128(output_nonce_input)
        .plaintext_u64(p.op)
        .plaintext_u64(p.borrow_index_1e9)
        .plaintext_u64(p.price)
        .plaintext_u64(p.ltv_bps)
}

/// `liquidate_pool(pool, target, liquidator, input: Enc<Shared, LiqInput>,
/// target_owner: Shared, liquidator_owner: Shared, borrow_index_1e9, price,
/// liq_threshold_bps, bonus_bps)` arguments, in that order.
#[allow(clippy::too_many_arguments)]
fn liquidate_pool_args(
    pool: &PoolState,
    target: &Position,
    liquidator: &Position,
    input_pubkey: [u8; 32],
    input_nonce: u128,
    input: &[[u8; 32]; 3],
    target_nonce_input: u128,
    liquidator_nonce_input: u128,
    p: &PrivateParams,
) -> ArgBuilder {
    with_position(
        with_position(with_pool(ArgBuilder::new(), pool), target),
        liquidator,
    )
    .x25519_pubkey(input_pubkey)
    .plaintext_u128(input_nonce)
    .encrypted_u64(input[0])
    .encrypted_u64(input[1])
    .encrypted_u64(input[2])
    .x25519_pubkey(target.viewing_pubkey)
    .plaintext_u128(target_nonce_input)
    .x25519_pubkey(liquidator.viewing_pubkey)
    .plaintext_u128(liquidator_nonce_input)
    .plaintext_u64(p.borrow_index_1e9)
    .plaintext_u64(p.price)
    .plaintext_u64(p.liq_threshold_bps)
    .plaintext_u64(p.bonus_bps)
}

/// The private callbacks' custom accounts, in struct order: Market,
/// LendingPool, Pool State (if any), Position, liquidator Position (if any),
/// Operation Intent.
fn private_callback_accounts(
    market: Pubkey,
    lending_pool: Pubkey,
    pool: Option<Pubkey>,
    position: Pubkey,
    second: Option<Pubkey>,
    operation_intent: Pubkey,
) -> Vec<CallbackAccount> {
    let account = |pubkey, is_writable| CallbackAccount {
        pubkey,
        is_writable,
    };
    let mut accounts = vec![account(market, false), account(lending_pool, false)];
    if let Some(pool) = pool {
        accounts.push(account(pool, true));
    }
    accounts.push(account(position, true));
    if let Some(second) = second {
        accounts.push(account(second, true));
    }
    accounts.push(account(operation_intent, true));
    accounts
}

#[queue_computation_accounts("pool_op", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, sequence: u64)]
pub struct PoolOpQueue<'info> {
    /// The Position owner; signs, pays rent and the computation fee.
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, seeds = [&SIGN_PDA_SEED], bump)]
    pub sign_pda_account: Box<Account<'info, ArciumSignerAccount>>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_POOL_OP))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Box<Account<'info, FeePool>>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Box<Account<'info, ClockAccount>>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    /// Risk parameters, caps and pauses.
    #[account(address = market.asset_state)]
    pub asset_state: Box<Account<'info, AssetState>>,
    /// The public index authority; accrued at queue time.
    #[account(
        mut,
        seeds = [LENDING_POOL_SEED, market.key().as_ref()],
        bump = lending_pool.bump,
    )]
    pub lending_pool: Box<Account<'info, LendingPool>>,
    /// The encrypted Pool; Locked by this operation.
    #[account(mut, address = market.pool_state)]
    pub pool_state: Box<Account<'info, PoolState>>,
    /// Locked by this operation.
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, market.key().as_ref(), payer.key().as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    #[account(
        mut,
        seeds = [OPERATION_COUNTER_SEED, payer.key().as_ref()],
        bump = operation_counter.bump,
    )]
    pub operation_counter: Box<Account<'info, OperationCounter>>,
    #[account(
        init,
        payer = payer,
        space = OperationIntent::ACCOUNT_LEN,
        seeds = [OPERATION_INTENT_SEED, payer.key().as_ref(), &sequence.to_le_bytes()],
        bump,
    )]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
    /// The Pyth `PriceUpdateV2` account (`lending_pool.oracle`); read for a
    /// borrow, any account otherwise.
    /// CHECK: key, owner and contents are validated against the pool.
    pub price_update: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[callback_accounts("pool_op")]
#[derive(Accounts)]
pub struct PoolOpCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_POOL_OP))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(
        seeds = [LENDING_POOL_SEED, market.key().as_ref()],
        bump = lending_pool.bump,
    )]
    pub lending_pool: Box<Account<'info, LendingPool>>,
    #[account(
        mut,
        address = market.pool_state,
        seeds = [POOL_STATE_SEED, market.key().as_ref()],
        bump = pool_state.bump,
    )]
    pub pool_state: Box<Account<'info, PoolState>>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    /// The attempt's Operation Intent; `classify_callback` verifies it.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

#[init_computation_definition_accounts("pool_op", payer)]
#[derive(Accounts)]
pub struct InitPoolOpCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

#[queue_computation_accounts("position_private", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, sequence: u64)]
pub struct PositionPrivateQueue<'info> {
    /// The Position owner; signs, pays rent and the computation fee.
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, seeds = [&SIGN_PDA_SEED], bump)]
    pub sign_pda_account: Box<Account<'info, ArciumSignerAccount>>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_POSITION_PRIVATE))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Box<Account<'info, FeePool>>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Box<Account<'info, ClockAccount>>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(address = market.asset_state)]
    pub asset_state: Box<Account<'info, AssetState>>,
    #[account(
        mut,
        seeds = [LENDING_POOL_SEED, market.key().as_ref()],
        bump = lending_pool.bump,
    )]
    pub lending_pool: Box<Account<'info, LendingPool>>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, market.key().as_ref(), payer.key().as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    #[account(
        mut,
        seeds = [OPERATION_COUNTER_SEED, payer.key().as_ref()],
        bump = operation_counter.bump,
    )]
    pub operation_counter: Box<Account<'info, OperationCounter>>,
    #[account(
        init,
        payer = payer,
        space = OperationIntent::ACCOUNT_LEN,
        seeds = [OPERATION_INTENT_SEED, payer.key().as_ref(), &sequence.to_le_bytes()],
        bump,
    )]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
    /// The Pyth `PriceUpdateV2` account; read for a withdrawal.
    /// CHECK: key, owner and contents are validated against the pool.
    pub price_update: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[callback_accounts("position_private")]
#[derive(Accounts)]
pub struct PositionPrivateCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_POSITION_PRIVATE))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(
        seeds = [LENDING_POOL_SEED, market.key().as_ref()],
        bump = lending_pool.bump,
    )]
    pub lending_pool: Box<Account<'info, LendingPool>>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    /// The attempt's Operation Intent; `classify_callback` verifies it.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

#[init_computation_definition_accounts("position_private", payer)]
#[derive(Accounts)]
pub struct InitPositionPrivateCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

#[queue_computation_accounts("liquidate_pool", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, sequence: u64)]
pub struct LiquidatePoolQueue<'info> {
    /// The liquidator; signs, pays rent and the computation fee, and owns
    /// `liquidator_position`.
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, seeds = [&SIGN_PDA_SEED], bump)]
    pub sign_pda_account: Box<Account<'info, ArciumSignerAccount>>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_LIQUIDATE_POOL))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Box<Account<'info, FeePool>>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Box<Account<'info, ClockAccount>>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(address = market.asset_state)]
    pub asset_state: Box<Account<'info, AssetState>>,
    #[account(
        mut,
        constraint = lending_pool.market == market.key() @ ErrorCode::PrivateOpIntentMismatch,
    )]
    pub lending_pool: Box<Account<'info, LendingPool>>,
    #[account(mut, address = market.pool_state)]
    pub pool_state: Box<Account<'info, PoolState>>,
    /// The target (borrower) Position of this Market.
    #[account(
        mut,
        constraint = position.market == market.key() @ ErrorCode::PrivateOpIntentMismatch,
    )]
    pub position: Box<Account<'info, Position>>,
    /// The liquidator's own Position: pays the repay amount, receives the
    /// seized collateral.
    #[account(
        mut,
        constraint = liquidator_position.market == market.key() @ ErrorCode::PrivateOpIntentMismatch,
        constraint = liquidator_position.owner == payer.key() @ ErrorCode::PositionOpNotOwner,
    )]
    pub liquidator_position: Box<Account<'info, Position>>,
    /// `reserve_intent` also requires `operation_counter.owner == payer`.
    #[account(
        mut,
        constraint = operation_counter.owner == payer.key() @ ErrorCode::PositionOpNotOwner,
    )]
    pub operation_counter: Box<Account<'info, OperationCounter>>,
    #[account(
        init,
        payer = payer,
        space = OperationIntent::ACCOUNT_LEN,
        seeds = [OPERATION_INTENT_SEED, payer.key().as_ref(), &sequence.to_le_bytes()],
        bump,
    )]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
    /// The Pyth `PriceUpdateV2` account (`lending_pool.oracle`).
    /// CHECK: key, owner and contents are validated against the pool.
    pub price_update: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[callback_accounts("liquidate_pool")]
#[derive(Accounts)]
pub struct LiquidatePoolCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_LIQUIDATE_POOL))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(
        seeds = [LENDING_POOL_SEED, market.key().as_ref()],
        bump = lending_pool.bump,
    )]
    pub lending_pool: Box<Account<'info, LendingPool>>,
    #[account(
        mut,
        address = market.pool_state,
        seeds = [POOL_STATE_SEED, market.key().as_ref()],
        bump = pool_state.bump,
    )]
    pub pool_state: Box<Account<'info, PoolState>>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, position.market.as_ref(), position.owner.as_ref()],
        bump = position.bump,
    )]
    pub position: Box<Account<'info, Position>>,
    #[account(
        mut,
        seeds = [POSITION_ACCOUNT_SEED, liquidator_position.market.as_ref(), liquidator_position.owner.as_ref()],
        bump = liquidator_position.bump,
    )]
    pub liquidator_position: Box<Account<'info, Position>>,
    /// The attempt's Operation Intent; `classify_callback` verifies it.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

#[init_computation_definition_accounts("liquidate_pool", payer)]
#[derive(Accounts)]
pub struct InitLiquidatePoolCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}

/// The WAD index the band circuit measures the Pool at: the LendingPool
/// borrow index (1e9 lifted to WAD) when a LendingPool in band mode is passed,
/// else the Asset State's accounting index (Task 7 behavior).
fn band_circuit_index(asset: &AssetState, lending_pool: Option<&LendingPool>) -> Result<u128> {
    match lending_pool {
        Some(pool) if pool.rate_source == state::lending_pool::RATE_SOURCE_BAND => Ok(pool
            .borrow_index_1e9
            .checked_mul(math::WAD / math::lend::INDEX_SCALE)
            .ok_or(ErrorCode::LendOpInvalid)?),
        _ => Ok(asset.accounting_borrow_index),
    }
}

#[queue_computation_accounts("pool_band", payer)]
#[derive(Accounts)]
#[instruction(computation_offset: u64, sequence: u64)]
pub struct UpdatePoolBandQueue<'info> {
    /// Anyone; pays rent and the computation fee.
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, seeds = [&SIGN_PDA_SEED], bump)]
    pub sign_pda_account: Box<Account<'info, ArciumSignerAccount>>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut, address = derive_mempool_pda!(mxe_account))]
    /// CHECK: mempool_account, checked by the arcium program
    pub mempool_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_execpool_pda!(mxe_account))]
    /// CHECK: executing_pool, checked by the arcium program
    pub executing_pool: UncheckedAccount<'info>,
    #[account(mut, address = derive_comp_pda!(computation_offset, mxe_account))]
    /// CHECK: computation_account, checked by the arcium program
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_POOL_BAND))]
    pub comp_def_account: Box<Account<'info, ComputationDefinitionAccount>>,
    #[account(mut, address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(mut, address = ARCIUM_FEE_POOL_ACCOUNT_ADDRESS)]
    pub pool_account: Box<Account<'info, FeePool>>,
    #[account(mut, address = ARCIUM_CLOCK_ACCOUNT_ADDRESS)]
    pub clock_account: Box<Account<'info, ClockAccount>>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    #[account(address = market.asset_state)]
    pub asset_state: Box<Account<'info, AssetState>>,
    /// Read for the encrypted payload and its generation; not Locked.
    #[account(address = market.pool_state)]
    pub pool_state: Box<Account<'info, PoolState>>,
    /// The public borrow index the band is measured at.
    #[account(seeds = [LENDING_POOL_SEED, market.key().as_ref()], bump = lending_pool.bump)]
    pub lending_pool: Box<Account<'info, LendingPool>>,
    /// The Pool-owned Intent counter (created on first use).
    #[account(
        init_if_needed,
        payer = payer,
        space = OperationCounter::ACCOUNT_LEN,
        seeds = [OPERATION_COUNTER_SEED, pool_state.key().as_ref()],
        bump,
    )]
    pub operation_counter: Box<Account<'info, OperationCounter>>,
    #[account(
        init,
        payer = payer,
        space = OperationIntent::ACCOUNT_LEN,
        seeds = [OPERATION_INTENT_SEED, pool_state.key().as_ref(), &sequence.to_le_bytes()],
        bump,
    )]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
    pub system_program: Program<'info, System>,
    pub arcium_program: Program<'info, Arcium>,
}

#[callback_accounts("pool_band")]
#[derive(Accounts)]
pub struct PoolBandCallback<'info> {
    pub arcium_program: Program<'info, Arcium>,
    #[account(address = derive_comp_def_pda!(COMP_DEF_OFFSET_POOL_BAND))]
    pub comp_def_account: Account<'info, ComputationDefinitionAccount>,
    #[account(address = derive_mxe_pda!())]
    pub mxe_account: Account<'info, MXEAccount>,
    /// CHECK: computation_account, checked by the arcium program via callback context constraints
    pub computation_account: UncheckedAccount<'info>,
    #[account(address = derive_cluster_pda!(mxe_account))]
    pub cluster_account: Box<Account<'info, Cluster>>,
    #[account(address = ::arcium_anchor::solana_instructions_sysvar::ID)]
    /// CHECK: instructions_sysvar, checked by the account constraint
    pub instructions_sysvar: UncheckedAccount<'info>,
    #[account(seeds = [MARKET_SEED], bump = market.bump)]
    pub market: Box<Account<'info, Market>>,
    /// Written only for an `Applied` decision.
    #[account(mut, address = market.asset_state)]
    pub asset_state: Box<Account<'info, AssetState>>,
    #[account(address = market.pool_state)]
    pub pool_state: Box<Account<'info, PoolState>>,
    /// The attempt's Operation Intent; `classify_callback` verifies it.
    #[account(mut)]
    pub operation_intent: Box<Account<'info, OperationIntent>>,
}

#[init_computation_definition_accounts("pool_band", payer)]
#[derive(Accounts)]
pub struct InitPoolBandCompDef<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, address = derive_mxe_pda!())]
    pub mxe_account: Box<Account<'info, MXEAccount>>,
    #[account(mut)]
    /// CHECK: comp_def_account, checked by the arcium program; not initialized yet
    pub comp_def_account: UncheckedAccount<'info>,
    #[account(mut, address = derive_mxe_lut_pda!(mxe_account.lut_offset_slot))]
    /// CHECK: address_lookup_table, checked by the arcium program
    pub address_lookup_table: UncheckedAccount<'info>,
    #[account(address = LUT_PROGRAM_ID)]
    /// CHECK: lut_program is the Address Lookup Table program
    pub lut_program: UncheckedAccount<'info>,
    pub arcium_program: Program<'info, Arcium>,
    pub system_program: Program<'info, System>,
}
