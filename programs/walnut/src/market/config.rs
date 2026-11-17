//! Timelocked configuration changes (Task 4.4, Requirements 4.5, 4.6, 4.12,
//! and 4.13).
//!
//! Every change to the Asset State risk parameters, rate parameters, caps, or
//! the timelock delay itself goes through three steps:
//!
//! 1. [`propose`]: the risk authority names one [`ConfigValue`]. The change is
//!    validated against the current configuration (absolute bounds, maximum
//!    movement, and the Requirement 4.13 cross-bound) and stored in a
//!    [`ConfigProposal`] PDA with `earliest_execute_at = now + delay`.
//! 2. [`execute`]: anyone, once `now >= earliest_execute_at` and before the
//!    grace window ends, and only while the proposal is `Pending`. The group
//!    being changed must still hold the value recorded at proposal time, and
//!    the change is validated again against the then-current configuration.
//!    Execution writes the new values and increments the Asset State
//!    generation exactly once.
//! 3. [`cancel`]: the risk authority or the guardian (the cancellation role),
//!    while `Pending`.
//!
//! # Invalidation
//!
//! Every Operation Intent captures the Asset State generation at queue time,
//! and the Task 3.3 callback path marks an Intent `Stale` (no financial write)
//! when the current generation differs. The design does not split
//! configuration changes into compatible and incompatible ones, so every
//! executed change is treated as incompatible with every pending computation:
//! one execution bumps the generation once and every Intent queued before it
//! resolves `Stale`. Proposing and cancelling change no configuration and do
//! not bump the generation.
//!
//! # Bounds
//!
//! [`bounds`] holds one [`Bound`] per numeric field: an inclusive absolute
//! range and the largest absolute change one execution may make. Ratios are
//! basis points. The USDC caps and the Market count caps (Task 4.5) use the
//! rules in `market::caps`: absolute ceilings, ordering, and a limit on how
//! far one change may raise a cap. The timelock delay may take any value in
//! its range in one change, but never less than [`MIN_TIMELOCK_DELAY_SECONDS`].
//!
//! # Groups
//!
//! `Risk`, `Rates`, `Caps`, and `TimelockDelay` live in the Asset State.
//! `CountCaps` (Task 4.5) lives in the Market (`position_count_cap`,
//! `debt_position_count_cap`), so [`GovernedConfig`] snapshots both accounts
//! and execution writes the Market for that group. A proposal stores the
//! count caps in two 16-byte fields taken from its reserved tail, so the
//! proposal size and every earlier offset are unchanged.
//!
//! # Cross-bound (Requirement 4.13)
//!
//! With `L` the Liquidation Threshold, `B = 1 + bonus`, `T` the target Health
//! Factor, and a documented rounding margin `m = CROSS_BOUND_MARGIN_BPS`
//! (1%), every allowed configuration satisfies, in exact integer basis-point
//! arithmetic:
//!
//! ```text
//! L × B ≤ 1 − m          (so L × B < 1)
//! T     ≥ L × B + m      (so T > L × B)
//! ```
//!
//! Also `ltv < liquidation_threshold` and `slope1 ≤ slope2`.
//!
//! # Timing
//!
//! Times are Unix seconds from the Clock sysvar. Execution needs
//! `now >= max(earliest_execute_at, proposed_at + current delay)`, so raising
//! the delay also delays pending proposals, and lowering it never shortens
//! one. Execution must happen before the proposal's stored `expires_at`
//! (`earliest_execute_at + EXECUTION_GRACE_SECONDS`, fixed at proposal time);
//! after that the proposal can only be cancelled, so a forgotten proposal
//! cannot be executed by surprise, even if a later delay raise moves its ready
//! time. Because the delay is at most [`MAX_TIMELOCK_DELAY_SECONDS`] and the
//! grace window is as long, a raised delay still leaves at least one day
//! between the ready time and `expires_at`.

use anchor_lang::prelude::*;
use core::result::Result;

use super::caps::{check_caps_change, check_count_caps_change};
use super::roles::{authorize, Role};
use super::state::{AssetState, Caps, CountCaps, Market, MarketStatus, RateParams, RiskParams};
use super::validation::MarketError;

/// Seed of a configuration proposal PDA (followed by the market and the
/// little-endian proposal id).
pub const CONFIG_PROPOSAL_SEED: &[u8] = b"config_proposal";
/// Layout version written into every proposal.
pub const CONFIG_PROPOSAL_LAYOUT_VERSION: u8 = 1;
/// The timelock delay can never be set below 24 hours.
pub const MIN_TIMELOCK_DELAY_SECONDS: u32 = 86_400;
/// The timelock delay can never be set above 14 days.
pub const MAX_TIMELOCK_DELAY_SECONDS: u32 = 1_209_600;
/// How long after it becomes executable a proposal stays executable.
pub const EXECUTION_GRACE_SECONDS: i64 = 1_209_600;
/// One in basis points.
pub const BPS: u64 = 10_000;
/// Rounding margin of the Requirement 4.13 cross-bound, in basis points.
pub const CROSS_BOUND_MARGIN_BPS: u64 = 100;

/// An inclusive absolute range and the largest change one execution may make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bound {
    /// Smallest allowed value.
    pub min: u64,
    /// Largest allowed value.
    pub max: u64,
    /// Largest `|new - old|` one execution may apply.
    pub max_step: u64,
}

impl Bound {
    const fn new(min: u64, max: u64, max_step: u64) -> Self {
        Self { min, max, max_step }
    }

    /// Whether `value` is inside the range.
    pub fn contains(self, value: u64) -> bool {
        self.min <= value && value <= self.max
    }

    /// Whether moving from `old` to `new` stays within `max_step`.
    pub fn step_allowed(self, old: u64, new: u64) -> bool {
        old.abs_diff(new) <= self.max_step
    }
}

/// Absolute bounds and maximum movement per execution (Requirement 4.12).
pub mod bounds {
    use super::Bound;

    /// `ltv_bps`.
    pub const LTV: Bound = Bound::new(1_000, 8_000, 500);
    /// `liquidation_threshold_bps`.
    pub const LIQUIDATION_THRESHOLD: Bound = Bound::new(1_000, 8_500, 500);
    /// `liquidation_bonus_bps`.
    pub const LIQUIDATION_BONUS: Bound = Bound::new(100, 1_500, 200);
    /// `target_health_factor_bps` (strictly above 1.0).
    pub const TARGET_HEALTH_FACTOR: Bound = Bound::new(10_100, 15_000, 1_000);
    /// `close_factor_bps`.
    pub const CLOSE_FACTOR: Bound = Bound::new(1_000, 10_000, 2_500);
    /// `reserve_factor_bps`.
    pub const RESERVE_FACTOR: Bound = Bound::new(0, 5_000, 1_000);
    /// `oracle_max_age_seconds`.
    pub const ORACLE_MAX_AGE_SECONDS: Bound = Bound::new(5, 300, 60);
    /// `oracle_max_confidence_bps`.
    pub const ORACLE_MAX_CONFIDENCE: Bound = Bound::new(10, 500, 100);
    /// `decision_expiry_slots`.
    pub const DECISION_EXPIRY_SLOTS: Bound = Bound::new(10, 1_500, 150);
    /// `base_rate_bps`.
    pub const BASE_RATE: Bound = Bound::new(0, 2_000, 200);
    /// `slope1_bps`.
    pub const SLOPE1: Bound = Bound::new(0, 5_000, 400);
    /// `slope2_bps`.
    pub const SLOPE2: Bound = Bound::new(0, 50_000, 5_000);
    /// `optimal_utilization_bps`.
    pub const OPTIMAL_UTILIZATION: Bound = Bound::new(5_000, 9_500, 1_000);
    /// `band_max_age_slots`.
    pub const BAND_MAX_AGE_SLOTS: Bound = Bound::new(150, 216_000, 9_000);
    /// `timelock.delay_seconds`: no movement limit inside the range.
    pub const TIMELOCK_DELAY_SECONDS: Bound = Bound::new(
        super::MIN_TIMELOCK_DELAY_SECONDS as u64,
        super::MAX_TIMELOCK_DELAY_SECONDS as u64,
        u64::MAX,
    );
}

/// Which configuration group a proposal changes.
#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub enum ConfigGroup {
    /// `AssetState.risk`.
    #[default]
    Risk,
    /// `AssetState.rates`.
    Rates,
    /// `AssetState.caps`.
    Caps,
    /// `AssetState.timelock.delay_seconds`.
    TimelockDelay,
    /// `Market.position_count_cap` and `Market.debt_position_count_cap`
    /// (Task 4.5).
    CountCaps,
}

/// One proposed group value: the `propose_config_change` argument and the
/// old/new values carried by the events.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigValue {
    /// New risk parameters.
    Risk {
        /// The whole group.
        params: RiskParams,
    },
    /// New rate parameters.
    Rates {
        /// The whole group.
        params: RateParams,
    },
    /// New caps.
    Caps {
        /// The whole group.
        caps: Caps,
    },
    /// New timelock delay.
    TimelockDelay {
        /// Seconds.
        delay_seconds: u32,
    },
    /// New Market count caps (Task 4.5).
    CountCaps {
        /// Both count caps.
        caps: CountCaps,
    },
}

impl ConfigValue {
    /// The group this value belongs to.
    pub fn group(&self) -> ConfigGroup {
        match self {
            ConfigValue::Risk { .. } => ConfigGroup::Risk,
            ConfigValue::Rates { .. } => ConfigGroup::Rates,
            ConfigValue::Caps { .. } => ConfigGroup::Caps,
            ConfigValue::TimelockDelay { .. } => ConfigGroup::TimelockDelay,
            ConfigValue::CountCaps { .. } => ConfigGroup::CountCaps,
        }
    }
}

/// A fixed-size snapshot of every Asset State configuration group (72 bytes).
/// The Market count caps are stored next to it (see [`GovernedConfig`]).
/// Proposals store snapshots instead of a `ConfigValue` because a Borsh enum
/// serializes to a variant-dependent length, which would move later offsets.
#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfigValues {
    /// Risk parameters.
    pub risk: RiskParams,
    /// Rate parameters.
    pub rates: RateParams,
    /// Caps.
    pub caps: Caps,
    /// Timelock delay.
    pub timelock_delay_seconds: u32,
}

impl ConfigValues {
    /// The current configuration of `asset`.
    pub fn of(asset: &AssetState) -> Self {
        Self {
            risk: asset.risk,
            rates: asset.rates,
            caps: asset.caps,
            timelock_delay_seconds: asset.timelock.delay_seconds,
        }
    }
}

/// Every governed group: the Asset State groups plus the Market count caps
/// (Task 4.5). In-memory only; proposals store its two parts separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GovernedConfig {
    /// The Asset State groups.
    pub values: ConfigValues,
    /// The Market count caps.
    pub count_caps: CountCaps,
}

impl GovernedConfig {
    /// The current configuration of `market` and `asset`.
    pub fn of(market: &Market, asset: &AssetState) -> Self {
        Self {
            values: ConfigValues::of(asset),
            count_caps: market.count_caps(),
        }
    }

    /// This snapshot with `value`'s group replaced.
    pub fn with(mut self, value: ConfigValue) -> Self {
        match value {
            ConfigValue::Risk { params } => self.values.risk = params,
            ConfigValue::Rates { params } => self.values.rates = params,
            ConfigValue::Caps { caps } => self.values.caps = caps,
            ConfigValue::TimelockDelay { delay_seconds } => {
                self.values.timelock_delay_seconds = delay_seconds
            }
            ConfigValue::CountCaps { caps } => self.count_caps = caps,
        }
        self
    }

    /// The value of one group.
    pub fn get(&self, group: ConfigGroup) -> ConfigValue {
        match group {
            ConfigGroup::Risk => ConfigValue::Risk {
                params: self.values.risk,
            },
            ConfigGroup::Rates => ConfigValue::Rates {
                params: self.values.rates,
            },
            ConfigGroup::Caps => ConfigValue::Caps {
                caps: self.values.caps,
            },
            ConfigGroup::TimelockDelay => ConfigValue::TimelockDelay {
                delay_seconds: self.values.timelock_delay_seconds,
            },
            ConfigGroup::CountCaps => ConfigValue::CountCaps {
                caps: self.count_caps,
            },
        }
    }
}

/// Writes one group value into `market` (count caps) or `asset` (every other
/// group).
pub(crate) fn write_value(market: &mut Market, asset: &mut AssetState, value: ConfigValue) {
    match value {
        ConfigValue::Risk { params } => asset.risk = params,
        ConfigValue::Rates { params } => asset.rates = params,
        ConfigValue::Caps { caps } => asset.caps = caps,
        ConfigValue::TimelockDelay { delay_seconds } => {
            asset.timelock.delay_seconds = delay_seconds
        }
        ConfigValue::CountCaps { caps } => market.set_count_caps(caps),
    }
}

// ---- Pure checks ---------------------------------------------------------------

fn risk_fields(risk: &RiskParams) -> [(u64, Bound); 9] {
    [
        (u64::from(risk.ltv_bps), bounds::LTV),
        (
            u64::from(risk.liquidation_threshold_bps),
            bounds::LIQUIDATION_THRESHOLD,
        ),
        (
            u64::from(risk.liquidation_bonus_bps),
            bounds::LIQUIDATION_BONUS,
        ),
        (
            u64::from(risk.target_health_factor_bps),
            bounds::TARGET_HEALTH_FACTOR,
        ),
        (u64::from(risk.close_factor_bps), bounds::CLOSE_FACTOR),
        (u64::from(risk.reserve_factor_bps), bounds::RESERVE_FACTOR),
        (
            u64::from(risk.oracle_max_age_seconds),
            bounds::ORACLE_MAX_AGE_SECONDS,
        ),
        (
            u64::from(risk.oracle_max_confidence_bps),
            bounds::ORACLE_MAX_CONFIDENCE,
        ),
        (
            u64::from(risk.decision_expiry_slots),
            bounds::DECISION_EXPIRY_SLOTS,
        ),
    ]
}

fn rate_fields(rates: &RateParams) -> [(u64, Bound); 5] {
    [
        (u64::from(rates.base_rate_bps), bounds::BASE_RATE),
        (u64::from(rates.slope1_bps), bounds::SLOPE1),
        (u64::from(rates.slope2_bps), bounds::SLOPE2),
        (
            u64::from(rates.optimal_utilization_bps),
            bounds::OPTIMAL_UTILIZATION,
        ),
        (rates.band_max_age_slots, bounds::BAND_MAX_AGE_SLOTS),
    ]
}

/// Requirement 4.13 with the documented margin (see the module docs).
pub fn check_cross_bound(risk: &RiskParams) -> Result<(), MarketError> {
    let margin = CROSS_BOUND_MARGIN_BPS * BPS;
    // L × B in bps², at most 8,500 × 11,500 here, far below u64::MAX.
    let threshold_times_bonus =
        u64::from(risk.liquidation_threshold_bps) * (BPS + u64::from(risk.liquidation_bonus_bps));
    if threshold_times_bonus > BPS * BPS - margin {
        return Err(MarketError::LiquidationCrossBound);
    }
    if u64::from(risk.target_health_factor_bps) * BPS < threshold_times_bonus + margin {
        return Err(MarketError::LiquidationCrossBound);
    }
    Ok(())
}

/// Absolute bounds, `ltv < liquidation_threshold`, and the cross-bound.
pub fn check_risk_bounds(risk: &RiskParams) -> Result<(), MarketError> {
    if risk_fields(risk)
        .iter()
        .any(|(value, bound)| !bound.contains(*value))
    {
        return Err(MarketError::ConfigOutOfBounds);
    }
    if risk.ltv_bps >= risk.liquidation_threshold_bps {
        return Err(MarketError::ConfigOutOfBounds);
    }
    check_cross_bound(risk)
}

/// Absolute bounds and `slope1 ≤ slope2`.
pub fn check_rate_bounds(rates: &RateParams) -> Result<(), MarketError> {
    if rate_fields(rates)
        .iter()
        .any(|(value, bound)| !bound.contains(*value))
        || rates.slope1_bps > rates.slope2_bps
    {
        return Err(MarketError::ConfigOutOfBounds);
    }
    Ok(())
}

/// The timelock delay range; it never drops below the design minimum.
pub fn check_delay_bounds(delay_seconds: u32) -> Result<(), MarketError> {
    if !bounds::TIMELOCK_DELAY_SECONDS.contains(u64::from(delay_seconds)) {
        return Err(MarketError::TimelockDelayOutOfBounds);
    }
    Ok(())
}

fn check_movement<const N: usize>(
    old: [(u64, Bound); N],
    new: [(u64, Bound); N],
) -> Result<(), MarketError> {
    let too_far = old
        .iter()
        .zip(new.iter())
        .any(|((old, bound), (new, _))| !bound.step_allowed(*old, *new));
    if too_far {
        return Err(MarketError::ConfigMovementTooLarge);
    }
    Ok(())
}

/// Maximum movement of every risk field.
pub fn check_risk_movement(old: &RiskParams, new: &RiskParams) -> Result<(), MarketError> {
    check_movement(risk_fields(old), risk_fields(new))
}

/// Maximum movement of every rate field.
pub fn check_rate_movement(old: &RateParams, new: &RateParams) -> Result<(), MarketError> {
    check_movement(rate_fields(old), rate_fields(new))
}

/// Validates moving `current`'s group to `new`: unchanged values are refused,
/// then bounds (including the cross-bound) and maximum movement are checked.
pub fn validate_change(current: &GovernedConfig, new: ConfigValue) -> Result<(), MarketError> {
    if current.get(new.group()) == new {
        return Err(MarketError::ConfigUnchanged);
    }
    match new {
        ConfigValue::Risk { params } => {
            check_risk_bounds(&params)?;
            check_risk_movement(&current.values.risk, &params)
        }
        ConfigValue::Rates { params } => {
            check_rate_bounds(&params)?;
            check_rate_movement(&current.values.rates, &params)
        }
        ConfigValue::Caps { caps } => check_caps_change(&current.values.caps, &caps),
        ConfigValue::TimelockDelay { delay_seconds } => check_delay_bounds(delay_seconds),
        ConfigValue::CountCaps { caps } => check_count_caps_change(&current.count_caps, &caps),
    }
}

// ---- Proposal account ------------------------------------------------------------

/// Status of a configuration proposal. `Executed` and `Cancelled` are final.
#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub enum ProposalStatus {
    /// Waiting for the delay; may be executed or cancelled.
    #[default]
    Pending,
    /// Applied to the Asset State.
    Executed,
    /// Withdrawn; can never execute.
    Cancelled,
}

/// One timelocked configuration change,
/// `PDA["config_proposal" | market | id (u64 LE)]`, 372 bytes.
///
/// | Offset | Field                 | Type                 |
/// | ------ | --------------------- | -------------------- |
/// | 8      | `layout_version`      | u8                   |
/// | 9      | `bump`                | u8                   |
/// | 10     | `status`              | ProposalStatus       |
/// | 11     | `group`               | ConfigGroup          |
/// | 12     | `market`              | Pubkey               |
/// | 44     | `id`                  | u64                  |
/// | 52     | `proposer`            | Pubkey               |
/// | 84     | `proposed_at`         | i64                  |
/// | 92     | `earliest_execute_at` | i64                  |
/// | 100    | `expires_at`          | i64                  |
/// | 108    | `proposal_generation` | u64                  |
/// | 116    | `old_values`          | ConfigValues 72      |
/// | 188    | `new_values`          | ConfigValues 72      |
/// | 260    | `resolved_at`         | i64                  |
/// | 268    | `resolved_by`         | Pubkey               |
/// | 300    | `executed_generation` | u64                  |
/// | 308    | `old_count_caps`      | CountCaps 16 (4.5)   |
/// | 324    | `new_count_caps`      | CountCaps 16 (4.5)   |
/// | 340    | `reserved`            | [u8; 32]             |
#[account]
#[derive(InitSpace, Debug, PartialEq, Eq)]
pub struct ConfigProposal {
    /// [`CONFIG_PROPOSAL_LAYOUT_VERSION`].
    pub layout_version: u8,
    /// The PDA bump.
    pub bump: u8,
    /// Pending, executed, or cancelled.
    pub status: ProposalStatus,
    /// The group being changed. Only this group of `new_values` is applied.
    pub group: ConfigGroup,
    /// The owning Market.
    pub market: Pubkey,
    /// Proposal id (`TimelockState.next_proposal_id` at proposal time).
    pub id: u64,
    /// The risk authority that proposed it.
    pub proposer: Pubkey,
    /// Unix time of the proposal.
    pub proposed_at: i64,
    /// `proposed_at + delay` with the delay in force at proposal time.
    pub earliest_execute_at: i64,
    /// `earliest_execute_at + EXECUTION_GRACE_SECONDS`.
    pub expires_at: i64,
    /// Asset State generation at proposal time.
    pub proposal_generation: u64,
    /// The whole configuration at proposal time. Execution requires the
    /// changed group to still hold this value.
    pub old_values: ConfigValues,
    /// `old_values` with the group replaced by the proposed value.
    pub new_values: ConfigValues,
    /// Unix time of execution or cancellation; zero while pending.
    pub resolved_at: i64,
    /// The executor or canceller; all zero while pending.
    pub resolved_by: Pubkey,
    /// Asset State generation written by execution; zero otherwise.
    pub executed_generation: u64,
    /// The Market count caps at proposal time (Task 4.5).
    pub old_count_caps: CountCaps,
    /// `old_count_caps`, replaced by the proposed value for a `CountCaps`
    /// proposal.
    pub new_count_caps: CountCaps,
    /// Zeroed extension space.
    pub reserved: [u8; 32],
}

impl ConfigProposal {
    /// Account data length including Anchor's 8-byte discriminator.
    pub const ACCOUNT_LEN: usize = 8 + Self::INIT_SPACE;

    /// The whole governed configuration at proposal time.
    pub fn old_config(&self) -> GovernedConfig {
        GovernedConfig {
            values: self.old_values,
            count_caps: self.old_count_caps,
        }
    }

    /// The configuration the proposal would produce.
    pub fn new_config(&self) -> GovernedConfig {
        GovernedConfig {
            values: self.new_values,
            count_caps: self.new_count_caps,
        }
    }

    /// The proposed value of the changed group.
    pub fn new_value(&self) -> ConfigValue {
        self.new_config().get(self.group)
    }

    /// The value of the changed group at proposal time.
    pub fn old_value(&self) -> ConfigValue {
        self.old_config().get(self.group)
    }
}

/// Derives the proposal address and bump for `market` and `id`.
pub fn derive_config_proposal_address(
    market: &Pubkey,
    id: u64,
    program_id: &Pubkey,
) -> (Pubkey, u8) {
    let id = id.to_le_bytes();
    let seeds: [&[u8]; 3] = [CONFIG_PROPOSAL_SEED, market.as_ref(), &id];
    let (address, bump) = anchor_lang::derive_program_address(&seeds, &program_id.to_bytes());
    (Pubkey::new_from_array(address), bump)
}

// ---- Transitions -----------------------------------------------------------------

/// What an execution changed, for the event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfigExecution {
    /// The group value before execution.
    pub old_value: ConfigValue,
    /// The group value after execution.
    pub new_value: ConfigValue,
    /// Asset State generation before execution.
    pub old_generation: u64,
    /// Asset State generation after execution (`old_generation + 1`).
    pub new_generation: u64,
}

/// The risk authority proposes `change`. `proposal_id` must be the Asset
/// State's `next_proposal_id`, which is then incremented. Returns the proposal
/// to store (the caller sets `bump`). Nothing changes on error.
pub fn propose(
    market: &Market,
    asset: &mut AssetState,
    signer: &Pubkey,
    proposal_id: u64,
    change: ConfigValue,
    now: i64,
) -> Result<ConfigProposal, MarketError> {
    authorize(market, Role::Risk, signer)?;
    if proposal_id != asset.timelock.next_proposal_id {
        return Err(MarketError::ConfigProposalIdMismatch);
    }
    let current = GovernedConfig::of(market, asset);
    validate_change(&current, change)?;
    let target = current.with(change);
    let next_id = proposal_id
        .checked_add(1)
        .ok_or(MarketError::ConfigArithmeticOverflow)?;
    let earliest = now
        .checked_add(i64::from(asset.timelock.delay_seconds))
        .ok_or(MarketError::ConfigArithmeticOverflow)?;
    let expires_at = earliest
        .checked_add(EXECUTION_GRACE_SECONDS)
        .ok_or(MarketError::ConfigArithmeticOverflow)?;
    asset.timelock.next_proposal_id = next_id;
    Ok(ConfigProposal {
        layout_version: CONFIG_PROPOSAL_LAYOUT_VERSION,
        bump: 0,
        status: ProposalStatus::Pending,
        group: change.group(),
        market: asset.market,
        id: proposal_id,
        proposer: *signer,
        proposed_at: now,
        earliest_execute_at: earliest,
        expires_at,
        proposal_generation: asset.generation,
        old_values: current.values,
        new_values: target.values,
        resolved_at: 0,
        resolved_by: Pubkey::default(),
        executed_generation: 0,
        old_count_caps: current.count_caps,
        new_count_caps: target.count_caps,
        reserved: [0; 32],
    })
}

/// The first Unix time at which `proposal` may execute under the current
/// delay: `max(earliest_execute_at, proposed_at + delay)`.
pub fn ready_at(asset: &AssetState, proposal: &ConfigProposal) -> Result<i64, MarketError> {
    let under_current_delay = proposal
        .proposed_at
        .checked_add(i64::from(asset.timelock.delay_seconds))
        .ok_or(MarketError::ConfigArithmeticOverflow)?;
    Ok(proposal.earliest_execute_at.max(under_current_delay))
}

/// Anyone executes a pending proposal after its delay and before its grace
/// window ends. Writes the group (count caps go to the Market, every other
/// group to the Asset State), increments the Asset State generation once, and
/// marks the proposal `Executed`. Nothing changes on error.
pub fn execute(
    market: &mut Market,
    asset: &mut AssetState,
    proposal: &mut ConfigProposal,
    executor: &Pubkey,
    now: i64,
) -> Result<ConfigExecution, MarketError> {
    if market.status != MarketStatus::Active {
        return Err(MarketError::MarketNotActive);
    }
    if proposal.status != ProposalStatus::Pending {
        return Err(MarketError::ConfigProposalNotPending);
    }
    let ready = ready_at(asset, proposal)?;
    if now < ready {
        return Err(MarketError::TimelockNotElapsed);
    }
    // The stored expiry is fixed at proposal time. Recomputing it from the
    // current delay would let a later delay raise reopen a proposal that has
    // already expired (Task 4.6 regression).
    if now >= proposal.expires_at {
        return Err(MarketError::ConfigProposalExpired);
    }
    let current = GovernedConfig::of(market, asset);
    let old_value = current.get(proposal.group);
    if old_value != proposal.old_value() {
        return Err(MarketError::ConfigProposalBaseChanged);
    }
    let new_value = proposal.new_value();
    // Re-validate against the then-current configuration.
    validate_change(&current, new_value)?;
    let old_generation = asset.generation;
    let new_generation = old_generation
        .checked_add(1)
        .ok_or(MarketError::ConfigArithmeticOverflow)?;

    write_value(market, asset, new_value);
    asset.generation = new_generation;
    proposal.status = ProposalStatus::Executed;
    proposal.resolved_at = now;
    proposal.resolved_by = *executor;
    proposal.executed_generation = new_generation;
    Ok(ConfigExecution {
        old_value,
        new_value,
        old_generation,
        new_generation,
    })
}

/// The risk authority or the guardian cancels a pending proposal. Nothing
/// changes on error.
pub fn cancel(
    market: &Market,
    proposal: &mut ConfigProposal,
    signer: &Pubkey,
    now: i64,
) -> Result<(), MarketError> {
    if authorize(market, Role::Risk, signer).is_err()
        && authorize(market, Role::Guardian, signer).is_err()
    {
        if market.status != MarketStatus::Active {
            return Err(MarketError::MarketNotActive);
        }
        return Err(MarketError::NotConfigCanceller);
    }
    if proposal.status != ProposalStatus::Pending {
        return Err(MarketError::ConfigProposalNotPending);
    }
    proposal.status = ProposalStatus::Cancelled;
    proposal.resolved_at = now;
    proposal.resolved_by = *signer;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::state::{PauseState, TimelockState};
    use crate::operation::{
        account_commitment, apply_decision, classify_callback, input_commitment, queue,
        AccountBinding, AccountRole, CallbackContext, CallbackDecision, CallbackReport,
        CurrentDependencies, InputField, InputValue, IntentStatus, OperationIntent, OperationKind,
        StaleReason,
    };
    use crate::state::{Generation, ProtectedAccount};
    use anchor_lang::{AccountDeserialize, AccountSerialize, Discriminator, Space};

    const DELAY: i64 = TimelockState::LAUNCH_DEFAULT_DELAY_SECONDS as i64;
    const T0: i64 = 1_700_000_000;
    const RISK: Pubkey = Pubkey::new_from_array([4; 32]);
    const GUARDIAN: Pubkey = Pubkey::new_from_array([6; 32]);
    const ADMIN: Pubkey = Pubkey::new_from_array([1; 32]);
    const STRANGER: Pubkey = Pubkey::new_from_array([9; 32]);

    fn key(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
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
            debt_position_count_cap: 5,
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
            caps: Caps {
                supply_cap_usdc: 1_000,
                total_debt_cap_usdc: 500,
                position_debt_cap_usdc: 100,
            },
            pauses: PauseState::default(),
            timelock: TimelockState {
                delay_seconds: TimelockState::LAUNCH_DEFAULT_DELAY_SECONDS,
                next_proposal_id: 0,
            },
            reserved: [0; 128],
        }
    }

    fn risk_change(f: impl FnOnce(&mut RiskParams)) -> ConfigValue {
        let mut params = RiskParams::LAUNCH_DEFAULT;
        f(&mut params);
        ConfigValue::Risk { params }
    }

    fn propose_now(asset: &mut AssetState, change: ConfigValue) -> ConfigProposal {
        let id = asset.timelock.next_proposal_id;
        propose(&market(), asset, &RISK, id, change, T0).unwrap()
    }

    fn snapshot(asset: &AssetState) -> Vec<u8> {
        let mut bytes = Vec::new();
        asset.try_serialize(&mut bytes).unwrap();
        bytes
    }

    // ---- Launch defaults and bounds ------------------------------------------

    #[test]
    fn launch_defaults_are_inside_every_bound() {
        assert_eq!(check_risk_bounds(&RiskParams::LAUNCH_DEFAULT), Ok(()));
        assert_eq!(check_rate_bounds(&RateParams::LAUNCH_DEFAULT), Ok(()));
        assert_eq!(
            check_delay_bounds(TimelockState::LAUNCH_DEFAULT_DELAY_SECONDS),
            Ok(())
        );
    }

    #[test]
    fn out_of_range_fields_are_rejected() {
        let cases: [fn(&mut RiskParams); 9] = [
            |r| r.ltv_bps = 8_001,
            |r| r.liquidation_threshold_bps = 999,
            |r| r.liquidation_bonus_bps = 99,
            |r| r.target_health_factor_bps = 10_000,
            |r| r.close_factor_bps = 10_001,
            |r| r.reserve_factor_bps = 5_001,
            |r| r.oracle_max_age_seconds = 301,
            |r| r.oracle_max_confidence_bps = 9,
            |r| r.decision_expiry_slots = 1_501,
        ];
        for case in cases {
            let mut risk = RiskParams::LAUNCH_DEFAULT;
            case(&mut risk);
            assert_eq!(
                check_risk_bounds(&risk),
                Err(MarketError::ConfigOutOfBounds)
            );
        }
        let mut risk = RiskParams::LAUNCH_DEFAULT;
        risk.ltv_bps = risk.liquidation_threshold_bps;
        assert_eq!(
            check_risk_bounds(&risk),
            Err(MarketError::ConfigOutOfBounds)
        );

        let mut rates = RateParams::LAUNCH_DEFAULT;
        rates.slope1_bps = rates.slope2_bps + 1;
        assert_eq!(
            check_rate_bounds(&rates),
            Err(MarketError::ConfigOutOfBounds)
        );
        let mut rates = RateParams::LAUNCH_DEFAULT;
        rates.band_max_age_slots = 149;
        assert_eq!(
            check_rate_bounds(&rates),
            Err(MarketError::ConfigOutOfBounds)
        );
    }

    #[test]
    fn cross_bound_edges_use_the_documented_margin() {
        // L × B = 0.99 exactly is allowed; one bps more is not.
        let mut risk = RiskParams::LAUNCH_DEFAULT;
        risk.liquidation_threshold_bps = 9_000;
        risk.liquidation_bonus_bps = 1_000; // 0.9 × 1.1 = 0.99
        risk.target_health_factor_bps = 10_000; // 0.99 + 0.01
        assert_eq!(check_cross_bound(&risk), Ok(()));
        risk.liquidation_bonus_bps = 1_001;
        assert_eq!(
            check_cross_bound(&risk),
            Err(MarketError::LiquidationCrossBound)
        );
        // T must exceed L × B by at least the margin.
        let base = RiskParams::LAUNCH_DEFAULT; // L × B = 0.6825
        let tight = RiskParams {
            target_health_factor_bps: 6_925,
            ..base
        };
        assert_eq!(check_cross_bound(&tight), Ok(()));
        let too_tight = RiskParams {
            target_health_factor_bps: 6_924,
            ..base
        };
        assert_eq!(
            check_cross_bound(&too_tight),
            Err(MarketError::LiquidationCrossBound)
        );
    }

    #[test]
    fn movement_limits_apply_per_field() {
        let base = RiskParams::LAUNCH_DEFAULT;
        let ok = RiskParams {
            ltv_bps: base.ltv_bps + 500,
            ..base
        };
        assert_eq!(check_risk_movement(&base, &ok), Ok(()));
        let far = RiskParams {
            ltv_bps: base.ltv_bps + 501,
            ..base
        };
        assert_eq!(
            check_risk_movement(&base, &far),
            Err(MarketError::ConfigMovementTooLarge)
        );
        let down = RiskParams {
            liquidation_bonus_bps: base.liquidation_bonus_bps - 201,
            ..base
        };
        assert_eq!(
            check_risk_movement(&base, &down),
            Err(MarketError::ConfigMovementTooLarge)
        );
        let rates = RateParams::LAUNCH_DEFAULT;
        let jump = RateParams {
            slope2_bps: rates.slope2_bps + 5_001,
            ..rates
        };
        assert_eq!(
            check_rate_movement(&rates, &jump),
            Err(MarketError::ConfigMovementTooLarge)
        );
    }

    #[test]
    fn delay_cannot_drop_below_the_minimum() {
        for bad in [
            0,
            MIN_TIMELOCK_DELAY_SECONDS - 1,
            MAX_TIMELOCK_DELAY_SECONDS + 1,
        ] {
            assert_eq!(
                check_delay_bounds(bad),
                Err(MarketError::TimelockDelayOutOfBounds)
            );
        }
        let mut a = asset();
        assert_eq!(
            propose(
                &market(),
                &mut a,
                &RISK,
                0,
                ConfigValue::TimelockDelay { delay_seconds: 60 },
                T0
            ),
            Err(MarketError::TimelockDelayOutOfBounds)
        );
        assert_eq!(a, asset());
    }

    // ---- Propose / execute / cancel ------------------------------------------

    #[test]
    fn propose_wait_execute_bumps_generation_once() {
        let mut a = asset();
        let change = risk_change(|r| r.ltv_bps = 5_500);
        let mut p = propose_now(&mut a, change);
        assert_eq!(p.id, 0);
        assert_eq!(a.timelock.next_proposal_id, 1);
        assert_eq!(p.status, ProposalStatus::Pending);
        assert_eq!(p.group, ConfigGroup::Risk);
        assert_eq!(p.earliest_execute_at, T0 + DELAY);
        assert_eq!(p.expires_at, T0 + DELAY + EXECUTION_GRACE_SECONDS);
        assert_eq!(p.proposal_generation, 1);
        assert_eq!(a.generation, 1, "proposing does not bump");
        assert_eq!(a.risk, RiskParams::LAUNCH_DEFAULT, "nothing applied yet");

        let before = snapshot(&a);
        assert_eq!(
            execute(&mut market(), &mut a, &mut p, &STRANGER, T0 + DELAY - 1),
            Err(MarketError::TimelockNotElapsed)
        );
        assert_eq!(snapshot(&a), before);

        let done = execute(&mut market(), &mut a, &mut p, &STRANGER, T0 + DELAY).unwrap();
        assert_eq!(
            done.old_value,
            ConfigValue::Risk {
                params: RiskParams::LAUNCH_DEFAULT
            }
        );
        assert_eq!(done.new_value, change);
        assert_eq!((done.old_generation, done.new_generation), (1, 2));
        assert_eq!(a.generation, 2);
        assert_eq!(a.risk.ltv_bps, 5_500);
        assert_eq!(p.status, ProposalStatus::Executed);
        assert_eq!(p.resolved_by, STRANGER);
        assert_eq!(p.executed_generation, 2);

        assert_eq!(
            execute(&mut market(), &mut a, &mut p, &STRANGER, T0 + DELAY + 1),
            Err(MarketError::ConfigProposalNotPending)
        );
        assert_eq!(a.generation, 2);
    }

    #[test]
    fn only_the_risk_authority_proposes() {
        for signer in [ADMIN, GUARDIAN, STRANGER, key(5)] {
            let mut a = asset();
            assert_eq!(
                propose(
                    &market(),
                    &mut a,
                    &signer,
                    0,
                    risk_change(|r| r.ltv_bps = 5_100),
                    T0
                ),
                Err(MarketError::NotRiskAuthority)
            );
            assert_eq!(a, asset());
        }
    }

    #[test]
    fn wrong_id_and_unchanged_values_are_rejected() {
        let mut a = asset();
        assert_eq!(
            propose(
                &market(),
                &mut a,
                &RISK,
                1,
                risk_change(|r| r.ltv_bps = 5_100),
                T0
            ),
            Err(MarketError::ConfigProposalIdMismatch)
        );
        assert_eq!(
            propose(&market(), &mut a, &RISK, 0, risk_change(|_| {}), T0),
            Err(MarketError::ConfigUnchanged)
        );
        assert_eq!(a, asset());
    }

    #[test]
    fn cancel_by_risk_or_guardian_only_and_never_executes() {
        for canceller in [RISK, GUARDIAN] {
            let mut a = asset();
            let mut p = propose_now(&mut a, risk_change(|r| r.close_factor_bps = 6_000));
            for signer in [ADMIN, STRANGER] {
                assert_eq!(
                    cancel(&market(), &mut p, &signer, T0 + 1),
                    Err(MarketError::NotConfigCanceller)
                );
            }
            assert_eq!(cancel(&market(), &mut p, &canceller, T0 + 1), Ok(()));
            assert_eq!(p.status, ProposalStatus::Cancelled);
            assert_eq!(p.resolved_by, canceller);
            assert_eq!(
                cancel(&market(), &mut p, &canceller, T0 + 2),
                Err(MarketError::ConfigProposalNotPending)
            );
            assert_eq!(
                execute(&mut market(), &mut a, &mut p, &STRANGER, T0 + DELAY),
                Err(MarketError::ConfigProposalNotPending)
            );
            assert_eq!(a.risk, RiskParams::LAUNCH_DEFAULT);
            assert_eq!(a.generation, 1);
        }
    }

    #[test]
    fn expired_proposals_cannot_execute() {
        let mut a = asset();
        let mut p = propose_now(&mut a, risk_change(|r| r.reserve_factor_bps = 1_500));
        assert_eq!(
            execute(
                &mut market(),
                &mut a,
                &mut p,
                &STRANGER,
                T0 + DELAY + EXECUTION_GRACE_SECONDS
            ),
            Err(MarketError::ConfigProposalExpired)
        );
        assert_eq!(cancel(&market(), &mut p, &GUARDIAN, T0), Ok(()));
    }

    #[test]
    fn a_changed_base_blocks_execution_but_other_groups_do_not() {
        let mut a = asset();
        let mut first = propose_now(&mut a, risk_change(|r| r.ltv_bps = 5_400));
        let mut second = propose_now(&mut a, risk_change(|r| r.close_factor_bps = 6_000));
        let mut rates = propose_now(
            &mut a,
            ConfigValue::Rates {
                params: RateParams {
                    slope1_bps: 500,
                    ..RateParams::LAUNCH_DEFAULT
                },
            },
        );
        let at = T0 + DELAY;
        execute(&mut market(), &mut a, &mut first, &STRANGER, at).unwrap();
        // `second` was built on the old risk group, so applying it would
        // silently revert the LTV change.
        assert_eq!(
            execute(&mut market(), &mut a, &mut second, &STRANGER, at),
            Err(MarketError::ConfigProposalBaseChanged)
        );
        // A different group still executes.
        execute(&mut market(), &mut a, &mut rates, &STRANGER, at).unwrap();
        assert_eq!(a.generation, 3);
        assert_eq!(a.risk.ltv_bps, 5_400);
        assert_eq!(a.rates.slope1_bps, 500);
    }

    #[test]
    fn raising_the_delay_delays_pending_proposals() {
        let mut a = asset();
        let mut pending = propose_now(&mut a, risk_change(|r| r.ltv_bps = 5_200));
        let mut raise = propose_now(
            &mut a,
            ConfigValue::TimelockDelay {
                delay_seconds: 3 * 86_400,
            },
        );
        execute(&mut market(), &mut a, &mut raise, &STRANGER, T0 + DELAY).unwrap();
        assert_eq!(a.timelock.delay_seconds, 3 * 86_400);
        assert_eq!(
            execute(&mut market(), &mut a, &mut pending, &STRANGER, T0 + DELAY),
            Err(MarketError::TimelockNotElapsed)
        );
        execute(
            &mut market(),
            &mut a,
            &mut pending,
            &STRANGER,
            T0 + 3 * 86_400,
        )
        .unwrap();
        assert_eq!(a.generation, 3);
    }

    #[test]
    fn execute_requires_an_active_market() {
        let mut a = asset();
        let mut p = propose_now(&mut a, risk_change(|r| r.ltv_bps = 5_200));
        let mut inactive = Market {
            status: MarketStatus::Uninitialized,
            ..market()
        };
        assert_eq!(
            execute(&mut inactive, &mut a, &mut p, &STRANGER, T0 + DELAY),
            Err(MarketError::MarketNotActive)
        );
    }

    // ---- Invalidation of pending computations --------------------------------

    /// Queue an Intent capturing generation G, execute a configuration change
    /// (G -> G + 1), deliver the authenticated callback: it is `Stale`, the
    /// Lock is released, no generation is bumped, and no value moves. Without
    /// the change the same callback is `Applied`.
    #[test]
    fn executed_change_stales_a_pending_intent_without_moving_value() {
        for change_config in [false, true] {
            let mut a = asset();
            let mut p = propose_now(&mut a, risk_change(|r| r.ltv_bps = 5_200));

            let bindings = [AccountBinding {
                key: key(0x51),
                role: AccountRole::Position,
                expected_owner: crate::ID,
                mint: None,
                authority: None,
                writable: true,
                generation: Some(7),
            }];
            let cipher = [1u8, 2, 3];
            let fields = [
                InputField::new(1, InputValue::U64(250)),
                InputField::new(2, InputValue::Bytes(&cipher)),
            ];
            let intent_key = key(0x52);
            let mut intent = OperationIntent {
                key: intent_key,
                owner: key(0x53),
                payer: key(0x54),
                reclaim_authority: key(0x54),
                kind: OperationKind::Borrow,
                computation_offset: 9,
                asset_generation: a.generation,
                oracle_generation: a.oracle_generation,
                rate_epoch_generation: a.rate_epoch_generation,
                accounting_borrow_index: a.accounting_borrow_index,
                accounting_index_timestamp: a.accounting_index_timestamp,
                risk_borrow_index_at_expiry: a.accounting_borrow_index,
                queued_slot: 10,
                decision_expiry_slot: 1_000,
                lock_expiry_slot: 2_000,
                position: Some(key(0x51)),
                position_generation: Some(7),
                accounts_commitment: account_commitment(&bindings),
                inputs_commitment: input_commitment(OperationKind::Borrow, &fields),
                ..OperationIntent::default()
            };
            let mut position = ProtectedAccount::new(key(0x51));
            position.generation = Generation::new(7);
            queue(&intent, &mut [&mut position], intent.lock_expiry_slot).unwrap();
            let captured = intent.asset_generation;

            if change_config {
                execute(&mut market(), &mut a, &mut p, &STRANGER, T0 + DELAY).unwrap();
                assert_eq!(a.generation, captured + 1);
            }

            let mut current = CurrentDependencies::captured_by(&intent);
            current.asset_generation = a.generation;
            let ctx = CallbackContext {
                output_authenticated: true,
                computation_offset: 9,
                intent_key,
                kind: OperationKind::Borrow,
                accounts: &bindings,
                input_fields: &fields,
                current,
                current_slot: 20,
                report: CallbackReport::Success { nonzero: true },
            };
            let decision = classify_callback(&intent, &ctx).unwrap();
            let mut ledger = 0u64;
            let result = apply_decision(&mut intent, &mut [&mut position], decision, 20).unwrap();
            if decision == CallbackDecision::Applied {
                ledger += 250;
            }
            assert!(
                !position.lock.is_active(),
                "the Lock is released either way"
            );
            assert_eq!(result.locks_released, 1);
            if change_config {
                assert_eq!(
                    decision,
                    CallbackDecision::Stale(StaleReason::AssetGeneration)
                );
                assert_eq!(intent.status, IntentStatus::Stale);
                assert_eq!(result.generations_bumped, 0);
                assert_eq!(position.generation.value(), 7);
                assert_eq!(ledger, 0, "a stale callback moves no value");
            } else {
                assert_eq!(decision, CallbackDecision::Applied);
                assert_eq!(intent.status, IntentStatus::Applied);
                assert_eq!(ledger, 250);
            }
        }
    }

    // ---- Layout -----------------------------------------------------------------

    fn sample_proposal() -> ConfigProposal {
        let mut a = asset();
        let mut p = propose_now(&mut a, risk_change(|r| r.ltv_bps = 5_300));
        p.bump = 0xFB;
        p.id = 0x0102_0304_0506_0708;
        p.resolved_at = 0x11;
        p.resolved_by = key(0x22);
        p.executed_generation = 0x33;
        p
    }

    #[test]
    fn proposal_space_is_fixed() {
        assert_eq!(ConfigGroup::INIT_SPACE, 1);
        assert_eq!(ProposalStatus::INIT_SPACE, 1);
        assert_eq!(ConfigValues::INIT_SPACE, 72);
        assert_eq!(CountCaps::INIT_SPACE, 16);
        assert_eq!(ConfigProposal::INIT_SPACE, 364);
        assert_eq!(ConfigProposal::ACCOUNT_LEN, 372);
    }

    #[test]
    fn proposal_byte_layout() {
        let p = sample_proposal();
        let mut bytes = Vec::new();
        p.try_serialize(&mut bytes).unwrap();
        assert_eq!(bytes.len(), ConfigProposal::ACCOUNT_LEN);
        assert_eq!(&bytes[..8], ConfigProposal::DISCRIMINATOR);
        assert_eq!(bytes[8..12], [1, 0xFB, 0, 0]);
        assert_eq!(bytes[12..44], [0xA0; 32]);
        let read_u64 = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        assert_eq!(read_u64(44), 0x0102_0304_0506_0708);
        assert_eq!(bytes[52..84], [4; 32]);
        assert_eq!(read_u64(84) as i64, T0);
        assert_eq!(read_u64(92) as i64, T0 + DELAY);
        assert_eq!(read_u64(100) as i64, T0 + DELAY + EXECUTION_GRACE_SECONDS);
        assert_eq!(read_u64(108), 1);
        // old_values.risk.ltv at 116, new_values.risk.ltv at 188.
        assert_eq!(u16::from_le_bytes([bytes[116], bytes[117]]), 5_000);
        assert_eq!(u16::from_le_bytes([bytes[188], bytes[189]]), 5_300);
        // old_values.timelock_delay_seconds at 116 + 68.
        assert_eq!(
            u32::from_le_bytes(bytes[184..188].try_into().unwrap()),
            TimelockState::LAUNCH_DEFAULT_DELAY_SECONDS
        );
        assert_eq!(read_u64(260), 0x11);
        assert_eq!(bytes[268..300], [0x22; 32]);
        assert_eq!(read_u64(300), 0x33);
        // Task 4.5 count caps (the test Market has caps 10 and 5).
        assert_eq!((read_u64(308), read_u64(316)), (10, 5));
        assert_eq!((read_u64(324), read_u64(332)), (10, 5));
        assert_eq!(bytes[340..], [0u8; 32]);
        let decoded = ConfigProposal::try_deserialize(&mut bytes.as_slice()).unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn proposal_addresses_are_unique_per_id() {
        let market = key(0xA0);
        let a = derive_config_proposal_address(&market, 0, &crate::ID).0;
        let b = derive_config_proposal_address(&market, 1, &crate::ID).0;
        assert_ne!(a, b);
        assert_eq!(derive_config_proposal_address(&market, 0, &crate::ID).0, a);
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

        /// `value` moved by a random offset in `[-reach, reach]`, kept in
        /// `[0, cap]`.
        fn nudge(&mut self, value: u64, reach: u64, cap: u64) -> u64 {
            let offset = self.below(2 * reach + 1);
            (value + offset).saturating_sub(reach).min(cap)
        }
    }

    /// A random risk or rate change near the current configuration: most
    /// stay in bounds, some cross bounds, movement limits, or the cross-bound.
    fn random_change(rng: &mut Lcg, a: &AssetState) -> ConfigValue {
        match rng.below(5) {
            0 | 1 => {
                let r = a.risk;
                let mut n = r;
                n.ltv_bps = rng.nudge(r.ltv_bps.into(), 700, 9_000) as u16;
                n.liquidation_threshold_bps =
                    rng.nudge(r.liquidation_threshold_bps.into(), 700, 9_500) as u16;
                n.liquidation_bonus_bps =
                    rng.nudge(r.liquidation_bonus_bps.into(), 300, 2_000) as u16;
                n.target_health_factor_bps =
                    rng.nudge(r.target_health_factor_bps.into(), 1_500, 16_000) as u16;
                if rng.below(2) == 0 {
                    n.close_factor_bps = rng.nudge(r.close_factor_bps.into(), 3_000, 11_000) as u16;
                    n.decision_expiry_slots =
                        rng.nudge(r.decision_expiry_slots.into(), 200, 2_000) as u32;
                }
                ConfigValue::Risk { params: n }
            }
            2 | 3 => {
                let r = a.rates;
                let mut n = r;
                n.base_rate_bps = rng.nudge(r.base_rate_bps.into(), 300, 3_000) as u32;
                n.slope1_bps = rng.nudge(r.slope1_bps.into(), 500, 6_000) as u32;
                n.slope2_bps = rng.nudge(r.slope2_bps.into(), 6_000, 60_000) as u32;
                n.optimal_utilization_bps =
                    rng.nudge(r.optimal_utilization_bps.into(), 1_200, 10_000) as u16;
                ConfigValue::Rates { params: n }
            }
            _ => ConfigValue::TimelockDelay {
                delay_seconds: rng.nudge(a.timelock.delay_seconds.into(), 150_000, 1_400_000)
                    as u32,
            },
        }
    }

    /// Reference predicate for an allowed configuration (independent of
    /// `check_*`): every field in range, ltv < LT, slope1 ≤ slope2, and the
    /// cross-bound with the 1% margin.
    fn reference_valid(v: &ConfigValues) -> bool {
        let r = &v.risk;
        let lb =
            u64::from(r.liquidation_threshold_bps) * (10_000 + u64::from(r.liquidation_bonus_bps));
        (1_000..=8_000).contains(&r.ltv_bps)
            && (1_000..=8_500).contains(&r.liquidation_threshold_bps)
            && (100..=1_500).contains(&r.liquidation_bonus_bps)
            && (10_100..=15_000).contains(&r.target_health_factor_bps)
            && (1_000..=10_000).contains(&r.close_factor_bps)
            && r.reserve_factor_bps <= 5_000
            && (5..=300).contains(&r.oracle_max_age_seconds)
            && (10..=500).contains(&r.oracle_max_confidence_bps)
            && (10..=1_500).contains(&r.decision_expiry_slots)
            && r.ltv_bps < r.liquidation_threshold_bps
            && lb <= 99_000_000
            && u64::from(r.target_health_factor_bps) * 10_000 >= lb + 1_000_000
            && v.rates.base_rate_bps <= 2_000
            && v.rates.slope1_bps <= 5_000
            && v.rates.slope2_bps <= 50_000
            && v.rates.slope1_bps <= v.rates.slope2_bps
            && (5_000..=9_500).contains(&v.rates.optimal_utilization_bps)
            && (150..=216_000).contains(&v.rates.band_max_age_slots)
            && (86_400..=1_209_600).contains(&v.timelock_delay_seconds)
    }

    fn within_movement(old: &ConfigValues, new: &ConfigValues) -> bool {
        let (o, n) = (&old.risk, &new.risk);
        let d = |a: u64, b: u64| a.abs_diff(b);
        d(o.ltv_bps.into(), n.ltv_bps.into()) <= 500
            && d(
                o.liquidation_threshold_bps.into(),
                n.liquidation_threshold_bps.into(),
            ) <= 500
            && d(
                o.liquidation_bonus_bps.into(),
                n.liquidation_bonus_bps.into(),
            ) <= 200
            && d(
                o.target_health_factor_bps.into(),
                n.target_health_factor_bps.into(),
            ) <= 1_000
            && d(o.close_factor_bps.into(), n.close_factor_bps.into()) <= 2_500
            && d(o.reserve_factor_bps.into(), n.reserve_factor_bps.into()) <= 1_000
            && d(
                o.decision_expiry_slots.into(),
                n.decision_expiry_slots.into(),
            ) <= 150
            && d(
                old.rates.base_rate_bps.into(),
                new.rates.base_rate_bps.into(),
            ) <= 200
            && d(old.rates.slope1_bps.into(), new.rates.slope1_bps.into()) <= 400
            && d(old.rates.slope2_bps.into(), new.rates.slope2_bps.into()) <= 5_000
            && d(
                old.rates.optimal_utilization_bps.into(),
                new.rates.optimal_utilization_bps.into(),
            ) <= 1_000
    }

    /// **Validates: Requirements 4.5, 4.12, 4.13** - a proposal is accepted
    /// exactly when the resulting configuration satisfies the reference
    /// bounds and cross-bound and every field moves within its limit; an
    /// accepted change is never executable before its delay.
    #[test]
    fn prop_bounds_movement_and_cross_bound_gate_proposals() {
        let mut rng = Lcg(0x5EED_4401);
        let mut accepted = 0;
        for _ in 0..5_000 {
            let mut a = asset();
            let change = random_change(&mut rng, &a);
            let current = GovernedConfig::of(&market(), &a);
            let target = current.with(change);
            let expected = target != current
                && reference_valid(&target.values)
                && within_movement(&current.values, &target.values);
            let before = a.clone();
            let id = a.timelock.next_proposal_id;
            match propose(&market(), &mut a, &RISK, id, change, T0) {
                Ok(mut p) => {
                    assert!(expected, "{change:?} accepted");
                    accepted += 1;
                    let early = T0 + rng.below(DELAY as u64) as i64;
                    assert_eq!(
                        execute(&mut market(), &mut a, &mut p, &STRANGER, early),
                        Err(MarketError::TimelockNotElapsed)
                    );
                }
                Err(_) => {
                    assert!(!expected, "{change:?} rejected");
                    assert_eq!(a, before);
                }
            }
        }
        assert!(accepted > 200, "the generator must reach accepted changes");
    }

    /// **Validates: Requirements 4.5, 4.6, 4.12, 4.13** - over random
    /// sequences of propose, execute (at random times), and cancel, a
    /// proposal executes at most once, never before its delay, never after
    /// being cancelled; every execution bumps the Asset State generation by
    /// exactly one (no other step bumps it); and after every step the
    /// configuration satisfies every bound and the cross-bound, with each
    /// execution moving each field within its limit. A failed step changes
    /// nothing.
    #[test]
    fn prop_timelock_state_machine_invariants() {
        let mut rng = Lcg(0x5EED_4402);
        let mut executions = 0usize;
        for _ in 0..300 {
            let mut a = asset();
            let mut now = T0;
            let mut proposals: Vec<ConfigProposal> = Vec::new();
            let mut executed_ids: Vec<u64> = Vec::new();
            let mut cancelled_ids: Vec<u64> = Vec::new();
            let start_generation = a.generation;
            let mut run_executions = 0u64;
            for _ in 0..30 {
                now += rng.below(DELAY as u64 / 2) as i64;
                let before = ConfigValues::of(&a);
                let generation_before = a.generation;
                match rng.below(4) {
                    0 | 1 => {
                        let change = random_change(&mut rng, &a);
                        let id = a.timelock.next_proposal_id;
                        if let Ok(p) = propose(&market(), &mut a, &RISK, id, change, now) {
                            proposals.push(p);
                        }
                        assert_eq!(a.generation, generation_before);
                    }
                    2 if !proposals.is_empty() => {
                        let index = rng.below(proposals.len() as u64) as usize;
                        let ready = ready_at(&a, &proposals[index]).unwrap();
                        let p = &mut proposals[index];
                        let was_pending = p.status == ProposalStatus::Pending;
                        match execute(&mut market(), &mut a, p, &STRANGER, now) {
                            Ok(done) => {
                                assert!(was_pending);
                                assert!(now >= ready && now >= p.earliest_execute_at);
                                assert!(!executed_ids.contains(&p.id));
                                assert!(!cancelled_ids.contains(&p.id));
                                executed_ids.push(p.id);
                                assert_eq!(done.new_generation, generation_before + 1);
                                assert_eq!(a.generation, generation_before + 1);
                                assert!(within_movement(&before, &ConfigValues::of(&a)));
                                run_executions += 1;
                            }
                            Err(_) => {
                                assert_eq!(ConfigValues::of(&a), before);
                                assert_eq!(a.generation, generation_before);
                            }
                        }
                    }
                    _ if !proposals.is_empty() => {
                        let index = rng.below(proposals.len() as u64) as usize;
                        let signer = [RISK, GUARDIAN, STRANGER][rng.below(3) as usize];
                        let p = &mut proposals[index];
                        if cancel(&market(), p, &signer, now).is_ok() {
                            assert!(!executed_ids.contains(&p.id));
                            cancelled_ids.push(p.id);
                        }
                        assert_eq!(a.generation, generation_before);
                    }
                    _ => {}
                }
                assert!(reference_valid(&ConfigValues::of(&a)), "{a:?}");
                assert_eq!(check_cross_bound(&a.risk), Ok(()));
            }
            assert_eq!(a.generation, start_generation + run_executions);
            for p in &proposals {
                match p.status {
                    ProposalStatus::Executed => assert!(executed_ids.contains(&p.id)),
                    ProposalStatus::Cancelled => assert!(cancelled_ids.contains(&p.id)),
                    ProposalStatus::Pending => {}
                }
            }
            executions += run_executions as usize;
        }
        assert!(executions > 100, "the generator must reach executions");
    }
}
