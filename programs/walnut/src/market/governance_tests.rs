//! Task 4.6 governance checkpoint (Requirement 4).
//!
//! Drives the real pure governance transitions of Tasks 4.1-4.5 (roles,
//! pauses, the configuration timelock, cap decreases, and the public count
//! transitions) with randomized and exhaustive-short action sequences, and
//! compares every step with an independent reference model written from
//! Requirement 4 and the module docs. After every step it asserts:
//!
//! * Unauthorized access: a privileged action by any signer other than the
//!   exact role holder fails with that role's error and changes nothing.
//! * Timelock bypass: no timelocked value changes before its delay, measured
//!   with the larger of the proposal-time and current delay, and never at or
//!   after the proposal's stored `expires_at`. Only cap decreases apply
//!   immediately, and only downward.
//! * Pause expiry: an expired guardian pause never blocks, a guardian-only
//!   pause never lasts longer than `GUARDIAN_MAX_PAUSE_SLOTS`, and rescue
//!   actions are never blocked.
//! * Generation invalidation: each executed change and each cap decrease
//!   bumps the Asset State generation exactly once and an Intent captured
//!   before it classifies `Stale(AssetGeneration)`. Nothing else bumps it.
//! * Cap boundaries and counts: `value == cap` passes, `cap + 1` resolves to
//!   zero, counts match the Positions, and the bound ordering always holds.
//! * Cross-bound: `L × (1 + bonus) <= 0.99` and `T >= L × (1 + bonus) + 0.01`
//!   in exact basis-point arithmetic.
//!
//! Property tests are named `prop_*` (run by `pnpm run test:rust:property`).

use anchor_lang::prelude::Pubkey;
use anchor_lang::AccountSerialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;

use super::caps::{
    borrow_cap_outcome, check_caps_bounds, check_count_caps_bounds, check_position_debt_cap,
    check_supply_cap, check_total_debt_cap, decrease_caps, new_risk_outcome,
    on_position_initialized, reserve_debt_slot, settle_debt_intent, supply_cap_outcome, CapCheck,
    DebtSlot, DebtTransition, NewRiskOutcome, COUNT_CAP_RAISE_FLOOR, MAX_POSITION_COUNT_CAP,
    MAX_USDC_CAP, USDC, USDC_CAP_RAISE_FLOOR,
};
use super::config::{
    bounds, cancel, check_cross_bound, check_rate_bounds, check_risk_bounds, execute, propose,
    Bound, ConfigProposal, ConfigValue, GovernedConfig, ProposalStatus, EXECUTION_GRACE_SECONDS,
    MAX_TIMELOCK_DELAY_SECONDS, MIN_TIMELOCK_DELAY_SECONDS,
};
use super::pause::{
    clear_guardian_pause, effective_pause_flags, ensure_action_allowed, guardian_pause,
    guardian_pause_active, renew_guardian_pause, set_pause, Action, GUARDIAN_MAX_PAUSE_SLOTS,
    GUARDIAN_PAUSE_COOLDOWN_SLOTS,
};
use super::roles::{
    accept_admin, cancel_admin_transfer, propose_admin, set_role, set_upgrade_authority, RoleKind,
};
use super::state::{
    pause_flags, AssetState, Caps, CountCaps, Market, MarketStatus, PauseState, RateParams,
    RiskParams, TimelockState,
};
use super::validation::{MarketError, ProgramAccountView, BPF_LOADER_UPGRADEABLE_ID};
use crate::operation::{
    account_commitment, apply_decision, classify_callback, input_commitment, queue, AccountBinding,
    AccountRole, CallbackContext, CallbackDecision, CallbackReport, CurrentDependencies,
    InputField, InputValue, IntentStatus, OperationIntent, OperationKind, StaleReason,
};
use crate::state::{Generation, ProtectedAccount};

// ---- Fixture ----------------------------------------------------------------------

const T0: i64 = 1_700_000_000;
const SLOT0: u64 = 1_000_000;
const DELAY: i64 = TimelockState::LAUNCH_DEFAULT_DELAY_SECONDS as i64;
const DAY: i64 = 86_400;
const MARKET_KEY: Pubkey = Pubkey::new_from_array([0xA0; 32]);
const PROGRAM_DATA_KEY: Pubkey = Pubkey::new_from_array([0xA1; 32]);
const ZERO: Pubkey = Pubkey::new_from_array([0; 32]);

// Indices into `World::keys`. The first `SIGNERS` keys can sign.
// 1 = key 2 (pending-admin candidate), 2 = upgrade, 4 = treasury,
// 8 = off-curve multisig PDA, 9 = zero, 10 = the Market PDA.
const ADMIN: usize = 0;
const RISK: usize = 3;
const GUARDIAN: usize = 5;
const ATTACKER: usize = 6;
const ALT: usize = 7;
const SIGNERS: usize = 9;
const KEYS: usize = 11;

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}

/// A Squads-style multisig vault PDA (off the ed25519 curve).
fn multisig_pda(seed: u8) -> Pubkey {
    let squads = Pubkey::from_str_const("SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf");
    let multisig = key(seed);
    let seeds: [&[u8]; 4] = [b"multisig", multisig.as_ref(), b"vault", &[0]];
    let (address, _) = anchor_lang::derive_program_address(&seeds, &squads.to_bytes());
    Pubkey::new_from_array(address)
}

/// The Market's own accounts, restated from the fixture below.
fn protocol_keys() -> [Pubkey; 8] {
    [
        MARKET_KEY,
        key(0x41),
        key(0x42),
        key(0x43),
        key(0x44),
        key(0x47),
        key(0x48),
        crate::ID,
    ]
}

/// admin 1, upgrade 3, risk 4, treasury 5, guardian 6; count caps 6 / 3.
fn market() -> Market {
    Market {
        layout_version: 1,
        status: MarketStatus::Active,
        bump: 0xFE,
        vault_authority_bump: 0xFD,
        admin: key(1),
        pending_admin: ZERO,
        upgrade_authority: key(3),
        risk_authority: key(4),
        treasury_authority: key(5),
        guardian_authority: key(6),
        asset_state: key(0x41),
        pool_state: key(0x42),
        recovery_position: key(0x43),
        vault_authority: key(0x44),
        wsol_mint: key(0x45),
        usdc_mint: key(0x46),
        wsol_vault: key(0x47),
        usdc_vault: key(0x48),
        treasury_usdc_account: ZERO,
        position_count: 0,
        debt_position_count: 0,
        unsettled_recovery_claim_count: 0,
        position_count_cap: 6,
        debt_position_count_cap: 3,
        initialized_slot: 1,
        debt_position_reservations: 0,
        reserved: [0; 120],
    }
}

fn asset() -> AssetState {
    AssetState {
        layout_version: 1,
        bump: 0xFC,
        market: MARKET_KEY,
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
            supply_cap_usdc: 1_000_000 * USDC,
            total_debt_cap_usdc: 500_000 * USDC,
            position_debt_cap_usdc: 10_000 * USDC,
        },
        pauses: PauseState::default(),
        timelock: TimelockState {
            delay_seconds: TimelockState::LAUNCH_DEFAULT_DELAY_SECONDS,
            next_proposal_id: 0,
        },
        reserved: [0; 128],
    }
}

fn program_state() -> Vec<u8> {
    let mut data = 2u32.to_le_bytes().to_vec();
    data.extend(PROGRAM_DATA_KEY.to_bytes());
    data
}

fn program_data_state(authority: Pubkey) -> Vec<u8> {
    let mut data = 3u32.to_le_bytes().to_vec();
    data.extend(7u64.to_le_bytes());
    data.push(1);
    data.extend(authority.to_bytes());
    data
}

fn views<'a>(
    program: &'a [u8],
    program_data: &'a [u8],
) -> (ProgramAccountView<'a>, ProgramAccountView<'a>) {
    (
        ProgramAccountView {
            key: crate::ID,
            owner: BPF_LOADER_UPGRADEABLE_ID,
            data: program,
        },
        ProgramAccountView {
            key: PROGRAM_DATA_KEY,
            owner: BPF_LOADER_UPGRADEABLE_ID,
            data: program_data,
        },
    )
}

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

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len() as u64) as usize]
    }
}

// ---- Independent reference rules ----------------------------------------------------

/// Requirement 4.13 with the documented 1% margin, scaled by 10^8:
/// `L(1 + b) <= 0.99` and `T >= L(1 + b) + 0.01`.
fn ref_cross_bound(lt_bps: u64, bonus_bps: u64, thf_bps: u64) -> bool {
    let lb = lt_bps * (10_000 + bonus_bps);
    lb <= 99_000_000 && thf_bps * 10_000 >= lb + 1_000_000
}

fn in_bound(b: Bound, v: u64) -> bool {
    b.min <= v && v <= b.max
}

fn step_ok(b: Bound, old: u64, new: u64) -> bool {
    let distance = old.abs_diff(new);
    distance <= b.max_step
}

fn risk_list(r: &RiskParams) -> [(u64, Bound); 9] {
    [
        (r.ltv_bps.into(), bounds::LTV),
        (
            r.liquidation_threshold_bps.into(),
            bounds::LIQUIDATION_THRESHOLD,
        ),
        (r.liquidation_bonus_bps.into(), bounds::LIQUIDATION_BONUS),
        (
            r.target_health_factor_bps.into(),
            bounds::TARGET_HEALTH_FACTOR,
        ),
        (r.close_factor_bps.into(), bounds::CLOSE_FACTOR),
        (r.reserve_factor_bps.into(), bounds::RESERVE_FACTOR),
        (
            r.oracle_max_age_seconds.into(),
            bounds::ORACLE_MAX_AGE_SECONDS,
        ),
        (
            r.oracle_max_confidence_bps.into(),
            bounds::ORACLE_MAX_CONFIDENCE,
        ),
        (
            r.decision_expiry_slots.into(),
            bounds::DECISION_EXPIRY_SLOTS,
        ),
    ]
}

fn rate_list(r: &RateParams) -> [(u64, Bound); 5] {
    [
        (r.base_rate_bps.into(), bounds::BASE_RATE),
        (r.slope1_bps.into(), bounds::SLOPE1),
        (r.slope2_bps.into(), bounds::SLOPE2),
        (
            r.optimal_utilization_bps.into(),
            bounds::OPTIMAL_UTILIZATION,
        ),
        (r.band_max_age_slots, bounds::BAND_MAX_AGE_SLOTS),
    ]
}

fn raise_ok(old: u64, new: u64, floor: u64) -> bool {
    new <= old || new <= core::cmp::max(old.saturating_mul(2), old.saturating_add(floor))
}

fn group_value(c: &GovernedConfig, like: &ConfigValue) -> ConfigValue {
    match like {
        ConfigValue::Risk { .. } => ConfigValue::Risk {
            params: c.values.risk,
        },
        ConfigValue::Rates { .. } => ConfigValue::Rates {
            params: c.values.rates,
        },
        ConfigValue::Caps { .. } => ConfigValue::Caps {
            caps: c.values.caps,
        },
        ConfigValue::TimelockDelay { .. } => ConfigValue::TimelockDelay {
            delay_seconds: c.values.timelock_delay_seconds,
        },
        ConfigValue::CountCaps { .. } => ConfigValue::CountCaps { caps: c.count_caps },
    }
}

fn apply_value(mut c: GovernedConfig, v: &ConfigValue) -> GovernedConfig {
    match *v {
        ConfigValue::Risk { params } => c.values.risk = params,
        ConfigValue::Rates { params } => c.values.rates = params,
        ConfigValue::Caps { caps } => c.values.caps = caps,
        ConfigValue::TimelockDelay { delay_seconds } => {
            c.values.timelock_delay_seconds = delay_seconds
        }
        ConfigValue::CountCaps { caps } => c.count_caps = caps,
    }
    c
}

/// The exact error the documented rules give for moving `cur` to `change`.
fn ref_validate(cur: &GovernedConfig, change: &ConfigValue) -> Result<(), MarketError> {
    use MarketError::*;
    if group_value(cur, change) == *change {
        return Err(ConfigUnchanged);
    }
    match change {
        ConfigValue::Risk { params: n } => {
            let new = risk_list(n);
            if new.iter().any(|(v, b)| !in_bound(*b, *v))
                || n.ltv_bps >= n.liquidation_threshold_bps
            {
                return Err(ConfigOutOfBounds);
            }
            if !ref_cross_bound(
                n.liquidation_threshold_bps.into(),
                n.liquidation_bonus_bps.into(),
                n.target_health_factor_bps.into(),
            ) {
                return Err(LiquidationCrossBound);
            }
            let old = risk_list(&cur.values.risk);
            if old
                .iter()
                .zip(new.iter())
                .any(|((o, b), (v, _))| !step_ok(*b, *o, *v))
            {
                return Err(ConfigMovementTooLarge);
            }
        }
        ConfigValue::Rates { params: n } => {
            let new = rate_list(n);
            if new.iter().any(|(v, b)| !in_bound(*b, *v)) || n.slope1_bps > n.slope2_bps {
                return Err(ConfigOutOfBounds);
            }
            let old = rate_list(&cur.values.rates);
            if old
                .iter()
                .zip(new.iter())
                .any(|((o, b), (v, _))| !step_ok(*b, *o, *v))
            {
                return Err(ConfigMovementTooLarge);
            }
        }
        ConfigValue::Caps { caps: n } => {
            if n.supply_cap_usdc > MAX_USDC_CAP
                || n.total_debt_cap_usdc > n.supply_cap_usdc
                || n.position_debt_cap_usdc > n.total_debt_cap_usdc
            {
                return Err(ConfigOutOfBounds);
            }
            let o = cur.values.caps;
            if !raise_ok(o.supply_cap_usdc, n.supply_cap_usdc, USDC_CAP_RAISE_FLOOR)
                || !raise_ok(
                    o.total_debt_cap_usdc,
                    n.total_debt_cap_usdc,
                    USDC_CAP_RAISE_FLOOR,
                )
                || !raise_ok(
                    o.position_debt_cap_usdc,
                    n.position_debt_cap_usdc,
                    USDC_CAP_RAISE_FLOOR,
                )
            {
                return Err(ConfigMovementTooLarge);
            }
        }
        ConfigValue::TimelockDelay { delay_seconds } => {
            if *delay_seconds < 86_400 || *delay_seconds > 1_209_600 {
                return Err(TimelockDelayOutOfBounds);
            }
        }
        ConfigValue::CountCaps { caps: n } => {
            if n.position_count_cap > MAX_POSITION_COUNT_CAP
                || n.debt_position_count_cap > n.position_count_cap
            {
                return Err(ConfigOutOfBounds);
            }
            let o = cur.count_caps;
            if !raise_ok(
                o.position_count_cap,
                n.position_count_cap,
                COUNT_CAP_RAISE_FLOOR,
            ) || !raise_ok(
                o.debt_position_count_cap,
                n.debt_position_count_cap,
                COUNT_CAP_RAISE_FLOOR,
            ) {
                return Err(ConfigMovementTooLarge);
            }
        }
    }
    Ok(())
}

/// Whether every field of a cap group is at most its current value.
fn ref_is_decrease(cur: &GovernedConfig, change: &ConfigValue) -> bool {
    match change {
        ConfigValue::Caps { caps: n } => {
            let o = cur.values.caps;
            n.supply_cap_usdc <= o.supply_cap_usdc
                && n.total_debt_cap_usdc <= o.total_debt_cap_usdc
                && n.position_debt_cap_usdc <= o.position_debt_cap_usdc
        }
        ConfigValue::CountCaps { caps: n } => {
            n.position_count_cap <= cur.count_caps.position_count_cap
                && n.debt_position_count_cap <= cur.count_caps.debt_position_count_cap
        }
        _ => false,
    }
}

/// Every governed field as a flat list, for "did anything rise" checks.
fn flat(c: &GovernedConfig) -> Vec<u64> {
    let mut v: Vec<u64> = risk_list(&c.values.risk).iter().map(|x| x.0).collect();
    v.extend(rate_list(&c.values.rates).iter().map(|x| x.0));
    v.extend([
        c.values.caps.supply_cap_usdc,
        c.values.caps.total_debt_cap_usdc,
        c.values.caps.position_debt_cap_usdc,
        c.values.timelock_delay_seconds.into(),
        c.count_caps.position_count_cap,
        c.count_caps.debt_position_count_cap,
    ]);
    v
}

fn ref_pause_flags(flags: u16) -> Result<(), MarketError> {
    if flags == 0 || flags & !0x03FF != 0 {
        return Err(MarketError::InvalidPauseFlags);
    }
    if flags & (pause_flags::REPAY | pause_flags::ADD_COLLATERAL) != 0 {
        return Err(MarketError::PauseNotAllowed);
    }
    Ok(())
}

// ---- Change builders ------------------------------------------------------------------

fn near(rng: &mut Lcg, old: u64, b: Bound) -> u64 {
    let step = b.max_step.min(1 << 32);
    match rng.below(9) {
        0 => old.saturating_add(step),
        1 => old.saturating_sub(step),
        2 => old.saturating_add(step + 1),
        3 => old.saturating_sub(step + 1),
        4 => old.saturating_add(rng.below(step + 1)),
        5 => old.saturating_sub(rng.below(step + 1)),
        6 => b.min.saturating_sub(rng.below(2)),
        7 => b.max.saturating_add(rng.below(2)),
        _ => old,
    }
}

fn u16_of(v: u64) -> u16 {
    u16::try_from(v).unwrap_or(u16::MAX)
}

fn u32_of(v: u64) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

fn cap_near(rng: &mut Lcg, old: u64, floor: u64, max: u64) -> u64 {
    match rng.below(11) {
        0 => old.saturating_sub(1),
        1 => old / 2,
        2 => old.saturating_add(1),
        3 => old.saturating_mul(2),
        4 => old.saturating_mul(2).saturating_add(1),
        5 => old.saturating_add(floor),
        6 => old.saturating_add(floor + 1),
        7 => max,
        8 => max + 1,
        9 => old.saturating_sub(rng.below(old / 4 + 1)),
        _ => 0,
    }
}

/// A change of group `kind % 5` near `cur`: most are valid, some cross a
/// bound, a movement limit, or the ordering.
fn build_change(kind: u8, seed: u64, cur: &GovernedConfig) -> ConfigValue {
    let mut rng = Lcg(seed ^ 0x9E37_79B9_7F4A_7C15);
    match kind % 5 {
        0 => {
            let mut n = cur.values.risk;
            for _ in 0..=rng.below(3) {
                let field = rng.below(9) as usize;
                let (old, b) = risk_list(&n)[field];
                let v = near(&mut rng, old, b);
                match field {
                    0 => n.ltv_bps = u16_of(v),
                    1 => n.liquidation_threshold_bps = u16_of(v),
                    2 => n.liquidation_bonus_bps = u16_of(v),
                    3 => n.target_health_factor_bps = u16_of(v),
                    4 => n.close_factor_bps = u16_of(v),
                    5 => n.reserve_factor_bps = u16_of(v),
                    6 => n.oracle_max_age_seconds = u32_of(v),
                    7 => n.oracle_max_confidence_bps = u16_of(v),
                    _ => n.decision_expiry_slots = u32_of(v),
                }
            }
            ConfigValue::Risk { params: n }
        }
        1 => {
            let mut n = cur.values.rates;
            for _ in 0..=rng.below(2) {
                let field = rng.below(5) as usize;
                let (old, b) = rate_list(&n)[field];
                let v = near(&mut rng, old, b);
                match field {
                    0 => n.base_rate_bps = u32_of(v),
                    1 => n.slope1_bps = u32_of(v),
                    2 => n.slope2_bps = u32_of(v),
                    3 => n.optimal_utilization_bps = u16_of(v),
                    _ => n.band_max_age_slots = v,
                }
            }
            ConfigValue::Rates { params: n }
        }
        2 => {
            let mut n = cur.values.caps;
            for _ in 0..=rng.below(2) {
                match rng.below(3) {
                    0 => {
                        n.supply_cap_usdc = cap_near(
                            &mut rng,
                            n.supply_cap_usdc,
                            USDC_CAP_RAISE_FLOOR,
                            MAX_USDC_CAP,
                        )
                    }
                    1 => {
                        n.total_debt_cap_usdc = cap_near(
                            &mut rng,
                            n.total_debt_cap_usdc,
                            USDC_CAP_RAISE_FLOOR,
                            n.supply_cap_usdc,
                        )
                    }
                    _ => {
                        n.position_debt_cap_usdc = cap_near(
                            &mut rng,
                            n.position_debt_cap_usdc,
                            USDC_CAP_RAISE_FLOOR,
                            n.total_debt_cap_usdc,
                        )
                    }
                }
            }
            ConfigValue::Caps { caps: n }
        }
        3 => {
            let old = cur.values.timelock_delay_seconds;
            let delay_seconds = rng.pick(&[
                MIN_TIMELOCK_DELAY_SECONDS - 1,
                MIN_TIMELOCK_DELAY_SECONDS,
                old / 2,
                old.saturating_add(3_600),
                old.saturating_mul(3),
                MAX_TIMELOCK_DELAY_SECONDS,
                MAX_TIMELOCK_DELAY_SECONDS + 1,
                old,
            ]);
            ConfigValue::TimelockDelay { delay_seconds }
        }
        _ => {
            let mut n = cur.count_caps;
            if rng.below(2) == 0 {
                n.position_count_cap = cap_near(
                    &mut rng,
                    n.position_count_cap,
                    COUNT_CAP_RAISE_FLOOR,
                    MAX_POSITION_COUNT_CAP,
                );
            } else {
                n.debt_position_count_cap = cap_near(
                    &mut rng,
                    n.debt_position_count_cap,
                    COUNT_CAP_RAISE_FLOOR,
                    n.position_count_cap,
                );
            }
            ConfigValue::CountCaps { caps: n }
        }
    }
}

// ---- World and reference model ------------------------------------------------------

#[derive(Clone)]
struct World {
    keys: [Pubkey; KEYS],
    market: Market,
    asset: AssetState,
    proposals: Vec<ConfigProposal>,
    has_debt: Vec<bool>,
    /// Pending Intents that may hold a debt slot: (Position, action).
    intents: Vec<(usize, Action)>,
    now: i64,
    slot: u64,
}

impl World {
    fn new() -> Self {
        Self {
            keys: [
                key(1),
                key(2),
                key(3),
                key(4),
                key(5),
                key(6),
                key(9),
                key(10),
                multisig_pda(0x77),
                ZERO,
                MARKET_KEY,
            ],
            market: market(),
            asset: asset(),
            proposals: Vec::new(),
            has_debt: Vec::new(),
            intents: Vec::new(),
            now: T0,
            slot: SLOT0,
        }
    }

    fn locked(&self, position: usize) -> bool {
        self.intents.iter().any(|(p, _)| *p == position)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RefProposal {
    base: ConfigValue,
    new: ConfigValue,
    proposed_at: i64,
    earliest: i64,
    expires: i64,
    status: ProposalStatus,
}

#[derive(Clone, Debug)]
struct Model {
    admin: Pubkey,
    pending: Pubkey,
    upgrade: Pubkey,
    risk: Pubkey,
    treasury: Pubkey,
    guardian: Pubkey,
    flags: u16,
    gflags: u16,
    gexpiry: u64,
    /// Slot at which the current guardian pause was set fresh.
    gstart: u64,
    /// Whether governance renewed the current guardian pause.
    renewed: bool,
    config: GovernedConfig,
    generation: u64,
    next_id: u64,
    proposals: Vec<RefProposal>,
    position_count: u64,
    debt_count: u64,
    reservations: u64,
    has_debt: Vec<bool>,
}

impl Model {
    fn initial() -> Self {
        let a = asset();
        let m = market();
        Self {
            admin: m.admin,
            pending: ZERO,
            upgrade: m.upgrade_authority,
            risk: m.risk_authority,
            treasury: m.treasury_authority,
            guardian: m.guardian_authority,
            flags: 0,
            gflags: 0,
            gexpiry: 0,
            gstart: 0,
            renewed: false,
            config: GovernedConfig::of(&m, &a),
            generation: a.generation,
            next_id: 0,
            proposals: Vec::new(),
            position_count: 0,
            debt_count: 0,
            reservations: 0,
            has_debt: Vec::new(),
        }
    }

    fn role(&self, kind: RoleKind) -> Pubkey {
        match kind {
            RoleKind::Upgrade => self.upgrade,
            RoleKind::Risk => self.risk,
            RoleKind::Treasury => self.treasury,
            RoleKind::Guardian => self.guardian,
        }
    }

    fn set_role(&mut self, kind: RoleKind, k: Pubkey) {
        match kind {
            RoleKind::Upgrade => self.upgrade = k,
            RoleKind::Risk => self.risk = k,
            RoleKind::Treasury => self.treasury = k,
            RoleKind::Guardian => self.guardian = k,
        }
    }

    fn guardian_active(&self, slot: u64) -> bool {
        self.gflags != 0 && slot < self.gexpiry
    }

    fn ready(&self, p: &RefProposal) -> i64 {
        p.earliest
            .max(p.proposed_at + i64::from(self.config.values.timelock_delay_seconds))
    }
}

#[derive(Clone, Copy, Debug)]
enum Time {
    Secs(i64),
    ToReady(usize, i64),
    ToExpiry(usize, i64),
    Slots(u64),
    ToGuardianExpiry(u64),
    ToCooldownEnd(i64),
}

#[derive(Clone, Copy, Debug)]
enum How {
    Applied(bool),
    Contradict,
    NotApplied(CallbackDecision),
}

#[derive(Clone, Copy, Debug)]
enum Step {
    ProposeAdmin {
        signer: usize,
        new: usize,
    },
    AcceptAdmin {
        signer: usize,
    },
    CancelAdmin {
        signer: usize,
    },
    SetRole {
        signer: usize,
        kind: RoleKind,
        new: usize,
    },
    SetUpgrade {
        signer: usize,
        new: usize,
        loader_agrees: bool,
    },
    GuardianPause {
        signer: usize,
        flags: u16,
        duration: u64,
    },
    SetPause {
        signer: usize,
        flags: u16,
        paused: bool,
    },
    Renew {
        signer: usize,
        extra: u64,
    },
    ClearGuardian {
        signer: usize,
    },
    Propose {
        signer: usize,
        id_skew: i64,
        kind: u8,
        seed: u64,
    },
    Execute {
        signer: usize,
        which: usize,
    },
    Cancel {
        signer: usize,
        which: usize,
    },
    Decrease {
        signer: usize,
        kind: u8,
        seed: u64,
    },
    Advance(Time),
    InitPosition,
    QueueBorrow {
        pos: usize,
    },
    QueueRepay {
        pos: usize,
    },
    Settle {
        which: usize,
        how: How,
    },
}

impl Step {
    fn name(&self) -> &'static str {
        match self {
            Step::ProposeAdmin { .. } => "propose_admin",
            Step::AcceptAdmin { .. } => "accept_admin",
            Step::CancelAdmin { .. } => "cancel_admin_transfer",
            Step::SetRole { .. } => "set_role",
            Step::SetUpgrade { .. } => "set_upgrade_authority",
            Step::GuardianPause { .. } => "guardian_pause",
            Step::SetPause { .. } => "set_pause",
            Step::Renew { .. } => "renew_guardian_pause",
            Step::ClearGuardian { .. } => "clear_guardian_pause",
            Step::Propose { .. } => "propose",
            Step::Execute { .. } => "execute",
            Step::Cancel { .. } => "cancel",
            Step::Decrease { .. } => "decrease_caps",
            Step::Advance(_) => "advance",
            Step::InitPosition => "on_position_initialized",
            Step::QueueBorrow { .. } => "reserve_debt_slot",
            Step::QueueRepay { .. } => "queue_repay",
            Step::Settle { .. } => "settle_debt_intent",
        }
    }
}

/// Per step kind: [skipped, rejected, accepted, rejected-unauthorized].
type Stats = BTreeMap<&'static str, [u64; 4]>;

fn snapshot(w: &World) -> Vec<u8> {
    let mut b = Vec::new();
    w.market.try_serialize(&mut b).unwrap();
    w.asset.try_serialize(&mut b).unwrap();
    for p in &w.proposals {
        p.try_serialize(&mut b).unwrap();
    }
    b.extend(w.has_debt.iter().map(|d| u8::from(*d)));
    b.extend((w.intents.len() as u64).to_le_bytes());
    b
}

fn is_auth_error(e: &MarketError) -> bool {
    use MarketError::*;
    matches!(
        e,
        NotMarketAdmin
            | NotPendingAdmin
            | NoPendingAdmin
            | NotRiskAuthority
            | NotGuardianAuthority
            | NotConfigCanceller
    )
}

const CIPHER: [u8; 3] = [1, 2, 3];

fn probe_bindings() -> [AccountBinding; 1] {
    [AccountBinding {
        key: key(0x51),
        role: AccountRole::Position,
        expected_owner: crate::ID,
        mint: None,
        authority: None,
        writable: true,
        generation: Some(7),
    }]
}

fn probe_fields() -> [InputField<'static>; 2] {
    [
        InputField::new(1, InputValue::U64(250)),
        InputField::new(2, InputValue::Bytes(&CIPHER)),
    ]
}

/// A queued Borrow Intent template capturing Asset State generation 0.
fn probe_template() -> &'static OperationIntent {
    static TEMPLATE: OnceLock<OperationIntent> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        let a = asset();
        OperationIntent {
            key: key(0x52),
            owner: key(0x53),
            payer: key(0x54),
            reclaim_authority: key(0x54),
            kind: OperationKind::Borrow,
            computation_offset: 9,
            asset_generation: 0,
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
            accounts_commitment: account_commitment(&probe_bindings()),
            inputs_commitment: input_commitment(OperationKind::Borrow, &probe_fields()),
            ..OperationIntent::default()
        }
    })
}

/// The callback decision for an Intent captured at `captured` when the Asset
/// State generation is now `current` (every other dependency unchanged).
fn classify_probe(captured: u64, current: u64) -> CallbackDecision {
    let mut intent = *probe_template();
    intent.asset_generation = captured;
    let bindings = probe_bindings();
    let fields = probe_fields();
    let mut deps = CurrentDependencies::captured_by(&intent);
    deps.asset_generation = current;
    let ctx = CallbackContext {
        output_authenticated: true,
        computation_offset: 9,
        intent_key: intent.key,
        kind: OperationKind::Borrow,
        accounts: &bindings,
        input_fields: &fields,
        current: deps,
        current_slot: 20,
        report: CallbackReport::Success { nonzero: true },
    };
    classify_callback(&intent, &ctx).unwrap()
}

// ---- One step: model expectation, real call, comparison ---------------------------------

fn run_step(w: &mut World, m: &mut Model, step: Step, stats: &mut Stats, index: u64) -> bool {
    use MarketError::*;
    let before_bytes = snapshot(w);
    let before_config = GovernedConfig::of(&w.market, &w.asset);
    let before_generation = w.asset.generation;
    let before_delay = i64::from(w.asset.timelock.delay_seconds);
    let mut next = m.clone();
    let protocol = protocol_keys();
    let slot = w.slot;
    let now = w.now;

    let outcome: Option<(Result<(), MarketError>, Result<(), MarketError>)> = match step {
        Step::ProposeAdmin { signer, new } => {
            let (s, n) = (w.keys[signer], w.keys[new]);
            let expect = if s != m.admin {
                Err(NotMarketAdmin)
            } else if n == ZERO {
                Err(InvalidAuthority)
            } else if protocol.contains(&n) {
                Err(InvalidRoleKey)
            } else if n == m.admin {
                Err(RoleKeyUnchanged)
            } else {
                next.pending = n;
                Ok(())
            };
            let real = propose_admin(&mut w.market, &MARKET_KEY, &s, n).map(drop);
            Some((expect, real))
        }
        Step::AcceptAdmin { signer } => {
            let s = w.keys[signer];
            let expect = if m.pending == ZERO {
                Err(NoPendingAdmin)
            } else if s != m.pending {
                Err(NotPendingAdmin)
            } else {
                next.admin = m.pending;
                next.pending = ZERO;
                Ok(())
            };
            Some((expect, accept_admin(&mut w.market, &s).map(drop)))
        }
        Step::CancelAdmin { signer } => {
            let s = w.keys[signer];
            let expect = if s != m.admin {
                Err(NotMarketAdmin)
            } else if m.pending == ZERO {
                Err(NoPendingAdmin)
            } else {
                next.pending = ZERO;
                Ok(())
            };
            Some((expect, cancel_admin_transfer(&mut w.market, &s).map(drop)))
        }
        Step::SetRole { signer, kind, new } => {
            let (s, n) = (w.keys[signer], w.keys[new]);
            let mut others = RoleKind::ALL.iter().filter(|k| **k != kind);
            let expect = if s != m.admin {
                Err(NotMarketAdmin)
            } else if n == ZERO {
                Err(InvalidAuthority)
            } else if protocol.contains(&n) {
                Err(InvalidRoleKey)
            } else if n == m.role(kind) {
                Err(RoleKeyUnchanged)
            } else if others.any(|k| m.role(*k) == n) {
                Err(RoleKeyNotDistinct)
            } else {
                next.set_role(kind, n);
                Ok(())
            };
            let real = set_role(&mut w.market, &MARKET_KEY, &s, kind, n).map(drop);
            Some((expect, real))
        }
        Step::SetUpgrade {
            signer,
            new,
            loader_agrees,
        } => {
            let (s, n) = (w.keys[signer], w.keys[new]);
            let loader = if loader_agrees { n } else { key(0x99) };
            let expect = if s != m.admin {
                Err(NotMarketAdmin)
            } else if n == ZERO {
                Err(InvalidAuthority)
            } else if protocol.contains(&n) {
                Err(InvalidRoleKey)
            } else if loader != n {
                Err(UpgradeAuthorityMismatch)
            } else if n == m.upgrade {
                Err(RoleKeyUnchanged)
            } else if [m.risk, m.treasury, m.guardian].contains(&n) {
                Err(RoleKeyNotDistinct)
            } else {
                next.upgrade = n;
                Ok(())
            };
            let program = program_state();
            let data = program_data_state(loader);
            let (pv, dv) = views(&program, &data);
            let real = set_upgrade_authority(&mut w.market, &MARKET_KEY, &s, &pv, &dv, n);
            Some((expect, real.map(drop)))
        }
        Step::GuardianPause {
            signer,
            flags,
            duration,
        } => {
            let s = w.keys[signer];
            let expect = if s != m.guardian {
                Err(NotGuardianAuthority)
            } else if let Err(e) = ref_pause_flags(flags) {
                Err(e)
            } else if duration == 0 || duration > 216_000 {
                Err(InvalidPauseDuration)
            } else if m.guardian_active(slot) {
                next.gflags |= flags;
                Ok(())
            } else if m.gexpiry != 0 && slot < m.gexpiry + 9_000 {
                Err(GuardianPauseCooldown)
            } else {
                next.gflags = flags;
                next.gexpiry = slot + duration;
                next.gstart = slot;
                next.renewed = false;
                Ok(())
            };
            let real = guardian_pause(&w.market, &mut w.asset.pauses, &s, flags, duration, slot);
            Some((expect, real))
        }
        Step::SetPause {
            signer,
            flags,
            paused,
        } => {
            let s = w.keys[signer];
            let expect = if s != m.risk {
                Err(NotRiskAuthority)
            } else if let Err(e) = ref_pause_flags(flags) {
                Err(e)
            } else {
                if paused {
                    next.flags |= flags;
                } else {
                    next.flags &= !flags;
                }
                Ok(())
            };
            let real = set_pause(&w.market, &mut w.asset.pauses, &s, flags, paused);
            Some((expect, real))
        }
        Step::Renew { signer, extra } => {
            let s = w.keys[signer];
            let new_expiry = m.gexpiry.saturating_add(extra);
            let expect = if s != m.risk {
                Err(NotRiskAuthority)
            } else if !m.guardian_active(slot) {
                Err(NoActiveGuardianPause)
            } else if new_expiry <= m.gexpiry {
                Err(InvalidPauseExpiry)
            } else {
                next.gexpiry = new_expiry;
                next.renewed = true;
                Ok(())
            };
            let real = renew_guardian_pause(&w.market, &mut w.asset.pauses, &s, new_expiry, slot);
            Some((expect, real))
        }
        Step::ClearGuardian { signer } => {
            let s = w.keys[signer];
            let expect = if s != m.risk {
                Err(NotRiskAuthority)
            } else if m.gflags == 0 && m.gexpiry == 0 {
                Err(NoActiveGuardianPause)
            } else {
                next.gflags = 0;
                next.gexpiry = 0;
                next.renewed = false;
                Ok(())
            };
            Some((
                expect,
                clear_guardian_pause(&w.market, &mut w.asset.pauses, &s),
            ))
        }
        Step::Propose {
            signer,
            id_skew,
            kind,
            seed,
        } => {
            let s = w.keys[signer];
            let id = (m.next_id as i64 + id_skew).max(0) as u64;
            let change = build_change(kind, seed, &m.config);
            let expect = if s != m.risk {
                Err(NotRiskAuthority)
            } else if id != m.next_id {
                Err(ConfigProposalIdMismatch)
            } else if let Err(e) = ref_validate(&m.config, &change) {
                Err(e)
            } else {
                let earliest = now + i64::from(m.config.values.timelock_delay_seconds);
                next.proposals.push(RefProposal {
                    base: group_value(&m.config, &change),
                    new: change,
                    proposed_at: now,
                    earliest,
                    expires: earliest + 1_209_600,
                    status: ProposalStatus::Pending,
                });
                next.next_id += 1;
                Ok(())
            };
            let real = propose(&w.market, &mut w.asset, &s, id, change, now).map(|p| {
                assert_eq!(p.id as usize, w.proposals.len());
                w.proposals.push(p);
            });
            Some((expect, real))
        }
        Step::Execute { signer, which } if which < m.proposals.len() => {
            let s = w.keys[signer];
            let p = &m.proposals[which];
            let ready = m.ready(p);
            let expect = if p.status != ProposalStatus::Pending {
                Err(ConfigProposalNotPending)
            } else if now < ready {
                Err(TimelockNotElapsed)
            } else if now >= p.expires {
                Err(ConfigProposalExpired)
            } else if group_value(&m.config, &p.new) != p.base {
                Err(ConfigProposalBaseChanged)
            } else if let Err(e) = ref_validate(&m.config, &p.new) {
                Err(e)
            } else {
                next.config = apply_value(m.config, &p.new);
                next.generation += 1;
                next.proposals[which].status = ProposalStatus::Executed;
                Ok(())
            };
            let real = execute(
                &mut w.market,
                &mut w.asset,
                &mut w.proposals[which],
                &s,
                now,
            );
            if real.is_ok() {
                let p = &w.proposals[which];
                assert!(now >= p.earliest_execute_at, "{step:?}: before delay");
                assert!(now >= p.proposed_at + before_delay, "{step:?}: delay");
                assert!(now - p.proposed_at >= i64::from(MIN_TIMELOCK_DELAY_SECONDS));
                assert!(now < p.expires_at, "{step:?}: expired proposal executed");
            }
            Some((expect, real.map(drop)))
        }
        Step::Cancel { signer, which } if which < m.proposals.len() => {
            let s = w.keys[signer];
            let expect = if s != m.risk && s != m.guardian {
                Err(NotConfigCanceller)
            } else if m.proposals[which].status != ProposalStatus::Pending {
                Err(ConfigProposalNotPending)
            } else {
                next.proposals[which].status = ProposalStatus::Cancelled;
                Ok(())
            };
            let real = cancel(&w.market, &mut w.proposals[which], &s, now);
            Some((expect, real))
        }
        Step::Decrease { signer, kind, seed } => {
            let s = w.keys[signer];
            let change = build_change(kind, seed, &m.config);
            let expect = if s != m.risk {
                Err(NotRiskAuthority)
            } else if !ref_is_decrease(&m.config, &change) {
                Err(CapChangeNotDecrease)
            } else if let Err(e) = ref_validate(&m.config, &change) {
                Err(e)
            } else {
                next.config = apply_value(m.config, &change);
                next.generation += 1;
                Ok(())
            };
            let real = decrease_caps(&mut w.market, &mut w.asset, &s, change).map(drop);
            Some((expect, real))
        }
        Step::Advance(t) => {
            match t {
                Time::Secs(d) => w.now += d,
                Time::ToReady(i, off) | Time::ToExpiry(i, off) if i < m.proposals.len() => {
                    let p = &m.proposals[i];
                    let target = match t {
                        Time::ToReady(..) => m.ready(p),
                        _ => p.expires,
                    } + off;
                    w.now = w.now.max(target);
                }
                Time::Slots(n) => w.slot += n,
                Time::ToGuardianExpiry(off) => w.slot = w.slot.max(m.gexpiry + off),
                Time::ToCooldownEnd(off) => {
                    let target = (m.gexpiry + 9_000) as i64 + off;
                    w.slot = w.slot.max(target.max(0) as u64);
                }
                _ => {}
            }
            None
        }
        Step::InitPosition => {
            let expect = if m.position_count + 1 > m.config.count_caps.position_count_cap {
                Err(PositionCountCapReached)
            } else {
                next.position_count += 1;
                next.has_debt.push(false);
                Ok(())
            };
            let real = on_position_initialized(&mut w.market).map(|change| {
                assert!(change.new_count <= change.cap);
                w.has_debt.push(false);
            });
            Some((expect, real))
        }
        Step::QueueBorrow { pos } if pos < w.has_debt.len() && !w.locked(pos) => {
            let expected = if m.has_debt[pos] {
                DebtSlot::NotNeeded
            } else if m.debt_count + m.reservations >= m.config.count_caps.debt_position_count_cap {
                DebtSlot::Unavailable
            } else {
                next.reservations += 1;
                DebtSlot::Reserved
            };
            let got = reserve_debt_slot(&mut w.market, Action::Borrow, w.has_debt[pos]).unwrap();
            assert_eq!(got, expected, "{step:?}");
            if got == DebtSlot::Unavailable {
                stats.entry("debt_slot_unavailable").or_default()[2] += 1;
            } else {
                w.intents.push((pos, Action::Borrow));
            }
            if got == DebtSlot::Reserved {
                assert!(
                    w.market.debt_position_count + w.market.debt_position_reservations
                        <= w.market.debt_position_count_cap
                );
            }
            Some((Ok(()), Ok(())))
        }
        Step::QueueRepay { pos } if pos < w.has_debt.len() && w.has_debt[pos] && !w.locked(pos) => {
            let got = reserve_debt_slot(&mut w.market, Action::Repay, true).unwrap();
            assert_eq!(got, DebtSlot::NotNeeded);
            w.intents.push((pos, Action::Repay));
            Some((Ok(()), Ok(())))
        }
        Step::Settle { which, how } if which < w.intents.len() => {
            let (pos, action) = w.intents[which];
            let was = w.has_debt[pos];
            let transition = match how {
                How::Applied(flip) => {
                    let is = if action == Action::Borrow {
                        was || flip
                    } else {
                        was && flip
                    };
                    DebtTransition::from_callback(CallbackDecision::Applied, was, is)
                }
                How::Contradict => {
                    DebtTransition::from_callback(CallbackDecision::Applied, !was, true)
                }
                How::NotApplied(decision) => {
                    let t = DebtTransition::from_callback(decision, was, !was);
                    assert_eq!(t, None, "only Applied builds a transition");
                    t
                }
            };
            let expect = if matches!(how, How::Contradict) {
                Err(DebtTransitionMismatch)
            } else {
                let had = m.has_debt[pos];
                if action == Action::Borrow && !had {
                    next.reservations -= 1;
                }
                if let Some(t) = transition {
                    match (t.was_nonzero(), t.is_nonzero()) {
                        (false, true) => {
                            next.debt_count += 1;
                            next.has_debt[pos] = true;
                        }
                        (true, false) => {
                            next.debt_count -= 1;
                            next.has_debt[pos] = false;
                        }
                        _ => {}
                    }
                }
                Ok(())
            };
            let mut flag = w.has_debt[pos];
            let real = settle_debt_intent(&mut w.market, &mut flag, action, transition);
            w.has_debt[pos] = flag;
            if real.is_ok() {
                w.intents.remove(which);
            }
            Some((expect, real.map(drop)))
        }
        _ => None,
    };

    let entry = stats.entry(step.name()).or_default();
    let accepted = match outcome {
        None => {
            entry[0] += 1;
            assert_eq!(snapshot(w).len(), before_bytes.len());
            false
        }
        Some((expect, real)) => {
            assert_eq!(real, expect, "step {index}: {step:?}");
            if real.is_ok() {
                entry[2] += 1;
                *m = next;
                true
            } else {
                entry[1] += 1;
                if real.as_ref().err().is_some_and(is_auth_error) {
                    entry[3] += 1;
                }
                assert_eq!(
                    snapshot(w),
                    before_bytes,
                    "{step:?}: a failure changed state"
                );
                false
            }
        }
    };

    agree(w, m);
    check_invariants(
        w,
        m,
        &step,
        accepted,
        &before_config,
        before_generation,
        index,
    );
    accepted
}

/// The real state equals the reference model.
fn agree(w: &World, m: &Model) {
    let k = &w.market;
    assert_eq!(
        (
            k.admin,
            k.pending_admin,
            k.upgrade_authority,
            k.risk_authority,
            k.treasury_authority,
            k.guardian_authority
        ),
        (m.admin, m.pending, m.upgrade, m.risk, m.treasury, m.guardian)
    );
    let p = w.asset.pauses;
    assert_eq!(
        (p.flags, p.guardian_flags, p.guardian_expiry_slot),
        (m.flags, m.gflags, m.gexpiry)
    );
    assert_eq!(GovernedConfig::of(k, &w.asset), m.config);
    assert_eq!(w.asset.generation, m.generation);
    assert_eq!(w.asset.timelock.next_proposal_id, m.next_id);
    assert_eq!(w.proposals.len(), m.proposals.len());
    for (real, model) in w.proposals.iter().zip(&m.proposals) {
        assert_eq!(real.status, model.status);
        assert_eq!(real.proposed_at, model.proposed_at);
        assert_eq!(real.earliest_execute_at, model.earliest);
        assert_eq!(real.expires_at, model.expires);
        assert_eq!(real.new_value(), model.new);
        assert_eq!(real.old_value(), model.base);
    }
    assert_eq!(
        (
            k.position_count,
            k.debt_position_count,
            k.debt_position_reservations
        ),
        (m.position_count, m.debt_count, m.reservations)
    );
    assert_eq!(w.has_debt, m.has_debt);
}

fn check_invariants(
    w: &World,
    m: &Model,
    step: &Step,
    accepted: bool,
    before_config: &GovernedConfig,
    before_generation: u64,
    index: u64,
) {
    let k = &w.market;
    let a = &w.asset;
    let protocol = protocol_keys();

    // Roles: four distinct valid separated keys; a valid admin.
    let roles = [
        k.upgrade_authority,
        k.risk_authority,
        k.treasury_authority,
        k.guardian_authority,
    ];
    for (i, r) in roles.iter().enumerate() {
        assert!(*r != ZERO && !protocol.contains(r));
        assert!(!roles[i + 1..].contains(r), "separated roles share a key");
    }
    assert!(k.admin != ZERO && !protocol.contains(&k.admin));
    assert!(k.pending_admin == ZERO || !protocol.contains(&k.pending_admin));

    // Timelock: a governed value changes only by an executed proposal, or
    // moves down only through a cap decrease.
    let config = GovernedConfig::of(k, a);
    let executed = accepted && matches!(step, Step::Execute { .. });
    let decreased = accepted && matches!(step, Step::Decrease { .. });
    if config != *before_config {
        assert!(executed || decreased, "{step:?} changed configuration");
        if decreased {
            let (old, new) = (flat(before_config), flat(&config));
            assert!(old.iter().zip(&new).all(|(o, n)| n <= o), "{step:?} raised");
            assert_eq!(config.values.risk, before_config.values.risk);
            assert_eq!(config.values.rates, before_config.values.rates);
            assert_eq!(
                config.values.timelock_delay_seconds,
                before_config.values.timelock_delay_seconds
            );
        }
    }

    // Generation: exactly one bump per execution or decrease, none otherwise.
    let bumped = a.generation - before_generation;
    assert_eq!(bumped, u64::from(executed || decreased), "{step:?}");
    if bumped == 1 || index.is_multiple_of(8) {
        let decision = classify_probe(before_generation, a.generation);
        if bumped == 1 {
            assert_eq!(
                decision,
                CallbackDecision::Stale(StaleReason::AssetGeneration)
            );
        } else {
            assert_eq!(decision, CallbackDecision::Applied);
        }
    }

    // Pauses: rescue never blocks; expired guardian pauses never block;
    // a guardian-only pause never lasts longer than the maximum.
    let p = &a.pauses;
    let rescue = pause_flags::NEVER_PAUSABLE;
    assert_eq!(p.flags & rescue, 0);
    assert_eq!(p.guardian_flags & rescue, 0);
    for action in Action::ALL.iter().filter(|x| x.is_rescue()) {
        for at in [w.slot, w.slot + 1, p.guardian_expiry_slot, u64::MAX] {
            assert_eq!(ensure_action_allowed(p, *action, at), Ok(()));
        }
    }
    if w.slot >= p.guardian_expiry_slot {
        assert_eq!(
            effective_pause_flags(p, w.slot),
            p.flags & pause_flags::PAUSABLE
        );
    }
    if p.guardian_flags != 0 && !m.renewed {
        assert!(p.guardian_expiry_slot > m.gstart);
        assert!(p.guardian_expiry_slot - m.gstart <= GUARDIAN_MAX_PAUSE_SLOTS);
    }

    // Caps: ordering, and the boundary rule at the current caps.
    let caps = a.caps;
    assert_eq!(check_caps_bounds(&caps), Ok(()));
    assert_eq!(check_count_caps_bounds(&k.count_caps()), Ok(()));
    assert!(caps.position_debt_cap_usdc <= caps.total_debt_cap_usdc);
    assert!(caps.total_debt_cap_usdc <= caps.supply_cap_usdc);
    assert!(caps.supply_cap_usdc <= MAX_USDC_CAP);
    assert!(k.debt_position_count_cap <= k.position_count_cap);
    assert!(k.position_count_cap <= MAX_POSITION_COUNT_CAP);
    let supply = u128::from(caps.supply_cap_usdc);
    assert_eq!(
        supply_cap_outcome(&caps, supply, 0),
        NewRiskOutcome::Proceed
    );
    assert_eq!(supply_cap_outcome(&caps, supply, 1), NewRiskOutcome::Zero);
    let pos_cap = u128::from(caps.position_debt_cap_usdc);
    assert_eq!(
        borrow_cap_outcome(&caps, 0, pos_cap, 0),
        NewRiskOutcome::Proceed
    );
    assert_eq!(
        borrow_cap_outcome(&caps, 0, pos_cap, 1),
        NewRiskOutcome::Zero
    );

    // Counts follow the Positions.
    assert_eq!(k.position_count as usize, w.has_debt.len());
    let debt = w.has_debt.iter().filter(|d| **d).count();
    assert_eq!(k.debt_position_count as usize, debt);
    let held = w
        .intents
        .iter()
        .filter(|(pos, action)| *action == Action::Borrow && !w.has_debt[*pos])
        .count();
    assert_eq!(k.debt_position_reservations as usize, held);

    // Cross-bound in exact basis-point arithmetic.
    let r = &a.risk;
    assert!(ref_cross_bound(
        r.liquidation_threshold_bps.into(),
        r.liquidation_bonus_bps.into(),
        r.target_health_factor_bps.into(),
    ));
    assert_eq!(check_risk_bounds(r), Ok(()));
    assert_eq!(check_rate_bounds(&a.rates), Ok(()));
}

// ---- Random step generator --------------------------------------------------------------

fn random_flags(rng: &mut Lcg) -> u16 {
    match rng.below(6) {
        0 => 0,
        1 => 1 << 10,
        2 => (rng.sample() as u16) & pause_flags::ALL,
        _ => {
            let pausable = [
                pause_flags::SUPPLY,
                pause_flags::WITHDRAW_SUPPLY,
                pause_flags::BORROW,
                pause_flags::WITHDRAW_COLLATERAL,
                pause_flags::SHIELD,
                pause_flags::UNSHIELD,
                pause_flags::LIQUIDATION_START,
                pause_flags::LIQUIDATION_COMPLETE,
            ];
            rng.pick(&pausable) | rng.pick(&pausable)
        }
    }
}

fn random_step(rng: &mut Lcg, w: &World, m: &Model) -> Step {
    let holder = |k: Pubkey| w.keys[..SIGNERS].iter().position(|x| *x == k);
    let signer = |rng: &mut Lcg, k: Pubkey| -> usize {
        if rng.below(2) == 0 {
            if let Some(i) = holder(k) {
                return i;
            }
        }
        rng.below(SIGNERS as u64) as usize
    };
    let pending: Vec<usize> = m
        .proposals
        .iter()
        .enumerate()
        .filter(|(_, p)| p.status == ProposalStatus::Pending)
        .map(|(i, _)| i)
        .collect();
    let which = |rng: &mut Lcg| -> usize {
        if !pending.is_empty() && rng.below(10) < 8 {
            rng.pick(&pending)
        } else {
            rng.below(m.proposals.len() as u64 + 1) as usize
        }
    };
    let new_key = |rng: &mut Lcg| rng.below(KEYS as u64) as usize;
    match rng.below(100) {
        0..=3 => Step::ProposeAdmin {
            signer: signer(rng, m.admin),
            new: new_key(rng),
        },
        4..=6 => Step::AcceptAdmin {
            signer: signer(rng, m.pending),
        },
        7..=8 => Step::CancelAdmin {
            signer: signer(rng, m.admin),
        },
        9..=13 => Step::SetRole {
            signer: signer(rng, m.admin),
            kind: rng.pick(&[RoleKind::Risk, RoleKind::Treasury, RoleKind::Guardian]),
            new: new_key(rng),
        },
        14..=15 => Step::SetUpgrade {
            signer: signer(rng, m.admin),
            new: new_key(rng),
            loader_agrees: rng.below(3) != 0,
        },
        16..=21 => Step::GuardianPause {
            signer: signer(rng, m.guardian),
            flags: random_flags(rng),
            duration: rng.pick(&[
                0,
                1,
                100,
                GUARDIAN_MAX_PAUSE_SLOTS - 1,
                GUARDIAN_MAX_PAUSE_SLOTS,
                GUARDIAN_MAX_PAUSE_SLOTS + 1,
            ]),
        },
        22..=25 => Step::SetPause {
            signer: signer(rng, m.risk),
            flags: random_flags(rng),
            paused: rng.below(2) == 0,
        },
        26..=28 => Step::Renew {
            signer: signer(rng, m.risk),
            extra: rng.pick(&[0, 1, 1_000, 2 * GUARDIAN_MAX_PAUSE_SLOTS]),
        },
        29..=30 => Step::ClearGuardian {
            signer: signer(rng, m.risk),
        },
        31..=44 => Step::Propose {
            signer: signer(rng, m.risk),
            id_skew: rng.pick(&[0, 0, 0, 0, 0, 1, -1]),
            kind: rng.below(5) as u8,
            seed: rng.sample(),
        },
        45..=56 => Step::Execute {
            signer: rng.below(SIGNERS as u64) as usize,
            which: which(rng),
        },
        57..=60 => Step::Cancel {
            signer: if rng.below(2) == 0 {
                signer(rng, m.guardian)
            } else {
                signer(rng, m.risk)
            },
            which: which(rng),
        },
        61..=66 => Step::Decrease {
            signer: signer(rng, m.risk),
            kind: rng.pick(&[2, 4]),
            seed: rng.sample(),
        },
        67..=82 => {
            let i = which(rng);
            Step::Advance(match rng.below(8) {
                0 | 1 => Time::Secs(rng.pick(&[1, 60, 3_600, DAY, DELAY / 2, 7 * DAY, 16 * DAY])),
                2 => Time::ToReady(i, rng.pick(&[-1, 0, 1])),
                3 => Time::ToExpiry(i, rng.pick(&[-1, 0])),
                4 | 5 => Time::Slots(rng.pick(&[
                    1,
                    50,
                    GUARDIAN_PAUSE_COOLDOWN_SLOTS - 1,
                    GUARDIAN_MAX_PAUSE_SLOTS,
                ])),
                6 => Time::ToGuardianExpiry(rng.pick(&[0, 0, 1])),
                _ => Time::ToCooldownEnd(rng.pick(&[-1, 0])),
            })
        }
        83..=86 => Step::InitPosition,
        87..=90 => Step::QueueBorrow {
            pos: rng.below(w.has_debt.len() as u64 + 1) as usize,
        },
        91..=92 => Step::QueueRepay {
            pos: rng.below(w.has_debt.len() as u64 + 1) as usize,
        },
        _ => Step::Settle {
            which: rng.below(w.intents.len() as u64 + 1) as usize,
            how: match rng.below(8) {
                0..=3 => How::Applied(rng.below(2) == 0),
                4 => How::Contradict,
                _ => How::NotApplied(rng.pick(&[
                    CallbackDecision::Zero,
                    CallbackDecision::Stale(StaleReason::AssetGeneration),
                    CallbackDecision::Expired,
                    CallbackDecision::Failed,
                ])),
            },
        },
    }
}

fn print_stats(label: &str, stats: &Stats) {
    eprintln!("{label}: step -> [skipped, rejected, accepted, unauthorized]");
    for (name, counts) in stats {
        eprintln!("  {name:24} {counts:?}");
    }
}

// ---- Property tests ---------------------------------------------------------------------

/// Random sequences over the whole action alphabet, with every signer
/// (admin, pending admin, upgrade, risk, treasury, guardian, attacker, a
/// spare key, and an off-curve multisig PDA) and rotating roles, compared
/// step by step with the reference model.
///
/// **Validates: Requirements 4.3, 4.4, 4.5, 4.6, 4.7, 4.8, 4.9, 4.10, 4.11,
/// 4.12, 4.13**
#[test]
fn prop_governance_model_random_sequences() {
    let mut rng = Lcg(0x0046_60FE_12A7);
    let mut stats = Stats::new();
    let mut index = 0u64;
    for _ in 0..1_000 {
        let mut w = World::new();
        let mut m = Model::initial();
        for _ in 0..80 {
            let step = random_step(&mut rng, &w, &m);
            run_step(&mut w, &mut m, step, &mut stats, index);
            index += 1;
        }
    }
    print_stats("random", &stats);
    for name in [
        "propose_admin",
        "accept_admin",
        "cancel_admin_transfer",
        "set_role",
        "set_upgrade_authority",
        "guardian_pause",
        "set_pause",
        "renew_guardian_pause",
        "clear_guardian_pause",
        "propose",
        "execute",
        "cancel",
        "decrease_caps",
        "on_position_initialized",
        "settle_debt_intent",
    ] {
        let counts = stats[name];
        assert!(counts[1] > 0 && counts[2] > 0, "{name} coverage {counts:?}");
    }
    assert!(stats["reserve_debt_slot"][2] > 0 && stats["debt_slot_unavailable"][2] > 0);
    for name in ["propose", "decrease_caps", "set_pause", "guardian_pause"] {
        assert!(stats[name][3] > 0, "{name} unauthorized coverage");
    }
}

/// A primed world (two Positions, one with debt; a risk proposal and a
/// delay-raise proposal pending; an active guardian pause), then every
/// sequence of three steps from a fixed alphabet that hits the edges:
/// execute at ready - 1 / ready / expiry - 1 / expiry, cancel, re-propose,
/// cap decrease, guardian re-pause and expiry, renewal, role rotation, admin
/// handover to the risk key, and debt-slot reservation and settlement.
///
/// **Validates: Requirements 4.3, 4.5, 4.6, 4.9, 4.10, 4.11, 4.13**
#[test]
fn prop_governance_model_exhaustive_short_sequences() {
    let mut stats = Stats::new();
    let mut w = World::new();
    let mut m = Model::initial();
    // First seed whose change of group `kind` is valid (and, when asked, a
    // cap decrease) against the model's configuration.
    let seed_for = |kind: u8, m: &Model, decrease: bool| -> u64 {
        (0..)
            .find(|s| {
                let change = build_change(kind, *s, &m.config);
                ref_validate(&m.config, &change).is_ok()
                    && (!decrease || ref_is_decrease(&m.config, &change))
            })
            .unwrap()
    };
    let risk_seed = seed_for(0, &m, false);
    let delay_seed = (0..)
        .find(|s| {
            matches!(
                build_change(3, *s, &m.config),
                ConfigValue::TimelockDelay { delay_seconds }
                    if delay_seconds > TimelockState::LAUNCH_DEFAULT_DELAY_SECONDS
                        && delay_seconds <= MAX_TIMELOCK_DELAY_SECONDS
            )
        })
        .unwrap();
    let prime = [
        Step::InitPosition,
        Step::InitPosition,
        Step::QueueBorrow { pos: 1 },
        Step::Settle {
            which: 0,
            how: How::Applied(true),
        },
        Step::Propose {
            signer: RISK,
            id_skew: 0,
            kind: 0,
            seed: risk_seed,
        },
        Step::Advance(Time::Secs(3_600)),
        Step::Propose {
            signer: RISK,
            id_skew: 0,
            kind: 3,
            seed: delay_seed,
        },
        Step::GuardianPause {
            signer: GUARDIAN,
            flags: pause_flags::SUPPLY,
            duration: 100,
        },
    ];
    let mut index = 0;
    for step in prime {
        run_step(&mut w, &mut m, step, &mut stats, index);
        index += 1;
    }
    // Make sure the priming reached the intended state.
    assert_eq!(w.has_debt, vec![false, true]);
    assert!(w
        .proposals
        .iter()
        .all(|p| p.status == ProposalStatus::Pending));
    assert_eq!(w.proposals.len(), 2);
    assert!(guardian_pause_active(&w.asset.pauses, w.slot));
    let repropose_seed = seed_for(0, &m, false);
    let decrease_seed = seed_for(2, &m, true);

    let alphabet = [
        Step::Execute {
            signer: ATTACKER,
            which: 0,
        },
        Step::Execute {
            signer: ATTACKER,
            which: 1,
        },
        Step::Cancel {
            signer: GUARDIAN,
            which: 0,
        },
        Step::Cancel {
            signer: ADMIN,
            which: 0,
        },
        Step::Propose {
            signer: RISK,
            id_skew: 0,
            kind: 0,
            seed: repropose_seed,
        },
        Step::Propose {
            signer: ADMIN,
            id_skew: 0,
            kind: 3,
            seed: delay_seed,
        },
        Step::Decrease {
            signer: RISK,
            kind: 2,
            seed: decrease_seed,
        },
        Step::Advance(Time::ToReady(0, -1)),
        Step::Advance(Time::ToReady(0, 0)),
        Step::Advance(Time::ToExpiry(0, -1)),
        Step::Advance(Time::ToExpiry(0, 0)),
        Step::GuardianPause {
            signer: GUARDIAN,
            flags: pause_flags::BORROW,
            duration: GUARDIAN_MAX_PAUSE_SLOTS,
        },
        Step::Advance(Time::ToGuardianExpiry(0)),
        Step::Renew {
            signer: RISK,
            extra: 10,
        },
        Step::SetRole {
            signer: ADMIN,
            kind: RoleKind::Risk,
            new: ALT,
        },
        Step::ProposeAdmin {
            signer: ADMIN,
            new: RISK,
        },
        Step::AcceptAdmin { signer: RISK },
        Step::QueueBorrow { pos: 0 },
        Step::Settle {
            which: 0,
            how: How::Applied(true),
        },
        Step::SetPause {
            signer: RISK,
            flags: pause_flags::SUPPLY | pause_flags::BORROW,
            paused: true,
        },
    ];
    let n = alphabet.len();
    let mut sequences = 0u64;
    for a in 0..n {
        for b in 0..n {
            for c in 0..n {
                let mut w = w.clone();
                let mut m = m.clone();
                for i in [a, b, c] {
                    run_step(&mut w, &mut m, alphabet[i], &mut stats, index);
                    index += 1;
                }
                sequences += 1;
            }
        }
    }
    assert_eq!(sequences, (n * n * n) as u64);
    print_stats("exhaustive", &stats);
    assert!(stats["execute"][2] > 0 && stats["execute"][1] > 0);
    assert!(stats["decrease_caps"][2] > 0);
}

/// Guardian-only histories: every guardian pause interval, observed through
/// the real `guardian_pause_active`, lasts at most `GUARDIAN_MAX_PAUSE_SLOTS`,
/// and a new interval starts no earlier than `GUARDIAN_PAUSE_COOLDOWN_SLOTS`
/// after the previous one ended.
///
/// **Validates: Requirement 4.9**
#[test]
fn prop_guardian_only_pauses_never_exceed_max_or_skip_cooldown() {
    let mut rng = Lcg(0x06A2_D1A4);
    let k = market();
    let guardian = k.guardian_authority;
    let mut intervals = 0u64;
    for _ in 0..200 {
        let mut pauses = PauseState::default();
        let mut slot = SLOT0;
        let mut last_end: Option<u64> = None;
        for _ in 0..80 {
            slot += rng.pick(&[
                1,
                17,
                GUARDIAN_PAUSE_COOLDOWN_SLOTS - 1,
                GUARDIAN_PAUSE_COOLDOWN_SLOTS,
                GUARDIAN_MAX_PAUSE_SLOTS / 3,
                GUARDIAN_MAX_PAUSE_SLOTS,
            ]);
            let was_active = guardian_pause_active(&pauses, slot);
            let expiry_before = pauses.guardian_expiry_slot;
            let flags = random_flags(&mut rng);
            let duration = rng.below(GUARDIAN_MAX_PAUSE_SLOTS + 2);
            if guardian_pause(&k, &mut pauses, &guardian, flags, duration, slot).is_ok() {
                if was_active {
                    assert_eq!(pauses.guardian_expiry_slot, expiry_before);
                } else {
                    let end = pauses.guardian_expiry_slot;
                    assert!(end > slot && end - slot <= GUARDIAN_MAX_PAUSE_SLOTS);
                    if let Some(previous) = last_end {
                        assert!(slot >= previous + GUARDIAN_PAUSE_COOLDOWN_SLOTS);
                    }
                    last_end = Some(end);
                    intervals += 1;
                }
            }
            assert!(!guardian_pause_active(&pauses, pauses.guardian_expiry_slot));
        }
    }
    assert!(intervals > 200, "the generator must reach fresh pauses");
}

// ---- Targeted tests: unauthorized access --------------------------------------------------

type Privileged = fn(&mut World, &Pubkey) -> Result<(), MarketError>;
type Holders = fn(&Market) -> Vec<Pubkey>;

fn act(name: &'static str, f: Privileged, h: Holders) -> (&'static str, Privileged, Holders) {
    (name, f, h)
}

/// A world with a pending admin (key 2), an active guardian pause, and one
/// pending proposal, so every privileged action can succeed for its holder.
fn loaded_world() -> World {
    let mut w = World::new();
    w.market.pending_admin = key(2);
    w.asset.pauses = PauseState {
        flags: 0,
        guardian_flags: pause_flags::SUPPLY,
        guardian_expiry_slot: SLOT0 + 100,
    };
    let risk = w.market.risk_authority;
    let change = ConfigValue::TimelockDelay {
        delay_seconds: 3 * DAY as u32,
    };
    let p = propose(&w.market, &mut w.asset, &risk, 0, change, T0).unwrap();
    w.proposals.push(p);
    w
}

fn privileged_actions() -> Vec<(&'static str, Privileged, Holders)> {
    fn admin(m: &Market) -> Vec<Pubkey> {
        vec![m.admin]
    }
    fn pending(m: &Market) -> Vec<Pubkey> {
        vec![m.pending_admin]
    }
    fn risk(m: &Market) -> Vec<Pubkey> {
        vec![m.risk_authority]
    }
    fn guardian(m: &Market) -> Vec<Pubkey> {
        vec![m.guardian_authority]
    }
    fn canceller(m: &Market) -> Vec<Pubkey> {
        vec![m.risk_authority, m.guardian_authority]
    }
    vec![
        act(
            "propose_admin",
            |w, s| propose_admin(&mut w.market, &MARKET_KEY, s, key(10)).map(drop),
            admin,
        ),
        act(
            "accept_admin",
            |w, s| accept_admin(&mut w.market, s).map(drop),
            pending,
        ),
        act(
            "cancel_admin_transfer",
            |w, s| cancel_admin_transfer(&mut w.market, s).map(drop),
            admin,
        ),
        act(
            "set_risk_authority",
            |w, s| set_role(&mut w.market, &MARKET_KEY, s, RoleKind::Risk, key(10)).map(drop),
            admin,
        ),
        act(
            "set_treasury_authority",
            |w, s| set_role(&mut w.market, &MARKET_KEY, s, RoleKind::Treasury, key(10)).map(drop),
            admin,
        ),
        act(
            "set_guardian_authority",
            |w, s| set_role(&mut w.market, &MARKET_KEY, s, RoleKind::Guardian, key(10)).map(drop),
            admin,
        ),
        act(
            "set_upgrade_authority",
            |w, s| {
                let program = program_state();
                let data = program_data_state(key(10));
                let (pv, dv) = views(&program, &data);
                set_upgrade_authority(&mut w.market, &MARKET_KEY, s, &pv, &dv, key(10)).map(drop)
            },
            admin,
        ),
        act(
            "guardian_pause",
            |w, s| {
                guardian_pause(
                    &w.market,
                    &mut w.asset.pauses,
                    s,
                    pause_flags::BORROW,
                    10,
                    w.slot,
                )
            },
            guardian,
        ),
        act(
            "set_pause",
            |w, s| set_pause(&w.market, &mut w.asset.pauses, s, pause_flags::BORROW, true),
            risk,
        ),
        act(
            "renew_guardian_pause",
            |w, s| renew_guardian_pause(&w.market, &mut w.asset.pauses, s, SLOT0 + 500, w.slot),
            risk,
        ),
        act(
            "clear_guardian_pause",
            |w, s| clear_guardian_pause(&w.market, &mut w.asset.pauses, s),
            risk,
        ),
        act(
            "propose_config_change",
            |w, s| {
                let change = ConfigValue::TimelockDelay {
                    delay_seconds: 4 * DAY as u32,
                };
                propose(&w.market, &mut w.asset, s, 1, change, w.now).map(|p| w.proposals.push(p))
            },
            risk,
        ),
        act(
            "cancel_config_change",
            |w, s| cancel(&w.market, &mut w.proposals[0], s, w.now),
            canceller,
        ),
        act(
            "decrease_caps",
            |w, s| {
                let mut caps = w.asset.caps;
                caps.position_debt_cap_usdc -= 1;
                decrease_caps(&mut w.market, &mut w.asset, s, ConfigValue::Caps { caps }).map(drop)
            },
            risk,
        ),
    ]
}

fn expected_role_error(name: &str) -> MarketError {
    match name {
        "accept_admin" => MarketError::NotPendingAdmin,
        "guardian_pause" => MarketError::NotGuardianAuthority,
        "set_pause"
        | "renew_guardian_pause"
        | "clear_guardian_pause"
        | "propose_config_change"
        | "decrease_caps" => MarketError::NotRiskAuthority,
        "cancel_config_change" => MarketError::NotConfigCanceller,
        _ => MarketError::NotMarketAdmin,
    }
}

/// Every privileged action, for every signer: the exact holder succeeds;
/// everyone else (including the admin for risk/guardian actions, and the
/// risk, guardian, upgrade, and treasury keys for admin actions) fails with
/// the role's error and changes nothing. Run with plain keys and with every
/// role held by a distinct off-curve multisig PDA.
#[test]
fn unauthorized_signers_change_nothing_for_every_privileged_action() {
    let plain = loaded_world();
    let mut multisig = loaded_world();
    multisig.market.admin = multisig_pda(0x70);
    multisig.market.pending_admin = multisig_pda(0x71);
    multisig.market.upgrade_authority = multisig_pda(0x72);
    multisig.market.risk_authority = multisig_pda(0x73);
    multisig.market.treasury_authority = multisig_pda(0x74);
    multisig.market.guardian_authority = multisig_pda(0x75);
    for base in [plain, multisig] {
        let k = &base.market;
        let mut signers = vec![
            k.admin,
            k.pending_admin,
            k.upgrade_authority,
            k.risk_authority,
            k.treasury_authority,
            k.guardian_authority,
            key(9),
            multisig_pda(0x76),
            ZERO,
            MARKET_KEY,
            k.vault_authority,
        ];
        signers.extend(base.keys);
        let mut checked = 0;
        for (name, action, holders) in privileged_actions() {
            let allowed = holders(&base.market);
            for signer in &signers {
                let mut w = base.clone();
                let before = snapshot(&w);
                let result = action(&mut w, signer);
                if allowed.contains(signer) {
                    assert_eq!(result, Ok(()), "{name} by its holder");
                } else {
                    assert_eq!(result, Err(expected_role_error(name)), "{name} by {signer}");
                    assert_eq!(snapshot(&w), before, "{name}: rejected call changed state");
                    checked += 1;
                }
            }
        }
        assert!(checked > 150);
    }
}

/// An inactive Market authorizes nobody.
#[test]
fn inactive_market_rejects_every_privileged_action() {
    let mut base = loaded_world();
    base.market.status = MarketStatus::Uninitialized;
    for (name, action, holders) in privileged_actions() {
        for signer in holders(&base.market) {
            let mut w = base.clone();
            let before = snapshot(&w);
            assert_eq!(
                action(&mut w, &signer),
                Err(MarketError::MarketNotActive),
                "{name}"
            );
            assert_eq!(snapshot(&w), before);
        }
    }
    let mut w = base.clone();
    let risk = w.market.risk_authority;
    assert_eq!(
        execute(
            &mut w.market,
            &mut w.asset,
            &mut w.proposals[0],
            &risk,
            T0 + 4 * DAY
        ),
        Err(MarketError::MarketNotActive)
    );
}

// ---- Targeted tests: timelock bypass ------------------------------------------------------

fn risk_value(f: impl FnOnce(&mut RiskParams)) -> ConfigValue {
    let mut params = RiskParams::LAUNCH_DEFAULT;
    f(&mut params);
    ConfigValue::Risk { params }
}

fn propose_at(w: &mut World, change: ConfigValue, now: i64) -> usize {
    let risk = w.market.risk_authority;
    let id = w.asset.timelock.next_proposal_id;
    let p = propose(&w.market, &mut w.asset, &risk, id, change, now).unwrap();
    w.proposals.push(p);
    w.proposals.len() - 1
}

fn execute_at(w: &mut World, which: usize, now: i64) -> Result<(), MarketError> {
    execute(
        &mut w.market,
        &mut w.asset,
        &mut w.proposals[which],
        &key(9),
        now,
    )
    .map(drop)
}

#[test]
fn execute_at_earliest_but_not_one_second_before() {
    let mut w = World::new();
    let p = propose_at(&mut w, risk_value(|r| r.ltv_bps = 5_200), T0);
    assert_eq!(w.proposals[p].earliest_execute_at, T0 + DELAY);
    let before = snapshot(&w);
    assert_eq!(
        execute_at(&mut w, p, T0 + DELAY - 1),
        Err(MarketError::TimelockNotElapsed)
    );
    assert_eq!(snapshot(&w), before);
    assert_eq!(execute_at(&mut w, p, T0 + DELAY), Ok(()));
    assert_eq!(w.asset.risk.ltv_bps, 5_200);
}

#[test]
fn execute_in_last_grace_second_but_not_at_expiry() {
    let mut w = World::new();
    let p = propose_at(&mut w, risk_value(|r| r.ltv_bps = 5_200), T0);
    let expires = w.proposals[p].expires_at;
    assert_eq!(expires, T0 + DELAY + EXECUTION_GRACE_SECONDS);
    let mut late = w.clone();
    assert_eq!(
        execute_at(&mut late, p, expires),
        Err(MarketError::ConfigProposalExpired)
    );
    assert_eq!(execute_at(&mut w, p, expires - 1), Ok(()));
}

/// Regression (Task 4.6): a proposal that expired must stay expired. Before
/// the fix, `execute` recomputed the deadline from the current delay, so
/// raising the delay after a proposal had expired made it executable again.
#[test]
fn expired_proposal_stays_expired_after_a_delay_raise() {
    let mut w = World::new();
    let forgotten = propose_at(&mut w, risk_value(|r| r.ltv_bps = 5_200), T0);
    let expires = w.proposals[forgotten].expires_at;
    assert_eq!(
        execute_at(&mut w.clone(), forgotten, expires),
        Err(MarketError::ConfigProposalExpired)
    );
    // After it expired, a delay raise to the maximum is proposed and executed.
    let raise_at = expires + 1;
    let raise = propose_at(
        &mut w,
        ConfigValue::TimelockDelay {
            delay_seconds: MAX_TIMELOCK_DELAY_SECONDS,
        },
        raise_at,
    );
    execute_at(&mut w, raise, raise_at + DELAY).unwrap();
    assert_eq!(w.asset.timelock.delay_seconds, MAX_TIMELOCK_DELAY_SECONDS);
    // Under the new delay the recomputed window would be open again.
    let now = raise_at + DELAY + 1;
    assert!(now < T0 + i64::from(MAX_TIMELOCK_DELAY_SECONDS) + EXECUTION_GRACE_SECONDS);
    let before = snapshot(&w);
    assert_eq!(
        execute_at(&mut w, forgotten, now),
        Err(MarketError::ConfigProposalExpired)
    );
    assert_eq!(snapshot(&w), before);
    assert_eq!(w.asset.risk.ltv_bps, RiskParams::LAUNCH_DEFAULT.ltv_bps);
}

/// Raising the delay pushes a pending proposal back; lowering it never
/// brings one forward; the stored expiry bounds both.
#[test]
fn raising_delay_delays_and_lowering_never_shortens() {
    // Raise: 2 days -> 5 days.
    let mut w = World::new();
    let pending = propose_at(&mut w, risk_value(|r| r.ltv_bps = 5_200), T0);
    let raise = propose_at(
        &mut w,
        ConfigValue::TimelockDelay {
            delay_seconds: 5 * DAY as u32,
        },
        T0,
    );
    execute_at(&mut w, raise, T0 + DELAY).unwrap();
    assert_eq!(
        execute_at(&mut w, pending, T0 + 5 * DAY - 1),
        Err(MarketError::TimelockNotElapsed)
    );
    assert_eq!(execute_at(&mut w, pending, T0 + 5 * DAY), Ok(()));

    // Lower: 2 days -> 1 day; a proposal made under 2 days still waits 2.
    let mut w = World::new();
    let lower = propose_at(
        &mut w,
        ConfigValue::TimelockDelay {
            delay_seconds: MIN_TIMELOCK_DELAY_SECONDS,
        },
        T0,
    );
    let pending = propose_at(&mut w, risk_value(|r| r.ltv_bps = 5_200), T0 + 10);
    execute_at(&mut w, lower, T0 + DELAY).unwrap();
    assert_eq!(
        execute_at(&mut w, pending, T0 + 10 + DELAY - 1),
        Err(MarketError::TimelockNotElapsed)
    );
    assert_eq!(execute_at(&mut w, pending, T0 + 10 + DELAY), Ok(()));
}

/// Executed and cancelled proposals are final, a proposal id cannot be
/// reused or skipped, and a base-changed proposal cannot execute.
#[test]
fn final_reused_and_base_changed_proposals_never_execute() {
    let mut w = World::new();
    let risk = w.market.risk_authority;
    let a = propose_at(&mut w, risk_value(|r| r.ltv_bps = 5_200), T0);
    let b = propose_at(&mut w, risk_value(|r| r.close_factor_bps = 6_000), T0);
    let c = propose_at(&mut w, risk_value(|r| r.reserve_factor_bps = 1_500), T0);
    cancel(
        &w.market,
        &mut w.proposals[c],
        &w.market.guardian_authority,
        T0,
    )
    .unwrap();
    execute_at(&mut w, a, T0 + DELAY).unwrap();
    assert_eq!(
        execute_at(&mut w, a, T0 + DELAY + 1),
        Err(MarketError::ConfigProposalNotPending)
    );
    assert_eq!(
        execute_at(&mut w, c, T0 + DELAY),
        Err(MarketError::ConfigProposalNotPending)
    );
    assert_eq!(
        execute_at(&mut w, b, T0 + DELAY),
        Err(MarketError::ConfigProposalBaseChanged)
    );
    for id in [0, 1, 2, 4] {
        assert_eq!(
            propose(
                &w.market,
                &mut w.asset,
                &risk,
                id,
                risk_value(|r| r.ltv_bps = 5_300),
                T0
            )
            .map(drop),
            Err(MarketError::ConfigProposalIdMismatch)
        );
    }
    assert_eq!(w.asset.generation, 2);
}

/// A cap decrease blocks a pending raise of the same group, and a new raise
/// from the decreased value waits the full delay.
#[test]
fn cap_decrease_then_raise_needs_the_full_delay() {
    let mut w = World::new();
    let risk = w.market.risk_authority;
    let old = w.asset.caps;
    let raised = Caps {
        supply_cap_usdc: old.supply_cap_usdc + USDC,
        ..old
    };
    let raise = propose_at(&mut w, ConfigValue::Caps { caps: raised }, T0);
    let lowered = Caps {
        supply_cap_usdc: old.supply_cap_usdc - USDC,
        ..old
    };
    decrease_caps(
        &mut w.market,
        &mut w.asset,
        &risk,
        ConfigValue::Caps { caps: lowered },
    )
    .unwrap();
    assert_eq!(
        execute_at(&mut w, raise, T0 + DELAY),
        Err(MarketError::ConfigProposalBaseChanged)
    );
    // An immediate raise is refused.
    assert_eq!(
        decrease_caps(
            &mut w.market,
            &mut w.asset,
            &risk,
            ConfigValue::Caps { caps: raised }
        )
        .map(drop),
        Err(MarketError::CapChangeNotDecrease)
    );
    let again = propose_at(&mut w, ConfigValue::Caps { caps: old }, T0 + 1);
    assert_eq!(
        execute_at(&mut w, again, T0 + 1 + DELAY - 1),
        Err(MarketError::TimelockNotElapsed)
    );
    assert_eq!(execute_at(&mut w, again, T0 + 1 + DELAY), Ok(()));
    assert_eq!(w.asset.caps, old);
}

// ---- Targeted tests: pause expiry -----------------------------------------------------

#[test]
fn guardian_pause_blocks_before_its_expiry_slot_and_not_at_it() {
    let k = market();
    let mut p = PauseState::default();
    guardian_pause(
        &k,
        &mut p,
        &k.guardian_authority,
        pause_flags::BORROW,
        GUARDIAN_MAX_PAUSE_SLOTS,
        SLOT0,
    )
    .unwrap();
    let expiry = SLOT0 + GUARDIAN_MAX_PAUSE_SLOTS;
    assert_eq!(p.guardian_expiry_slot, expiry);
    assert_eq!(
        ensure_action_allowed(&p, Action::Borrow, expiry - 1),
        Err(MarketError::ActionPaused)
    );
    assert_eq!(ensure_action_allowed(&p, Action::Borrow, expiry), Ok(()));
    // The guardian cannot stretch it: a re-pause keeps the expiry.
    guardian_pause(
        &k,
        &mut p,
        &k.guardian_authority,
        pause_flags::SUPPLY,
        GUARDIAN_MAX_PAUSE_SLOTS,
        expiry - 1,
    )
    .unwrap();
    assert_eq!(p.guardian_expiry_slot, expiry);
    assert_eq!(ensure_action_allowed(&p, Action::Supply, expiry), Ok(()));
}

#[test]
fn guardian_repause_waits_exactly_until_the_cooldown_end() {
    let k = market();
    let g = k.guardian_authority;
    let mut p = PauseState::default();
    guardian_pause(&k, &mut p, &g, pause_flags::BORROW, 10, SLOT0).unwrap();
    let ready = SLOT0 + 10 + GUARDIAN_PAUSE_COOLDOWN_SLOTS;
    let before = p;
    assert_eq!(
        guardian_pause(&k, &mut p, &g, pause_flags::BORROW, 10, ready - 1),
        Err(MarketError::GuardianPauseCooldown)
    );
    assert_eq!(p, before);
    assert_eq!(
        guardian_pause(&k, &mut p, &g, pause_flags::BORROW, 10, ready),
        Ok(())
    );
    assert_eq!(p.guardian_expiry_slot, ready + 10);
}

/// Every raw pause mask (including the rescue bits, which no instruction can
/// set), active or expired: rescue actions are never blocked.
#[test]
fn rescue_actions_are_never_blocked_by_any_pause_state() {
    let rescue: Vec<Action> = Action::ALL.into_iter().filter(|a| a.is_rescue()).collect();
    assert_eq!(rescue.len(), 7);
    for mask in 0..=pause_flags::ALL {
        for (flags, guardian_flags) in [(mask, pause_flags::ALL), (pause_flags::ALL, mask)] {
            let p = PauseState {
                flags,
                guardian_flags,
                guardian_expiry_slot: SLOT0,
            };
            for slot in [0, SLOT0 - 1, SLOT0, u64::MAX] {
                for action in &rescue {
                    assert_eq!(ensure_action_allowed(&p, *action, slot), Ok(()));
                }
            }
        }
    }
}

// ---- Targeted tests: generation invalidation -------------------------------------------

/// Executions and cap decreases bump the generation once and stale an
/// Intent queued before them without moving value; proposing, cancelling,
/// pausing, and role changes do not bump it.
#[test]
fn only_executions_and_cap_decreases_bump_the_generation() {
    type Op = fn(&mut World);
    fn op(name: &'static str, f: Op, bumps: bool) -> (&'static str, Op, bool) {
        (name, f, bumps)
    }
    let ops = [
        op(
            "execute",
            |w| {
                let p = propose_at(w, risk_value(|r| r.ltv_bps = 5_200), T0);
                execute_at(w, p, T0 + DELAY).unwrap();
            },
            true,
        ),
        op(
            "decrease_caps",
            |w| {
                let mut caps = w.asset.caps;
                caps.supply_cap_usdc -= 1;
                let risk = w.market.risk_authority;
                decrease_caps(
                    &mut w.market,
                    &mut w.asset,
                    &risk,
                    ConfigValue::Caps { caps },
                )
                .unwrap();
            },
            true,
        ),
        op(
            "propose",
            |w| {
                propose_at(w, risk_value(|r| r.ltv_bps = 5_200), T0);
            },
            false,
        ),
        op(
            "cancel",
            |w| {
                let p = propose_at(w, risk_value(|r| r.ltv_bps = 5_200), T0);
                let g = w.market.guardian_authority;
                cancel(&w.market, &mut w.proposals[p], &g, T0).unwrap();
            },
            false,
        ),
        op(
            "failed execute",
            |w| {
                let p = propose_at(w, risk_value(|r| r.ltv_bps = 5_200), T0);
                execute_at(w, p, T0).unwrap_err();
            },
            false,
        ),
        op(
            "guardian_pause",
            |w| {
                let g = w.market.guardian_authority;
                guardian_pause(
                    &w.market,
                    &mut w.asset.pauses,
                    &g,
                    pause_flags::ALL & !0x18,
                    5,
                    w.slot,
                )
                .unwrap();
            },
            false,
        ),
        op(
            "set_pause",
            |w| {
                let r = w.market.risk_authority;
                set_pause(
                    &w.market,
                    &mut w.asset.pauses,
                    &r,
                    pause_flags::BORROW,
                    true,
                )
                .unwrap();
            },
            false,
        ),
        op(
            "set_role",
            |w| {
                let admin = w.market.admin;
                set_role(&mut w.market, &MARKET_KEY, &admin, RoleKind::Risk, key(10)).unwrap();
                propose_admin(&mut w.market, &MARKET_KEY, &admin, key(2)).unwrap();
            },
            false,
        ),
    ];
    for (name, run, bumps) in ops {
        let mut w = World::new();
        // Queue a real Borrow Intent capturing the current generation.
        let bindings = probe_bindings();
        let fields = probe_fields();
        let mut intent = *probe_template();
        intent.asset_generation = w.asset.generation;
        let mut position = ProtectedAccount::new(key(0x51));
        position.generation = Generation::new(7);
        queue(&intent, &mut [&mut position], intent.lock_expiry_slot).unwrap();
        let captured = w.asset.generation;

        run(&mut w);
        assert_eq!(w.asset.generation, captured + u64::from(bumps), "{name}");

        let mut current = CurrentDependencies::captured_by(&intent);
        current.asset_generation = w.asset.generation;
        let ctx = CallbackContext {
            output_authenticated: true,
            computation_offset: 9,
            intent_key: intent.key,
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
        assert!(!position.lock.is_active(), "{name}: Lock released");
        assert_eq!(result.locks_released, 1);
        if bumps {
            assert_eq!(
                decision,
                CallbackDecision::Stale(StaleReason::AssetGeneration),
                "{name}"
            );
            assert_eq!(intent.status, IntentStatus::Stale);
            assert_eq!(result.generations_bumped, 0);
            assert_eq!(position.generation.value(), 7);
            assert_eq!(ledger, 0, "{name}: a stale callback moves no value");
        } else {
            assert_eq!(decision, CallbackDecision::Applied, "{name}");
            assert_eq!(ledger, 250);
        }
    }
}

// ---- Targeted tests: cap boundaries -----------------------------------------------------

#[test]
fn cap_checks_pass_at_the_cap_and_resolve_to_zero_one_above() {
    let caps = asset().caps;
    let supply = u128::from(caps.supply_cap_usdc);
    let total = u128::from(caps.total_debt_cap_usdc);
    let position = u128::from(caps.position_debt_cap_usdc);
    assert_eq!(check_supply_cap(&caps, supply), CapCheck::Within);
    assert_eq!(check_supply_cap(&caps, supply + 1), CapCheck::Exceeded);
    assert_eq!(check_total_debt_cap(&caps, total), CapCheck::Within);
    assert_eq!(check_total_debt_cap(&caps, total + 1), CapCheck::Exceeded);
    assert_eq!(check_position_debt_cap(&caps, position), CapCheck::Within);
    assert_eq!(
        check_position_debt_cap(&caps, position + 1),
        CapCheck::Exceeded
    );
    assert_eq!(
        new_risk_outcome(&[CapCheck::Within, CapCheck::Exceeded]),
        NewRiskOutcome::Zero
    );
    // Exactly reaching each cap proceeds; one more base unit is zero.
    assert_eq!(
        supply_cap_outcome(&caps, supply - 5, 5),
        NewRiskOutcome::Proceed
    );
    assert_eq!(
        supply_cap_outcome(&caps, supply - 5, 6),
        NewRiskOutcome::Zero
    );
    assert_eq!(
        borrow_cap_outcome(&caps, total - 5, 0, 5),
        NewRiskOutcome::Proceed
    );
    assert_eq!(
        borrow_cap_outcome(&caps, total - 5, 0, 6),
        NewRiskOutcome::Zero
    );
    assert_eq!(
        borrow_cap_outcome(&caps, 0, position - 5, 5),
        NewRiskOutcome::Proceed
    );
    assert_eq!(
        borrow_cap_outcome(&caps, 0, position - 5, 6),
        NewRiskOutcome::Zero
    );
    // Passive interest already above the cap: every amount resolves to zero,
    // including zero itself and overflowing sums, and nothing errors.
    assert_eq!(
        supply_cap_outcome(&caps, supply + 1, 0),
        NewRiskOutcome::Zero
    );
    assert_eq!(
        supply_cap_outcome(&caps, u128::MAX, u64::MAX),
        NewRiskOutcome::Zero
    );
}

#[test]
fn count_caps_stop_exactly_at_the_cap() {
    let mut k = market();
    for expected in 1..=k.position_count_cap {
        assert_eq!(on_position_initialized(&mut k).unwrap().new_count, expected);
    }
    assert_eq!(k.position_count, k.position_count_cap);
    let before = k.clone();
    assert_eq!(
        on_position_initialized(&mut k).map(drop),
        Err(MarketError::PositionCountCapReached)
    );
    assert_eq!(k, before);
    for _ in 0..k.debt_position_count_cap {
        assert_eq!(
            reserve_debt_slot(&mut k, Action::Borrow, false),
            Ok(DebtSlot::Reserved)
        );
    }
    assert_eq!(
        k.debt_position_count + k.debt_position_reservations,
        k.debt_position_count_cap
    );
    let before = k.clone();
    assert_eq!(
        reserve_debt_slot(&mut k, Action::Borrow, false),
        Ok(DebtSlot::Unavailable)
    );
    assert_eq!(k, before);
    // A borrow from a Position that already has debt needs no slot.
    assert_eq!(
        reserve_debt_slot(&mut k, Action::Borrow, true),
        Ok(DebtSlot::NotNeeded)
    );
}

#[test]
fn cap_bounds_accept_exact_edges_and_reject_one_past() {
    let at_max = Caps {
        supply_cap_usdc: MAX_USDC_CAP,
        total_debt_cap_usdc: MAX_USDC_CAP,
        position_debt_cap_usdc: MAX_USDC_CAP,
    };
    assert_eq!(check_caps_bounds(&at_max), Ok(()));
    for over in [
        Caps {
            supply_cap_usdc: MAX_USDC_CAP + 1,
            ..at_max
        },
        Caps {
            total_debt_cap_usdc: 10,
            position_debt_cap_usdc: 11,
            ..at_max
        },
        Caps {
            supply_cap_usdc: 10,
            total_debt_cap_usdc: 11,
            position_debt_cap_usdc: 0,
        },
    ] {
        assert_eq!(
            check_caps_bounds(&over),
            Err(MarketError::ConfigOutOfBounds)
        );
    }
    let counts = CountCaps {
        position_count_cap: MAX_POSITION_COUNT_CAP,
        debt_position_count_cap: MAX_POSITION_COUNT_CAP,
    };
    assert_eq!(check_count_caps_bounds(&counts), Ok(()));
    for over in [
        CountCaps {
            position_count_cap: MAX_POSITION_COUNT_CAP + 1,
            debt_position_count_cap: 0,
        },
        CountCaps {
            position_count_cap: 5,
            debt_position_count_cap: 6,
        },
    ] {
        assert_eq!(
            check_count_caps_bounds(&over),
            Err(MarketError::ConfigOutOfBounds)
        );
    }
    // Raise limit edge: max(2 × old, old + floor) passes, one more fails.
    let mut w = World::new();
    let old = w.asset.caps;
    let edge =
        (old.position_debt_cap_usdc * 2).max(old.position_debt_cap_usdc + USDC_CAP_RAISE_FLOOR);
    let risk = w.market.risk_authority;
    for (value, ok) in [(edge, true), (edge + 1, false)] {
        let caps = Caps {
            position_debt_cap_usdc: value,
            ..old
        };
        let id = w.asset.timelock.next_proposal_id;
        let result = propose(
            &w.market,
            &mut w.asset,
            &risk,
            id,
            ConfigValue::Caps { caps },
            T0,
        );
        assert_eq!(result.is_ok(), ok, "{value}");
        if !ok {
            assert_eq!(result.map(drop), Err(MarketError::ConfigMovementTooLarge));
        }
    }
}

// ---- Targeted tests: liquidation cross-bound ------------------------------------------

/// Every liquidation configuration the real bounds admit (on a 25-bps grid,
/// including both ends of each range) satisfies the cross-bound. With the
/// current bounds the cross-bound is implied by the field bounds
/// (`0.85 × 1.15 = 0.9775 <= 0.99` and `1.01 >= 0.9875`); the check stays as
/// defence in depth and the next test shows it is not vacuous.
#[test]
fn cross_bound_holds_for_every_in_bounds_liquidation_config() {
    let grid = |b: Bound| (b.min..=b.max).step_by(50).chain([b.max]);
    let mut accepted = 0u64;
    for lt in grid(bounds::LIQUIDATION_THRESHOLD) {
        for bonus in grid(bounds::LIQUIDATION_BONUS) {
            for thf in grid(bounds::TARGET_HEALTH_FACTOR).step_by(4) {
                let params = RiskParams {
                    ltv_bps: bounds::LTV.min as u16,
                    liquidation_threshold_bps: lt as u16,
                    liquidation_bonus_bps: bonus as u16,
                    target_health_factor_bps: thf as u16,
                    ..RiskParams::LAUNCH_DEFAULT
                };
                if check_risk_bounds(&params).is_ok() {
                    accepted += 1;
                    assert!(ref_cross_bound(lt, bonus, thf), "{lt} {bonus} {thf}");
                }
            }
        }
    }
    assert!(accepted > 10_000);
}

/// With temporarily widened (test-only) bounds the field ranges admit
/// configurations that break Requirement 4.13, and `check_cross_bound`
/// rejects exactly those, so the check is not vacuous.
#[test]
fn cross_bound_check_catches_widened_bounds() {
    const WIDE_LT: Bound = Bound {
        min: 5_000,
        max: 9_900,
        max_step: 0,
    };
    const WIDE_BONUS: Bound = Bound {
        min: 0,
        max: 3_000,
        max_step: 0,
    };
    const WIDE_THF: Bound = Bound {
        min: 9_000,
        max: 15_000,
        max_step: 0,
    };
    let mut caught = 0u64;
    let mut passed = 0u64;
    for lt in (WIDE_LT.min..=WIDE_LT.max).step_by(50) {
        for bonus in (WIDE_BONUS.min..=WIDE_BONUS.max).step_by(50) {
            for thf in (WIDE_THF.min..=WIDE_THF.max).step_by(100) {
                let params = RiskParams {
                    liquidation_threshold_bps: lt as u16,
                    liquidation_bonus_bps: bonus as u16,
                    target_health_factor_bps: thf as u16,
                    ..RiskParams::LAUNCH_DEFAULT
                };
                let reference = ref_cross_bound(lt, bonus, thf);
                assert_eq!(check_cross_bound(&params).is_ok(), reference);
                if reference {
                    passed += 1;
                } else {
                    caught += 1;
                    assert_eq!(
                        check_cross_bound(&params),
                        Err(MarketError::LiquidationCrossBound)
                    );
                }
            }
        }
    }
    assert!(caught > 1_000 && passed > 1_000, "{caught} / {passed}");
}

#[test]
fn cross_bound_exact_edges() {
    let at = |lt: u16, bonus: u16, thf: u16| {
        check_cross_bound(&RiskParams {
            liquidation_threshold_bps: lt,
            liquidation_bonus_bps: bonus,
            target_health_factor_bps: thf,
            ..RiskParams::LAUNCH_DEFAULT
        })
    };
    // 0.9 × 1.1 = 0.99 exactly: allowed; T = 1.00 = 0.99 + 0.01 exactly.
    assert_eq!(at(9_000, 1_000, 10_000), Ok(()));
    assert_eq!(
        at(9_000, 1_000, 9_999),
        Err(MarketError::LiquidationCrossBound)
    );
    assert_eq!(
        at(9_000, 1_001, 11_000),
        Err(MarketError::LiquidationCrossBound)
    );
    assert_eq!(
        at(9_001, 1_000, 11_000),
        Err(MarketError::LiquidationCrossBound)
    );
}
