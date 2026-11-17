//! Launch caps and public lifecycle counts (Task 4.5, Requirements 1.3, 4.10,
//! 4.11, 6.6, 7.6, and 8.14).
//!
//! # USDC caps and where each check runs
//!
//! The three USDC caps live in `AssetState.caps` and are public. The values
//! they bound are encrypted, so the comparisons run inside the Arcis circuit;
//! the functions below are the exact plaintext reference the circuits (Tasks
//! 7 to 9) must match, and the program-side rule for the result.
//!
//! | Cap                      | Compared value (encrypted, in-circuit)                | Checked by          |
//! | ------------------------ | ----------------------------------------------------- | ------------------- |
//! | `supply_cap_usdc`        | supplier equity after the proposed supply             | Supply circuit      |
//! | `total_debt_cap_usdc`    | total debt after the borrow, at the projected risk index | Borrow circuit   |
//! | `position_debt_cap_usdc` | Position debt after the borrow, at the projected risk index | Borrow circuit |
//!
//! A value passes when `value <= cap` ([`CapCheck::Within`]). The queue
//! instruction reads the caps and the projected risk Borrow Index from the
//! Asset State and the Intent, passes them to the circuit as plaintext
//! arguments, and commits them in `inputs_commitment`. They are also covered
//! by the captured Asset State generation: every cap change bumps it, so a
//! callback computed against old caps resolves `Stale`.
//!
//! # Passive interest and new risk
//!
//! Interest accrual is not an action and never consults a cap, so accrual can
//! never fail because of one ([`cap_scope`] gives no check to accrual, repay,
//! liquidation, Absorb, withdrawals, callbacks, or recovery). Passive interest
//! may therefore leave the market above a cap. Every later risk-increasing
//! action then compares `before + amount > cap`, which holds for any amount,
//! so it resolves to zero ([`NewRiskOutcome::Zero`]): the circuit approves no
//! value, the callback classifies `Zero`, the Lock is released, and nothing
//! errors or strands a Lock.
//!
//! # Public counts
//!
//! The Market holds `position_count`, `debt_position_count`, their caps, and
//! `debt_position_reservations`; each Position holds a public `has_debt`.
//!
//! * [`on_position_initialized`] runs when a Position PDA is created
//!   (synchronously, before any Lock exists). It increments `position_count`
//!   and fails the whole transaction at the cap, which strands nothing.
//! * Borrow is the only action that can move debt from zero to nonzero. Its
//!   queue instruction calls [`reserve_debt_slot`] while the Position has no
//!   debt. With no slot left it gets [`DebtSlot::Unavailable`] and resolves to
//!   zero without queueing. Otherwise one slot is reserved, so concurrent
//!   borrows cannot jointly pass the cap:
//!   `debt_position_count + debt_position_reservations <= cap` holds after
//!   every reservation.
//! * When the Intent becomes terminal by any path (`Applied`, `Zero`,
//!   `Stale`, `Expired`, `Failed`, or abandonment) the program calls
//!   [`settle_debt_intent`] exactly once. It releases the slot and, only for
//!   an `Applied` decision, applies the zero/nonzero transition the verified
//!   output revealed ([`DebtTransition`] can only be built from
//!   `CallbackDecision::Applied`). The transition must agree with the public
//!   `has_debt` and the action's direction (Borrow raises, Repay, Liquidation,
//!   and Absorb clear); a contradiction is rejected and changes nothing.
//!
//! `has_debt` cannot change while the Intent is pending because the Position
//! is Locked by it, so "the Position had no debt" is the same at queue and at
//! settlement and identifies exactly the Intents that hold a slot.
//!
//! # Cap changes
//!
//! Raising any cap adds room for new risk, which Requirement 4.5 treats as an
//! adverse risk change, so increases go only through the Task 4.4 timelock
//! (`Caps` and `CountCaps` groups). Lowering caps only blocks new risk and
//! never touches existing positions, so [`decrease_caps`] applies a decrease
//! immediately, signed by the risk authority. The guardian is not allowed: its
//! emergency tool is the expiring pause, and a cap decrease does not expire.
//! A decrease below the current count or debt only blocks new risk. Every cap
//! change bumps the Asset State generation once (Requirement 4.6), and a
//! pending proposal for the same group then fails with
//! `ConfigProposalBaseChanged`, so a decrease cannot be undone silently.
//!
//! Bounds ([`check_caps_bounds`], [`check_count_caps_bounds`]):
//! `position_debt <= total_debt <= supply <= MAX_USDC_CAP` and
//! `debt_position_count_cap <= position_count_cap <= MAX_POSITION_COUNT_CAP`.
//! One change may raise a cap to at most `max(2 × old, old + floor)`.

use anchor_lang::prelude::*;
use core::result::Result;

use super::config::{validate_change, write_value, ConfigExecution, ConfigValue, GovernedConfig};
use super::pause::Action;
use super::roles::{authorize, Role};
use super::state::{AssetState, Caps, CountCaps, Market};
use super::validation::MarketError;
use crate::operation::CallbackDecision;

/// One USDC in base units (6 decimals).
pub const USDC: u64 = 1_000_000;
/// Largest allowed USDC cap: 1,000,000,000 USDC.
pub const MAX_USDC_CAP: u64 = 1_000_000_000 * USDC;
/// One change may always raise a USDC cap by this much: 100,000 USDC.
pub const USDC_CAP_RAISE_FLOOR: u64 = 100_000 * USDC;
/// Largest allowed `position_count_cap`.
pub const MAX_POSITION_COUNT_CAP: u64 = 1_000_000;
/// One change may always raise a count cap by this much.
pub const COUNT_CAP_RAISE_FLOOR: u64 = 100;

// ---- Cap bounds and movement --------------------------------------------------

/// Whether moving a cap from `old` to `new` stays within the raise limit.
/// Decreases are always within it.
pub fn raise_allowed(old: u64, new: u64, floor: u64) -> bool {
    new <= old || new <= old.saturating_mul(2).max(old.saturating_add(floor))
}

/// Absolute ceiling and ordering of the USDC caps.
pub fn check_caps_bounds(caps: &Caps) -> Result<(), MarketError> {
    if caps.supply_cap_usdc > MAX_USDC_CAP
        || caps.total_debt_cap_usdc > caps.supply_cap_usdc
        || caps.position_debt_cap_usdc > caps.total_debt_cap_usdc
    {
        return Err(MarketError::ConfigOutOfBounds);
    }
    Ok(())
}

/// Absolute ceiling and ordering of the count caps.
pub fn check_count_caps_bounds(caps: &CountCaps) -> Result<(), MarketError> {
    if caps.position_count_cap > MAX_POSITION_COUNT_CAP
        || caps.debt_position_count_cap > caps.position_count_cap
    {
        return Err(MarketError::ConfigOutOfBounds);
    }
    Ok(())
}

/// Bounds of `new` and the raise limit of every USDC cap.
pub fn check_caps_change(old: &Caps, new: &Caps) -> Result<(), MarketError> {
    check_caps_bounds(new)?;
    let pairs = [
        (old.supply_cap_usdc, new.supply_cap_usdc),
        (old.total_debt_cap_usdc, new.total_debt_cap_usdc),
        (old.position_debt_cap_usdc, new.position_debt_cap_usdc),
    ];
    if pairs
        .iter()
        .any(|(old, new)| !raise_allowed(*old, *new, USDC_CAP_RAISE_FLOOR))
    {
        return Err(MarketError::ConfigMovementTooLarge);
    }
    Ok(())
}

/// Bounds of `new` and the raise limit of both count caps.
pub fn check_count_caps_change(old: &CountCaps, new: &CountCaps) -> Result<(), MarketError> {
    check_count_caps_bounds(new)?;
    let pairs = [
        (old.position_count_cap, new.position_count_cap),
        (old.debt_position_count_cap, new.debt_position_count_cap),
    ];
    if pairs
        .iter()
        .any(|(old, new)| !raise_allowed(*old, *new, COUNT_CAP_RAISE_FLOOR))
    {
        return Err(MarketError::ConfigMovementTooLarge);
    }
    Ok(())
}

/// Whether `change` is a cap group whose every field is at most its current
/// value. Every other group, and any raise, is not a decrease.
pub fn is_cap_decrease(current: &GovernedConfig, change: &ConfigValue) -> bool {
    match change {
        ConfigValue::Caps { caps } => {
            let old = &current.values.caps;
            caps.supply_cap_usdc <= old.supply_cap_usdc
                && caps.total_debt_cap_usdc <= old.total_debt_cap_usdc
                && caps.position_debt_cap_usdc <= old.position_debt_cap_usdc
        }
        ConfigValue::CountCaps { caps } => {
            let old = &current.count_caps;
            caps.position_count_cap <= old.position_count_cap
                && caps.debt_position_count_cap <= old.debt_position_count_cap
        }
        _ => false,
    }
}

/// The risk authority lowers caps immediately. `change` must be a `Caps` or
/// `CountCaps` value with no field above its current value and at least one
/// below it, and must satisfy the cap bounds. Writes the group and bumps the
/// Asset State generation once. Nothing changes on error.
pub fn decrease_caps(
    market: &mut Market,
    asset: &mut AssetState,
    signer: &Pubkey,
    change: ConfigValue,
) -> Result<ConfigExecution, MarketError> {
    authorize(market, Role::Risk, signer)?;
    let current = GovernedConfig::of(market, asset);
    if !is_cap_decrease(&current, &change) {
        return Err(MarketError::CapChangeNotDecrease);
    }
    validate_change(&current, change)?;
    let old_generation = asset.generation;
    let new_generation = old_generation
        .checked_add(1)
        .ok_or(MarketError::ConfigArithmeticOverflow)?;
    let old_value = current.get(change.group());
    write_value(market, asset, change);
    asset.generation = new_generation;
    Ok(ConfigExecution {
        old_value,
        new_value: change,
        old_generation,
        new_generation,
    })
}

// ---- Cap checks (reference for the circuits) ---------------------------------

/// Result of one cap comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapCheck {
    /// `value <= cap`.
    Within,
    /// `value > cap`: the risk-increasing action must approve zero.
    Exceeded,
}

impl CapCheck {
    /// Compares `value` with `cap`.
    pub fn of(value: u128, cap: u64) -> Self {
        if value <= u128::from(cap) {
            CapCheck::Within
        } else {
            CapCheck::Exceeded
        }
    }
}

/// Supply cap: encrypted supplier equity after the proposed supply
/// (Requirement 4.10). Runs in the Supply circuit.
pub fn check_supply_cap(caps: &Caps, supplier_equity_after_usdc: u128) -> CapCheck {
    CapCheck::of(supplier_equity_after_usdc, caps.supply_cap_usdc)
}

/// Total debt cap: total debt after the borrow, at the projected risk index.
/// Runs in the Borrow circuit.
pub fn check_total_debt_cap(caps: &Caps, total_debt_at_risk_after_usdc: u128) -> CapCheck {
    CapCheck::of(total_debt_at_risk_after_usdc, caps.total_debt_cap_usdc)
}

/// Per-Position debt cap: the Position's debt after the borrow, at the
/// projected risk index. Runs in the Borrow circuit.
pub fn check_position_debt_cap(caps: &Caps, position_debt_at_risk_after_usdc: u128) -> CapCheck {
    CapCheck::of(
        position_debt_at_risk_after_usdc,
        caps.position_debt_cap_usdc,
    )
}

/// Whether a risk-increasing action may move value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewRiskOutcome {
    /// Every cap check passed.
    Proceed,
    /// A cap was exceeded: approve zero (no value moves, no error).
    Zero,
}

/// `Proceed` exactly when every check is `Within`.
pub fn new_risk_outcome(checks: &[CapCheck]) -> NewRiskOutcome {
    if checks.iter().all(|check| *check == CapCheck::Within) {
        NewRiskOutcome::Proceed
    } else {
        NewRiskOutcome::Zero
    }
}

/// Reference for the Supply circuit's cap step. `equity_before_usdc` already
/// includes passive interest; an overflowing sum resolves to zero.
pub fn supply_cap_outcome(caps: &Caps, equity_before_usdc: u128, amount: u64) -> NewRiskOutcome {
    match equity_before_usdc.checked_add(u128::from(amount)) {
        Some(after) => new_risk_outcome(&[check_supply_cap(caps, after)]),
        None => NewRiskOutcome::Zero,
    }
}

/// Reference for the Borrow circuit's cap steps. Both debts are projected at
/// the risk index through Decision Expiry; an overflowing sum resolves to
/// zero.
pub fn borrow_cap_outcome(
    caps: &Caps,
    total_debt_at_risk_before_usdc: u128,
    position_debt_at_risk_before_usdc: u128,
    amount: u64,
) -> NewRiskOutcome {
    let amount = u128::from(amount);
    match (
        total_debt_at_risk_before_usdc.checked_add(amount),
        position_debt_at_risk_before_usdc.checked_add(amount),
    ) {
        (Some(total), Some(position)) => new_risk_outcome(&[
            check_total_debt_cap(caps, total),
            check_position_debt_cap(caps, position),
        ]),
        _ => NewRiskOutcome::Zero,
    }
}

/// Which caps an action checks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CapScope {
    /// `supply_cap_usdc` (in-circuit).
    pub supply: bool,
    /// `total_debt_cap_usdc` (in-circuit).
    pub total_debt: bool,
    /// `position_debt_cap_usdc` (in-circuit).
    pub position_debt: bool,
    /// `position_count_cap` (program, at Position creation).
    pub position_count: bool,
    /// `debt_position_count_cap` (program, debt-slot reservation).
    pub debt_position_count: bool,
}

impl CapScope {
    /// Whether any cap applies.
    pub fn any(self) -> bool {
        self != CapScope::default()
    }
}

/// The caps `action` checks. Only Supply, Borrow, and Position initialization
/// add capped risk; nothing else (including interest accrual, which is not an
/// action) checks a cap.
pub const fn cap_scope(action: Action) -> CapScope {
    match action {
        Action::Supply => CapScope {
            supply: true,
            total_debt: false,
            position_debt: false,
            position_count: false,
            debt_position_count: false,
        },
        Action::Borrow => CapScope {
            supply: false,
            total_debt: true,
            position_debt: true,
            position_count: false,
            debt_position_count: true,
        },
        Action::PositionInit => CapScope {
            supply: false,
            total_debt: false,
            position_debt: false,
            position_count: true,
            debt_position_count: false,
        },
        _ => CapScope {
            supply: false,
            total_debt: false,
            position_debt: false,
            position_count: false,
            debt_position_count: false,
        },
    }
}

/// Whether `action` may move a Position's debt from zero to nonzero.
pub const fn creates_debt(action: Action) -> bool {
    matches!(action, Action::Borrow)
}

/// Whether `action` may move a Position's debt from nonzero to zero.
pub const fn clears_debt(action: Action) -> bool {
    matches!(action, Action::Repay | Action::Liquidation | Action::Absorb)
}

// ---- Position count ---------------------------------------------------------------

/// A public count change, for the events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountChange {
    /// Count before.
    pub old_count: u64,
    /// Count after.
    pub new_count: u64,
    /// The cap in force.
    pub cap: u64,
}

/// A new Position PDA was created: increments `position_count`, refusing to
/// pass `position_count_cap`. Nothing changes on error.
pub fn on_position_initialized(market: &mut Market) -> Result<CountChange, MarketError> {
    let old_count = market.position_count;
    let new_count = old_count
        .checked_add(1)
        .ok_or(MarketError::LifecycleCountOutOfRange)?;
    if new_count > market.position_count_cap {
        return Err(MarketError::PositionCountCapReached);
    }
    market.position_count = new_count;
    Ok(CountChange {
        old_count,
        new_count,
        cap: market.position_count_cap,
    })
}

/// A Position PDA was closed (Task 5.1): decrements `position_count`. Every
/// counted Position was counted once by [`on_position_initialized`], so the
/// count never underflows; an underflow is rejected. Nothing changes on error.
pub fn on_position_closed(market: &mut Market) -> Result<CountChange, MarketError> {
    let old_count = market.position_count;
    let new_count = old_count
        .checked_sub(1)
        .ok_or(MarketError::LifecycleCountOutOfRange)?;
    market.position_count = new_count;
    Ok(CountChange {
        old_count,
        new_count,
        cap: market.position_count_cap,
    })
}

// ---- Debt-Position count ----------------------------------------------------------

/// Result of [`reserve_debt_slot`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebtSlot {
    /// The action cannot create a debt Position (wrong action, or the
    /// Position already has debt); nothing reserved.
    NotNeeded,
    /// One slot reserved; released by [`settle_debt_intent`].
    Reserved,
    /// No slot left: the queue resolves the borrow to zero without queueing.
    Unavailable,
}

/// Whether an Intent of `action` on a Position with `has_debt` holds a debt
/// slot. Valid from queue to settlement because the Position is Locked.
pub const fn holds_debt_slot(action: Action, has_debt: bool) -> bool {
    cap_scope(action).debt_position_count && !has_debt
}

/// Queue-time debt-Position cap check. Reserves a slot for a borrow from a
/// Position without debt when `count + reservations < cap`. Nothing changes
/// unless `Reserved` is returned.
pub fn reserve_debt_slot(
    market: &mut Market,
    action: Action,
    has_debt: bool,
) -> Result<DebtSlot, MarketError> {
    if !holds_debt_slot(action, has_debt) {
        return Ok(DebtSlot::NotNeeded);
    }
    let occupied = market
        .debt_position_count
        .checked_add(market.debt_position_reservations)
        .ok_or(MarketError::LifecycleCountOutOfRange)?;
    if occupied >= market.debt_position_count_cap {
        return Ok(DebtSlot::Unavailable);
    }
    market.debt_position_reservations = market
        .debt_position_reservations
        .checked_add(1)
        .ok_or(MarketError::LifecycleCountOutOfRange)?;
    Ok(DebtSlot::Reserved)
}

/// The zero/nonzero debt transition revealed by a verified computation output.
/// It can only be built from an `Applied` callback decision, so counts never
/// move on client input or on a non-applied outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DebtTransition {
    was_nonzero: bool,
    is_nonzero: bool,
}

impl DebtTransition {
    /// The revealed booleans of an authenticated output. `None` unless the
    /// decision is `Applied`.
    pub fn from_callback(
        decision: CallbackDecision,
        was_nonzero: bool,
        is_nonzero: bool,
    ) -> Option<Self> {
        match decision {
            CallbackDecision::Applied => Some(Self {
                was_nonzero,
                is_nonzero,
            }),
            _ => None,
        }
    }

    /// Whether debt was nonzero before the operation.
    pub fn was_nonzero(&self) -> bool {
        self.was_nonzero
    }

    /// Whether debt is nonzero after the operation.
    pub fn is_nonzero(&self) -> bool {
        self.is_nonzero
    }
}

/// What [`settle_debt_intent`] did, for the events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DebtSettlement {
    /// `has_debt` before.
    pub has_debt_before: bool,
    /// `has_debt` after.
    pub has_debt_after: bool,
    /// `debt_position_count` before.
    pub old_count: u64,
    /// `debt_position_count` after.
    pub new_count: u64,
    /// Whether a reserved slot was released.
    pub slot_released: bool,
    /// `debt_position_count_cap` in force.
    pub cap: u64,
}

impl DebtSettlement {
    /// Whether the public count or flag changed (emit `DebtPositionCountChanged`).
    pub fn changed(&self) -> bool {
        self.old_count != self.new_count
    }
}

/// Settles a terminal Intent of `action` for the Position whose public flag is
/// `has_debt`. Call exactly once per terminal Intent, by every terminal path.
/// Releases the slot the Intent holds, then applies `transition` (present only
/// for `Applied`). Nothing changes on error.
pub fn settle_debt_intent(
    market: &mut Market,
    has_debt: &mut bool,
    action: Action,
    transition: Option<DebtTransition>,
) -> Result<DebtSettlement, MarketError> {
    let before = *has_debt;
    let old_count = market.debt_position_count;
    let slot_released = holds_debt_slot(action, before);
    let reservations = if slot_released {
        market
            .debt_position_reservations
            .checked_sub(1)
            .ok_or(MarketError::LifecycleCountOutOfRange)?
    } else {
        market.debt_position_reservations
    };
    let (after, new_count) = match transition {
        None => (before, old_count),
        Some(t) => {
            if t.was_nonzero != before {
                return Err(MarketError::DebtTransitionMismatch);
            }
            match (t.was_nonzero, t.is_nonzero) {
                (false, true) => {
                    if !creates_debt(action) {
                        return Err(MarketError::DebtTransitionMismatch);
                    }
                    let count = old_count
                        .checked_add(1)
                        .ok_or(MarketError::LifecycleCountOutOfRange)?;
                    (true, count)
                }
                (true, false) => {
                    if !clears_debt(action) {
                        return Err(MarketError::DebtTransitionMismatch);
                    }
                    let count = old_count
                        .checked_sub(1)
                        .ok_or(MarketError::LifecycleCountOutOfRange)?;
                    (false, count)
                }
                _ => (before, old_count),
            }
        }
    };
    market.debt_position_reservations = reservations;
    market.debt_position_count = new_count;
    *has_debt = after;
    Ok(DebtSettlement {
        has_debt_before: before,
        has_debt_after: after,
        old_count,
        new_count,
        slot_released,
        cap: market.debt_position_count_cap,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::config::{execute, propose, ProposalStatus};
    use crate::market::state::{MarketStatus, PauseState, RateParams, RiskParams, TimelockState};
    use crate::operation::StaleReason;

    const T0: i64 = 1_700_000_000;
    const DELAY: i64 = TimelockState::LAUNCH_DEFAULT_DELAY_SECONDS as i64;
    const RISK: Pubkey = Pubkey::new_from_array([4; 32]);
    const GUARDIAN: Pubkey = Pubkey::new_from_array([6; 32]);
    const ADMIN: Pubkey = Pubkey::new_from_array([1; 32]);
    const STRANGER: Pubkey = Pubkey::new_from_array([9; 32]);

    fn key(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    fn caps() -> Caps {
        Caps {
            supply_cap_usdc: 1_000_000 * USDC,
            total_debt_cap_usdc: 500_000 * USDC,
            position_debt_cap_usdc: 10_000 * USDC,
        }
    }

    fn market() -> Market {
        Market {
            layout_version: 1,
            status: MarketStatus::Active,
            bump: 0xFE,
            vault_authority_bump: 0xFD,
            admin: ADMIN,
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
            debt_position_count_cap: 4,
            initialized_slot: 1,
            debt_position_reservations: 0,
            reserved: [0; 120],
        }
    }

    fn asset() -> AssetState {
        AssetState {
            layout_version: 1,
            bump: 0xFC,
            market: key(0xA0),
            generation: 1,
            oracle_generation: 1,
            rate_epoch_generation: 1,
            accounting_borrow_index: crate::market::WAD,
            accounting_index_timestamp: T0,
            utilization_band: 0,
            utilization_observation_slot: 0,
            risk: RiskParams::LAUNCH_DEFAULT,
            rates: RateParams::LAUNCH_DEFAULT,
            caps: caps(),
            pauses: PauseState::default(),
            timelock: TimelockState {
                delay_seconds: TimelockState::LAUNCH_DEFAULT_DELAY_SECONDS,
                next_proposal_id: 0,
            },
            reserved: [0; 128],
        }
    }

    fn applied(was: bool, is: bool) -> Option<DebtTransition> {
        DebtTransition::from_callback(CallbackDecision::Applied, was, is)
    }

    // ---- Cap checks ---------------------------------------------------------

    #[test]
    fn cap_checks_pass_at_the_cap_and_fail_above_it() {
        let c = caps();
        let supply = u128::from(c.supply_cap_usdc);
        assert_eq!(check_supply_cap(&c, supply), CapCheck::Within);
        assert_eq!(check_supply_cap(&c, supply + 1), CapCheck::Exceeded);
        let total = u128::from(c.total_debt_cap_usdc);
        assert_eq!(check_total_debt_cap(&c, total), CapCheck::Within);
        assert_eq!(check_total_debt_cap(&c, total + 1), CapCheck::Exceeded);
        let position = u128::from(c.position_debt_cap_usdc);
        assert_eq!(check_position_debt_cap(&c, position), CapCheck::Within);
        assert_eq!(
            check_position_debt_cap(&c, position + 1),
            CapCheck::Exceeded
        );
        // A zero cap allows no new risk.
        let zero = Caps::default();
        assert_eq!(supply_cap_outcome(&zero, 0, 1), NewRiskOutcome::Zero);
        assert_eq!(borrow_cap_outcome(&zero, 0, 0, 1), NewRiskOutcome::Zero);
        // Values above u64 are compared exactly.
        assert_eq!(check_supply_cap(&c, u128::MAX), CapCheck::Exceeded);
    }

    #[test]
    fn borrow_needs_both_debt_caps() {
        let c = caps();
        let room = c.position_debt_cap_usdc;
        assert_eq!(borrow_cap_outcome(&c, 0, 0, room), NewRiskOutcome::Proceed);
        assert_eq!(borrow_cap_outcome(&c, 0, 0, room + 1), NewRiskOutcome::Zero);
        let total = u128::from(c.total_debt_cap_usdc);
        assert_eq!(borrow_cap_outcome(&c, total, 0, 1), NewRiskOutcome::Zero);
        assert_eq!(
            borrow_cap_outcome(&c, u128::MAX, 0, 1),
            NewRiskOutcome::Zero,
            "overflow resolves to zero"
        );
    }

    #[test]
    fn passive_interest_above_a_cap_makes_new_risk_zero() {
        let c = caps();
        // Interest moved equity and debt above the caps; nothing failed.
        let equity = u128::from(c.supply_cap_usdc) + 7;
        let total = u128::from(c.total_debt_cap_usdc) + 7;
        let position = u128::from(c.position_debt_cap_usdc) + 7;
        assert_eq!(check_supply_cap(&c, equity), CapCheck::Exceeded);
        for amount in [0, 1, 1_000 * USDC] {
            assert_eq!(supply_cap_outcome(&c, equity, amount), NewRiskOutcome::Zero);
            assert_eq!(
                borrow_cap_outcome(&c, total, 0, amount),
                NewRiskOutcome::Zero
            );
            assert_eq!(
                borrow_cap_outcome(&c, 0, position, amount),
                NewRiskOutcome::Zero
            );
        }
        // Repay, liquidation, Absorb, withdrawals, and callbacks check no cap.
        for action in [
            Action::Repay,
            Action::Liquidation,
            Action::Absorb,
            Action::WithdrawSupply,
            Action::Callback,
            Action::AbandonIntent,
            Action::AddCollateral,
        ] {
            assert!(!cap_scope(action).any(), "{action:?}");
        }
    }

    #[test]
    fn only_supply_borrow_and_position_init_check_caps() {
        for action in Action::ALL {
            let scope = cap_scope(action);
            let expected = matches!(
                action,
                Action::Supply | Action::Borrow | Action::PositionInit
            );
            assert_eq!(scope.any(), expected, "{action:?}");
        }
        assert!(cap_scope(Action::Supply).supply);
        let borrow = cap_scope(Action::Borrow);
        assert!(borrow.total_debt && borrow.position_debt && borrow.debt_position_count);
        assert!(cap_scope(Action::PositionInit).position_count);
    }

    // ---- Position count -----------------------------------------------------

    #[test]
    fn position_count_stops_at_its_cap() {
        let mut m = market();
        for n in 1..=m.position_count_cap {
            let change = on_position_initialized(&mut m).unwrap();
            assert_eq!((change.old_count, change.new_count), (n - 1, n));
        }
        let before = m.clone();
        assert_eq!(
            on_position_initialized(&mut m),
            Err(MarketError::PositionCountCapReached)
        );
        assert_eq!(m, before);
        m.position_count = u64::MAX;
        m.position_count_cap = u64::MAX;
        assert_eq!(
            on_position_initialized(&mut m),
            Err(MarketError::LifecycleCountOutOfRange)
        );
    }

    // ---- Debt-Position count ------------------------------------------------

    #[test]
    fn borrow_from_zero_reserves_then_counts_on_applied() {
        let mut m = market();
        let mut has_debt = false;
        assert_eq!(
            reserve_debt_slot(&mut m, Action::Borrow, has_debt),
            Ok(DebtSlot::Reserved)
        );
        assert_eq!(m.debt_position_reservations, 1);
        let done = settle_debt_intent(&mut m, &mut has_debt, Action::Borrow, applied(false, true))
            .unwrap();
        assert!(done.slot_released && done.changed());
        assert!(has_debt);
        assert_eq!(
            (m.debt_position_count, m.debt_position_reservations),
            (1, 0)
        );

        // A second borrow on a Position with debt needs no slot.
        assert_eq!(
            reserve_debt_slot(&mut m, Action::Borrow, has_debt),
            Ok(DebtSlot::NotNeeded)
        );
        let done =
            settle_debt_intent(&mut m, &mut has_debt, Action::Borrow, applied(true, true)).unwrap();
        assert!(!done.slot_released && !done.changed());

        // Full repay clears it.
        let done =
            settle_debt_intent(&mut m, &mut has_debt, Action::Repay, applied(true, false)).unwrap();
        assert!(done.changed());
        assert!(!has_debt);
        assert_eq!(m.debt_position_count, 0);
    }

    #[test]
    fn non_applied_outcomes_release_the_slot_and_change_no_count() {
        for decision in [
            CallbackDecision::Zero,
            CallbackDecision::Stale(StaleReason::AssetGeneration),
            CallbackDecision::Expired,
            CallbackDecision::Failed,
            CallbackDecision::Ignore,
            CallbackDecision::LeaveQueued,
        ] {
            assert_eq!(DebtTransition::from_callback(decision, false, true), None);
            let mut m = market();
            let mut has_debt = false;
            reserve_debt_slot(&mut m, Action::Borrow, false).unwrap();
            let transition = DebtTransition::from_callback(decision, false, true);
            let done =
                settle_debt_intent(&mut m, &mut has_debt, Action::Borrow, transition).unwrap();
            assert!(done.slot_released && !done.changed());
            assert!(!has_debt);
            assert_eq!(
                (m.debt_position_count, m.debt_position_reservations),
                (0, 0)
            );
        }
    }

    #[test]
    fn debt_cap_resolves_to_zero_not_error() {
        let mut m = market();
        for _ in 0..m.debt_position_count_cap {
            assert_eq!(
                reserve_debt_slot(&mut m, Action::Borrow, false),
                Ok(DebtSlot::Reserved)
            );
        }
        let before = m.clone();
        assert_eq!(
            reserve_debt_slot(&mut m, Action::Borrow, false),
            Ok(DebtSlot::Unavailable)
        );
        assert_eq!(m, before, "an unavailable slot changes nothing");
        // Other actions and Positions with debt are unaffected.
        assert_eq!(
            reserve_debt_slot(&mut m, Action::Borrow, true),
            Ok(DebtSlot::NotNeeded)
        );
        assert_eq!(
            reserve_debt_slot(&mut m, Action::Repay, false),
            Ok(DebtSlot::NotNeeded)
        );
    }

    #[test]
    fn contradictory_transitions_are_rejected_without_change() {
        let cases = [
            // Revealed "was nonzero" disagrees with the public flag.
            (false, Action::Borrow, applied(true, true)),
            (true, Action::Repay, applied(false, false)),
            // Wrong direction for the action.
            (false, Action::Repay, applied(false, true)),
            (true, Action::Borrow, applied(true, false)),
            (false, Action::Supply, applied(false, true)),
        ];
        for (has_debt, action, transition) in cases {
            let mut m = market();
            m.debt_position_count = 1;
            m.debt_position_reservations = 1;
            let before = m.clone();
            let mut flag = has_debt;
            assert_eq!(
                settle_debt_intent(&mut m, &mut flag, action, transition),
                Err(MarketError::DebtTransitionMismatch)
            );
            assert_eq!((m, flag), (before, has_debt));
        }
    }

    #[test]
    fn count_underflow_and_overflow_are_rejected() {
        let mut m = market();
        let mut has_debt = false;
        // Settling a borrow that never reserved underflows the reservations.
        assert_eq!(
            settle_debt_intent(&mut m, &mut has_debt, Action::Borrow, None),
            Err(MarketError::LifecycleCountOutOfRange)
        );
        let mut has_debt = true;
        assert_eq!(
            settle_debt_intent(&mut m, &mut has_debt, Action::Repay, applied(true, false)),
            Err(MarketError::LifecycleCountOutOfRange)
        );
        assert!(has_debt);
        m.debt_position_count = u64::MAX;
        m.debt_position_reservations = 1;
        m.debt_position_count_cap = u64::MAX;
        assert_eq!(
            reserve_debt_slot(&mut m, Action::Borrow, false),
            Err(MarketError::LifecycleCountOutOfRange)
        );
        let mut has_debt = false;
        assert_eq!(
            settle_debt_intent(&mut m, &mut has_debt, Action::Borrow, applied(false, true)),
            Err(MarketError::LifecycleCountOutOfRange)
        );
        assert_eq!(m.debt_position_reservations, 1);
    }

    // ---- Cap changes --------------------------------------------------------

    #[test]
    fn cap_bounds_and_raise_limit() {
        assert_eq!(check_caps_bounds(&caps()), Ok(()));
        assert_eq!(check_caps_bounds(&Caps::default()), Ok(()));
        let bad = [
            Caps {
                supply_cap_usdc: MAX_USDC_CAP + 1,
                ..caps()
            },
            Caps {
                total_debt_cap_usdc: caps().supply_cap_usdc + 1,
                ..caps()
            },
            Caps {
                position_debt_cap_usdc: caps().total_debt_cap_usdc + 1,
                ..caps()
            },
        ];
        for c in bad {
            assert_eq!(check_caps_bounds(&c), Err(MarketError::ConfigOutOfBounds));
        }
        let double = Caps {
            supply_cap_usdc: 2 * caps().supply_cap_usdc,
            ..caps()
        };
        assert_eq!(check_caps_change(&caps(), &double), Ok(()));
        let too_far = Caps {
            supply_cap_usdc: 2 * caps().supply_cap_usdc + 1,
            ..caps()
        };
        assert_eq!(
            check_caps_change(&caps(), &too_far),
            Err(MarketError::ConfigMovementTooLarge)
        );
        // From zero, one change may raise up to the floor.
        assert!(raise_allowed(0, USDC_CAP_RAISE_FLOOR, USDC_CAP_RAISE_FLOOR));
        assert!(!raise_allowed(
            0,
            USDC_CAP_RAISE_FLOOR + 1,
            USDC_CAP_RAISE_FLOOR
        ));
        assert!(raise_allowed(u64::MAX, 0, 1));
        let counts = CountCaps {
            position_count_cap: 10,
            debt_position_count_cap: 11,
        };
        assert_eq!(
            check_count_caps_bounds(&counts),
            Err(MarketError::ConfigOutOfBounds)
        );
        assert_eq!(
            check_count_caps_bounds(&CountCaps {
                position_count_cap: MAX_POSITION_COUNT_CAP + 1,
                debt_position_count_cap: 0,
            }),
            Err(MarketError::ConfigOutOfBounds)
        );
    }

    #[test]
    fn decreases_are_immediate_risk_authority_only() {
        let lower = ConfigValue::Caps {
            caps: Caps {
                total_debt_cap_usdc: 100_000 * USDC,
                ..caps()
            },
        };
        for signer in [GUARDIAN, ADMIN, STRANGER] {
            let (mut m, mut a) = (market(), asset());
            assert_eq!(
                decrease_caps(&mut m, &mut a, &signer, lower),
                Err(MarketError::NotRiskAuthority)
            );
            assert_eq!((m, a), (market(), asset()));
        }
        let (mut m, mut a) = (market(), asset());
        let done = decrease_caps(&mut m, &mut a, &RISK, lower).unwrap();
        assert_eq!(done.old_value, ConfigValue::Caps { caps: caps() });
        assert_eq!(done.new_value, lower);
        assert_eq!((done.old_generation, done.new_generation), (1, 2));
        assert_eq!(a.caps.total_debt_cap_usdc, 100_000 * USDC);
        assert_eq!(a.generation, 2);

        let counts = ConfigValue::CountCaps {
            caps: CountCaps {
                position_count_cap: 8,
                debt_position_count_cap: 2,
            },
        };
        decrease_caps(&mut m, &mut a, &RISK, counts).unwrap();
        assert_eq!((m.position_count_cap, m.debt_position_count_cap), (8, 2));
        assert_eq!(a.generation, 3);
    }

    #[test]
    fn immediate_path_rejects_raises_other_groups_and_bad_bounds() {
        let raise = ConfigValue::Caps {
            caps: Caps {
                supply_cap_usdc: caps().supply_cap_usdc + 1,
                total_debt_cap_usdc: 0,
                position_debt_cap_usdc: 0,
            },
        };
        let raise_count = ConfigValue::CountCaps {
            caps: CountCaps {
                position_count_cap: 11,
                debt_position_count_cap: 0,
            },
        };
        let other = ConfigValue::TimelockDelay {
            delay_seconds: 86_400,
        };
        for change in [raise, raise_count, other] {
            let (mut m, mut a) = (market(), asset());
            assert_eq!(
                decrease_caps(&mut m, &mut a, &RISK, change),
                Err(MarketError::CapChangeNotDecrease)
            );
            assert_eq!((m, a), (market(), asset()));
        }
        let (mut m, mut a) = (market(), asset());
        assert_eq!(
            decrease_caps(&mut m, &mut a, &RISK, ConfigValue::Caps { caps: caps() }),
            Err(MarketError::ConfigUnchanged)
        );
        // Lowering supply below total debt breaks the ordering.
        let unordered = ConfigValue::Caps {
            caps: Caps {
                supply_cap_usdc: 1,
                ..caps()
            },
        };
        assert_eq!(
            decrease_caps(&mut m, &mut a, &RISK, unordered),
            Err(MarketError::ConfigOutOfBounds)
        );
        assert_eq!((m, a), (market(), asset()));
    }

    #[test]
    fn increases_go_through_the_timelock() {
        let (mut m, mut a) = (market(), asset());
        let raise = ConfigValue::CountCaps {
            caps: CountCaps {
                position_count_cap: 20,
                debt_position_count_cap: 8,
            },
        };
        let mut p = propose(&m, &mut a, &RISK, 0, raise, T0).unwrap();
        assert_eq!(
            p.old_value(),
            ConfigValue::CountCaps {
                caps: m.count_caps()
            }
        );
        assert_eq!(p.new_value(), raise);
        assert_eq!(m.count_caps(), market().count_caps(), "nothing applied yet");
        assert_eq!(
            execute(&mut m, &mut a, &mut p, &STRANGER, T0 + DELAY - 1),
            Err(MarketError::TimelockNotElapsed)
        );
        let done = execute(&mut m, &mut a, &mut p, &STRANGER, T0 + DELAY).unwrap();
        assert_eq!(done.new_value, raise);
        assert_eq!(p.status, ProposalStatus::Executed);
        assert_eq!((m.position_count_cap, m.debt_position_count_cap), (20, 8));
        assert_eq!(a.generation, 2);

        // A raise past the limit is refused at proposal time.
        let far = ConfigValue::Caps {
            caps: Caps {
                supply_cap_usdc: 3 * caps().supply_cap_usdc,
                ..caps()
            },
        };
        assert_eq!(
            propose(&m, &mut a, &RISK, 1, far, T0),
            Err(MarketError::ConfigMovementTooLarge)
        );
    }

    #[test]
    fn a_decrease_blocks_a_pending_proposal_on_the_same_group() {
        let (mut m, mut a) = (market(), asset());
        let raise = ConfigValue::Caps {
            caps: Caps {
                supply_cap_usdc: 2 * caps().supply_cap_usdc,
                ..caps()
            },
        };
        let mut p = propose(&m, &mut a, &RISK, 0, raise, T0).unwrap();
        let lower = ConfigValue::Caps {
            caps: Caps {
                position_debt_cap_usdc: 1,
                ..caps()
            },
        };
        decrease_caps(&mut m, &mut a, &RISK, lower).unwrap();
        assert_eq!(
            execute(&mut m, &mut a, &mut p, &STRANGER, T0 + DELAY),
            Err(MarketError::ConfigProposalBaseChanged)
        );
        assert_eq!(a.caps.position_debt_cap_usdc, 1);
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

        fn below(&mut self, n: u64) -> u64 {
            self.sample() % n
        }

        fn chance(&mut self, one_in: u64) -> bool {
            self.below(one_in) == 0
        }
    }

    /// One in-flight Intent in the model world. `stale` is set when a cap
    /// change bumped the Asset State generation after it was queued, so its
    /// callback can only resolve `Stale`.
    struct Pending {
        position: usize,
        action: Action,
        stale: bool,
    }

    /// **Validates: Requirements 4.10, 4.11, 6.6, 7.6, 8.14** - over random
    /// sequences of Position creation, borrow, repay, liquidation, and Absorb
    /// queues, callbacks with random decisions and revealed transitions, and
    /// cap decreases: every increment of a count happens at or below its cap
    /// (new risk never passes a cap); `debt_position_count` always equals the
    /// number of Positions with `has_debt`; reservations equal the in-flight
    /// borrows of Positions without debt; counts move only on `Applied`
    /// decisions; and a rejected step changes nothing.
    #[test]
    fn prop_counts_follow_authenticated_transitions_and_respect_caps() {
        let mut rng = Lcg(0x5EED_4501);
        let mut applied_changes = 0usize;
        let mut unavailable = 0usize;
        for _ in 0..1_000 {
            let mut m = market();
            m.position_count_cap = 2 + rng.below(12);
            m.debt_position_count_cap = rng.below(m.position_count_cap + 1);
            let mut flags: Vec<bool> = Vec::new();
            let mut locked: Vec<bool> = Vec::new();
            let mut pending: Vec<Pending> = Vec::new();
            for _ in 0..60 {
                match rng.below(6) {
                    0 => {
                        let before = m.clone();
                        match on_position_initialized(&mut m) {
                            Ok(change) => {
                                assert!(change.new_count <= m.position_count_cap);
                                flags.push(false);
                                locked.push(false);
                            }
                            Err(error) => {
                                assert_eq!(error, MarketError::PositionCountCapReached);
                                assert_eq!(m, before);
                            }
                        }
                    }
                    1 | 2 if !flags.is_empty() => {
                        let position = rng.below(flags.len() as u64) as usize;
                        if locked[position] {
                            continue;
                        }
                        let action = match rng.below(5) {
                            0 | 1 => Action::Borrow,
                            2 => Action::Repay,
                            3 => Action::Liquidation,
                            _ => Action::Absorb,
                        };
                        let before = m.clone();
                        match reserve_debt_slot(&mut m, action, flags[position]).unwrap() {
                            DebtSlot::Unavailable => {
                                unavailable += 1;
                                assert_eq!(m, before);
                                continue;
                            }
                            DebtSlot::Reserved => {
                                assert!(
                                    m.debt_position_count + m.debt_position_reservations
                                        <= m.debt_position_count_cap
                                );
                            }
                            DebtSlot::NotNeeded => assert_eq!(m, before),
                        }
                        locked[position] = true;
                        pending.push(Pending {
                            position,
                            action,
                            stale: false,
                        });
                    }
                    3 | 4 if !pending.is_empty() => {
                        let index = rng.below(pending.len() as u64) as usize;
                        let Pending {
                            position,
                            action,
                            stale,
                        } = pending.swap_remove(index);
                        let decision = match rng.below(5) {
                            _ if stale => CallbackDecision::Stale(StaleReason::AssetGeneration),
                            0 => CallbackDecision::Zero,
                            1 => CallbackDecision::Stale(StaleReason::AssetGeneration),
                            2 => CallbackDecision::Failed,
                            _ => CallbackDecision::Applied,
                        };
                        // The circuit reveals the true "before" flag, except
                        // in an occasional forged-looking contradiction.
                        let was = if rng.chance(10) {
                            !flags[position]
                        } else {
                            flags[position]
                        };
                        let is = rng.chance(2);
                        let transition = DebtTransition::from_callback(decision, was, is);
                        let (before_m, before_flag) = (m.clone(), flags[position]);
                        let mut flag = flags[position];
                        match settle_debt_intent(&mut m, &mut flag, action, transition) {
                            Ok(done) => {
                                if done.changed() {
                                    assert_eq!(decision, CallbackDecision::Applied);
                                    applied_changes += 1;
                                }
                                if done.new_count > done.old_count {
                                    assert!(done.new_count <= m.debt_position_count_cap);
                                }
                                flags[position] = flag;
                                locked[position] = false;
                            }
                            Err(error) => {
                                assert_eq!(error, MarketError::DebtTransitionMismatch);
                                assert_eq!((m.clone(), flag), (before_m, before_flag));
                                // A rejected callback reverts; timeout
                                // recovery later settles with no transition.
                                let done =
                                    settle_debt_intent(&mut m, &mut flag, action, None).unwrap();
                                assert!(!done.changed());
                                locked[position] = false;
                            }
                        }
                    }
                    // Risk authority lowers the debt-count cap; the
                    // generation bump stales every pending Intent.
                    5 if m.debt_position_count_cap > 0 && rng.chance(4) => {
                        m.debt_position_count_cap -= 1;
                        pending.iter_mut().for_each(|p| p.stale = true);
                    }
                    _ => {}
                }
                let with_debt = flags.iter().filter(|f| **f).count() as u64;
                assert_eq!(m.debt_position_count, with_debt);
                let holding = pending
                    .iter()
                    .filter(|p| holds_debt_slot(p.action, flags[p.position]))
                    .count() as u64;
                assert_eq!(m.debt_position_reservations, holding);
                assert_eq!(m.position_count, flags.len() as u64);
            }
        }
        assert!(
            applied_changes > 250,
            "the generator must reach transitions"
        );
        assert!(unavailable > 50, "the generator must reach the cap");
    }

    /// **Validates: Requirements 4.10, 4.11** - only an `Applied` decision
    /// yields a transition; every other decision leaves `has_debt` and the
    /// debt count unchanged whatever booleans the output carried.
    #[test]
    fn prop_transitions_only_from_applied_decisions() {
        let mut rng = Lcg(0x5EED_4502);
        let decisions = [
            CallbackDecision::Ignore,
            CallbackDecision::LeaveQueued,
            CallbackDecision::Applied,
            CallbackDecision::Zero,
            CallbackDecision::Stale(StaleReason::PositionGeneration),
            CallbackDecision::Expired,
            CallbackDecision::Failed,
        ];
        for _ in 0..5_000 {
            let decision = decisions[rng.below(decisions.len() as u64) as usize];
            let (was, is) = (rng.chance(2), rng.chance(2));
            let transition = DebtTransition::from_callback(decision, was, is);
            assert_eq!(transition.is_some(), decision == CallbackDecision::Applied);
            if decision != CallbackDecision::Applied {
                let mut m = market();
                m.debt_position_count = 1;
                let mut flag = rng.chance(2);
                let before = flag;
                let action = if rng.chance(2) {
                    Action::Repay
                } else {
                    Action::Liquidation
                };
                let done = settle_debt_intent(&mut m, &mut flag, action, transition).unwrap();
                assert!(!done.changed());
                assert_eq!((flag, m.debt_position_count), (before, 1));
            }
        }
    }

    /// **Validates: Requirement 4.10** - cap checks never error for any value
    /// (passive accrual cannot fail because of a cap), and once a value is
    /// above its cap every new-risk amount resolves to zero.
    #[test]
    fn prop_passive_accrual_never_errors_and_new_risk_is_zero_above_cap() {
        let mut rng = Lcg(0x5EED_4503);
        for _ in 0..20_000 {
            let supply = rng.sample() % (MAX_USDC_CAP + 1);
            let total = rng.sample() % (supply + 1);
            let position = rng.sample() % (total + 1);
            let c = Caps {
                supply_cap_usdc: supply,
                total_debt_cap_usdc: total,
                position_debt_cap_usdc: position,
            };
            let accrued = u128::from(rng.sample()) * u128::from(1 + rng.below(4));
            let amount = rng.sample();
            // Reference: proceed iff before + amount stays within every cap.
            let supply_ok = accrued + u128::from(amount) <= u128::from(supply);
            assert_eq!(
                supply_cap_outcome(&c, accrued, amount) == NewRiskOutcome::Proceed,
                supply_ok
            );
            let other = u128::from(rng.below(position + 1));
            let borrow_ok = accrued + u128::from(amount) <= u128::from(total)
                && other + u128::from(amount) <= u128::from(position);
            assert_eq!(
                borrow_cap_outcome(&c, accrued, other, amount) == NewRiskOutcome::Proceed,
                borrow_ok
            );
            if accrued > u128::from(supply) {
                assert_eq!(
                    supply_cap_outcome(&c, accrued, amount),
                    NewRiskOutcome::Zero
                );
            }
            if accrued > u128::from(total) {
                assert_eq!(
                    borrow_cap_outcome(&c, accrued, 0, amount),
                    NewRiskOutcome::Zero
                );
            }
        }
    }

    /// **Validates: Requirements 4.5, 4.6, 4.10** - for random cap and
    /// count-cap changes: the immediate path accepts exactly the in-bounds,
    /// non-equal decreases (bumping the generation once) and refuses every
    /// raise; the timelock path accepts exactly the in-bounds changes within
    /// the raise limit and never executes before the delay.
    #[test]
    fn prop_raises_need_the_timelock_and_decreases_are_immediate() {
        let mut rng = Lcg(0x5EED_4504);
        let mut decreases = 0;
        let mut raises = 0;
        for _ in 0..5_000 {
            let (mut m, mut a) = (market(), asset());
            let nudge = |rng: &mut Lcg, value: u64, scale: u64| -> u64 {
                match rng.below(4) {
                    0 => value,
                    1 => value.saturating_sub(rng.below(scale + 1)),
                    2 => value.saturating_add(rng.below(scale + 1)),
                    _ => value.saturating_mul(1 + rng.below(4)),
                }
            };
            let change = if rng.chance(2) {
                let c = a.caps;
                ConfigValue::Caps {
                    caps: Caps {
                        supply_cap_usdc: nudge(&mut rng, c.supply_cap_usdc, 2_000_000 * USDC),
                        total_debt_cap_usdc: nudge(&mut rng, c.total_debt_cap_usdc, 600_000 * USDC),
                        position_debt_cap_usdc: nudge(
                            &mut rng,
                            c.position_debt_cap_usdc,
                            20_000 * USDC,
                        ),
                    },
                }
            } else {
                let c = m.count_caps();
                ConfigValue::CountCaps {
                    caps: CountCaps {
                        position_count_cap: nudge(&mut rng, c.position_count_cap, 200),
                        debt_position_count_cap: nudge(&mut rng, c.debt_position_count_cap, 200),
                    },
                }
            };
            let current = GovernedConfig::of(&m, &a);
            let (old, new, floor, bounds_ok): (Vec<u64>, Vec<u64>, u64, bool) = match change {
                ConfigValue::Caps { caps } => (
                    vec![
                        a.caps.supply_cap_usdc,
                        a.caps.total_debt_cap_usdc,
                        a.caps.position_debt_cap_usdc,
                    ],
                    vec![
                        caps.supply_cap_usdc,
                        caps.total_debt_cap_usdc,
                        caps.position_debt_cap_usdc,
                    ],
                    USDC_CAP_RAISE_FLOOR,
                    caps.position_debt_cap_usdc <= caps.total_debt_cap_usdc
                        && caps.total_debt_cap_usdc <= caps.supply_cap_usdc
                        && caps.supply_cap_usdc <= MAX_USDC_CAP,
                ),
                ConfigValue::CountCaps { caps } => (
                    vec![m.position_count_cap, m.debt_position_count_cap],
                    vec![caps.position_count_cap, caps.debt_position_count_cap],
                    COUNT_CAP_RAISE_FLOOR,
                    caps.debt_position_count_cap <= caps.position_count_cap
                        && caps.position_count_cap <= MAX_POSITION_COUNT_CAP,
                ),
                _ => unreachable!(),
            };
            let unchanged = old == new;
            let all_lower = old.iter().zip(&new).all(|(o, n)| n <= o);
            let within_raise = old
                .iter()
                .zip(&new)
                .all(|(o, n)| n <= o || *n <= (2 * o).max(o + floor));

            let snapshot = (m.clone(), a.clone());
            let immediate = decrease_caps(&mut m, &mut a, &RISK, change);
            let expect_immediate = all_lower && !unchanged && bounds_ok;
            assert_eq!(immediate.is_ok(), expect_immediate, "{change:?}");
            if expect_immediate {
                decreases += 1;
                assert_eq!(a.generation, snapshot.1.generation + 1);
                assert_eq!(GovernedConfig::of(&m, &a), current.with(change));
            } else {
                assert_eq!((m.clone(), a.clone()), snapshot);
            }

            let (mut m, mut a) = snapshot;
            let proposal = propose(&m, &mut a, &RISK, 0, change, T0);
            let expect_timelock = !unchanged && bounds_ok && within_raise;
            assert_eq!(proposal.is_ok(), expect_timelock, "{change:?}");
            if let Ok(mut p) = proposal {
                if !all_lower {
                    raises += 1;
                }
                let early = T0 + rng.below(DELAY as u64) as i64;
                assert_eq!(
                    execute(&mut m, &mut a, &mut p, &STRANGER, early),
                    Err(MarketError::TimelockNotElapsed)
                );
                execute(&mut m, &mut a, &mut p, &STRANGER, T0 + DELAY).unwrap();
                assert_eq!(GovernedConfig::of(&m, &a), current.with(change));
            }
        }
        assert!(decreases > 300 && raises > 300, "{decreases} {raises}");
    }
}
