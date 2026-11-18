//! Granular pause controls (Task 4.3, Requirements 4.5, 4.7, 4.8, and 4.9).
//!
//! # Model
//!
//! `AssetState.pauses` (12 bytes, unchanged since Task 4.1) holds two pause
//! masks over the `pause_flags` bits and one expiry:
//!
//! * `flags`: durable governance pauses. They block until governance clears
//!   them.
//! * `guardian_flags` with `guardian_expiry_slot`: guardian pauses. They block
//!   only while `now_slot < guardian_expiry_slot`, so they expire on their own.
//!
//! A pausable bit blocks when it is set in `flags`, or set in `guardian_flags`
//! while the guardian pause is active. [`ensure_not_paused`] is the check every
//! queue instruction runs (design, queue path step 1).
//!
//! # Action classes
//!
//! Requirement 4.7 names ten controls, which are the `pause_flags` bits. They
//! fall into four classes:
//!
//! | Class           | Bits                                              |
//! | --------------- | ------------------------------------------------- |
//! | Risk-increasing | `SUPPLY`, `BORROW`, `WITHDRAW_COLLATERAL`, `SHIELD` |
//! | Withdrawal      | `WITHDRAW_SUPPLY`, `UNSHIELD`                     |
//! | Settlement      | `LIQUIDATION_START`, `LIQUIDATION_COMPLETE`       |
//! | Rescue          | `REPAY`, `ADD_COLLATERAL` (never pausable)        |
//!
//! Requirement 4.8 keeps repay, collateral addition, callback completion,
//! Pending Shield recovery, and safe lock/intent recovery available in
//! emergency mode. The `REPAY` and `ADD_COLLATERAL` bits therefore exist so the
//! ten controls stay distinguishable, but no instruction may set them
//! (`PauseNotAllowed`) and [`ensure_not_paused`] ignores them even if set.
//! Rescue actions map to no pause bit at all ([`action_pause_category`]
//! returns `None`), so no pause state can block them.
//!
//! # Operation mapping
//!
//! [`Action`] lists every operation the design names. The first ten mirror
//! [`OperationKind`]; the rest are design operations that have no Intent kind
//! yet:
//!
//! | Action                      | Kind? | Pause bit              |
//! | --------------------------- | ----- | ---------------------- |
//! | `Probe`                     | yes   | none (test probe)      |
//! | `PositionInit`              | yes   | none (no value moves)  |
//! | `Shield`                    | yes   | `SHIELD`               |
//! | `Unshield`                  | yes   | `UNSHIELD`             |
//! | `Supply`                    | yes   | `SUPPLY`               |
//! | `WithdrawSupply`            | yes   | `WITHDRAW_SUPPLY`      |
//! | `Borrow`                    | yes   | `BORROW`               |
//! | `Repay`                     | yes   | none (rescue)          |
//! | `Health`                    | yes   | `LIQUIDATION_START`    |
//! | `Liquidation`               | yes   | `LIQUIDATION_COMPLETE` |
//! | `AddCollateral`             | no    | none (rescue)          |
//! | `WithdrawCollateral`        | no    | `WITHDRAW_COLLATERAL`  |
//! | `ReserveWithdrawal`         | no    | `WITHDRAW_SUPPLY`      |
//! | `OpenLiquidation`           | no    | `LIQUIDATION_START`    |
//! | `Absorb`                    | no    | `LIQUIDATION_COMPLETE` |
//! | `SettleRecoveryClaim`       | no    | `LIQUIDATION_COMPLETE` |
//! | `Callback`                  | no    | none (rescue)          |
//! | `AbandonIntent`             | no    | none (rescue)          |
//! | `PendingShieldRetry`        | no    | none (rescue)          |
//! | `PendingShieldRefund`       | no    | none (rescue)          |
//! | `RecoverySettlementRefund`  | no    | none (rescue)          |
//! | `ViewingKeyRotation`        | no    | none (no value moves)  |
//! | `UtilizationBandUpdate`     | no    | none (read-only)       |
//!
//! A health check is the first step of liquidation (it freezes an unhealthy
//! Position), so it shares `LIQUIDATION_START` with `open_liquidation`.
//! Reserve withdrawal takes Pool Cash out of the USDC vault, so it shares
//! `WITHDRAW_SUPPLY`. Absorb and `settle_recovery_claim` finish bad-debt
//! liquidation, so they share `LIQUIDATION_COMPLETE`.
//!
//! # Who may pause
//!
//! | Instruction                            | Signer   | Effect                                     |
//! | -------------------------------------- | -------- | ------------------------------------------ |
//! | `guardian_pause(flags, duration)`      | guardian | Sets guardian bits with an expiry.         |
//! | `set_pause(flags, paused)`             | risk     | Sets or clears durable bits.               |
//! | `renew_guardian_pause(expiry_slot)`    | risk     | Moves an active guardian expiry later.     |
//! | `clear_guardian_pause()`               | risk     | Clears every guardian bit and its expiry.  |
//!
//! Guardian rules (Requirement 4.9):
//!
//! * `1 <= duration_slots <= GUARDIAN_MAX_PAUSE_SLOTS`; the expiry is
//!   `now + duration_slots`.
//! * While a guardian pause is active, another guardian pause adds bits but
//!   keeps the existing expiry, so the guardian never extends a pause.
//! * After a guardian pause expires on its own, the guardian must wait
//!   `GUARDIAN_PAUSE_COOLDOWN_SLOTS` past the expiry before pausing again, so
//!   back-to-back pauses cannot make a guardian pause permanent.
//! * Only governance renews (extends) a guardian pause, or turns it into a
//!   durable pause with `set_pause`. Clearing resets the guardian state to
//!   zero, which also ends any cooldown.
//!
//! The admin is not an implicit pauser: governance here is the risk authority,
//! the Squads multisig that owns risk parameters.
//!
//! # Timelock and generations
//!
//! Requirement 4.5 allows emergency pauses to be immediate, and the design
//! does not place unpausing behind the timelock, so every transition applies
//! immediately. The `apply_*` functions are the validated, unauthorized
//! transitions; a Task 4.4 timelocked execute could authorize its own
//! proposal and call them. Pause changes do not bump the Asset State
//! generation: pauses gate new queues only, and already-queued work must still
//! complete through its callback.

use anchor_lang::prelude::{AnchorDeserialize, AnchorSerialize, Pubkey};

use super::roles::{authorize, Role};
use super::state::{pause_flags, Market, PauseState};
use super::validation::MarketError;
use crate::operation::OperationKind;

/// Longest guardian pause: 216,000 slots, about 24 hours at 400 ms slots.
pub const GUARDIAN_MAX_PAUSE_SLOTS: u64 = 216_000;
/// Wait after a guardian pause expires on its own before the guardian may
/// pause again: 9,000 slots, about one hour.
pub const GUARDIAN_PAUSE_COOLDOWN_SLOTS: u64 = 9_000;

/// The eight pausable controls. The rescue bits (`REPAY`, `ADD_COLLATERAL`)
/// have no category because they can never block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseCategory {
    /// New supply.
    Supply,
    /// Supply withdrawal (and reserve withdrawal).
    WithdrawSupply,
    /// New borrow.
    Borrow,
    /// Collateral withdrawal.
    WithdrawCollateral,
    /// Shield.
    Shield,
    /// Unshield.
    Unshield,
    /// Liquidation start: health freeze and `open_liquidation`.
    LiquidationStart,
    /// Liquidation completion: partial liquidation, Absorb, and recovery
    /// settlement.
    LiquidationComplete,
}

/// The action classes of Task 4.3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionClass {
    /// Adds risk or new value at risk.
    RiskIncreasing,
    /// Moves value out of the vaults.
    Withdrawal,
    /// Liquidation and bad-debt settlement.
    Settlement,
    /// Reduces risk or finishes or recovers existing work. Never paused.
    Rescue,
}

impl PauseCategory {
    /// Every pausable category, in bit order.
    pub const ALL: [PauseCategory; 8] = [
        PauseCategory::Supply,
        PauseCategory::WithdrawSupply,
        PauseCategory::Borrow,
        PauseCategory::WithdrawCollateral,
        PauseCategory::Shield,
        PauseCategory::Unshield,
        PauseCategory::LiquidationStart,
        PauseCategory::LiquidationComplete,
    ];

    /// The `pause_flags` bit of this category.
    pub const fn flag(self) -> u16 {
        match self {
            PauseCategory::Supply => pause_flags::SUPPLY,
            PauseCategory::WithdrawSupply => pause_flags::WITHDRAW_SUPPLY,
            PauseCategory::Borrow => pause_flags::BORROW,
            PauseCategory::WithdrawCollateral => pause_flags::WITHDRAW_COLLATERAL,
            PauseCategory::Shield => pause_flags::SHIELD,
            PauseCategory::Unshield => pause_flags::UNSHIELD,
            PauseCategory::LiquidationStart => pause_flags::LIQUIDATION_START,
            PauseCategory::LiquidationComplete => pause_flags::LIQUIDATION_COMPLETE,
        }
    }

    /// The class this category belongs to.
    pub const fn class(self) -> ActionClass {
        match self {
            PauseCategory::Supply
            | PauseCategory::Borrow
            | PauseCategory::WithdrawCollateral
            | PauseCategory::Shield => ActionClass::RiskIncreasing,
            PauseCategory::WithdrawSupply | PauseCategory::Unshield => ActionClass::Withdrawal,
            PauseCategory::LiquidationStart | PauseCategory::LiquidationComplete => {
                ActionClass::Settlement
            }
        }
    }
}

/// Every operation the design names (see the module table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// `OperationKind::Probe`.
    Probe,
    /// `OperationKind::PositionInit`.
    PositionInit,
    /// `OperationKind::Shield`.
    Shield,
    /// `OperationKind::Unshield`.
    Unshield,
    /// `OperationKind::Supply`.
    Supply,
    /// `OperationKind::WithdrawSupply`.
    WithdrawSupply,
    /// `OperationKind::Borrow`.
    Borrow,
    /// `OperationKind::Repay`.
    Repay,
    /// `OperationKind::Health`.
    Health,
    /// `OperationKind::Liquidation` (partial liquidation).
    Liquidation,
    /// Collateral addition (no Intent kind yet).
    AddCollateral,
    /// Collateral withdrawal (no Intent kind yet).
    WithdrawCollateral,
    /// Reserve withdrawal to the treasury (no Intent kind yet).
    ReserveWithdrawal,
    /// `open_liquidation`.
    OpenLiquidation,
    /// `absorb`.
    Absorb,
    /// `settle_recovery_claim`.
    SettleRecoveryClaim,
    /// Any authenticated callback finishing already-queued work.
    Callback,
    /// Permissionless abandonment of an expired Intent.
    AbandonIntent,
    /// Pending Shield retry.
    PendingShieldRetry,
    /// Pending Shield refund.
    PendingShieldRefund,
    /// Pending Recovery Settlement refund.
    RecoverySettlementRefund,
    /// Viewing Key rotation.
    ViewingKeyRotation,
    /// `update_utilization_band`.
    UtilizationBandUpdate,
}

impl Action {
    /// Every action, for exhaustive tests.
    pub const ALL: [Action; 23] = [
        Action::Probe,
        Action::PositionInit,
        Action::Shield,
        Action::Unshield,
        Action::Supply,
        Action::WithdrawSupply,
        Action::Borrow,
        Action::Repay,
        Action::Health,
        Action::Liquidation,
        Action::AddCollateral,
        Action::WithdrawCollateral,
        Action::ReserveWithdrawal,
        Action::OpenLiquidation,
        Action::Absorb,
        Action::SettleRecoveryClaim,
        Action::Callback,
        Action::AbandonIntent,
        Action::PendingShieldRetry,
        Action::PendingShieldRefund,
        Action::RecoverySettlementRefund,
        Action::ViewingKeyRotation,
        Action::UtilizationBandUpdate,
    ];

    /// The Requirement 4.8 actions that stay available in emergency mode.
    pub const fn is_rescue(self) -> bool {
        matches!(
            self,
            Action::Repay
                | Action::AddCollateral
                | Action::Callback
                | Action::AbandonIntent
                | Action::PendingShieldRetry
                | Action::PendingShieldRefund
                | Action::RecoverySettlementRefund
        )
    }

    /// The class of this action, or `None` for actions outside the four
    /// classes (probe, Position initialization, key rotation, band update).
    pub fn class(self) -> Option<ActionClass> {
        if self.is_rescue() {
            return Some(ActionClass::Rescue);
        }
        action_pause_category(self).map(PauseCategory::class)
    }
}

impl From<OperationKind> for Action {
    fn from(kind: OperationKind) -> Self {
        match kind {
            OperationKind::Probe => Action::Probe,
            OperationKind::PositionInit => Action::PositionInit,
            OperationKind::Shield => Action::Shield,
            OperationKind::Unshield => Action::Unshield,
            OperationKind::Supply => Action::Supply,
            OperationKind::WithdrawSupply => Action::WithdrawSupply,
            OperationKind::Borrow => Action::Borrow,
            OperationKind::Repay => Action::Repay,
            OperationKind::Health => Action::Health,
            OperationKind::Liquidation => Action::Liquidation,
            OperationKind::ViewingKeyRotation => Action::ViewingKeyRotation,
            OperationKind::UtilizationBand => Action::UtilizationBandUpdate,
            OperationKind::ReserveWithdrawal => Action::ReserveWithdrawal,
            OperationKind::RecoverEmptyPool => Action::ReserveWithdrawal,
            OperationKind::PositionOp | OperationKind::LendOp | OperationKind::Disclose => {
                Action::Probe
            }
        }
    }
}

/// The pause category that gates `action`, or `None` if no pause can block
/// it.
pub const fn action_pause_category(action: Action) -> Option<PauseCategory> {
    match action {
        Action::Shield => Some(PauseCategory::Shield),
        Action::Unshield => Some(PauseCategory::Unshield),
        Action::Supply => Some(PauseCategory::Supply),
        Action::WithdrawSupply | Action::ReserveWithdrawal => Some(PauseCategory::WithdrawSupply),
        Action::Borrow => Some(PauseCategory::Borrow),
        Action::WithdrawCollateral => Some(PauseCategory::WithdrawCollateral),
        Action::Health | Action::OpenLiquidation => Some(PauseCategory::LiquidationStart),
        Action::Liquidation | Action::Absorb | Action::SettleRecoveryClaim => {
            Some(PauseCategory::LiquidationComplete)
        }
        Action::Probe
        | Action::PositionInit
        | Action::Repay
        | Action::AddCollateral
        | Action::Callback
        | Action::AbandonIntent
        | Action::PendingShieldRetry
        | Action::PendingShieldRefund
        | Action::RecoverySettlementRefund
        | Action::ViewingKeyRotation
        | Action::UtilizationBandUpdate => None,
    }
}

/// The pause category that gates queueing an Intent of `kind`.
pub fn pause_category(kind: OperationKind) -> Option<PauseCategory> {
    action_pause_category(kind.into())
}

/// Whether the guardian pause is still in force at `now_slot`.
pub fn guardian_pause_active(pauses: &PauseState, now_slot: u64) -> bool {
    pauses.guardian_flags != 0 && now_slot < pauses.guardian_expiry_slot
}

/// The pausable bits that block at `now_slot`. Rescue bits never appear.
pub fn effective_pause_flags(pauses: &PauseState, now_slot: u64) -> u16 {
    let guardian = if guardian_pause_active(pauses, now_slot) {
        pauses.guardian_flags
    } else {
        0
    };
    (pauses.flags | guardian) & pause_flags::PAUSABLE
}

/// Whether `category` is paused at `now_slot`.
pub fn is_paused(pauses: &PauseState, category: PauseCategory, now_slot: u64) -> bool {
    effective_pause_flags(pauses, now_slot) & category.flag() != 0
}

/// Fails with `ActionPaused` if `category` is paused at `now_slot`. Every
/// queue instruction with a category runs this before any other write.
pub fn ensure_not_paused(
    pauses: &PauseState,
    category: PauseCategory,
    now_slot: u64,
) -> Result<(), MarketError> {
    if is_paused(pauses, category, now_slot) {
        return Err(MarketError::ActionPaused);
    }
    Ok(())
}

/// [`ensure_not_paused`] for an [`Action`]. Always `Ok` for actions without a
/// category, which includes every rescue action and callback completion.
pub fn ensure_action_allowed(
    pauses: &PauseState,
    action: Action,
    now_slot: u64,
) -> Result<(), MarketError> {
    match action_pause_category(action) {
        Some(category) => ensure_not_paused(pauses, category, now_slot),
        None => Ok(()),
    }
}

/// What a pause instruction did. Carried in [`crate::events::PauseUpdated`].
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseChange {
    /// Governance set durable bits.
    GovernancePause,
    /// Governance cleared durable bits.
    GovernanceUnpause,
    /// The guardian set expiring bits.
    GuardianPause,
    /// Governance moved the guardian expiry later.
    GuardianRenew,
    /// Governance cleared every guardian bit.
    GuardianClear,
}

/// Requires `flags` to be a non-empty set of pausable bits.
pub fn validate_pause_flags(flags: u16) -> Result<(), MarketError> {
    if flags == 0 || flags & !pause_flags::ALL != 0 {
        return Err(MarketError::InvalidPauseFlags);
    }
    if flags & pause_flags::NEVER_PAUSABLE != 0 {
        return Err(MarketError::PauseNotAllowed);
    }
    Ok(())
}

/// Validated, unauthorized durable pause change: sets `flags` when `paused`,
/// else clears them. Guardian bits are untouched.
pub fn apply_durable_pause(
    pauses: &mut PauseState,
    flags: u16,
    paused: bool,
) -> Result<(), MarketError> {
    validate_pause_flags(flags)?;
    if paused {
        pauses.flags |= flags;
    } else {
        pauses.flags &= !flags;
    }
    Ok(())
}

/// Validated, unauthorized guardian pause (rules in the module docs).
pub fn apply_guardian_pause(
    pauses: &mut PauseState,
    flags: u16,
    duration_slots: u64,
    now_slot: u64,
) -> Result<(), MarketError> {
    validate_pause_flags(flags)?;
    if duration_slots == 0 || duration_slots > GUARDIAN_MAX_PAUSE_SLOTS {
        return Err(MarketError::InvalidPauseDuration);
    }
    if guardian_pause_active(pauses, now_slot) {
        pauses.guardian_flags |= flags;
        return Ok(());
    }
    if pauses.guardian_expiry_slot != 0
        && now_slot
            < pauses
                .guardian_expiry_slot
                .saturating_add(GUARDIAN_PAUSE_COOLDOWN_SLOTS)
    {
        return Err(MarketError::GuardianPauseCooldown);
    }
    pauses.guardian_flags = flags;
    pauses.guardian_expiry_slot = now_slot
        .checked_add(duration_slots)
        .ok_or(MarketError::InvalidPauseDuration)?;
    Ok(())
}

/// Validated, unauthorized guardian renewal: an active guardian pause gets a
/// later `new_expiry_slot`.
pub fn apply_guardian_renewal(
    pauses: &mut PauseState,
    new_expiry_slot: u64,
    now_slot: u64,
) -> Result<(), MarketError> {
    if !guardian_pause_active(pauses, now_slot) {
        return Err(MarketError::NoActiveGuardianPause);
    }
    if new_expiry_slot <= pauses.guardian_expiry_slot {
        return Err(MarketError::InvalidPauseExpiry);
    }
    pauses.guardian_expiry_slot = new_expiry_slot;
    Ok(())
}

/// Validated, unauthorized guardian clear: resets the guardian bits and
/// expiry to zero, which also ends any cooldown. Fails if nothing is stored.
pub fn apply_guardian_clear(pauses: &mut PauseState) -> Result<(), MarketError> {
    if pauses.guardian_flags == 0 && pauses.guardian_expiry_slot == 0 {
        return Err(MarketError::NoActiveGuardianPause);
    }
    pauses.guardian_flags = 0;
    pauses.guardian_expiry_slot = 0;
    Ok(())
}

/// The guardian pauses `flags` for `duration_slots`.
pub fn guardian_pause(
    market: &Market,
    pauses: &mut PauseState,
    signer: &Pubkey,
    flags: u16,
    duration_slots: u64,
    now_slot: u64,
) -> Result<(), MarketError> {
    authorize(market, Role::Guardian, signer)?;
    apply_guardian_pause(pauses, flags, duration_slots, now_slot)
}

/// The risk authority sets or clears durable `flags`.
pub fn set_pause(
    market: &Market,
    pauses: &mut PauseState,
    signer: &Pubkey,
    flags: u16,
    paused: bool,
) -> Result<(), MarketError> {
    authorize(market, Role::Risk, signer)?;
    apply_durable_pause(pauses, flags, paused)
}

/// The risk authority renews the active guardian pause.
pub fn renew_guardian_pause(
    market: &Market,
    pauses: &mut PauseState,
    signer: &Pubkey,
    new_expiry_slot: u64,
    now_slot: u64,
) -> Result<(), MarketError> {
    authorize(market, Role::Risk, signer)?;
    apply_guardian_renewal(pauses, new_expiry_slot, now_slot)
}

/// The risk authority clears every guardian pause.
pub fn clear_guardian_pause(
    market: &Market,
    pauses: &mut PauseState,
    signer: &Pubkey,
) -> Result<(), MarketError> {
    authorize(market, Role::Risk, signer)?;
    apply_guardian_clear(pauses)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::state::MarketStatus;

    fn key(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    const GUARDIAN: Pubkey = Pubkey::new_from_array([6; 32]);
    const RISK: Pubkey = Pubkey::new_from_array([4; 32]);

    fn market() -> Market {
        Market {
            layout_version: 1,
            status: MarketStatus::Active,
            bump: 0xFE,
            vault_authority_bump: 0xFD,
            admin: key(1),
            pending_admin: Pubkey::default(),
            upgrade_authority: key(3),
            risk_authority: RISK,
            treasury_authority: key(5),
            guardian_authority: GUARDIAN,
            asset_state: key(0x41),
            pool_state: key(0x42),
            recovery_position: key(0x43),
            vault_authority: key(0x44),
            wsol_mint: key(0x45),
            usdc_mint: key(0x46),
            wsol_vault: key(0x47),
            usdc_vault: key(0x48),
            treasury_usdc_account: Pubkey::default(),
            position_count: 0,
            debt_position_count: 0,
            unsettled_recovery_claim_count: 0,
            position_count_cap: 10,
            debt_position_count_cap: 5,
            initialized_slot: 1,
            debt_position_reservations: 0,
            reserved: [0; 120],
        }
    }

    const KINDS: [OperationKind; 11] = [
        OperationKind::Probe,
        OperationKind::PositionInit,
        OperationKind::Shield,
        OperationKind::Unshield,
        OperationKind::Supply,
        OperationKind::WithdrawSupply,
        OperationKind::Borrow,
        OperationKind::Repay,
        OperationKind::Health,
        OperationKind::Liquidation,
        OperationKind::ViewingKeyRotation,
    ];

    #[test]
    fn categories_cover_exactly_the_pausable_bits() {
        let mut union = 0u16;
        for category in PauseCategory::ALL {
            assert_eq!(category.flag().count_ones(), 1);
            assert_eq!(union & category.flag(), 0);
            union |= category.flag();
            assert_ne!(category.class(), ActionClass::Rescue);
        }
        assert_eq!(union, pause_flags::PAUSABLE);
        assert_eq!(
            pause_flags::PAUSABLE | pause_flags::NEVER_PAUSABLE,
            pause_flags::ALL
        );
        assert_eq!(pause_flags::PAUSABLE & pause_flags::NEVER_PAUSABLE, 0);
        assert_eq!(
            pause_flags::RISK_INCREASING | pause_flags::WITHDRAWAL | pause_flags::SETTLEMENT,
            pause_flags::PAUSABLE
        );
        for (class, mask) in [
            (ActionClass::RiskIncreasing, pause_flags::RISK_INCREASING),
            (ActionClass::Withdrawal, pause_flags::WITHDRAWAL),
            (ActionClass::Settlement, pause_flags::SETTLEMENT),
        ] {
            let bits = PauseCategory::ALL
                .iter()
                .filter(|c| c.class() == class)
                .fold(0, |acc, c| acc | c.flag());
            assert_eq!(bits, mask, "{class:?}");
        }
    }

    #[test]
    fn operation_kinds_map_to_the_documented_categories() {
        use PauseCategory as C;
        let expected = [
            None,
            None,
            Some(C::Shield),
            Some(C::Unshield),
            Some(C::Supply),
            Some(C::WithdrawSupply),
            Some(C::Borrow),
            None,
            Some(C::LiquidationStart),
            Some(C::LiquidationComplete),
            // Viewing Key rotation moves no value and is never paused.
            None,
        ];
        for (kind, category) in KINDS.iter().zip(expected) {
            assert_eq!(pause_category(*kind), category, "{kind:?}");
        }
        assert_eq!(
            action_pause_category(Action::WithdrawCollateral),
            Some(C::WithdrawCollateral)
        );
        assert_eq!(
            action_pause_category(Action::ReserveWithdrawal),
            Some(C::WithdrawSupply)
        );
        assert_eq!(
            action_pause_category(Action::OpenLiquidation),
            Some(C::LiquidationStart)
        );
        for action in [Action::Absorb, Action::SettleRecoveryClaim] {
            assert_eq!(action_pause_category(action), Some(C::LiquidationComplete));
        }
        // Every category gates at least one action.
        for category in PauseCategory::ALL {
            assert!(Action::ALL
                .iter()
                .any(|a| action_pause_category(*a) == Some(category)));
        }
    }

    #[test]
    fn rescue_actions_have_no_category() {
        let rescue: Vec<Action> = Action::ALL.into_iter().filter(|a| a.is_rescue()).collect();
        assert_eq!(rescue.len(), 7);
        for action in rescue {
            assert_eq!(action_pause_category(action), None);
            assert_eq!(action.class(), Some(ActionClass::Rescue));
        }
        assert_eq!(Action::Probe.class(), None);
        assert_eq!(Action::Borrow.class(), Some(ActionClass::RiskIncreasing));
        assert_eq!(
            Action::WithdrawCollateral.class(),
            Some(ActionClass::RiskIncreasing)
        );
        assert_eq!(Action::Unshield.class(), Some(ActionClass::Withdrawal));
        assert_eq!(Action::Absorb.class(), Some(ActionClass::Settlement));
    }

    #[test]
    fn rescue_bits_cannot_be_set_and_never_block() {
        let mut pauses = PauseState::default();
        for bits in [
            pause_flags::REPAY,
            pause_flags::ADD_COLLATERAL,
            pause_flags::ALL,
        ] {
            assert_eq!(
                apply_durable_pause(&mut pauses, bits, true),
                Err(MarketError::PauseNotAllowed)
            );
            assert_eq!(
                apply_guardian_pause(&mut pauses, bits, 10, 100),
                Err(MarketError::PauseNotAllowed)
            );
        }
        for bits in [0, 1 << 10, 1 << 15] {
            assert_eq!(
                apply_durable_pause(&mut pauses, bits, true),
                Err(MarketError::InvalidPauseFlags)
            );
        }
        assert_eq!(pauses, PauseState::default());
        // Even if the bits were written, they would not block.
        let forced = PauseState {
            flags: u16::MAX,
            guardian_flags: u16::MAX,
            guardian_expiry_slot: u64::MAX,
        };
        assert_eq!(effective_pause_flags(&forced, 0), pause_flags::PAUSABLE);
        assert_eq!(ensure_action_allowed(&forced, Action::Repay, 0), Ok(()));
        assert_eq!(
            ensure_action_allowed(&forced, Action::Borrow, 0),
            Err(MarketError::ActionPaused)
        );
    }

    #[test]
    fn guardian_pause_expires_and_cannot_be_extended_by_the_guardian() {
        let m = market();
        let mut p = PauseState::default();
        assert_eq!(
            guardian_pause(&m, &mut p, &RISK, pause_flags::BORROW, 10, 100),
            Err(MarketError::NotGuardianAuthority)
        );
        for bad in [0, GUARDIAN_MAX_PAUSE_SLOTS + 1, u64::MAX] {
            assert_eq!(
                guardian_pause(&m, &mut p, &GUARDIAN, pause_flags::BORROW, bad, 100),
                Err(MarketError::InvalidPauseDuration)
            );
        }
        guardian_pause(&m, &mut p, &GUARDIAN, pause_flags::BORROW, 10, 100).unwrap();
        assert_eq!(p.guardian_expiry_slot, 110);
        assert!(is_paused(&p, PauseCategory::Borrow, 109));
        assert!(!is_paused(&p, PauseCategory::Supply, 109));
        // A second pause while active adds bits but keeps the expiry.
        guardian_pause(
            &m,
            &mut p,
            &GUARDIAN,
            pause_flags::SUPPLY,
            GUARDIAN_MAX_PAUSE_SLOTS,
            105,
        )
        .unwrap();
        assert_eq!(p.guardian_expiry_slot, 110);
        assert!(is_paused(&p, PauseCategory::Supply, 109));
        // Expired at 110.
        assert!(!is_paused(&p, PauseCategory::Borrow, 110));
        assert_eq!(ensure_not_paused(&p, PauseCategory::Borrow, 110), Ok(()));
        // Cooldown before the guardian may pause again.
        let cooldown_end = 110 + GUARDIAN_PAUSE_COOLDOWN_SLOTS;
        assert_eq!(
            guardian_pause(
                &m,
                &mut p,
                &GUARDIAN,
                pause_flags::BORROW,
                10,
                cooldown_end - 1
            ),
            Err(MarketError::GuardianPauseCooldown)
        );
        guardian_pause(
            &m,
            &mut p,
            &GUARDIAN,
            pause_flags::UNSHIELD,
            10,
            cooldown_end,
        )
        .unwrap();
        // A new pause replaces the stale bits.
        assert_eq!(p.guardian_flags, pause_flags::UNSHIELD);
        assert_eq!(p.guardian_expiry_slot, cooldown_end + 10);
    }

    #[test]
    fn governance_renews_clears_and_sets_durable_pauses() {
        let m = market();
        let mut p = PauseState::default();
        assert_eq!(
            renew_guardian_pause(&m, &mut p, &RISK, 500, 100),
            Err(MarketError::NoActiveGuardianPause)
        );
        assert_eq!(
            clear_guardian_pause(&m, &mut p, &RISK),
            Err(MarketError::NoActiveGuardianPause)
        );
        guardian_pause(&m, &mut p, &GUARDIAN, pause_flags::WITHDRAWAL, 10, 100).unwrap();
        for signer in [GUARDIAN, key(1), key(3), key(5)] {
            assert_eq!(
                renew_guardian_pause(&m, &mut p, &signer, 500, 101),
                Err(MarketError::NotRiskAuthority)
            );
            assert_eq!(
                clear_guardian_pause(&m, &mut p, &signer),
                Err(MarketError::NotRiskAuthority)
            );
            assert_eq!(
                set_pause(&m, &mut p, &signer, pause_flags::BORROW, true),
                Err(MarketError::NotRiskAuthority)
            );
        }
        assert_eq!(
            renew_guardian_pause(&m, &mut p, &RISK, 110, 101),
            Err(MarketError::InvalidPauseExpiry)
        );
        renew_guardian_pause(&m, &mut p, &RISK, 500, 101).unwrap();
        assert!(is_paused(&p, PauseCategory::Unshield, 499));
        clear_guardian_pause(&m, &mut p, &RISK).unwrap();
        assert_eq!(p, PauseState::default());
        // No cooldown after a governance clear.
        guardian_pause(&m, &mut p, &GUARDIAN, pause_flags::SHIELD, 10, 102).unwrap();
        clear_guardian_pause(&m, &mut p, &RISK).unwrap();

        set_pause(&m, &mut p, &RISK, pause_flags::SETTLEMENT, true).unwrap();
        assert!(is_paused(&p, PauseCategory::LiquidationStart, u64::MAX));
        set_pause(&m, &mut p, &RISK, pause_flags::LIQUIDATION_START, false).unwrap();
        assert!(!is_paused(&p, PauseCategory::LiquidationStart, 0));
        assert!(is_paused(&p, PauseCategory::LiquidationComplete, 0));
        set_pause(&m, &mut p, &RISK, pause_flags::PAUSABLE, false).unwrap();
        assert_eq!(p, PauseState::default());
    }

    #[test]
    fn an_inactive_market_cannot_pause() {
        let m = Market {
            status: MarketStatus::Uninitialized,
            ..market()
        };
        let mut p = PauseState::default();
        assert_eq!(
            guardian_pause(&m, &mut p, &GUARDIAN, pause_flags::BORROW, 1, 1),
            Err(MarketError::MarketNotActive)
        );
        assert_eq!(
            set_pause(&m, &mut p, &RISK, pause_flags::BORROW, true),
            Err(MarketError::MarketNotActive)
        );
    }

    // ---- Property tests (run via `cargo test -- prop_`) -------------------

    /// Deterministic linear-congruential sampler (no external crate).
    struct Lcg(u64);
    impl Lcg {
        fn sample(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 11
        }

        fn pick<T: Copy>(&mut self, items: &[T]) -> T {
            items[(self.sample() % items.len() as u64) as usize]
        }

        fn pauses(&mut self) -> PauseState {
            PauseState {
                flags: self.sample() as u16,
                guardian_flags: self.sample() as u16,
                guardian_expiry_slot: self.sample() % 2_000,
            }
        }
    }

    /// **Validates: Requirements 4.9** - a guardian pause at or past its
    /// expiry never blocks: with no durable bits, every action is allowed.
    #[test]
    fn prop_expired_guardian_pause_never_blocks() {
        let mut rng = Lcg(0x5EED_4301);
        for _ in 0..5_000 {
            let mut p = rng.pauses();
            p.flags = 0;
            let now = p.guardian_expiry_slot + rng.sample() % 1_000;
            assert_eq!(effective_pause_flags(&p, now), 0);
            for action in Action::ALL {
                assert_eq!(ensure_action_allowed(&p, action, now), Ok(()));
            }
        }
    }

    /// **Validates: Requirements 4.8** - repay, collateral addition, callback
    /// completion, and recovery actions are allowed under every pause state,
    /// including states no instruction can produce.
    #[test]
    fn prop_always_available_actions_never_blocked() {
        let mut rng = Lcg(0x5EED_4302);
        let rescue: Vec<Action> = Action::ALL.into_iter().filter(|a| a.is_rescue()).collect();
        for _ in 0..5_000 {
            let p = rng.pauses();
            let now = rng.sample() % 3_000;
            for action in &rescue {
                assert_eq!(ensure_action_allowed(&p, *action, now), Ok(()));
            }
        }
    }

    /// **Validates: Requirements 4.5, 4.9** - a signer that does not hold the
    /// required role (guardian for `guardian_pause`, risk for the rest) is
    /// rejected with that role's error and leaves the pause state unchanged.
    #[test]
    fn prop_unauthorized_pause_and_unpause_rejected() {
        let mut rng = Lcg(0x5EED_4303);
        let people: Vec<Pubkey> = [1, 3, 4, 5, 6, 9].map(key).to_vec();
        let m = market();
        let mut rejected = 0;
        for _ in 0..5_000 {
            let mut p = rng.pauses();
            let before = p;
            let signer = rng.pick(&people);
            let now = rng.sample() % 3_000;
            let flags = rng.pick(&PauseCategory::ALL).flag();
            let (role_holder, error, result) = match rng.sample() % 4 {
                0 => (
                    GUARDIAN,
                    MarketError::NotGuardianAuthority,
                    guardian_pause(&m, &mut p, &signer, flags, 10, now),
                ),
                1 => (
                    RISK,
                    MarketError::NotRiskAuthority,
                    set_pause(&m, &mut p, &signer, flags, rng.sample().is_multiple_of(2)),
                ),
                2 => (
                    RISK,
                    MarketError::NotRiskAuthority,
                    renew_guardian_pause(&m, &mut p, &signer, now + 5_000, now),
                ),
                _ => (
                    RISK,
                    MarketError::NotRiskAuthority,
                    clear_guardian_pause(&m, &mut p, &signer),
                ),
            };
            if signer == role_holder {
                continue;
            }
            assert_eq!(result, Err(error));
            assert_eq!(p, before);
            rejected += 1;
        }
        assert!(rejected > 3_000);
    }

    /// **Validates: Requirements 4.9** - after any accepted guardian pause, the
    /// guardian expiry is never more than `GUARDIAN_MAX_PAUSE_SLOTS` past the
    /// slot of the call, and a guardian call never moves an expiry later
    /// while the pause is active.
    #[test]
    fn prop_guardian_cannot_exceed_max_duration() {
        let mut rng = Lcg(0x5EED_4304);
        let m = market();
        for _ in 0..500 {
            let mut p = PauseState::default();
            let mut now = rng.sample() % 1_000;
            for _ in 0..20 {
                now += rng.sample() % (GUARDIAN_MAX_PAUSE_SLOTS / 4);
                let flags = rng.pick(&PauseCategory::ALL).flag();
                let duration = rng.sample() % (GUARDIAN_MAX_PAUSE_SLOTS * 2);
                let before = p;
                let result = guardian_pause(&m, &mut p, &GUARDIAN, flags, duration, now);
                if duration == 0 || duration > GUARDIAN_MAX_PAUSE_SLOTS {
                    assert_eq!(result, Err(MarketError::InvalidPauseDuration));
                }
                if result.is_err() {
                    assert_eq!(p, before);
                    continue;
                }
                assert!(p.guardian_expiry_slot <= now + GUARDIAN_MAX_PAUSE_SLOTS);
                assert!(p.guardian_expiry_slot > now);
                if guardian_pause_active(&before, now) {
                    assert_eq!(p.guardian_expiry_slot, before.guardian_expiry_slot);
                }
            }
        }
    }

    /// **Validates: Requirements 4.7** - pausing one category (durably or by
    /// an active guardian pause) blocks exactly the actions mapped to it and
    /// no other action.
    #[test]
    fn prop_category_blocks_exactly_its_operations() {
        let mut rng = Lcg(0x5EED_4305);
        let m = market();
        for _ in 0..2_000 {
            let category = rng.pick(&PauseCategory::ALL);
            let now = 100 + rng.sample() % 1_000;
            let mut p = PauseState::default();
            if rng.sample().is_multiple_of(2) {
                set_pause(&m, &mut p, &RISK, category.flag(), true).unwrap();
            } else {
                let duration = 1 + rng.sample() % GUARDIAN_MAX_PAUSE_SLOTS;
                guardian_pause(&m, &mut p, &GUARDIAN, category.flag(), duration, now).unwrap();
            }
            for action in Action::ALL {
                let blocked = ensure_action_allowed(&p, action, now).is_err();
                assert_eq!(
                    blocked,
                    action_pause_category(action) == Some(category),
                    "{category:?} {action:?}"
                );
            }
            for kind in KINDS {
                let blocked = pause_category(kind)
                    .map(|c| ensure_not_paused(&p, c, now).is_err())
                    .unwrap_or(false);
                assert_eq!(blocked, pause_category(kind) == Some(category));
            }
        }
    }
}
