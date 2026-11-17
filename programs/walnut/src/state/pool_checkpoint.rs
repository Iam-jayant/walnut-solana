//! Pool accounting checkpoint (Task 7.7, test-only).
//!
//! Follows the Task 3.5 / 4.6 / 5.6 pattern for Pool serialization and reserve
//! withdrawal. The REAL side drives the production functions the instructions
//! call (`queue_reserve_withdrawal`, `settle_reserve_withdrawal`,
//! `abandon_reserve_withdrawal`, `queue_band_update`, `settle_band_update`,
//! `accrue_independently`, `math::checkpoint::reserve_withdrawal` as the
//! plaintext twin of the MPC circuit, `rate::ensure_band_fresh`) and performs
//! the vault transfer where the callback handler does. A reversible test cipher
//! stands in for the MXE: the "circuit" opens the stored Pool payload, computes
//! the output, and seals it under a fresh nonce at queue time (the computation
//! reads the payload as it was queued), and every check reads the stored
//! payload back as integers.
//!
//! The independent REFERENCE MODEL (`M`) is written from the requirements. It
//! calls no production function: its own kinked rate, accrual, projection,
//! checkpoint, band, approval, Lock, generation, and staleness rules.
//!
//! After every action the harness checks, on the real side only, the named
//! invariants below (each failure message starts with the name), then requires
//! the real side and the model to agree on the outcome and a full view.
//!
//! * `LOCK SAFETY`: at most one queued Pool mutation; the Pool Lock is held
//!   exactly by it; a queue on a locked Pool is never accepted.
//! * `GENERATION`: the Pool generation moves by exactly 1 on `Applied`,
//!   `Abandoned`, and an external write, and by 0 otherwise.
//! * `INDEX FROZEN`: the accounting Borrow Index never decreases and cannot
//!   change while the Pool is Locked; it moves only at queue-time accrual, an
//!   independent accrual on an unlocked Pool, or a `Rates` execution.
//! * `INDEX DISTINCTION`: every Intent holds a projected risk index strictly
//!   above its accounting index; a queued Intent's accounting index is the
//!   current one.
//! * `ACCOUNTED INDEX`: the Pool's accounted index never exceeds the index and
//!   changes only on `Applied`, to that Intent's accounting index.
//! * `LEDGER`: vault USDC equals Pool Cash plus the funds held for others,
//!   vault plus treasury is constant, and an approved withdrawal moves exactly
//!   the approved amount out of the vault into the treasury.
//! * `EQUITY`: an approved withdrawal leaves supplier equity (at the Intent's
//!   accounting index) exactly equal to the checkpointed equity before it.
//! * `NEUTRALITY`: every outcome other than `Applied` leaves the encrypted
//!   payload, the accounted index, the vault, and the treasury unchanged.
//! * `TERMINAL`: a terminal Intent never changes; one action resolves at most
//!   one Intent.
//! * `NONCE`: the stored output nonce changes only on `Applied`, and no stored
//!   nonce repeats.
//! * `BAND`: the band and its observation slot change only when a current band
//!   callback writes, and band freshness follows the maximum age.
//!
//! Tests:
//!
//! * `prop_pool_exhaustive_lifecycle_sequences_up_to_length_5`,
//!   `prop_pool_exhaustive_adversarial_sequences_up_to_length_5`, and
//!   `prop_pool_exhaustive_governance_sequences_up_to_length_5`: every
//!   sequence of up to 5 actions (111,110 each) over three 10-action
//!   alphabets.
//! * `prop_pool_random_sequences_full_alphabet`: 20,000 random sequences of up
//!   to 40 actions.
//! * Scripted scenarios with asserted outcomes and ledger values.
//! * `pool_injected_faults_are_caught_by_the_named_invariant`: nine injected
//!   defects, each reported by its named check.

use anchor_lang::prelude::Pubkey;

use super::accounts::{EncryptedPoolState, PoolState, PoolStatus};
use super::band::{
    queue_band_update, settle_band_update, BandKeys, BandReport, BandRequest, BandSettlement,
};
use super::coordination::Coordination;
use super::pool_mutation::accrue_independently;
use super::reserve_withdrawal::{
    abandon_reserve_withdrawal, queue_reserve_withdrawal, settle_reserve_withdrawal,
    ReserveWithdrawalReport, ReserveWithdrawalRequest, ReserveWithdrawalSettlement,
};
use super::shield::TokenAccountView;
use crate::errors::ErrorCode;
use crate::market::{
    pause_flags, Caps, Market, MarketStatus, PauseState, RateParams, RiskParams, TimelockState,
};
use crate::market::{AssetState, WAD};
use crate::math::checkpoint::{checkpoint, reserve_withdrawal, supplier_equity, PoolPlain};
use crate::math::rate::{band_from_pool, ensure_band_fresh};
use crate::math::Lcg;
use crate::operation::{IntentStatus, OperationCounter, OperationIntent, OperationKind};

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}

const TREASURY: u8 = 5;
const TREASURY_TOKEN: u8 = 0x60;
const POOL: u8 = 0x42;
const MARKET: u8 = 0x30;
const START_SLOT: u64 = 100;
const START_TS: i64 = 1_000;
const DAY: i64 = 86_400;
const DECISION: u64 = 300;
const LOCK_GRACE: u64 = 150;
const MAX_BAND_AGE: u64 = 9_000;

// ---- Actions and outcomes ---------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Amt {
    Zero,
    One,
    /// The Pool's Reserves before the checkpoint.
    Reserves,
    /// The exact post-checkpoint minimum of Cash and Reserves.
    Boundary,
    /// One more than `Boundary`.
    BoundaryPlusOne,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Last,
    First,
    Prev,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Queue(Amt),
    CbHonest(Target),
    CbUnauth(Target),
    CbWrongBinding(Target),
    Abandon(Target),
    BandQueue,
    BandCb(Target),
    Advance(u64),
    Accrue,
    RatesExecute,
    ExternalPoolGen,
    AssetGenBump,
    TreasuryToggle,
    TreasuryRotate,
    PauseToggle,
    EnsureBandFresh,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rej {
    Paused,
    ZeroAmount,
    Treasury,
    Locked,
    Abandon,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Out {
    Queued,
    Rejected(Rej),
    Applied(u64),
    BandApplied(bool),
    Zero,
    Stale,
    Expired,
    Ignored,
    Reverted,
    Abandoned,
    NoIntent,
    Accrued,
    Fresh(bool),
    Toggled,
}

const LIFECYCLE: [Action; 10] = [
    Action::Queue(Amt::Boundary),
    Action::Queue(Amt::BoundaryPlusOne),
    Action::CbHonest(Target::Last),
    Action::CbUnauth(Target::Last),
    Action::Abandon(Target::Last),
    Action::Advance(10),
    Action::Advance(301),
    Action::Advance(451),
    Action::BandQueue,
    Action::BandCb(Target::Last),
];

const ADVERSARIAL: [Action; 10] = [
    Action::Queue(Amt::Reserves),
    Action::CbHonest(Target::Last),
    Action::CbHonest(Target::First),
    Action::CbWrongBinding(Target::Last),
    Action::Abandon(Target::Last),
    Action::Advance(10),
    Action::Advance(451),
    Action::Accrue,
    Action::ExternalPoolGen,
    Action::TreasuryRotate,
];

const GOVERNANCE: [Action; 10] = [
    Action::Queue(Amt::One),
    Action::CbHonest(Target::Last),
    Action::AssetGenBump,
    Action::RatesExecute,
    Action::PauseToggle,
    Action::TreasuryToggle,
    Action::BandQueue,
    Action::BandCb(Target::Last),
    Action::Advance(301),
    Action::EnsureBandFresh,
];

fn full_alphabet() -> Vec<Action> {
    let mut all = vec![
        Action::Queue(Amt::Zero),
        Action::Queue(Amt::One),
        Action::Queue(Amt::Reserves),
        Action::Queue(Amt::Boundary),
        Action::Queue(Amt::BoundaryPlusOne),
        Action::CbUnauth(Target::First),
        Action::CbWrongBinding(Target::First),
        Action::CbHonest(Target::Prev),
        Action::BandCb(Target::First),
        Action::Advance(9_100),
    ];
    for a in LIFECYCLE.iter().chain(&ADVERSARIAL).chain(&GOVERNANCE) {
        if !all.contains(a) {
            all.push(*a);
        }
    }
    all
}

// ---- The test cipher --------------------------------------------------------

fn mask(nonce: u128, field: usize) -> u128 {
    nonce
        .wrapping_mul(0x9E37_79B9_7F4A_7C15_F39C_C060_5CED_C835)
        .rotate_left(17 + field as u32)
        ^ (0xA5A5_5A5A_u128 << (field * 8))
        ^ (field as u128 + 1)
}

fn seal(plain: &PoolPlain, nonce: u128) -> EncryptedPoolState {
    let values = [
        plain.cash,
        plain.total_supply_shares,
        plain.total_debt_shares,
        plain.reserves,
    ];
    let mut ciphertexts = [[0u8; 32]; 4];
    for (i, value) in values.iter().enumerate() {
        ciphertexts[i][..16].copy_from_slice(&(value ^ mask(nonce, i)).to_le_bytes());
        ciphertexts[i][16..].copy_from_slice(&nonce.to_le_bytes());
    }
    EncryptedPoolState { nonce, ciphertexts }
}

fn open(state: &EncryptedPoolState) -> PoolPlain {
    let value = |i: usize| {
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&state.ciphertexts[i][..16]);
        u128::from_le_bytes(bytes) ^ mask(state.nonce, i)
    };
    PoolPlain {
        cash: value(0),
        total_supply_shares: value(1),
        total_debt_shares: value(2),
        reserves: value(3),
    }
}

// ---- The real side ----------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    /// A refused request writes the circuit output.
    ZeroWritesState,
    /// A stale callback transfers the planned amount anyway.
    TransferOnStale,
    /// A rejected independent accrual moves the index while the Pool is Locked.
    IndexMovesWhileLocked,
    /// Abandonment forgets the generation bump.
    AbandonKeepsGeneration,
    /// The projected risk index collapses into the accounting index.
    RiskEqualsAccounting,
    /// A stale callback leaves the Pool Lock held.
    LockLeakOnStale,
    /// A stale band callback writes the band.
    BandWrittenWhenStale,
    /// An approved withdrawal transfers twice.
    DoubleTransfer,
    /// An approved withdrawal transfers one unit more than approved.
    TransferOffByOne,
}

#[derive(Clone, Debug)]
enum RPlan {
    Reserve {
        out: EncryptedPoolState,
        approved: u64,
        requested: u64,
    },
    Band(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Honest,
    Unauth,
    WrongBinding,
}

#[derive(Clone)]
struct Real {
    market: Market,
    asset: AssetState,
    pool: PoolState,
    counter: OperationCounter,
    intents: Vec<(OperationIntent, RPlan)>,
    slot: u64,
    ts: i64,
    vault: u128,
    treasury: u128,
    others: u128,
    total: u128,
    nonce_ctr: u128,
    fault: Option<Fault>,
}

fn market_fixture() -> Market {
    Market {
        layout_version: 1,
        status: MarketStatus::Active,
        bump: 0xFE,
        vault_authority_bump: 0xFD,
        admin: key(1),
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
        treasury_usdc_account: key(TREASURY_TOKEN),
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

fn asset_fixture() -> AssetState {
    AssetState {
        layout_version: 1,
        bump: 1,
        market: key(MARKET),
        generation: 5,
        oracle_generation: 2,
        rate_epoch_generation: 3,
        accounting_borrow_index: WAD,
        accounting_index_timestamp: START_TS,
        utilization_band: 4,
        utilization_observation_slot: 10,
        risk: RiskParams::LAUNCH_DEFAULT,
        rates: RateParams::LAUNCH_DEFAULT,
        caps: Caps::default(),
        pauses: PauseState::default(),
        timelock: TimelockState::default(),
        reserved: [0; 128],
    }
}

/// The starting Pool contents: 0 rich, 1 canonical empty, 2 reserve-heavy.
fn variant_plain(variant: u8) -> PoolPlain {
    match variant % 3 {
        0 => PoolPlain {
            cash: 1_000_000_000,
            total_supply_shares: 1_500_000_000,
            total_debt_shares: 600_000_000,
            reserves: 5_000_000,
        },
        1 => PoolPlain::default(),
        _ => PoolPlain {
            cash: 800_000_000,
            total_supply_shares: 900_000_000,
            total_debt_shares: 100_000_000,
            reserves: 400_000_000,
        },
    }
}

const OTHERS: u128 = 250_000_000;

impl Real {
    fn new(variant: u8) -> Self {
        let plain = variant_plain(variant);
        let vault = plain.cash + OTHERS;
        Self {
            market: market_fixture(),
            asset: asset_fixture(),
            pool: PoolState {
                coordination: Coordination {
                    generation: 9,
                    ..Coordination::default()
                },
                bump: 1,
                layout_version: 1,
                status: PoolStatus::Active,
                market: key(MARKET),
                accounted_borrow_index: WAD,
                encrypted: seal(&plain, 1),
                unsettled_recovery_claim_count: 0,
            },
            counter: OperationCounter::new(key(POOL), 255),
            intents: Vec::new(),
            slot: START_SLOT,
            ts: START_TS,
            vault,
            treasury: 0,
            others: OTHERS,
            total: vault,
            nonce_ctr: 1,
            fault: None,
        }
    }

    fn plain(&self) -> PoolPlain {
        open(&self.pool.encrypted)
    }

    fn dest(&self) -> TokenAccountView {
        TokenAccountView {
            key: key(TREASURY_TOKEN),
            mint: key(0x46),
            owner: key(TREASURY),
        }
    }

    fn fresh_nonce(&mut self) -> u128 {
        self.nonce_ctr += 1;
        self.nonce_ctr
    }

    fn resolve(&self, model: &M, amt: Amt) -> u64 {
        model.resolve(amt)
    }

    fn target(&self, target: Target) -> Option<usize> {
        pick(self.intents.len(), target)
    }

    fn queue(&mut self, requested: u64) -> Out {
        let plain = self.plain();
        let signer = self.market.treasury_authority;
        let dest = self.dest();
        let request = ReserveWithdrawalRequest {
            intent_key: key(0x80 + self.intents.len() as u8),
            sequence: self.counter.next_sequence,
            computation_offset: 10_000 + self.intents.len() as u64,
            queued_slot: self.slot,
            now_ts: self.ts,
            amount: requested,
        };
        let result = queue_reserve_withdrawal(
            &self.market,
            key(MARKET),
            self.market.asset_state,
            &mut self.asset,
            &mut self.pool,
            key(POOL),
            &mut self.counter,
            &signer,
            &dest,
            &request,
        );
        let mut intent = match result {
            Ok(intent) => intent,
            Err(error) => {
                return Out::Rejected(match error {
                    ErrorCode::ActionPaused => Rej::Paused,
                    ErrorCode::ReserveWithdrawalZeroAmount => Rej::ZeroAmount,
                    ErrorCode::TreasuryAccountNotSet | ErrorCode::TreasuryAccountInvalid => {
                        Rej::Treasury
                    }
                    ErrorCode::PoolLockedForIndexMutation => Rej::Locked,
                    other => panic!("unexpected queue error {other:?}"),
                })
            }
        };
        if self.fault == Some(Fault::RiskEqualsAccounting) {
            intent.risk_borrow_index_at_expiry = intent.accounting_borrow_index;
        }
        // The circuit: checkpoint first, then all-or-zero approval, sealed
        // under a fresh MXE nonce. It reads the payload as queued.
        let out_plain = reserve_withdrawal(
            &plain,
            self.pool.accounted_borrow_index,
            intent.accounting_borrow_index,
            self.asset.risk.reserve_factor_bps,
            requested,
        );
        let nonce = self.fresh_nonce();
        self.intents.push((
            intent,
            RPlan::Reserve {
                out: seal(&out_plain.pool, nonce),
                approved: out_plain.approved,
                requested,
            },
        ));
        Out::Queued
    }

    fn band_keys(&self) -> BandKeys {
        BandKeys {
            payer: key(1),
            market: key(MARKET),
            asset_state: self.market.asset_state,
            pool: key(POOL),
        }
    }

    fn band_queue(&mut self) -> Out {
        let plain = self.plain();
        let request = BandRequest {
            intent_key: key(0x80 + self.intents.len() as u8),
            sequence: self.counter.next_sequence,
            computation_offset: 10_000 + self.intents.len() as u64,
            queued_slot: self.slot,
        };
        let intent = queue_band_update(
            self.market.status,
            &self.band_keys(),
            &self.asset,
            &self.pool,
            &mut self.counter,
            &request,
        )
        .expect("band queue");
        let band = band_from_pool(
            plain.cash,
            plain.total_debt_shares,
            intent.accounting_borrow_index,
        )
        .unwrap();
        self.intents.push((intent, RPlan::Band(band)));
        Out::Queued
    }

    fn callback(&mut self, index: usize, mode: Mode) -> Out {
        let (mut intent, plan) = self.intents[index].clone();
        match plan {
            RPlan::Reserve {
                out,
                approved,
                requested,
            } => {
                let mut destination = self.dest();
                if mode == Mode::WrongBinding {
                    destination.key = key(0x61);
                }
                let report = ReserveWithdrawalReport {
                    output_authenticated: mode != Mode::Unauth,
                    computation_offset: intent.computation_offset,
                    intent_key: intent.key,
                    output: out,
                    approved_amount: approved,
                    requested_amount: requested,
                    destination,
                    mint: self.market.usdc_mint,
                    vault: self.market.usdc_vault,
                    vault_authority: self.market.vault_authority,
                    token_program: anchor_spl::token::ID,
                    current_slot: self.slot,
                };
                let result = settle_reserve_withdrawal(
                    &self.market,
                    key(MARKET),
                    &self.asset,
                    &mut self.pool,
                    key(POOL),
                    &mut intent,
                    &report,
                );
                let settlement = match result {
                    Ok(settlement) => settlement,
                    Err(_) => return Out::Reverted,
                };
                self.intents[index].0 = intent;
                let outcome = match settlement {
                    ReserveWithdrawalSettlement::Approved { amount, .. } => {
                        assert!(self.vault >= amount as u128, "vault underflow");
                        self.vault -= amount as u128;
                        self.treasury += amount as u128;
                        if self.fault == Some(Fault::DoubleTransfer) {
                            self.vault -= amount as u128;
                            self.treasury += amount as u128;
                        }
                        if self.fault == Some(Fault::TransferOffByOne) {
                            self.vault -= 1;
                            self.treasury += 1;
                        }
                        Out::Applied(amount)
                    }
                    ReserveWithdrawalSettlement::Zero => {
                        if self.fault == Some(Fault::ZeroWritesState) {
                            self.pool.encrypted = report.output;
                        }
                        Out::Zero
                    }
                    ReserveWithdrawalSettlement::Failed(IntentStatus::Stale) => {
                        if self.fault == Some(Fault::TransferOnStale) && approved > 0 {
                            self.vault -= approved as u128;
                            self.treasury += approved as u128;
                        }
                        if self.fault == Some(Fault::LockLeakOnStale) {
                            self.pool.coordination.lock = crate::lock::Lock {
                                active: true,
                                computation_offset: intent.computation_offset,
                                operation_intent: intent.key,
                                expiry_slot: intent.lock_expiry_slot,
                            };
                        }
                        Out::Stale
                    }
                    ReserveWithdrawalSettlement::Failed(IntentStatus::Expired) => Out::Expired,
                    ReserveWithdrawalSettlement::Failed(status) => panic!("status {status:?}"),
                    ReserveWithdrawalSettlement::Ignored => Out::Ignored,
                };
                outcome
            }
            RPlan::Band(band) => {
                let report = BandReport {
                    output_authenticated: mode != Mode::Unauth,
                    computation_offset: intent.computation_offset,
                    intent_key: intent.key,
                    band,
                    current_slot: self.slot,
                };
                let keys = self.band_keys();
                let result =
                    settle_band_update(&keys, &mut self.asset, &self.pool, &mut intent, &report);
                let settlement = match result {
                    Ok(settlement) => settlement,
                    Err(_) => return Out::Reverted,
                };
                self.intents[index].0 = intent;
                match settlement {
                    BandSettlement::Applied { written, .. } => Out::BandApplied(written),
                    BandSettlement::Stale(_) => {
                        if self.fault == Some(Fault::BandWrittenWhenStale) {
                            self.asset.utilization_band = 19;
                            self.asset.utilization_observation_slot = self.slot;
                        }
                        Out::Stale
                    }
                    BandSettlement::Failed(IntentStatus::Expired) => Out::Expired,
                    BandSettlement::Failed(status) => panic!("status {status:?}"),
                    BandSettlement::Ignored => Out::Ignored,
                }
            }
        }
    }

    fn apply(&mut self, model: &M, action: Action) -> Out {
        match action {
            Action::Queue(amt) => {
                let requested = self.resolve(model, amt);
                self.queue(requested)
            }
            Action::CbHonest(t) | Action::CbUnauth(t) | Action::CbWrongBinding(t) => {
                let Some(i) = self.target(t) else {
                    return Out::NoIntent;
                };
                let is_reserve = matches!(self.intents[i].1, RPlan::Reserve { .. });
                let mode = match action {
                    Action::CbUnauth(_) => Mode::Unauth,
                    Action::CbWrongBinding(_) => Mode::WrongBinding,
                    _ => Mode::Honest,
                };
                if mode == Mode::WrongBinding && !is_reserve {
                    return Out::NoIntent;
                }
                self.callback(i, mode)
            }
            Action::BandCb(t) => {
                let Some(i) = self.target(t) else {
                    return Out::NoIntent;
                };
                if matches!(self.intents[i].1, RPlan::Reserve { .. }) {
                    return Out::NoIntent;
                }
                self.callback(i, Mode::Honest)
            }
            Action::Abandon(t) => {
                let Some(i) = self.target(t) else {
                    return Out::NoIntent;
                };
                if !matches!(self.intents[i].1, RPlan::Reserve { .. }) {
                    return Out::NoIntent;
                }
                let mut intent = self.intents[i].0;
                match abandon_reserve_withdrawal(&mut self.pool, key(POOL), &mut intent, self.slot)
                {
                    Ok(_) => {
                        self.intents[i].0 = intent;
                        if self.fault == Some(Fault::AbandonKeepsGeneration) {
                            self.pool.coordination.generation -= 1;
                        }
                        Out::Abandoned
                    }
                    Err(_) => Out::Rejected(Rej::Abandon),
                }
            }
            Action::BandQueue => self.band_queue(),
            Action::Advance(slots) => {
                self.slot += slots;
                self.ts += DAY;
                Out::Toggled
            }
            Action::Accrue => match accrue_independently(&mut self.asset, &self.pool, self.ts) {
                Ok(()) => Out::Accrued,
                Err(ErrorCode::PoolLockedForIndexMutation) => {
                    if self.fault == Some(Fault::IndexMovesWhileLocked) {
                        self.asset.accounting_borrow_index += 1;
                    }
                    Out::Rejected(Rej::Locked)
                }
                Err(other) => panic!("unexpected accrual error {other:?}"),
            },
            Action::RatesExecute => {
                match accrue_independently(&mut self.asset, &self.pool, self.ts) {
                    Ok(()) => {
                        self.asset.rate_epoch_generation += 1;
                        self.asset.generation += 1;
                        Out::Accrued
                    }
                    Err(ErrorCode::PoolLockedForIndexMutation) => Out::Rejected(Rej::Locked),
                    Err(other) => panic!("unexpected rates error {other:?}"),
                }
            }
            Action::ExternalPoolGen => {
                self.pool.coordination.generation += 1;
                Out::Toggled
            }
            Action::AssetGenBump => {
                self.asset.generation += 1;
                Out::Toggled
            }
            Action::TreasuryToggle => {
                self.market.treasury_usdc_account =
                    if self.market.treasury_usdc_account == Pubkey::default() {
                        key(TREASURY_TOKEN)
                    } else {
                        Pubkey::default()
                    };
                Out::Toggled
            }
            Action::TreasuryRotate => {
                self.market.treasury_authority = if self.market.treasury_authority == key(TREASURY)
                {
                    key(0x77)
                } else {
                    key(TREASURY)
                };
                Out::Toggled
            }
            Action::PauseToggle => {
                self.asset.pauses.flags ^= pause_flags::WITHDRAW_SUPPLY;
                Out::Toggled
            }
            Action::EnsureBandFresh => Out::Fresh(
                ensure_band_fresh(
                    self.asset.utilization_observation_slot,
                    self.slot,
                    self.asset.rates.band_max_age_slots,
                )
                .is_ok(),
            ),
        }
    }
}

fn pick(len: usize, target: Target) -> Option<usize> {
    match target {
        Target::Last => len.checked_sub(1),
        Target::First => (len > 0).then_some(0),
        Target::Prev => len.checked_sub(2),
    }
}

// ---- The independent reference model ---------------------------------------

#[derive(Clone, Copy, Debug)]
enum MPlan {
    Reserve {
        cash: u128,
        reserves: u128,
        approved: u64,
    },
    Band(u8),
}

#[derive(Clone, Copy, Debug)]
struct MI {
    reserve: bool,
    status: IntentStatus,
    plan: MPlan,
    acc_index: u128,
    acc_ts: i64,
    pool_gen: u64,
    asset_gen: u64,
    rate_epoch: u64,
    version: u64,
    queued_slot: u64,
    decision_expiry: u64,
    lock_expiry: u64,
}

#[derive(Clone)]
struct M {
    slot: u64,
    ts: i64,
    index: u128,
    index_ts: i64,
    band: u8,
    obs: u64,
    rate_epoch: u64,
    asset_gen: u64,
    pool_gen: u64,
    lock: Option<usize>,
    accounted: u128,
    cash: u128,
    supply: u128,
    debt_shares: u128,
    reserves: u128,
    version: u64,
    vault: u128,
    treasury: u128,
    intents: Vec<MI>,
    next_seq: u64,
    paused: bool,
    treasury_set: bool,
    rotated: bool,
}

fn cdiv(a: u128, b: u128) -> u128 {
    a.div_ceil(b)
}

/// Annual borrow rate (bps) of the launch curve: base 0, slope1 400 up to the
/// 80% kink, slope2 7,500 above it.
fn m_rate_bps(u: u128) -> u128 {
    if u <= 8_000 {
        cdiv(400 * u, 8_000)
    } else {
        400 + cdiv(7_500 * (u - 8_000), 2_000)
    }
}

fn m_band_rate(band: u8) -> u128 {
    let upper = (band as u128 + 1) * 500;
    cdiv(m_rate_bps(upper) * WAD, 10_000 * 31_536_000)
}

fn m_accrue(index: u128, from: i64, to: i64, band: u8) -> u128 {
    if to <= from {
        return index;
    }
    let dt = (to - from) as u128;
    index + cdiv(index * m_band_rate(band) * dt, WAD)
}

fn m_debt(shares: u128, index: u128) -> u128 {
    cdiv(shares * index, WAD)
}

fn m_band_of(cash: u128, debt: u128) -> u8 {
    let total = cash + debt;
    if total == 0 {
        return 0;
    }
    (1..20u128)
        .filter(|k| debt * 10_000 >= total * k * 500)
        .count() as u8
}

impl M {
    fn new(variant: u8) -> Self {
        let p = variant_plain(variant);
        let vault = p.cash + OTHERS;
        Self {
            slot: START_SLOT,
            ts: START_TS,
            index: WAD,
            index_ts: START_TS,
            band: 4,
            obs: 10,
            rate_epoch: 3,
            asset_gen: 5,
            pool_gen: 9,
            lock: None,
            accounted: WAD,
            cash: p.cash,
            supply: p.total_supply_shares,
            debt_shares: p.total_debt_shares,
            reserves: p.reserves,
            version: 0,
            vault,
            treasury: 0,
            intents: Vec::new(),
            next_seq: 0,
            paused: false,
            treasury_set: true,
            rotated: false,
        }
    }

    /// Post-accrual index at the current time (what a queue would capture).
    fn index_now(&self) -> u128 {
        m_accrue(self.index, self.index_ts, self.ts, self.band)
    }

    /// (cash', reserves', approved) of a request against the current Pool at
    /// accounting index `idx`.
    fn plan(&self, idx: u128, amount: u64) -> (u128, u128, u64) {
        let interest = m_debt(self.debt_shares, idx) - m_debt(self.debt_shares, self.accounted);
        let reserves = self.reserves + interest * 1_000 / 10_000;
        let a = amount as u128;
        if a > 0 && a <= self.cash && a <= reserves {
            (self.cash - a, reserves - a, amount)
        } else {
            (self.cash, self.reserves, 0)
        }
    }

    fn resolve(&self, amt: Amt) -> u64 {
        let idx = self.index_now();
        let interest = m_debt(self.debt_shares, idx) - m_debt(self.debt_shares, self.accounted);
        let reserves = self.reserves + interest * 1_000 / 10_000;
        let boundary = self.cash.min(reserves) as u64;
        match amt {
            Amt::Zero => 0,
            Amt::One => 1,
            Amt::Reserves => self.reserves as u64,
            Amt::Boundary => boundary,
            Amt::BoundaryPlusOne => boundary + 1,
        }
    }

    fn tgt(&self, t: Target) -> Option<usize> {
        pick(self.intents.len(), t)
    }

    fn stale(&self, i: &MI) -> bool {
        i.pool_gen != self.pool_gen
            || i.asset_gen != self.asset_gen
            || i.rate_epoch != self.rate_epoch
            || i.acc_index != self.index
            || i.acc_ts != self.index_ts
    }

    fn queue(&mut self, amount: u64) -> Out {
        if self.paused {
            return Out::Rejected(Rej::Paused);
        }
        if amount == 0 {
            return Out::Rejected(Rej::ZeroAmount);
        }
        if !self.treasury_set || self.rotated {
            return Out::Rejected(Rej::Treasury);
        }
        if self.lock.is_some() {
            return Out::Rejected(Rej::Locked);
        }
        let idx = self.index_now();
        let (cash, reserves, approved) = self.plan(idx, amount);
        if self.ts > self.index_ts {
            self.index = idx;
            self.index_ts = self.ts;
        }
        let expiry_ts = self.ts.max(self.index_ts) + DECISION as i64;
        let risk_check = m_accrue(self.index, self.index_ts, expiry_ts, self.band);
        assert!(risk_check > self.index);
        self.intents.push(MI {
            reserve: true,
            status: IntentStatus::Queued,
            plan: MPlan::Reserve {
                cash,
                reserves,
                approved,
            },
            acc_index: self.index,
            acc_ts: self.index_ts,
            pool_gen: self.pool_gen,
            asset_gen: self.asset_gen,
            rate_epoch: self.rate_epoch,
            version: self.version,
            queued_slot: self.slot,
            decision_expiry: self.slot + DECISION,
            lock_expiry: self.slot + DECISION + LOCK_GRACE,
        });
        self.lock = Some(self.intents.len() - 1);
        self.next_seq += 1;
        Out::Queued
    }

    fn band_queue(&mut self) -> Out {
        let band = m_band_of(self.cash, m_debt(self.debt_shares, self.index));
        self.intents.push(MI {
            reserve: false,
            status: IntentStatus::Queued,
            plan: MPlan::Band(band),
            acc_index: self.index,
            acc_ts: self.index_ts,
            pool_gen: self.pool_gen,
            asset_gen: self.asset_gen,
            rate_epoch: self.rate_epoch,
            version: self.version,
            queued_slot: self.slot,
            decision_expiry: self.slot + DECISION,
            lock_expiry: self.slot + DECISION + LOCK_GRACE,
        });
        self.next_seq += 1;
        Out::Queued
    }

    fn callback(&mut self, index: usize, unauth: bool, wrong_binding: bool) -> Out {
        let i = self.intents[index];
        if unauth {
            return Out::Reverted;
        }
        if i.status != IntentStatus::Queued {
            return Out::Ignored;
        }
        match i.plan {
            MPlan::Reserve {
                cash,
                reserves,
                approved,
            } => {
                if wrong_binding {
                    return Out::Reverted;
                }
                let release = |m: &mut M, status: IntentStatus| {
                    m.intents[index].status = status;
                    if m.lock == Some(index) {
                        m.lock = None;
                    }
                };
                if self.stale(&i) {
                    release(self, IntentStatus::Stale);
                    return Out::Stale;
                }
                if self.slot > i.decision_expiry {
                    release(self, IntentStatus::Expired);
                    return Out::Expired;
                }
                if approved == 0 {
                    release(self, IntentStatus::Zero);
                    return Out::Zero;
                }
                if !self.treasury_set || self.rotated {
                    return Out::Reverted;
                }
                release(self, IntentStatus::Applied);
                self.cash = cash;
                self.reserves = reserves;
                self.accounted = i.acc_index;
                self.version += 1;
                self.pool_gen += 1;
                self.vault -= approved as u128;
                self.treasury += approved as u128;
                Out::Applied(approved)
            }
            MPlan::Band(band) => {
                if wrong_binding {
                    return Out::NoIntent;
                }
                if i.version != self.version {
                    return Out::Reverted;
                }
                if self.stale(&i) {
                    self.intents[index].status = IntentStatus::Stale;
                    return Out::Stale;
                }
                if self.slot > i.decision_expiry {
                    self.intents[index].status = IntentStatus::Expired;
                    return Out::Expired;
                }
                self.intents[index].status = IntentStatus::Applied;
                let written = i.queued_slot >= self.obs;
                if written {
                    self.band = band;
                    self.obs = i.queued_slot;
                }
                Out::BandApplied(written)
            }
        }
    }

    fn apply(&mut self, action: Action) -> Out {
        match action {
            Action::Queue(amt) => {
                let amount = self.resolve(amt);
                self.queue(amount)
            }
            Action::CbHonest(t) | Action::CbUnauth(t) | Action::CbWrongBinding(t) => {
                let Some(i) = self.tgt(t) else {
                    return Out::NoIntent;
                };
                let wrong = matches!(action, Action::CbWrongBinding(_));
                if wrong && !self.intents[i].reserve {
                    return Out::NoIntent;
                }
                self.callback(i, matches!(action, Action::CbUnauth(_)), wrong)
            }
            Action::BandCb(t) => {
                let Some(i) = self.tgt(t) else {
                    return Out::NoIntent;
                };
                if self.intents[i].reserve {
                    return Out::NoIntent;
                }
                self.callback(i, false, false)
            }
            Action::Abandon(t) => {
                let Some(i) = self.tgt(t) else {
                    return Out::NoIntent;
                };
                if !self.intents[i].reserve {
                    return Out::NoIntent;
                }
                let it = self.intents[i];
                if it.status != IntentStatus::Queued
                    || self.lock != Some(i)
                    || self.slot <= it.lock_expiry
                {
                    return Out::Rejected(Rej::Abandon);
                }
                self.intents[i].status = IntentStatus::Abandoned;
                self.pool_gen += 1;
                self.lock = None;
                Out::Abandoned
            }
            Action::BandQueue => self.band_queue(),
            Action::Advance(slots) => {
                self.slot += slots;
                self.ts += DAY;
                Out::Toggled
            }
            Action::Accrue => {
                if self.lock.is_some() {
                    return Out::Rejected(Rej::Locked);
                }
                let idx = self.index_now();
                if self.ts > self.index_ts {
                    self.index = idx;
                    self.index_ts = self.ts;
                }
                Out::Accrued
            }
            Action::RatesExecute => {
                if self.lock.is_some() {
                    return Out::Rejected(Rej::Locked);
                }
                let idx = self.index_now();
                if self.ts > self.index_ts {
                    self.index = idx;
                    self.index_ts = self.ts;
                }
                self.rate_epoch += 1;
                self.asset_gen += 1;
                Out::Accrued
            }
            Action::ExternalPoolGen => {
                self.pool_gen += 1;
                Out::Toggled
            }
            Action::AssetGenBump => {
                self.asset_gen += 1;
                Out::Toggled
            }
            Action::TreasuryToggle => {
                self.treasury_set = !self.treasury_set;
                Out::Toggled
            }
            Action::TreasuryRotate => {
                self.rotated = !self.rotated;
                Out::Toggled
            }
            Action::PauseToggle => {
                self.paused = !self.paused;
                Out::Toggled
            }
            Action::EnsureBandFresh => {
                let fresh =
                    self.obs != 0 && self.obs <= self.slot && self.slot - self.obs <= MAX_BAND_AGE;
                Out::Fresh(fresh)
            }
        }
    }
}

// ---- Views, invariants, and the runner --------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct View {
    slot: u64,
    ts: i64,
    index: u128,
    index_ts: i64,
    band: u8,
    obs: u64,
    rate_epoch: u64,
    asset_gen: u64,
    pool_gen: u64,
    lock_holder: Option<usize>,
    accounted: u128,
    pool: (u128, u128, u128, u128),
    vault: u128,
    treasury: u128,
    statuses: Vec<IntentStatus>,
    next_seq: u64,
}

fn real_view(r: &Real) -> View {
    let p = r.plain();
    let lock = r.pool.coordination.lock;
    View {
        slot: r.slot,
        ts: r.ts,
        index: r.asset.accounting_borrow_index,
        index_ts: r.asset.accounting_index_timestamp,
        band: r.asset.utilization_band,
        obs: r.asset.utilization_observation_slot,
        rate_epoch: r.asset.rate_epoch_generation,
        asset_gen: r.asset.generation,
        pool_gen: r.pool.coordination.generation,
        lock_holder: if lock.active {
            r.intents
                .iter()
                .position(|(i, _)| i.key == lock.operation_intent)
        } else {
            None
        },
        accounted: r.pool.accounted_borrow_index,
        pool: (
            p.cash,
            p.total_supply_shares,
            p.total_debt_shares,
            p.reserves,
        ),
        vault: r.vault,
        treasury: r.treasury,
        statuses: r.intents.iter().map(|(i, _)| i.status).collect(),
        next_seq: r.counter.next_sequence,
    }
}

fn model_view(m: &M) -> View {
    View {
        slot: m.slot,
        ts: m.ts,
        index: m.index,
        index_ts: m.index_ts,
        band: m.band,
        obs: m.obs,
        rate_epoch: m.rate_epoch,
        asset_gen: m.asset_gen,
        pool_gen: m.pool_gen,
        lock_holder: m.lock,
        accounted: m.accounted,
        pool: (m.cash, m.supply, m.debt_shares, m.reserves),
        vault: m.vault,
        treasury: m.treasury,
        statuses: m.intents.iter().map(|i| i.status).collect(),
        next_seq: m.next_seq,
    }
}

macro_rules! fail {
    ($name:expr, $($arg:tt)*) => {
        return Err(format!("{}: {}", $name, format!($($arg)*)))
    };
}

fn check(
    before: &Real,
    after: &Real,
    action: Action,
    out: Out,
    seen_nonces: &mut Vec<u128>,
) -> Result<(), String> {
    let b = real_view(before);
    let a = real_view(after);
    let (pb, pa) = (before.plain(), after.plain());
    let applied = matches!(out, Out::Applied(_));

    // LOCK SAFETY.
    let queued: Vec<usize> = after
        .intents
        .iter()
        .enumerate()
        .filter(|(_, (i, _))| {
            i.kind == OperationKind::ReserveWithdrawal && i.status == IntentStatus::Queued
        })
        .map(|(n, _)| n)
        .collect();
    if queued.len() > 1 {
        fail!("LOCK SAFETY", "{} queued Pool mutations", queued.len());
    }
    let lock = after.pool.coordination.lock;
    match (lock.active, queued.first()) {
        (true, Some(&n)) => {
            if !after.intents[n].0.lock_owner().holds(&lock) {
                fail!("LOCK SAFETY", "the Lock is not held by the queued Intent");
            }
        }
        (true, None) => fail!("LOCK SAFETY", "a Lock is held with no queued Pool mutation"),
        (false, Some(_)) => fail!("LOCK SAFETY", "a queued Pool mutation holds no Lock"),
        (false, None) => {}
    }
    if out == Out::Queued
        && matches!(action, Action::Queue(_))
        && before.pool.coordination.lock.active
    {
        fail!("LOCK SAFETY", "a queue was accepted on a locked Pool");
    }

    // GENERATION.
    let expected_delta =
        u64::from(applied || out == Out::Abandoned || matches!(action, Action::ExternalPoolGen));
    if a.pool_gen != b.pool_gen + expected_delta {
        fail!(
            "GENERATION",
            "{} -> {} after {action:?} ({out:?}), expected +{expected_delta}",
            b.pool_gen,
            a.pool_gen
        );
    }

    // INDEX FROZEN.
    if a.index < b.index {
        fail!("INDEX FROZEN", "the accounting index decreased");
    }
    if before.pool.coordination.lock.active && a.index != b.index {
        fail!("INDEX FROZEN", "the index moved while the Pool was Locked");
    }
    let may_move = matches!(
        (action, out),
        (Action::Queue(_), Out::Queued)
            | (Action::Accrue, Out::Accrued)
            | (Action::RatesExecute, Out::Accrued)
    );
    if a.index != b.index && !may_move {
        fail!("INDEX FROZEN", "the index moved on {action:?} ({out:?})");
    }

    // INDEX DISTINCTION.
    for (n, (i, _)) in after.intents.iter().enumerate() {
        if i.kind != OperationKind::ReserveWithdrawal {
            continue;
        }
        if i.risk_borrow_index_at_expiry <= i.accounting_borrow_index {
            fail!(
                "INDEX DISTINCTION",
                "Intent {n}: risk index not above accounting"
            );
        }
        if i.accounting_borrow_index > a.index {
            fail!("INDEX DISTINCTION", "Intent {n} captured a future index");
        }
        if i.status == IntentStatus::Queued && i.accounting_borrow_index != a.index {
            fail!("INDEX DISTINCTION", "queued Intent {n} lost its index");
        }
    }

    // ACCOUNTED INDEX.
    if a.accounted > a.index {
        fail!("ACCOUNTED INDEX", "accounted above the accounting index");
    }
    if a.accounted != b.accounted {
        let ok = applied
            && after
                .intents
                .iter()
                .zip(&before.intents)
                .any(|((n, _), (o, _))| {
                    o.status == IntentStatus::Queued
                        && n.status == IntentStatus::Applied
                        && n.accounting_borrow_index == a.accounted
                });
        if !ok {
            fail!("ACCOUNTED INDEX", "moved on {action:?} ({out:?})");
        }
    }

    // LEDGER.
    if a.vault != pa.cash + after.others {
        fail!("LEDGER", "vault {} != cash {} + others", a.vault, pa.cash);
    }
    if a.vault + a.treasury != after.total {
        fail!(
            "LEDGER",
            "vault + treasury {} != {}",
            a.vault + a.treasury,
            after.total
        );
    }
    match out {
        Out::Applied(amount) => {
            let amount = amount as u128;
            if b.vault - a.vault != amount || a.treasury - b.treasury != amount {
                fail!("LEDGER", "transferred {} not {amount}", b.vault - a.vault);
            }
            if pb.cash - pa.cash != amount {
                fail!("LEDGER", "cash fell by {} not {amount}", pb.cash - pa.cash);
            }
        }
        _ => {
            if a.vault != b.vault || a.treasury != b.treasury {
                fail!("LEDGER", "value moved on {action:?} ({out:?})");
            }
        }
    }

    // EQUITY and NEUTRALITY.
    if applied {
        let accounting = after
            .intents
            .iter()
            .zip(&before.intents)
            .find(|((n, _), (o, _))| {
                o.status == IntentStatus::Queued && n.status == IntentStatus::Applied
            })
            .map(|((n, _), _)| n.accounting_borrow_index)
            .ok_or("EQUITY: no Intent became Applied")?;
        let cp = checkpoint(&pb, before.pool.accounted_borrow_index, accounting, 1_000)
            .map_err(|e| format!("EQUITY: checkpoint {e:?}"))?;
        let after_equity = supplier_equity(pa.cash, cp.new_debt, pa.reserves)
            .map_err(|e| format!("EQUITY: {e:?}"))?;
        if after_equity != cp.supplier_equity {
            fail!("EQUITY", "{after_equity} != {}", cp.supplier_equity);
        }
        if (pa.total_supply_shares, pa.total_debt_shares)
            != (pb.total_supply_shares, pb.total_debt_shares)
        {
            fail!("EQUITY", "share counts changed");
        }
    } else if after.pool.encrypted != before.pool.encrypted || a.accounted != b.accounted {
        fail!(
            "NEUTRALITY",
            "the payload or accounted index changed on {action:?} ({out:?})"
        );
    }

    // TERMINAL.
    let mut changed = 0;
    for (n, ((ai, _), (bi, _))) in after.intents.iter().zip(&before.intents).enumerate() {
        if ai.status != bi.status {
            changed += 1;
            if bi.status != IntentStatus::Queued {
                fail!("TERMINAL", "terminal Intent {n} changed");
            }
        }
    }
    if changed > 1 {
        fail!("TERMINAL", "{changed} Intents resolved in one action");
    }

    // NONCE.
    if after.pool.encrypted.nonce != before.pool.encrypted.nonce {
        if !applied {
            fail!("NONCE", "the stored nonce changed without Applied");
        }
        if seen_nonces.contains(&after.pool.encrypted.nonce) {
            fail!("NONCE", "a stored nonce repeated");
        }
        seen_nonces.push(after.pool.encrypted.nonce);
    }

    // BAND.
    if (a.band, a.obs) != (b.band, b.obs) && !matches!(out, Out::BandApplied(true)) {
        fail!("BAND", "the band changed on {action:?} ({out:?})");
    }
    if a.obs < b.obs {
        fail!("BAND", "the observation slot moved back");
    }
    if let Out::Fresh(fresh) = out {
        let expected = a.obs != 0 && a.obs <= a.slot && a.slot - a.obs <= MAX_BAND_AGE;
        if fresh != expected {
            fail!("BAND", "freshness {fresh} but expected {expected}");
        }
    }
    Ok(())
}

/// One action on the real side and the model, then every check and the
/// agreement. `Err` carries the failing check's message.
fn step(
    real: &mut Real,
    model: &mut M,
    seen: &mut Vec<u128>,
    action: Action,
    path: &[Action],
) -> Result<Out, String> {
    let before = real.clone();
    let model_before = model.clone();
    let out_model = model.apply(action);
    let out_real = real.apply(&model_before, action);
    check(&before, real, action, out_real, seen)
        .map_err(|e| format!("{e} ({action:?} after {path:?})"))?;
    if out_real != out_model {
        return Err(format!(
            "AGREEMENT: outcome {out_real:?} vs model {out_model:?} ({action:?} after {path:?})"
        ));
    }
    let (rv, mv) = (real_view(real), model_view(model));
    if rv != mv {
        return Err(format!(
            "AGREEMENT: view differs ({action:?} after {path:?}):\n real {rv:?}\nmodel {mv:?}"
        ));
    }
    Ok(out_real)
}

/// Runs `actions` from a fresh world; returns the final worlds and outcomes.
fn play(
    actions: &[Action],
    variant: u8,
    fault: Option<Fault>,
) -> Result<(Real, M, Vec<Out>), String> {
    let mut real = Real::new(variant);
    real.fault = fault;
    let mut model = M::new(variant);
    let mut seen = vec![real.pool.encrypted.nonce];
    let mut outs = Vec::new();
    for (n, &action) in actions.iter().enumerate() {
        outs.push(step(
            &mut real,
            &mut model,
            &mut seen,
            action,
            &actions[..n],
        )?);
    }
    Ok((real, model, outs))
}

fn play_ok(actions: &[Action], variant: u8) -> (Real, M, Vec<Out>) {
    play(actions, variant, None).unwrap_or_else(|e| panic!("{e}"))
}

/// Depth-first over every sequence of up to `depth` actions, reusing each
/// prefix's world. Returns the number of sequences visited.
fn exhaustive(alphabet: &[Action], depth: usize, variant: u8) -> usize {
    fn go(
        alphabet: &[Action],
        depth: usize,
        real: &Real,
        model: &M,
        seen: &[u128],
        path: &mut Vec<Action>,
        count: &mut usize,
    ) {
        if depth == 0 {
            return;
        }
        for &action in alphabet {
            let (mut r, mut m, mut s) = (real.clone(), model.clone(), seen.to_vec());
            if let Err(e) = step(&mut r, &mut m, &mut s, action, path) {
                panic!("{e}");
            }
            *count += 1;
            path.push(action);
            go(alphabet, depth - 1, &r, &m, &s, path, count);
            path.pop();
        }
    }
    let real = Real::new(variant);
    let model = M::new(variant);
    let seen = vec![real.pool.encrypted.nonce];
    let mut count = 0;
    go(
        alphabet,
        depth,
        &real,
        &model,
        &seen,
        &mut Vec::new(),
        &mut count,
    );
    count
}

#[test]
fn prop_pool_exhaustive_lifecycle_sequences_up_to_length_5() {
    assert_eq!(exhaustive(&LIFECYCLE, 5, 0), 111_110);
}

#[test]
fn prop_pool_exhaustive_adversarial_sequences_up_to_length_5() {
    assert_eq!(exhaustive(&ADVERSARIAL, 5, 2), 111_110);
}

#[test]
fn prop_pool_exhaustive_governance_sequences_up_to_length_5() {
    assert_eq!(exhaustive(&GOVERNANCE, 5, 0), 111_110);
}

#[test]
fn prop_pool_random_sequences_full_alphabet() {
    let alphabet = full_alphabet();
    let core = [
        Action::Queue(Amt::Boundary),
        Action::CbHonest(Target::Last),
        Action::Advance(10),
        Action::Advance(301),
        Action::BandQueue,
        Action::BandCb(Target::Last),
    ];
    let mut rng = Lcg(0x7700_0007);
    let mut actions_run = 0usize;
    let mut applied = 0usize;
    for _ in 0..20_000 {
        let variant = (rng.next() % 3) as u8;
        let len = 1 + (rng.next() % 40) as usize;
        let sequence: Vec<Action> = (0..len)
            .map(|_| {
                if rng.next() % 10 < 4 {
                    core[(rng.next() % core.len() as u64) as usize]
                } else {
                    alphabet[(rng.next() % alphabet.len() as u64) as usize]
                }
            })
            .collect();
        match play(&sequence, variant, None) {
            Ok((_, _, outs)) => {
                actions_run += outs.len();
                applied += outs.iter().filter(|o| matches!(o, Out::Applied(_))).count();
            }
            Err(e) => panic!("{e}\nvariant {variant}, sequence {sequence:?}"),
        }
    }
    assert!(actions_run > 300_000, "{actions_run}");
    assert!(applied > 1_000, "only {applied} approved withdrawals");
}

// ---- Scripted scenarios -----------------------------------------------------

use Action::*;

#[test]
fn pool_approved_withdrawal_moves_exactly_the_approved_amount() {
    let (real, model, outs) = play_ok(
        &[Advance(10), Queue(Amt::Boundary), CbHonest(Target::Last)],
        0,
    );
    let Out::Applied(amount) = outs[2] else {
        panic!("{outs:?}")
    };
    assert!(amount > 5_000_000, "interest was added to the reserves");
    assert_eq!(real.treasury, amount as u128);
    assert_eq!(real.vault + real.treasury, real.total);
    assert_eq!(real.plain().cash, 1_000_000_000 - amount as u128);
    assert_eq!(model.cash, real.plain().cash);
    assert_eq!(real.pool.coordination.generation, 10);
    assert!(!real.pool.coordination.lock.active);
    assert_eq!(
        real.pool.accounted_borrow_index,
        real.asset.accounting_borrow_index
    );
    // The accounting index moved at queue time, not at the callback.
    assert!(real.asset.accounting_borrow_index > WAD);
}

#[test]
fn pool_an_oversized_request_returns_zero_and_commits_nothing() {
    let (real, _, outs) = play_ok(
        &[
            Advance(10),
            Queue(Amt::BoundaryPlusOne),
            CbHonest(Target::Last),
        ],
        0,
    );
    assert_eq!(outs[2], Out::Zero);
    assert_eq!(real.treasury, 0);
    assert_eq!(
        real.plain().reserves,
        5_000_000,
        "the checkpoint is not committed"
    );
    assert_eq!(real.pool.accounted_borrow_index, WAD);
    assert_eq!(real.pool.coordination.generation, 9);
    assert!(!real.pool.coordination.lock.active);
    // The next withdrawal still checkpoints the whole interval once.
    let (real, _, outs) = play_ok(
        &[
            Advance(10),
            Queue(Amt::BoundaryPlusOne),
            CbHonest(Target::Last),
            Queue(Amt::Boundary),
            CbHonest(Target::Last),
        ],
        0,
    );
    assert!(matches!(outs[4], Out::Applied(_)));
    assert_eq!(real.pool.coordination.generation, 10);
}

#[test]
fn pool_stale_expired_and_late_callbacks_are_neutral() {
    let (r, _, o) = play_ok(
        &[Queue(Amt::Boundary), AssetGenBump, CbHonest(Target::Last)],
        0,
    );
    assert_eq!(o[2], Out::Stale);
    assert_eq!((r.treasury, r.pool.coordination.generation), (0, 9));
    let (r, _, o) = play_ok(
        &[
            Queue(Amt::Boundary),
            ExternalPoolGen,
            CbHonest(Target::Last),
        ],
        0,
    );
    assert_eq!(o[2], Out::Stale);
    assert_eq!((r.treasury, r.pool.coordination.generation), (0, 10));
    let (r, _, o) = play_ok(
        &[Queue(Amt::Boundary), Advance(301), CbHonest(Target::Last)],
        0,
    );
    assert_eq!(o[2], Out::Expired);
    assert_eq!(r.treasury, 0);
    let (r, _, o) = play_ok(
        &[
            Queue(Amt::Boundary),
            Advance(451),
            Abandon(Target::Last),
            CbHonest(Target::Last),
            CbHonest(Target::Last),
        ],
        0,
    );
    assert_eq!(&o[2..], [Out::Abandoned, Out::Ignored, Out::Ignored]);
    assert_eq!((r.treasury, r.pool.coordination.generation), (0, 10));
    assert!(!r.pool.coordination.lock.active);
}

#[test]
fn pool_concurrent_queue_and_independent_index_writes_fail_while_locked() {
    let (r, _, o) = play_ok(
        &[
            Queue(Amt::One),
            Queue(Amt::One),
            Advance(10),
            Accrue,
            RatesExecute,
            BandQueue,
        ],
        0,
    );
    assert_eq!(o[1], Out::Rejected(Rej::Locked));
    assert_eq!(o[3], Out::Rejected(Rej::Locked));
    assert_eq!(o[4], Out::Rejected(Rej::Locked));
    // The read-only band update is not blocked by the Lock.
    assert_eq!(o[5], Out::Queued);
    assert!(r.pool.coordination.lock.active);
    // After the callback the index may move again.
    let (r, _, o) = play_ok(
        &[
            Queue(Amt::One),
            CbHonest(Target::Last),
            Advance(10),
            Accrue,
            RatesExecute,
        ],
        0,
    );
    assert_eq!((o[3], o[4]), (Out::Accrued, Out::Accrued));
    assert_eq!(r.asset.rate_epoch_generation, 4);
}

#[test]
fn pool_pause_treasury_and_rotation_gate_queue_and_delivery() {
    let (_, _, o) = play_ok(
        &[PauseToggle, Queue(Amt::One), PauseToggle, Queue(Amt::One)],
        0,
    );
    assert_eq!((o[1], o[3]), (Out::Rejected(Rej::Paused), Out::Queued));
    let (_, _, o) = play_ok(
        &[
            TreasuryToggle,
            Queue(Amt::One),
            TreasuryRotate,
            Queue(Amt::One),
        ],
        0,
    );
    assert_eq!(o[1], Out::Rejected(Rej::Treasury));
    assert_eq!(o[3], Out::Rejected(Rej::Treasury));
    assert_eq!(
        play_ok(&[Queue(Amt::Zero)], 0).2[0],
        Out::Rejected(Rej::ZeroAmount)
    );
    // A treasury rotated after the queue cannot receive the transfer; the
    // callback reverts and the Intent still applies once the account is valid
    // again.
    let (r, _, o) = play_ok(
        &[
            Queue(Amt::One),
            TreasuryRotate,
            CbHonest(Target::Last),
            TreasuryRotate,
            CbHonest(Target::Last),
        ],
        0,
    );
    assert_eq!((o[2], o[4]), (Out::Reverted, Out::Applied(1)));
    assert_eq!(r.treasury, 1);
}

#[test]
fn pool_unauthenticated_wrong_binding_and_duplicate_callbacks_change_nothing() {
    let (r, _, o) = play_ok(
        &[
            Queue(Amt::One),
            CbUnauth(Target::Last),
            CbWrongBinding(Target::Last),
            CbHonest(Target::Last),
            CbHonest(Target::Last),
            CbUnauth(Target::Last),
            CbWrongBinding(Target::Last),
        ],
        0,
    );
    assert_eq!(
        &o[1..],
        [
            Out::Reverted,
            Out::Reverted,
            Out::Applied(1),
            Out::Ignored,
            Out::Reverted,
            Out::Ignored
        ]
    );
    assert_eq!(r.treasury, 1);
    assert_eq!(r.pool.coordination.generation, 10);
}

#[test]
fn pool_band_staleness_freshness_and_payload_binding() {
    // A band computed before an index accrual is stale.
    let (r, _, o) = play_ok(&[BandQueue, Advance(10), Accrue, BandCb(Target::Last)], 0);
    assert_eq!(o[3], Out::Stale);
    assert_eq!(r.asset.utilization_band, 4);
    // A current band is stored with the queue slot as its observation.
    let (r, _, o) = play_ok(&[BandQueue, BandCb(Target::Last), EnsureBandFresh], 0);
    assert_eq!((o[1], o[2]), (Out::BandApplied(true), Out::Fresh(true)));
    assert_eq!(r.asset.utilization_observation_slot, START_SLOT);
    assert_eq!(
        r.asset.utilization_band, 7,
        "600M debt / 1.6B total = 37.5%"
    );
    // An old observation blocks new risk.
    let (_, _, o) = play_ok(&[Advance(9_100), EnsureBandFresh], 0);
    assert_eq!(o[1], Out::Fresh(false));
    // A band whose Pool payload moved before delivery reverts (its input
    // commitment no longer matches) and never writes.
    let (r, _, o) = play_ok(
        &[
            BandQueue,
            Queue(Amt::One),
            CbHonest(Target::Last),
            BandCb(Target::First),
        ],
        0,
    );
    assert_eq!((o[2], o[3]), (Out::Applied(1), Out::Reverted));
    assert_eq!(r.asset.utilization_band, 4);
    // The Pool generation and the rate epoch each stale a band.
    let (_, _, o) = play_ok(&[BandQueue, ExternalPoolGen, BandCb(Target::Last)], 0);
    assert_eq!(o[2], Out::Stale);
    let (_, _, o) = play_ok(&[BandQueue, RatesExecute, BandCb(Target::Last)], 0);
    assert_eq!(o[2], Out::Stale);
}

#[test]
fn pool_empty_pool_refuses_every_withdrawal() {
    let (r, _, o) = play_ok(
        &[
            Advance(10),
            Queue(Amt::One),
            CbHonest(Target::Last),
            Queue(Amt::Boundary),
        ],
        1,
    );
    assert_eq!(o[2], Out::Zero);
    // The boundary of an empty Pool is zero, which is not a valid request.
    assert_eq!(o[3], Out::Rejected(Rej::ZeroAmount));
    assert_eq!(r.treasury, 0);
    assert_eq!(r.plain(), PoolPlain::default());
}

#[test]
fn pool_consecutive_withdrawals_each_move_exactly_their_amount() {
    let (r, _, o) = play_ok(
        &[
            Advance(10),
            Queue(Amt::Boundary),
            CbHonest(Target::Last),
            Advance(10),
            Queue(Amt::Boundary),
            CbHonest(Target::Last),
        ],
        2,
    );
    let (Out::Applied(a), Out::Applied(b)) = (o[2], o[5]) else {
        panic!("{o:?}")
    };
    assert_eq!(r.treasury, (a + b) as u128);
    assert_eq!(r.pool.coordination.generation, 11);
    assert_eq!(r.vault + r.treasury, r.total);
}

// ---- Negative controls ------------------------------------------------------

#[test]
fn pool_injected_faults_are_caught_by_the_named_invariant() {
    let cases: [(Fault, &[Action], &str); 9] = [
        (
            Fault::ZeroWritesState,
            &[Queue(Amt::BoundaryPlusOne), CbHonest(Target::Last)],
            "NEUTRALITY",
        ),
        (
            Fault::TransferOnStale,
            &[Queue(Amt::Boundary), AssetGenBump, CbHonest(Target::Last)],
            "LEDGER",
        ),
        (
            Fault::IndexMovesWhileLocked,
            &[Queue(Amt::One), Advance(10), Accrue],
            "INDEX FROZEN",
        ),
        (
            Fault::AbandonKeepsGeneration,
            &[Queue(Amt::One), Advance(451), Abandon(Target::Last)],
            "GENERATION",
        ),
        (
            Fault::RiskEqualsAccounting,
            &[Advance(10), Queue(Amt::One)],
            "INDEX DISTINCTION",
        ),
        (
            Fault::LockLeakOnStale,
            &[Queue(Amt::One), AssetGenBump, CbHonest(Target::Last)],
            "LOCK SAFETY",
        ),
        (
            Fault::BandWrittenWhenStale,
            &[BandQueue, ExternalPoolGen, BandCb(Target::Last)],
            "BAND",
        ),
        (
            Fault::DoubleTransfer,
            &[Queue(Amt::Boundary), CbHonest(Target::Last)],
            "LEDGER",
        ),
        (
            Fault::TransferOffByOne,
            &[Queue(Amt::Boundary), CbHonest(Target::Last)],
            "LEDGER",
        ),
    ];
    for (fault, script, expected) in cases {
        // The same script passes without the fault.
        play(script, 0, None).unwrap_or_else(|e| panic!("clean {fault:?}: {e}"));
        match play(script, 0, Some(fault)) {
            Ok(_) => panic!("{fault:?} went undetected"),
            Err(message) => assert!(
                message.starts_with(expected),
                "{fault:?} reported {message:?}, expected {expected}"
            ),
        }
    }
}

#[test]
fn pool_a_reference_model_defect_is_an_agreement_failure() {
    // A model that forgets to release the Lock on a refusal disagrees with
    // production, so a wrong model cannot hide behind a wrong implementation.
    let mut real = Real::new(0);
    let mut model = M::new(0);
    let mut seen = vec![real.pool.encrypted.nonce];
    for action in [Queue(Amt::BoundaryPlusOne), CbHonest(Target::Last)] {
        let before = real.clone();
        let model_before = model.clone();
        let out_model = model.apply(action);
        let out_real = real.apply(&model_before, action);
        check(&before, &real, action, out_real, &mut seen).unwrap();
        assert_eq!(out_real, out_model);
    }
    model.lock = Some(0);
    assert_ne!(real_view(&real), model_view(&model));
}
