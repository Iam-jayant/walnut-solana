//! Market, Asset State, vault, and governance state (Task 4).
//!
//! Task 4.1 creates the single SOL/USDC market (Requirement 4.1):
//!
//! * [`state`] holds the `Market`, `AssetState`, and `VaultAuthority` accounts
//!   and the fixed-size parameter groups they embed. Every field the design
//!   assigns to Market and Asset State is present now, so Tasks 4.2-4.5 add
//!   behaviour without moving offsets.
//! * [`validation`] holds the pure checks run by `initialize_market` (mint and
//!   token-program ownership, the upgrade-authority gate, repeat
//!   initialization, authority keys). They are plain functions so they run in
//!   host tests.
//! * [`roles`] (Task 4.2) holds [`authorize`], which every privileged
//!   instruction uses, and the pure two-step admin transfer and role-setter
//!   transitions.
//! * [`pause`] (Task 4.3) holds the granular pause model, the
//!   operation-to-category mapping, [`ensure_not_paused`], and the guardian
//!   and governance pause transitions.
//! * [`config`] (Task 4.4) holds the `ConfigProposal` PDA, parameter bounds,
//!   maximum movement, the liquidation cross-bound, and the propose, execute,
//!   and cancel transitions of the configuration timelock.
//! * [`caps`] (Task 4.5) holds the cap checks the circuits mirror, the
//!   public Position and debt-Position counts with their caps, the
//!   authenticated debt transitions, cap bounds, and the immediate cap
//!   decrease.
//! * `governance_tests` (Task 4.6, test-only) checks the governance surface
//!   above against an independent reference model.
//!
//! The Pool State lives in `state::accounts` with the other generation-protected
//! accounts, and the Recovery Position reuses the production `Position`.
//!
//! # PDA seeds
//!
//! ```text
//! Market            = PDA[ "market" ]
//! AssetState        = PDA[ "asset_state" | market ]
//! PoolState         = PDA[ "pool_state" | market ]
//! VaultAuthority    = PDA[ "vault_authority" | market ]
//! wSOL vault        = PDA[ "vault" | market | wsol_mint ]
//! USDC vault        = PDA[ "vault" | market | usdc_mint ]
//! Recovery Position = PDA[ "recovery_position" | market ]
//! ConfigProposal    = PDA[ "config_proposal" | market | id (u64 LE) ]
//! ```
//!
//! The Market seed has no variable part, so the program can hold exactly one
//! Market (Requirement 4.1, "one Market").

pub mod caps;
pub mod config;
#[cfg(test)]
mod governance_tests;
pub mod pause;
pub mod roles;
pub mod state;
pub mod validation;

pub use caps::{
    check_caps_bounds, check_count_caps_bounds, decrease_caps, on_position_closed,
    on_position_initialized, reserve_debt_slot, settle_debt_intent, CapCheck, DebtSettlement,
    DebtSlot, DebtTransition, NewRiskOutcome,
};
pub use config::{
    derive_config_proposal_address, ConfigExecution, ConfigGroup, ConfigProposal, ConfigValue,
    ConfigValues, GovernedConfig, ProposalStatus, CONFIG_PROPOSAL_SEED, MAX_TIMELOCK_DELAY_SECONDS,
    MIN_TIMELOCK_DELAY_SECONDS,
};

pub use pause::{
    action_pause_category, ensure_action_allowed, ensure_not_paused, guardian_pause_active,
    is_paused, pause_category, Action, ActionClass, PauseCategory, PauseChange,
    GUARDIAN_MAX_PAUSE_SLOTS, GUARDIAN_PAUSE_COOLDOWN_SLOTS,
};

pub use roles::{
    accept_admin, apply_role_change, authorize, cancel_admin_transfer,
    check_loader_upgrade_authority, check_market_roles, propose_admin, role_key, set_role,
    set_upgrade_authority, validate_role_key, Role, RoleKind,
};
pub use state::{
    pause_flags, AssetState, Caps, CountCaps, Market, MarketStatus, PauseState, RateParams,
    RiskParams, TimelockState, VaultAuthority,
};
pub use validation::{
    check_initialize_authorities, check_market_mints, check_pool_initializing,
    check_upgrade_authority, ensure_fresh, InitMarkers, MarketError, MintView, ProgramAccountView,
};

use anchor_lang::prelude::Pubkey;

/// Seed of the singleton Market PDA.
pub const MARKET_SEED: &[u8] = b"market";
/// Seed of the Asset State PDA.
pub const ASSET_STATE_SEED: &[u8] = b"asset_state";
/// Seed of the Pool State PDA.
pub const POOL_STATE_SEED: &[u8] = b"pool_state";
/// Seed of the vault-authority PDA that owns both vault token accounts.
pub const VAULT_AUTHORITY_SEED: &[u8] = b"vault_authority";
/// Seed of a vault token account (followed by the market and the mint).
pub const VAULT_SEED: &[u8] = b"vault";
/// Seed of the protocol-owned Recovery Position.
pub const RECOVERY_POSITION_SEED: &[u8] = b"recovery_position";

/// Layout version written into Market, Asset State, and Pool State.
pub const LAYOUT_VERSION: u8 = 1;
/// Circle USDC uses 6 decimals on every network.
pub const USDC_DECIMALS: u8 = 6;
/// WAD fixed-point scale of the Borrow Index (1.0 = 10^18).
pub const WAD: u128 = 1_000_000_000_000_000_000;

fn derive(seeds: &[&[u8]], program_id: &Pubkey) -> (Pubkey, u8) {
    let (address, bump) = anchor_lang::derive_program_address(seeds, &program_id.to_bytes());
    (Pubkey::new_from_array(address), bump)
}

/// Derives the Market address and bump.
pub fn derive_market_address(program_id: &Pubkey) -> (Pubkey, u8) {
    derive(&[MARKET_SEED], program_id)
}

/// Derives the Asset State address and bump for `market`.
pub fn derive_asset_state_address(market: &Pubkey, program_id: &Pubkey) -> (Pubkey, u8) {
    derive(&[ASSET_STATE_SEED, market.as_ref()], program_id)
}

/// Derives the Pool State address and bump for `market`.
pub fn derive_pool_state_address(market: &Pubkey, program_id: &Pubkey) -> (Pubkey, u8) {
    derive(&[POOL_STATE_SEED, market.as_ref()], program_id)
}

/// Derives the vault-authority address and bump for `market`.
pub fn derive_vault_authority_address(market: &Pubkey, program_id: &Pubkey) -> (Pubkey, u8) {
    derive(&[VAULT_AUTHORITY_SEED, market.as_ref()], program_id)
}

/// Derives the vault token-account address and bump for `market` and `mint`.
pub fn derive_vault_address(market: &Pubkey, mint: &Pubkey, program_id: &Pubkey) -> (Pubkey, u8) {
    derive(&[VAULT_SEED, market.as_ref(), mint.as_ref()], program_id)
}

/// Derives the Recovery Position address and bump for `market`.
pub fn derive_recovery_position_address(market: &Pubkey, program_id: &Pubkey) -> (Pubkey, u8) {
    derive(&[RECOVERY_POSITION_SEED, market.as_ref()], program_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::{derive_counter_address, derive_intent_address};

    fn program() -> Pubkey {
        crate::ID
    }

    #[test]
    fn seeds_are_distinct_and_short() {
        let seeds = [
            MARKET_SEED,
            ASSET_STATE_SEED,
            POOL_STATE_SEED,
            VAULT_AUTHORITY_SEED,
            VAULT_SEED,
            RECOVERY_POSITION_SEED,
            CONFIG_PROPOSAL_SEED,
        ];
        for (index, seed) in seeds.iter().enumerate() {
            assert!(seed.len() <= 32);
            assert!(!seeds[index + 1..].contains(seed));
        }
    }

    #[test]
    fn every_market_address_is_distinct() {
        let (market, _) = derive_market_address(&program());
        let wsol = anchor_spl::token::spl_token::native_mint::ID;
        let usdc = Pubkey::new_from_array([7; 32]);
        let addresses = [
            market,
            derive_asset_state_address(&market, &program()).0,
            derive_pool_state_address(&market, &program()).0,
            derive_vault_authority_address(&market, &program()).0,
            derive_vault_address(&market, &wsol, &program()).0,
            derive_vault_address(&market, &usdc, &program()).0,
            derive_recovery_position_address(&market, &program()).0,
            derive_counter_address(&market, &program()).0,
            derive_intent_address(&market, 0, &program()).0,
        ];
        for (index, address) in addresses.iter().enumerate() {
            assert!(!addresses[index + 1..].contains(address));
        }
        // Deterministic.
        assert_eq!(derive_market_address(&program()).0, market);
    }

    #[test]
    fn constants_match_the_design() {
        assert_eq!(USDC_DECIMALS, 6);
        assert_eq!(WAD, 10u128.pow(18));
        assert_eq!(LAYOUT_VERSION, 1);
    }
}
