//! Market, Asset State, and vault-authority accounts (Task 4.1).
//!
//! Every field the design assigns to these accounts ("Market and Asset State")
//! exists now with a fixed size, so Tasks 4.2-4.5 implement behaviour without
//! moving offsets. Optional keys are stored as `Pubkey::default()` (all zero)
//! for "none" rather than `Option<Pubkey>`, because Borsh writes `None` as one
//! byte and that would shift every later field.
//!
//! Each of `Market` and `AssetState` ends with a zeroed `reserved` tail. A later
//! task that needs a new field takes it from the front of `reserved` and
//! shrinks `reserved` by the same size, so the account size and every existing
//! offset stay unchanged.
//!
//! # Layouts (offsets include the 8-byte Anchor discriminator)
//!
//! `Market`, 668 bytes:
//!
//! | Offset  | Field                            | Type          |
//! | ------- | -------------------------------- | ------------- |
//! | 8       | `layout_version`                 | u8            |
//! | 9       | `status`                         | MarketStatus  |
//! | 10      | `bump`                           | u8            |
//! | 11      | `vault_authority_bump`           | u8            |
//! | 12      | `admin`                          | Pubkey        |
//! | 44      | `pending_admin`                  | Pubkey        |
//! | 76      | `upgrade_authority`              | Pubkey        |
//! | 108     | `risk_authority`                 | Pubkey        |
//! | 140     | `treasury_authority`             | Pubkey        |
//! | 172     | `guardian_authority`             | Pubkey        |
//! | 204     | `asset_state`                    | Pubkey        |
//! | 236     | `pool_state`                     | Pubkey        |
//! | 268     | `recovery_position`              | Pubkey        |
//! | 300     | `vault_authority`                | Pubkey        |
//! | 332     | `wsol_mint`                      | Pubkey        |
//! | 364     | `usdc_mint`                      | Pubkey        |
//! | 396     | `wsol_vault`                     | Pubkey        |
//! | 428     | `usdc_vault`                     | Pubkey        |
//! | 460     | `treasury_usdc_account`          | Pubkey        |
//! | 492     | `position_count`                 | u64           |
//! | 500     | `debt_position_count`            | u64           |
//! | 508     | `unsettled_recovery_claim_count` | u64           |
//! | 516     | `position_count_cap`             | u64           |
//! | 524     | `debt_position_count_cap`        | u64           |
//! | 532     | `initialized_slot`               | u64           |
//! | 540     | `debt_position_reservations`     | u64 (4.5)     |
//! | 548     | `reserved`                       | [u8; 120]     |
//!
//! `AssetState`, 319 bytes:
//!
//! | Offset  | Field                            | Type          |
//! | ------- | -------------------------------- | ------------- |
//! | 8       | `layout_version`                 | u8            |
//! | 9       | `bump`                           | u8            |
//! | 10      | `market`                         | Pubkey        |
//! | 42      | `generation`                     | u64           |
//! | 50      | `oracle_generation`              | u64           |
//! | 58      | `rate_epoch_generation`          | u64           |
//! | 66      | `accounting_borrow_index`        | u128 (WAD)    |
//! | 82      | `accounting_index_timestamp`     | i64           |
//! | 90      | `utilization_band`               | u8            |
//! | 91      | `utilization_observation_slot`   | u64           |
//! | 99      | `risk`                           | RiskParams 22 |
//! | 121     | `rates`                          | RateParams 22 |
//! | 143     | `caps`                           | Caps 24       |
//! | 167     | `pauses`                         | PauseState 12 |
//! | 179     | `timelock`                       | Timelock 12   |
//! | 191     | `reserved`                       | [u8; 128]     |
//!
//! `VaultAuthority`, 41 bytes: `market` 8..40, `bump` 40.

use anchor_lang::prelude::*;

/// Lifecycle of the Market account. Zeroed (not yet written) data reads as
/// `Uninitialized`, which is how repeat initialization is detected.
#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub enum MarketStatus {
    /// Account space exists but `initialize_market` has not written it.
    #[default]
    Uninitialized,
    /// `initialize_market` completed.
    Active,
}

/// The singleton Market: authorities, public lifecycle counts and their caps,
/// vaults, and the active Asset State (design: "Market and Asset State").
#[account]
#[derive(InitSpace, Debug, PartialEq, Eq)]
pub struct Market {
    /// Layout version (`market::LAYOUT_VERSION`).
    pub layout_version: u8,
    /// Lifecycle status.
    pub status: MarketStatus,
    /// The Market PDA bump.
    pub bump: u8,
    /// The vault-authority PDA bump, kept here so token CPIs can sign cheaply.
    pub vault_authority_bump: u8,
    /// Current admin. Changes only through `propose_admin` then
    /// `accept_admin` (Task 4.2, `market::roles`).
    pub admin: Pubkey,
    /// Proposed admin awaiting acceptance; all zero when none (Task 4.2).
    pub pending_admin: Pubkey,
    /// The program upgrade authority, recorded at initialization and updated
    /// by `set_upgrade_authority` only to the key the loader names (Task 4.2).
    pub upgrade_authority: Pubkey,
    /// Risk-parameter authority (Task 4.2/4.4).
    pub risk_authority: Pubkey,
    /// Treasury authority (Task 4.2, reserve withdrawal).
    pub treasury_authority: Pubkey,
    /// Guardian authority for expiring emergency pauses (Task 4.2/4.3).
    pub guardian_authority: Pubkey,
    /// The active Asset State.
    pub asset_state: Pubkey,
    /// The USDC Pool State.
    pub pool_state: Pubkey,
    /// The protocol-owned Recovery Position.
    pub recovery_position: Pubkey,
    /// The PDA that owns both vaults.
    pub vault_authority: Pubkey,
    /// The wSOL (native) mint.
    pub wsol_mint: Pubkey,
    /// The configured USDC mint.
    pub usdc_mint: Pubkey,
    /// The wSOL vault token account.
    pub wsol_vault: Pubkey,
    /// The USDC vault token account.
    pub usdc_vault: Pubkey,
    /// The single treasury USDC token account; all zero until configured.
    pub treasury_usdc_account: Pubkey,
    /// Public count of initialized Positions (Task 4.5 updates it).
    pub position_count: u64,
    /// Public count of Positions with `has_debt` (Task 4.5).
    pub debt_position_count: u64,
    /// Public count of unsettled Recovery Claims (Absorb and settlement).
    pub unsettled_recovery_claim_count: u64,
    /// Cap on `position_count` (Task 4.5 enforces it).
    pub position_count_cap: u64,
    /// Cap on `debt_position_count` (Task 4.5 enforces it).
    pub debt_position_count_cap: u64,
    /// Slot at which `initialize_market` ran.
    pub initialized_slot: u64,
    /// Debt-Position slots held by queued borrows of Positions without debt
    /// (Task 4.5, `market::caps`). `debt_position_count +
    /// debt_position_reservations` never exceeds the cap through new risk.
    pub debt_position_reservations: u64,
    /// Zeroed extension space; see the module docs.
    pub reserved: [u8; 120],
}

impl Market {
    /// Account data length including Anchor's 8-byte discriminator.
    pub const ACCOUNT_LEN: usize = 8 + Self::INIT_SPACE;

    /// The public count caps.
    pub fn count_caps(&self) -> CountCaps {
        CountCaps {
            position_count_cap: self.position_count_cap,
            debt_position_count_cap: self.debt_position_count_cap,
        }
    }

    /// Writes the public count caps.
    pub fn set_count_caps(&mut self, caps: CountCaps) {
        self.position_count_cap = caps.position_count_cap;
        self.debt_position_count_cap = caps.debt_position_count_cap;
    }
}

/// The Market's public count caps (Requirement 4.10) as one governed group.
/// Stored on the Market as `position_count_cap` and `debt_position_count_cap`;
/// this struct is the value carried by proposals and events (Task 4.5).
#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub struct CountCaps {
    /// Cap on `Market.position_count`.
    pub position_count_cap: u64,
    /// Cap on `Market.debt_position_count`.
    pub debt_position_count_cap: u64,
}

/// Risk parameters (Requirement 4.12). Ratios are basis points (10,000 = 1.0).
/// Task 4.4 adds bounds, maximum movement per change, and the Requirement 4.13
/// cross-bound check for every governed change.
#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RiskParams {
    /// Maximum loan-to-value for borrow and collateral withdrawal.
    pub ltv_bps: u16,
    /// Liquidation Threshold.
    pub liquidation_threshold_bps: u16,
    /// Liquidation bonus paid in collateral.
    pub liquidation_bonus_bps: u16,
    /// Target Health Factor after a partial liquidation.
    pub target_health_factor_bps: u16,
    /// Maximum share of debt one partial liquidation may repay.
    pub close_factor_bps: u16,
    /// Share of interest added to Reserves.
    pub reserve_factor_bps: u16,
    /// Maximum Pyth price age.
    pub oracle_max_age_seconds: u32,
    /// Maximum Pyth confidence as a share of price.
    pub oracle_max_confidence_bps: u16,
    /// Decision Expiry distance from the queue slot.
    pub decision_expiry_slots: u32,
}

impl RiskParams {
    /// Conservative launch defaults written by `initialize_market`. They satisfy
    /// Requirement 4.13 (`0.65 × 1.05 = 0.6825 < 1` and `1.10 > 0.6825`). The
    /// oracle values are the ARCHITECTURE.md defaults (60 s, 2%).
    pub const LAUNCH_DEFAULT: Self = Self {
        ltv_bps: 5_000,
        liquidation_threshold_bps: 6_500,
        liquidation_bonus_bps: 500,
        target_health_factor_bps: 11_000,
        close_factor_bps: 5_000,
        reserve_factor_bps: 1_000,
        oracle_max_age_seconds: 60,
        oracle_max_confidence_bps: 200,
        decision_expiry_slots: 150,
    };
}

/// Interest-rate parameters (annualized basis points) and the maximum age of a
/// utilization-band observation before risk-increasing actions return zero.
#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateParams {
    /// Borrow rate at zero utilization.
    pub base_rate_bps: u32,
    /// Slope up to the optimal utilization.
    pub slope1_bps: u32,
    /// Slope above the optimal utilization.
    pub slope2_bps: u32,
    /// Kink of the rate curve.
    pub optimal_utilization_bps: u16,
    /// Maximum age of the utilization-band observation.
    pub band_max_age_slots: u64,
}

impl RateParams {
    /// Launch defaults written by `initialize_market`.
    pub const LAUNCH_DEFAULT: Self = Self {
        base_rate_bps: 0,
        slope1_bps: 400,
        slope2_bps: 7_500,
        optimal_utilization_bps: 8_000,
        band_max_age_slots: 9_000,
    };
}

/// USDC caps (Requirement 4.10), in USDC base units (6 decimals). Zero means
/// no new risk of that kind is allowed. Rules live in `market::caps`
/// (Task 4.5): increases only through the timelock, decreases immediately.
#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub struct Caps {
    /// Cap on encrypted supplier equity after a new supply.
    pub supply_cap_usdc: u64,
    /// Cap on total debt at the projected risk index.
    pub total_debt_cap_usdc: u64,
    /// Cap on one Position's debt at the projected risk index.
    pub position_debt_cap_usdc: u64,
}

/// Bit positions of the granular pause controls (Requirement 4.7). Task 4.3
/// defines which actions each bit blocks and keeps rescue paths open.
pub mod pause_flags {
    /// New supply.
    pub const SUPPLY: u16 = 1 << 0;
    /// Supply withdrawal.
    pub const WITHDRAW_SUPPLY: u16 = 1 << 1;
    /// New borrow.
    pub const BORROW: u16 = 1 << 2;
    /// Repay.
    pub const REPAY: u16 = 1 << 3;
    /// Collateral addition.
    pub const ADD_COLLATERAL: u16 = 1 << 4;
    /// Collateral withdrawal.
    pub const WITHDRAW_COLLATERAL: u16 = 1 << 5;
    /// Shield.
    pub const SHIELD: u16 = 1 << 6;
    /// Unshield.
    pub const UNSHIELD: u16 = 1 << 7;
    /// Liquidation start.
    pub const LIQUIDATION_START: u16 = 1 << 8;
    /// Liquidation completion.
    pub const LIQUIDATION_COMPLETE: u16 = 1 << 9;
    /// Every defined bit.
    pub const ALL: u16 = (1 << 10) - 1;
    /// Rescue bits (Requirement 4.8). No instruction may set them and they
    /// never block (Task 4.3, `market::pause`).
    pub const NEVER_PAUSABLE: u16 = REPAY | ADD_COLLATERAL;
    /// Bits a pause instruction may set.
    pub const PAUSABLE: u16 = ALL & !NEVER_PAUSABLE;
    /// Risk-increasing class: new supply, borrow, collateral withdrawal, and
    /// shield.
    pub const RISK_INCREASING: u16 = SUPPLY | BORROW | WITHDRAW_COLLATERAL | SHIELD;
    /// Withdrawal class: supply (and reserve) withdrawal and unshield.
    pub const WITHDRAWAL: u16 = WITHDRAW_SUPPLY | UNSHIELD;
    /// Settlement class: liquidation start and completion (including Absorb
    /// and recovery settlement).
    pub const SETTLEMENT: u16 = LIQUIDATION_START | LIQUIDATION_COMPLETE;
}

/// Pause state. `flags` are governance pauses; `guardian_flags` are guardian
/// pauses that apply only while the slot is below `guardian_expiry_slot`
/// (Requirement 4.9). Rules live in `market::pause`.
#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub struct PauseState {
    /// Governance pause bits (`pause_flags`).
    pub flags: u16,
    /// Guardian pause bits (`pause_flags`).
    pub guardian_flags: u16,
    /// First slot at which `guardian_flags` no longer apply; zero when no
    /// guardian pause is stored.
    pub guardian_expiry_slot: u64,
}

/// Timelock state for configuration changes (Requirement 4.5). Proposals
/// live in `ConfigProposal` PDAs; rules are in `market::config` (Task 4.4).
#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub struct TimelockState {
    /// Minimum delay between proposal and execution.
    pub delay_seconds: u32,
    /// Identifier the next proposal receives.
    pub next_proposal_id: u64,
}

impl TimelockState {
    /// Launch default delay: 48 hours.
    pub const LAUNCH_DEFAULT_DELAY_SECONDS: u32 = 172_800;
}

/// Public per-market state: configuration, Borrow Index, rate epoch,
/// utilization band, caps, pauses, and timelock state.
#[account]
#[derive(InitSpace, Debug, PartialEq, Eq)]
pub struct AssetState {
    /// Layout version (`market::LAYOUT_VERSION`). Zero means not yet written.
    pub layout_version: u8,
    /// The PDA bump.
    pub bump: u8,
    /// The owning Market.
    pub market: Pubkey,
    /// Asset State (configuration) generation captured by every Intent.
    pub generation: u64,
    /// Oracle generation (anomaly mode increments it).
    pub oracle_generation: u64,
    /// Rate-epoch generation.
    pub rate_epoch_generation: u64,
    /// Public accounting Borrow Index (WAD). Starts at 1.0.
    pub accounting_borrow_index: u128,
    /// Unix timestamp of the accounting Borrow Index.
    pub accounting_index_timestamp: i64,
    /// Latest revealed utilization band index.
    pub utilization_band: u8,
    /// Slot of the latest utilization-band observation.
    pub utilization_observation_slot: u64,
    /// Risk parameters.
    pub risk: RiskParams,
    /// Rate parameters.
    pub rates: RateParams,
    /// USDC caps.
    pub caps: Caps,
    /// Pause controls.
    pub pauses: PauseState,
    /// Timelock state.
    pub timelock: TimelockState,
    /// Zeroed extension space; see the module docs.
    pub reserved: [u8; 128],
}

impl AssetState {
    /// Account data length including Anchor's 8-byte discriminator.
    pub const ACCOUNT_LEN: usize = 8 + Self::INIT_SPACE;
}

/// The PDA that owns the wSOL and USDC vault token accounts and signs vault
/// transfers (`["vault_authority", market]`).
#[account]
#[derive(InitSpace, Debug, PartialEq, Eq)]
pub struct VaultAuthority {
    /// The owning Market. All zero means not yet written.
    pub market: Pubkey,
    /// The PDA bump.
    pub bump: u8,
}

impl VaultAuthority {
    /// Account data length including Anchor's 8-byte discriminator.
    pub const ACCOUNT_LEN: usize = 8 + Self::INIT_SPACE;
}

#[cfg(test)]
mod tests {
    use super::*;
    use anchor_lang::{AccountDeserialize, AccountSerialize, Discriminator, Space};

    fn key(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    fn read_u64(bytes: &[u8], at: usize) -> u64 {
        u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
    }

    fn sample_market() -> Market {
        Market {
            layout_version: 1,
            status: MarketStatus::Active,
            bump: 0xFE,
            vault_authority_bump: 0xFD,
            admin: key(1),
            pending_admin: key(2),
            upgrade_authority: key(3),
            risk_authority: key(4),
            treasury_authority: key(5),
            guardian_authority: key(6),
            asset_state: key(7),
            pool_state: key(8),
            recovery_position: key(9),
            vault_authority: key(10),
            wsol_mint: key(11),
            usdc_mint: key(12),
            wsol_vault: key(13),
            usdc_vault: key(14),
            treasury_usdc_account: key(15),
            position_count: 0x11,
            debt_position_count: 0x12,
            unsettled_recovery_claim_count: 0x13,
            position_count_cap: 0x14,
            debt_position_count_cap: 0x15,
            initialized_slot: 0x16,
            debt_position_reservations: 0x17,
            reserved: [0; 120],
        }
    }

    fn sample_asset_state() -> AssetState {
        AssetState {
            layout_version: 1,
            bump: 0xFC,
            market: key(0x21),
            generation: 0x31,
            oracle_generation: 0x32,
            rate_epoch_generation: 0x33,
            accounting_borrow_index: 0x0102_0304_0506_0708_090A_0B0C_0D0E_0F10,
            accounting_index_timestamp: -5,
            utilization_band: 7,
            utilization_observation_slot: 0x34,
            risk: RiskParams::LAUNCH_DEFAULT,
            rates: RateParams::LAUNCH_DEFAULT,
            caps: Caps {
                supply_cap_usdc: 0x41,
                total_debt_cap_usdc: 0x42,
                position_debt_cap_usdc: 0x43,
            },
            pauses: PauseState {
                flags: 0x0201,
                guardian_flags: 0x0102,
                guardian_expiry_slot: 0x51,
            },
            timelock: TimelockState {
                delay_seconds: 0x61,
                next_proposal_id: 0x62,
            },
            reserved: [0; 128],
        }
    }

    #[test]
    fn market_space_is_fixed() {
        assert_eq!(MarketStatus::INIT_SPACE, 1);
        assert_eq!(Market::INIT_SPACE, 4 + 15 * 32 + 7 * 8 + 120);
        assert_eq!(Market::INIT_SPACE, 660);
        assert_eq!(Market::ACCOUNT_LEN, 668);
        assert_eq!(CountCaps::INIT_SPACE, 16);
    }

    #[test]
    fn asset_state_space_is_fixed() {
        assert_eq!(RiskParams::INIT_SPACE, 22);
        assert_eq!(RateParams::INIT_SPACE, 22);
        assert_eq!(Caps::INIT_SPACE, 24);
        assert_eq!(PauseState::INIT_SPACE, 12);
        assert_eq!(TimelockState::INIT_SPACE, 12);
        assert_eq!(AssetState::INIT_SPACE, 311);
        assert_eq!(AssetState::ACCOUNT_LEN, 319);
    }

    #[test]
    fn vault_authority_space_is_fixed() {
        assert_eq!(VaultAuthority::INIT_SPACE, 33);
        assert_eq!(VaultAuthority::ACCOUNT_LEN, 41);
    }

    #[test]
    fn market_byte_layout() {
        let market = sample_market();
        let mut bytes = Vec::new();
        market.try_serialize(&mut bytes).unwrap();
        assert_eq!(bytes.len(), Market::ACCOUNT_LEN);
        assert_eq!(&bytes[..8], Market::DISCRIMINATOR);
        assert_eq!(bytes[8..12], [1, 1, 0xFE, 0xFD]);
        // Fifteen keys from offset 12, each filled with its index.
        for index in 0..15u8 {
            let at = 12 + 32 * index as usize;
            assert_eq!(bytes[at..at + 32], [index + 1; 32], "key {index}");
        }
        for (index, at) in [492, 500, 508, 516, 524, 532, 540].iter().enumerate() {
            assert_eq!(read_u64(&bytes, *at), 0x11 + index as u64);
        }
        assert_eq!(bytes[548..], [0u8; 120]);
        // Count caps read from the same bytes as the two Market fields.
        assert_eq!(
            market.count_caps(),
            CountCaps {
                position_count_cap: 0x14,
                debt_position_count_cap: 0x15,
            }
        );
        let decoded = Market::try_deserialize(&mut bytes.as_slice()).unwrap();
        assert_eq!(decoded, market);
    }

    #[test]
    fn asset_state_byte_layout() {
        let asset = sample_asset_state();
        let mut bytes = Vec::new();
        asset.try_serialize(&mut bytes).unwrap();
        assert_eq!(bytes.len(), AssetState::ACCOUNT_LEN);
        assert_eq!(&bytes[..8], AssetState::DISCRIMINATOR);
        assert_eq!(bytes[8..10], [1, 0xFC]);
        assert_eq!(bytes[10..42], [0x21; 32]);
        assert_eq!(read_u64(&bytes, 42), 0x31);
        assert_eq!(read_u64(&bytes, 50), 0x32);
        assert_eq!(read_u64(&bytes, 58), 0x33);
        assert_eq!(
            u128::from_le_bytes(bytes[66..82].try_into().unwrap()),
            asset.accounting_borrow_index
        );
        assert_eq!(read_u64(&bytes, 82) as i64, -5);
        assert_eq!(bytes[90], 7);
        assert_eq!(read_u64(&bytes, 91), 0x34);
        // RiskParams at 99: ltv, threshold, ... decision_expiry_slots at 117.
        assert_eq!(u16::from_le_bytes([bytes[99], bytes[100]]), 5_000);
        assert_eq!(u16::from_le_bytes([bytes[101], bytes[102]]), 6_500);
        assert_eq!(u32::from_le_bytes(bytes[111..115].try_into().unwrap()), 60);
        assert_eq!(u32::from_le_bytes(bytes[117..121].try_into().unwrap()), 150);
        // RateParams at 121, band_max_age_slots at 135.
        assert_eq!(read_u64(&bytes, 135), 9_000);
        // Caps at 143.
        assert_eq!(read_u64(&bytes, 143), 0x41);
        assert_eq!(read_u64(&bytes, 151), 0x42);
        assert_eq!(read_u64(&bytes, 159), 0x43);
        // PauseState at 167.
        assert_eq!(u16::from_le_bytes([bytes[167], bytes[168]]), 0x0201);
        assert_eq!(u16::from_le_bytes([bytes[169], bytes[170]]), 0x0102);
        assert_eq!(read_u64(&bytes, 171), 0x51);
        // TimelockState at 179.
        assert_eq!(
            u32::from_le_bytes(bytes[179..183].try_into().unwrap()),
            0x61
        );
        assert_eq!(read_u64(&bytes, 183), 0x62);
        assert_eq!(bytes[191..], [0u8; 128]);
        let decoded = AssetState::try_deserialize(&mut bytes.as_slice()).unwrap();
        assert_eq!(decoded, asset);
    }

    #[test]
    fn zeroed_data_reads_as_uninitialized() {
        // `init_if_needed` hands the handler an all-zero account on first use.
        let mut market = Market::DISCRIMINATOR.to_vec();
        market.extend([0u8; Market::INIT_SPACE]);
        let decoded = Market::try_deserialize(&mut market.as_slice()).unwrap();
        assert_eq!(decoded.status, MarketStatus::Uninitialized);

        let mut asset = AssetState::DISCRIMINATOR.to_vec();
        asset.extend([0u8; AssetState::INIT_SPACE]);
        let decoded = AssetState::try_deserialize(&mut asset.as_slice()).unwrap();
        assert_eq!(decoded.layout_version, 0);
    }

    #[test]
    fn discriminators_are_distinct() {
        use crate::position::SpikePosition;
        use crate::state::{PoolState, Position};
        let all = [
            Market::DISCRIMINATOR,
            AssetState::DISCRIMINATOR,
            VaultAuthority::DISCRIMINATOR,
            crate::market::ConfigProposal::DISCRIMINATOR,
            PoolState::DISCRIMINATOR,
            Position::DISCRIMINATOR,
            SpikePosition::DISCRIMINATOR,
        ];
        for (index, discriminator) in all.iter().enumerate() {
            assert!(!all[index + 1..].contains(discriminator));
        }
    }

    #[test]
    fn pause_flags_cover_the_ten_required_controls() {
        let flags = [
            pause_flags::SUPPLY,
            pause_flags::WITHDRAW_SUPPLY,
            pause_flags::BORROW,
            pause_flags::REPAY,
            pause_flags::ADD_COLLATERAL,
            pause_flags::WITHDRAW_COLLATERAL,
            pause_flags::SHIELD,
            pause_flags::UNSHIELD,
            pause_flags::LIQUIDATION_START,
            pause_flags::LIQUIDATION_COMPLETE,
        ];
        let mut union = 0u16;
        for flag in flags {
            assert_eq!(flag.count_ones(), 1);
            assert_eq!(union & flag, 0);
            union |= flag;
        }
        assert_eq!(union, pause_flags::ALL);
    }

    #[test]
    fn launch_defaults_satisfy_the_liquidation_cross_bound() {
        // Requirement 4.13 in exact integer arithmetic (bps squared):
        // threshold × (1 + bonus) < 1 and target > threshold × (1 + bonus).
        let risk = RiskParams::LAUNCH_DEFAULT;
        let scaled = u64::from(risk.liquidation_threshold_bps)
            * (10_000 + u64::from(risk.liquidation_bonus_bps));
        assert!(scaled < 10_000 * 10_000);
        assert!(u64::from(risk.target_health_factor_bps) * 10_000 > scaled);
        assert!(risk.ltv_bps < risk.liquidation_threshold_bps);
        assert!(risk.close_factor_bps <= 10_000);
        assert!(risk.reserve_factor_bps <= 10_000);
    }
}
