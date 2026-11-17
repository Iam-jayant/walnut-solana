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
pub use instructions::initialize_market::*;
pub use instructions::pause::*;
pub use instructions::roles::*;

// Task 2.2 Arcium feasibility gate: persistent encrypted Position state. This
// account-backed module proves the complete encrypted Position payload can be
// stored, reread by a later computation, updated, and locally decrypted. It is
// feasibility scaffolding, not the production Position (Task 5).
pub mod position;

use errors::ErrorCode;
use events::{
    LiquidationReceiptsEmitted, OversizedOutputEmitted, PoolStateActivated, PositionStateStored,
    SumEvent, ViewingKeyRotated,
};
use market::{check_pool_initializing, Market, MARKET_SEED, POOL_STATE_SEED};
use position::{EncryptedPositionState, SpikePosition, POSITION_SECRET_FIELDS};
use state::{EncryptedPoolState, PoolState};

const COMP_DEF_OFFSET_ADD_TOGETHER: u32 = comp_def_offset("add_together");
const COMP_DEF_OFFSET_INIT_POSITION: u32 = comp_def_offset("init_position");
const COMP_DEF_OFFSET_APPLY_POSITION_DELTA: u32 = comp_def_offset("apply_position_delta");
const COMP_DEF_OFFSET_LIQUIDATION_RECEIPTS: u32 = comp_def_offset("liquidation_receipts");
const COMP_DEF_OFFSET_ROTATE_VIEWING_KEY: u32 = comp_def_offset("rotate_viewing_key");
const COMP_DEF_OFFSET_OVERSIZED_OUTPUT: u32 = comp_def_offset("oversized_output");
const COMP_DEF_OFFSET_INIT_POOL_STATE: u32 = comp_def_offset("init_pool_state");

/// PDA seed for the Task 2.2 `SpikePosition` account: one per owner.
const POSITION_SEED: &[u8] = b"spike_position";

declare_id!("5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea");

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
