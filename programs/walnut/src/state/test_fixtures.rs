//! Shared host-test fixtures for the Task 8 supply, recovery, and checkpoint
//! tests: one Market, Asset State, Pool, and Active Position.

use anchor_lang::prelude::Pubkey;

use super::accounts::{EncryptedPoolState, PoolState, PoolStatus, Position, PositionStatus};
use super::coordination::Coordination;
use crate::market::{
    AssetState, Caps, Market, MarketStatus, PauseState, RateParams, RiskParams, TimelockState,
};
use crate::math::WAD;

pub const ADMIN: u8 = 1;
pub const TREASURY: u8 = 5;
pub const MARKET: u8 = 0x30;
pub const POOL: u8 = 0x42;
pub const OWNER: u8 = 0x11;
pub const POSITION: u8 = 0x23;

pub fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}

pub fn market() -> Market {
    Market {
        layout_version: 1,
        status: MarketStatus::Active,
        bump: 0xFE,
        vault_authority_bump: 0xFD,
        admin: key(ADMIN),
        pending_admin: Pubkey::default(),
        upgrade_authority: key(3),
        risk_authority: key(4),
        treasury_authority: key(TREASURY),
        guardian_authority: key(6),
        asset_state: key(0x41),
        pool_state: key(POOL),
        recovery_position: key(0x43),
        vault_authority: key(0x44),
        wsol_mint: key(0x45),
        usdc_mint: key(0x46),
        wsol_vault: key(0x47),
        usdc_vault: key(0x48),
        treasury_usdc_account: key(0x60),
        position_count: 1,
        debt_position_count: 0,
        unsettled_recovery_claim_count: 0,
        position_count_cap: 10,
        debt_position_count_cap: 5,
        initialized_slot: 1,
        debt_position_reservations: 0,
        reserved: [0; 120],
    }
}

/// An Asset State whose band was observed at slot 90 and whose supply cap is
/// `u64::MAX`.
pub fn asset() -> AssetState {
    AssetState {
        layout_version: 1,
        bump: 1,
        market: key(MARKET),
        generation: 5,
        oracle_generation: 2,
        rate_epoch_generation: 3,
        accounting_borrow_index: WAD,
        accounting_index_timestamp: 1_000,
        utilization_band: 4,
        utilization_observation_slot: 90,
        risk: RiskParams::LAUNCH_DEFAULT,
        rates: RateParams::LAUNCH_DEFAULT,
        caps: Caps {
            supply_cap_usdc: u64::MAX,
            ..Caps::default()
        },
        pauses: PauseState::default(),
        timelock: TimelockState::default(),
        reserved: [0; 128],
    }
}

pub fn pool() -> PoolState {
    PoolState {
        coordination: Coordination {
            generation: 9,
            ..Coordination::default()
        },
        bump: 1,
        layout_version: 1,
        status: PoolStatus::Active,
        market: key(MARKET),
        accounted_borrow_index: WAD,
        encrypted: EncryptedPoolState {
            nonce: 77,
            ciphertexts: [[1; 32], [2; 32], [3; 32], [4; 32]],
        },
    }
}

pub fn position() -> Position {
    let mut p = Position::blank(key(OWNER), key(MARKET), 7);
    p.status = PositionStatus::Active;
    p.viewing_pubkey = [0x33; 32];
    p.coordination.generation = 3;
    p.encrypted_state.nonce = 500;
    p.encrypted_state.ciphertexts = [[0x51; 32], [0x52; 32], [0x53; 32], [0x54; 32], [0x55; 32]];
    p
}
