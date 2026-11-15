//! Walnut-Solana program.
//!
//! This is the Arcium/Anchor workspace scaffold plus the Task 2 feasibility
//! gate. It contains the standard `add_together` hello-world computation used
//! to verify the toolchain (Task 1.5) and the Task 2.2 persistent encrypted
//! Position state spike (`init_position` / `apply_position_delta`). No Walnut
//! financial logic is added until the Arcium feasibility gate (Task 2) passes.
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

// Task 2.2 Arcium feasibility gate: persistent encrypted Position state. This
// account-backed module proves the complete encrypted Position payload can be
// stored, reread by a later computation, updated, and locally decrypted. It is
// feasibility scaffolding, not the production Position (Task 5).
pub mod position;

use errors::ErrorCode;
use events::{PositionStateStored, SumEvent};
use position::{EncryptedPositionState, SpikePosition, POSITION_SECRET_FIELDS};

const COMP_DEF_OFFSET_ADD_TOGETHER: u32 = comp_def_offset("add_together");
const COMP_DEF_OFFSET_INIT_POSITION: u32 = comp_def_offset("init_position");
const COMP_DEF_OFFSET_APPLY_POSITION_DELTA: u32 = comp_def_offset("apply_position_delta");

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
