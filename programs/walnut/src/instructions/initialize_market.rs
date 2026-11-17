//! `initialize_market` (Task 4.1, Requirements 4.1 and 4.2).
//!
//! One instruction creates the Market, Asset State, Pool State (in
//! `Initializing`), vault authority, wSOL and USDC vault token accounts, and
//! the Recovery Position. The Pool's `Enc<Mxe>` zero payload is produced
//! afterwards by the `init_pool_state` computation, because only the MXE can
//! encrypt to itself.
//!
//! Order of checks in the handler, all before any write:
//!
//! 1. The signer is the program's upgrade authority
//!    ([`check_upgrade_authority`]).
//! 2. Risk, treasury, and guardian authorities are non-zero.
//! 3. Mints: classic SPL Token owner, wSOL is the native mint, USDC is a
//!    different 6-decimal mint ([`check_market_mints`]). `Program<Token>`
//!    already rejects any token program other than classic SPL Token, and
//!    `Account<Mint>` rejects a mint owned by another program (Token-2022)
//!    with Anchor's owner error.
//! 4. Every created account is still zeroed ([`ensure_fresh`]). The accounts
//!    use `init_if_needed` so a repeat call reaches this check and fails with
//!    a Walnut error code instead of the System Program's "already in use".
//!    Every address is a PDA of this program, so nobody else can pre-create or
//!    pre-fill one of them.
//! 5. The upgrade, risk, treasury, and guardian authorities are four
//!    different keys, and none is one of the Market's own accounts
//!    (`market::check_market_roles`, Task 4.2). Nothing has been written yet.
//!
//! Between steps 3 and 4 the initial USDC and count caps are checked against
//! the Task 4.5 bounds (`market::check_caps_bounds`,
//! `market::check_count_caps_bounds`).

use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::events::MarketInitialized;
use crate::market::{
    check_caps_bounds, check_count_caps_bounds, check_initialize_authorities, check_market_mints,
    check_market_roles, check_upgrade_authority, ensure_fresh, AssetState, Caps, CountCaps,
    InitMarkers, Market, MarketStatus, MintView, PauseState, ProgramAccountView, RateParams,
    RiskParams, TimelockState, VaultAuthority, ASSET_STATE_SEED, LAYOUT_VERSION, MARKET_SEED,
    POOL_STATE_SEED, RECOVERY_POSITION_SEED, VAULT_AUTHORITY_SEED, VAULT_SEED, WAD,
};
use crate::state::{Coordination, EncryptedPoolState, PoolState, PoolStatus, Position};

/// Configuration supplied at initialization. Risk and rate parameters start at
/// the documented launch defaults and change only through Task 4.4 governance.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitializeMarketArgs {
    /// Risk-parameter authority (a multisig on mainnet).
    pub risk_authority: Pubkey,
    /// Treasury authority.
    pub treasury_authority: Pubkey,
    /// Guardian authority for expiring emergency pauses.
    pub guardian_authority: Pubkey,
    /// Initial USDC caps. Zero blocks the matching new risk.
    pub caps: Caps,
    /// Initial cap on the public Position count.
    pub position_count_cap: u64,
    /// Initial cap on the public debt-Position count.
    pub debt_position_count_cap: u64,
}

/// Accounts of `initialize_market`.
#[derive(Accounts)]
pub struct InitializeMarket<'info> {
    /// The program upgrade authority; pays for every account and becomes the
    /// initial admin.
    #[account(mut)]
    pub authority: Signer<'info>,
    /// CHECK: the Walnut program account. Its address is fixed here and its
    /// loader state is parsed by `check_upgrade_authority`.
    #[account(address = crate::ID)]
    pub walnut_program: UncheckedAccount<'info>,
    /// CHECK: the program-data account. `check_upgrade_authority` requires it
    /// to be the one the program account points at, owned by the loader.
    pub program_data: UncheckedAccount<'info>,
    /// The wSOL (native) mint.
    pub wsol_mint: Box<Account<'info, Mint>>,
    /// The configured USDC mint.
    pub usdc_mint: Box<Account<'info, Mint>>,
    /// Classic SPL Token only.
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    #[account(
        init_if_needed,
        payer = authority,
        space = Market::ACCOUNT_LEN,
        seeds = [MARKET_SEED],
        bump,
    )]
    pub market: Box<Account<'info, Market>>,
    #[account(
        init_if_needed,
        payer = authority,
        space = AssetState::ACCOUNT_LEN,
        seeds = [ASSET_STATE_SEED, market.key().as_ref()],
        bump,
    )]
    pub asset_state: Box<Account<'info, AssetState>>,
    #[account(
        init_if_needed,
        payer = authority,
        space = PoolState::ACCOUNT_LEN,
        seeds = [POOL_STATE_SEED, market.key().as_ref()],
        bump,
    )]
    pub pool_state: Box<Account<'info, PoolState>>,
    #[account(
        init_if_needed,
        payer = authority,
        space = VaultAuthority::ACCOUNT_LEN,
        seeds = [VAULT_AUTHORITY_SEED, market.key().as_ref()],
        bump,
    )]
    pub vault_authority: Box<Account<'info, VaultAuthority>>,
    #[account(
        init_if_needed,
        payer = authority,
        seeds = [VAULT_SEED, market.key().as_ref(), wsol_mint.key().as_ref()],
        bump,
        token::mint = wsol_mint,
        token::authority = vault_authority,
    )]
    pub wsol_vault: Box<Account<'info, TokenAccount>>,
    #[account(
        init_if_needed,
        payer = authority,
        seeds = [VAULT_SEED, market.key().as_ref(), usdc_mint.key().as_ref()],
        bump,
        token::mint = usdc_mint,
        token::authority = vault_authority,
    )]
    pub usdc_vault: Box<Account<'info, TokenAccount>>,
    #[account(
        init_if_needed,
        payer = authority,
        space = Position::ACCOUNT_LEN,
        seeds = [RECOVERY_POSITION_SEED, market.key().as_ref()],
        bump,
    )]
    pub recovery_position: Box<Account<'info, Position>>,
}

fn mint_view(mint: &Account<'_, Mint>) -> MintView {
    MintView {
        key: mint.key(),
        owner: *mint.to_account_info().owner,
        decimals: mint.decimals,
        is_initialized: mint.is_initialized,
    }
}

/// Validates and writes every market account. See the module docs.
pub fn initialize_market_handler(
    mut ctx: Context<InitializeMarket>,
    args: InitializeMarketArgs,
) -> Result<()> {
    let accounts = &mut ctx.accounts;
    let authority = accounts.authority.key();

    {
        let program_info = accounts.walnut_program.to_account_info();
        let program_data_info = accounts.program_data.to_account_info();
        let program_bytes = program_info.try_borrow_data()?;
        let program_data_bytes = program_data_info.try_borrow_data()?;
        check_upgrade_authority(
            &ProgramAccountView {
                key: program_info.key(),
                owner: *program_info.owner,
                data: &program_bytes,
            },
            &ProgramAccountView {
                key: program_data_info.key(),
                owner: *program_data_info.owner,
                data: &program_data_bytes,
            },
            &authority,
        )?;
    }
    check_initialize_authorities(&[
        args.risk_authority,
        args.treasury_authority,
        args.guardian_authority,
    ])?;
    check_market_mints(
        &mint_view(&accounts.wsol_mint),
        &mint_view(&accounts.usdc_mint),
    )?;
    // Task 4.5: initial caps obey the same bounds as governed changes.
    check_caps_bounds(&args.caps)?;
    check_count_caps_bounds(&CountCaps {
        position_count_cap: args.position_count_cap,
        debt_position_count_cap: args.debt_position_count_cap,
    })?;
    ensure_fresh(&InitMarkers {
        market_status: accounts.market.status,
        asset_layout_version: accounts.asset_state.layout_version,
        pool_status: accounts.pool_state.status,
        recovery_position_owner: accounts.recovery_position.owner,
        vault_authority_market: accounts.vault_authority.market,
    })?;

    let clock = Clock::get()?;
    let bumps = &ctx.bumps;
    let market_key = accounts.market.key();

    let market = Market {
        layout_version: LAYOUT_VERSION,
        status: MarketStatus::Active,
        bump: bumps.market,
        vault_authority_bump: bumps.vault_authority,
        admin: authority,
        pending_admin: Pubkey::default(),
        upgrade_authority: authority,
        risk_authority: args.risk_authority,
        treasury_authority: args.treasury_authority,
        guardian_authority: args.guardian_authority,
        asset_state: accounts.asset_state.key(),
        pool_state: accounts.pool_state.key(),
        recovery_position: accounts.recovery_position.key(),
        vault_authority: accounts.vault_authority.key(),
        wsol_mint: accounts.wsol_mint.key(),
        usdc_mint: accounts.usdc_mint.key(),
        wsol_vault: accounts.wsol_vault.key(),
        usdc_vault: accounts.usdc_vault.key(),
        treasury_usdc_account: Pubkey::default(),
        position_count: 0,
        debt_position_count: 0,
        unsettled_recovery_claim_count: 0,
        position_count_cap: args.position_count_cap,
        debt_position_count_cap: args.debt_position_count_cap,
        initialized_slot: clock.slot,
        debt_position_reservations: 0,
        reserved: [0; 120],
    };
    // Task 4.2: the four separated roles must be distinct, valid keys.
    check_market_roles(&market, &market_key)?;
    accounts.market.set_inner(market);

    accounts.asset_state.set_inner(AssetState {
        layout_version: LAYOUT_VERSION,
        bump: bumps.asset_state,
        market: market_key,
        generation: 0,
        oracle_generation: 0,
        rate_epoch_generation: 0,
        accounting_borrow_index: WAD,
        accounting_index_timestamp: clock.unix_timestamp,
        utilization_band: 0,
        utilization_observation_slot: clock.slot,
        risk: RiskParams::LAUNCH_DEFAULT,
        rates: RateParams::LAUNCH_DEFAULT,
        caps: args.caps,
        pauses: PauseState::default(),
        timelock: TimelockState {
            delay_seconds: TimelockState::LAUNCH_DEFAULT_DELAY_SECONDS,
            next_proposal_id: 0,
        },
        reserved: [0; 128],
    });

    accounts.pool_state.set_inner(PoolState {
        coordination: Coordination::default(),
        bump: bumps.pool_state,
        layout_version: LAYOUT_VERSION,
        status: PoolStatus::Initializing,
        market: market_key,
        accounted_borrow_index: WAD,
        encrypted: EncryptedPoolState::default(),
        unsettled_recovery_claim_count: 0,
    });

    accounts.vault_authority.set_inner(VaultAuthority {
        market: market_key,
        bump: bumps.vault_authority,
    });

    accounts
        .recovery_position
        .set_inner(Position::recovery(market_key, bumps.recovery_position));

    emit!(MarketInitialized {
        market: market_key,
        asset_state: accounts.asset_state.key(),
        pool_state: accounts.pool_state.key(),
        recovery_position: accounts.recovery_position.key(),
        vault_authority: accounts.vault_authority.key(),
        wsol_vault: accounts.wsol_vault.key(),
        usdc_vault: accounts.usdc_vault.key(),
        usdc_mint: accounts.usdc_mint.key(),
        admin: authority,
        risk_authority: args.risk_authority,
        treasury_authority: args.treasury_authority,
        guardian_authority: args.guardian_authority,
    });
    Ok(())
}
