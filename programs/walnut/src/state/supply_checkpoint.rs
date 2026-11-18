//! Supply and withdrawal checkpoint (Task 8.4, test-only).
//!
//! Follows the Task 5.6 / 7.7 pattern. The REAL side drives the production
//! functions the instructions call (`queue_supply_flow`, `settle_supply_flow`,
//! `abandon_supply_flow`, the claim-count hooks) with two Positions that share
//! one Pool. A reversible test cipher stands in for the MXE: the "circuit"
//! opens the stored Pool and Position payloads at queue time, runs the
//! plaintext twin `math::supply::supply_flow`, and seals both outputs under
//! fresh nonces; every check reads the stored payloads back as integers.
//!
//! The independent REFERENCE MODEL (`M`) is written from the requirements and
//! calls no production function: its own checkpoint, share minting, NAV
//! withdrawal, Lock, generation, expiry, and abandonment rules. Index accrual
//! over time is covered by Task 7.7, so the model takes the accounting index
//! the real queue captured as an observed input and only checks it never
//! decreases.
//!
//! After every action the harness checks, on the real side only, the named
//! invariants below (each failure message starts with the name), then requires
//! the real side and the model to agree on the outcome and a full view.
//!
//! * `LEDGER`: vault USDC equals Pool Cash plus the sum of Free USDC (no value
//!   is created or lost by supply, withdrawal, or a failed callback).
//! * `ATOMIC`: `Applied` writes the Pool and the Position together: both
//!   payloads change and both generations move by exactly 1.
//! * `NEUTRALITY`: every outcome other than `Applied` leaves both payloads,
//!   the accounted index, and every generation unchanged (aside from the
//!   abandonment bump).
//! * `NAV`: an applied supply or withdrawal never lowers equity per share for
//!   the other suppliers.
//! * `ROUNDING`: minted shares are worth at most the amount paid; a claim is
//!   at most the pro-rata value of the burned shares.
//! * `NOHIST`: a new supplier's shares are worth at most what they paid at the
//!   instant they were minted, so no interest accrued earlier is captured.
//! * `LOCK`: an accepted queue holds the Pool and the Position Locks; a
//!   terminal Intent holds none.
//! * `NONCE`: no stored Pool or Position nonce repeats.
//!
//! Tests: exhaustive sequences up to length 4 over three alphabets and four
//! worlds, 3,000 random sequences up to 30 actions, scripted scenarios, and
//! `supply_injected_faults_are_caught_by_the_named_invariant` (eight defects).

use anchor_lang::prelude::Pubkey;

use super::accounts::{EncryptedPoolState, EncryptedPositionPayload, PoolState, Position};
use super::first_supplier::{record_absorb_accepted, record_recovery_claim_settled};
use super::nonce::output_nonce;
use super::supply::{
    abandon_supply_flow, queue_supply_flow, settle_supply_flow, SupplyFlowReport,
    SupplyFlowRequest, SupplyFlowSettlement,
};
use super::test_fixtures::{asset, key, market, pool, position, MARKET, OWNER, POOL, POSITION};
use crate::market::{pause_flags, AssetState, Market, WAD};
use crate::math::checkpoint::{checkpoint, PoolPlain};
use crate::math::supply::{supply_flow, SupplyPosition, SupplyRequest};
use crate::math::Lcg;
use crate::operation::{IntentStatus, OperationCounter, OperationIntent};

const START_SLOT: u64 = 100;
const START_TS: i64 = 1_000;
const DECISION: u64 = 1_500;
const LOCK_GRACE: u64 = 300;
const MAX_BAND_AGE: u64 = 9_000;
const MAX64: u128 = u64::MAX as u128;
const MAX_SHARES: u128 = 1 << 63;

// ---- Actions and outcomes ---------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Amt {
    Zero,
    One,
    Quarter,
    Whole,
    WholePlusOne,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Last,
    First,
    Prev,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    /// `(supplier, is_withdraw, amount)`.
    Queue(usize, bool, Amt),
    CbHonest(Target),
    CbUnauth(Target),
    CbWrongBinding(Target),
    Abandon(Target),
    Advance(u64),
    BandRefresh,
    PauseSupply,
    ClaimToggle,
    Shield(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum St {
    Queued,
    Applied,
    Zero,
    Expired,
    Abandoned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Out {
    Queued,
    Rejected,
    Applied(bool),
    Zero,
    Expired,
    Ignored,
    Reverted,
    Abandoned,
    Done,
    NoTarget,
}

fn pick(len: usize, target: Target) -> Option<usize> {
    match target {
        Target::Last => len.checked_sub(1),
        Target::First => (len > 0).then_some(0),
        Target::Prev => len.checked_sub(2),
    }
}

fn amount_of(amt: Amt, held: u128) -> u128 {
    match amt {
        Amt::Zero => 0,
        Amt::One => 1,
        Amt::Quarter => (held / 4).max(1),
        Amt::Whole => held,
        Amt::WholePlusOne => held + 1,
    }
}

// ---- The test cipher --------------------------------------------------------

fn mask(nonce: u128, field: usize) -> u128 {
    nonce
        .wrapping_mul(0x9E37_79B9_7F4A_7C15_F39C_C060_5CED_C835)
        .rotate_left(17 + field as u32)
        ^ (0xA5A5_5A5A_u128 << (field * 8))
        ^ (field as u128 + 1)
}

fn put(ct: &mut [u8; 32], value: u128, nonce: u128, field: usize) {
    ct[..16].copy_from_slice(&(value ^ mask(nonce, field)).to_le_bytes());
    ct[16..].copy_from_slice(&nonce.to_le_bytes());
}

fn get(ct: &[u8; 32], nonce: u128, field: usize) -> u128 {
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&ct[..16]);
    u128::from_le_bytes(bytes) ^ mask(nonce, field)
}

fn seal_pool(p: &PoolPlain, nonce: u128) -> EncryptedPoolState {
    let values = [
        p.cash,
        p.total_supply_shares,
        p.total_debt_shares,
        p.reserves,
    ];
    let mut ciphertexts = [[0u8; 32]; 4];
    for (i, v) in values.iter().enumerate() {
        put(&mut ciphertexts[i], *v, nonce, i);
    }
    EncryptedPoolState { nonce, ciphertexts }
}

fn open_pool(s: &EncryptedPoolState) -> PoolPlain {
    PoolPlain {
        cash: get(&s.ciphertexts[0], s.nonce, 0),
        total_supply_shares: get(&s.ciphertexts[1], s.nonce, 1),
        total_debt_shares: get(&s.ciphertexts[2], s.nonce, 2),
        reserves: get(&s.ciphertexts[3], s.nonce, 3),
    }
}

fn seal_pos(free: u64, shares: u128, nonce: u128) -> EncryptedPositionPayload {
    let values = [0, free as u128, 0, shares, 0];
    let mut ciphertexts = [[0u8; 32]; 5];
    for (i, v) in values.iter().enumerate() {
        put(&mut ciphertexts[i], *v, nonce, 10 + i);
    }
    EncryptedPositionPayload { nonce, ciphertexts }
}

fn open_pos(s: &EncryptedPositionPayload) -> (u64, u128) {
    (
        get(&s.ciphertexts[1], s.nonce, 11) as u64,
        get(&s.ciphertexts[3], s.nonce, 13),
    )
}

fn owner_key(who: usize) -> Pubkey {
    key(OWNER + who as u8)
}

fn pos_key(who: usize) -> Pubkey {
    key(POSITION + who as u8)
}

// ---- The real side ----------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    /// An applied callback stores the Pool but not the Position payload.
    PositionNotStored,
    /// Supply mints against the equity before the interest checkpoint.
    MintWithoutCheckpoint,
    /// Withdrawal skips the Cash coverage check.
    SkipCashCoverage,
    /// A refused request stores the circuit output.
    ZeroWritesState,
    /// An unauthenticated, corrupted output is accepted.
    AcceptUnauthenticated,
    /// Minting rounds up by one share.
    MintRoundsUp,
    /// The first-supplier check ignores the unsettled-claim flag.
    IgnoreClaimsFlag,
    /// An applied callback forgets one Position generation bump.
    PositionGenerationSkipped,
}

#[derive(Clone, Debug)]
struct Plan {
    pool_out: EncryptedPoolState,
    pos_nonce: u128,
    pos_ct: [[u8; 32]; 5],
    applied: bool,
    requested: u128,
}

#[derive(Clone, Debug)]
struct RIntent {
    intent: OperationIntent,
    who: usize,
    plan: Plan,
}

#[derive(Clone)]
struct Real {
    market: Market,
    asset: AssetState,
    pool: PoolState,
    pos: [Position; 2],
    counters: [OperationCounter; 2],
    intents: Vec<RIntent>,
    vault: u128,
    slot: u64,
    ts: i64,
    next_offset: u64,
    nonce_ctr: u128,
    fault: Option<Fault>,
}

fn world(variant: u8) -> (PoolPlain, [(u64, u128); 2]) {
    match variant {
        // Canonically empty, two funded suppliers.
        0 => (PoolPlain::default(), [(10_000, 0), (10_000, 0)]),
        // A live Pool with debt: interest accrues over time.
        1 => (
            PoolPlain {
                cash: 1_000_000_000_000,
                total_supply_shares: 1_000_000_000_000_000,
                total_debt_shares: 500_000_000_000,
                reserves: 0,
            },
            [(5_000, 600_000_000_000_000), (5_000, 400_000_000_000_000)],
        ),
        // Empty of shares but holding dust: the first supplier is refused.
        2 => (
            PoolPlain {
                cash: 7,
                ..PoolPlain::default()
            },
            [(10_000, 0), (10_000, 0)],
        ),
        // Supply shares are worth 10,000 units each: small supplies are dust.
        4 => (
            PoolPlain {
                cash: 10_000_000,
                total_supply_shares: 1_000,
                ..PoolPlain::default()
            },
            [(10_000, 1_000), (10_000, 0)],
        ),
        // Cash covers only a tenth of the supplier equity.
        _ => (
            PoolPlain {
                cash: 1_000,
                total_supply_shares: 1_000_000,
                total_debt_shares: 9_000,
                reserves: 0,
            },
            [(1_000, 1_000_000), (1_000, 0)],
        ),
    }
}

impl Real {
    fn new(variant: u8) -> Self {
        let (plain, holders) = world(variant);
        let mut pool = pool();
        pool.encrypted = seal_pool(&plain, 77);
        let mut pos = [position(), position()];
        let mut vault = plain.cash;
        for (who, (free, shares)) in holders.iter().enumerate() {
            pos[who].owner = owner_key(who);
            pos[who].encrypted_state = seal_pos(*free, *shares, 500 + who as u128);
            vault += *free as u128;
        }
        let mut asset = asset();
        asset.utilization_observation_slot = START_SLOT - 10;
        Real {
            market: market(),
            asset,
            pool,
            pos,
            counters: [
                OperationCounter::new(owner_key(0), 255),
                OperationCounter::new(owner_key(1), 255),
            ],
            intents: Vec::new(),
            vault,
            slot: START_SLOT,
            ts: START_TS,
            next_offset: 10_000,
            nonce_ctr: 0,
            fault: None,
        }
    }

    fn fresh_nonce(&mut self) -> u128 {
        self.nonce_ctr += 1;
        1_000_000 + self.nonce_ctr
    }

    fn holdings(&self, who: usize) -> (u64, u128) {
        open_pos(&self.pos[who].encrypted_state)
    }

    fn queue(&mut self, who: usize, withdraw: bool, amt: Amt) -> Out {
        let (free, shares) = self.holdings(who);
        let requested = amount_of(amt, if withdraw { shares } else { free as u128 });
        let n = self.intents.len();
        let request = SupplyFlowRequest {
            intent_key: key(0x70 + n as u8),
            sequence: self.counters[who].next_sequence,
            computation_offset: self.next_offset,
            queued_slot: self.slot,
            now_ts: self.ts,
            is_withdraw: withdraw,
            requested,
        };
        let result = queue_supply_flow(
            &self.market,
            key(MARKET),
            self.market.asset_state,
            &mut self.asset,
            &mut self.pool,
            key(POOL),
            &mut self.pos[who],
            pos_key(who),
            &mut self.counters[who],
            &owner_key(who),
            &request,
        );
        let Ok(intent) = result else {
            return Out::Rejected;
        };
        self.next_offset += 2;
        let plan = self.plan(who, &intent, withdraw, requested);
        self.intents.push(RIntent { intent, who, plan });
        Out::Queued
    }

    /// The MPC computation: the circuit twin over the payloads as queued.
    fn plan(
        &mut self,
        who: usize,
        intent: &OperationIntent,
        withdraw: bool,
        requested: u128,
    ) -> Plan {
        let pool_plain = open_pool(&self.pool.encrypted);
        let (free, shares) = self.holdings(who);
        let claims_clear = self.market.unsettled_recovery_claim_count == 0
            || self.fault == Some(Fault::IgnoreClaimsFlag);
        let accounted = self.pool.accounted_borrow_index;
        let index = if self.fault == Some(Fault::MintWithoutCheckpoint) && !withdraw {
            accounted
        } else {
            intent.accounting_borrow_index
        };
        let out = supply_flow(
            &pool_plain,
            &SupplyPosition {
                free_usdc: free,
                supply_shares: shares,
            },
            accounted,
            index,
            self.asset.risk.reserve_factor_bps,
            &SupplyRequest {
                is_withdraw: withdraw,
                requested,
                supply_cap: self.asset.caps.supply_cap_usdc,
                claims_clear,
            },
        );
        let mut pool_out = out.pool;
        let mut pos_out = out.position;
        let mut applied = out.applied;
        if self.fault == Some(Fault::SkipCashCoverage)
            && withdraw
            && !applied
            && requested <= shares
        {
            // The defect: pay the NAV claim even though Cash cannot cover it.
            let cp = checkpoint(&pool_plain, accounted, index, 1_000).unwrap();
            let claim = requested * cp.supplier_equity / pool_plain.total_supply_shares;
            pool_out = PoolPlain {
                cash: pool_plain.cash.saturating_sub(claim),
                total_supply_shares: pool_plain.total_supply_shares - requested,
                ..cp.pool
            };
            pos_out = SupplyPosition {
                free_usdc: free + claim as u64,
                supply_shares: shares - requested,
            };
            applied = true;
        }
        if self.fault == Some(Fault::MintRoundsUp) && out.applied && !withdraw {
            pool_out.total_supply_shares += 1;
            pos_out.supply_shares += 1;
        }
        let pool_nonce = self.fresh_nonce();
        let pos_nonce = output_nonce(&pos_key(who), intent.sequence);
        let pos_sealed = seal_pos(pos_out.free_usdc, pos_out.supply_shares, pos_nonce);
        Plan {
            pool_out: seal_pool(&pool_out, pool_nonce),
            pos_nonce,
            pos_ct: pos_sealed.ciphertexts,
            applied,
            requested,
        }
    }

    fn callback(&mut self, index: usize, unauth: bool, wrong_binding: bool) -> Out {
        let who = self.intents[index].who;
        let mut intent = self.intents[index].intent;
        let plan = self.intents[index].plan.clone();
        let before_pos = self.pos[who].encrypted_state;
        let mut report = SupplyFlowReport {
            output_authenticated: !unauth,
            computation_offset: intent.computation_offset,
            intent_key: intent.key,
            pool_output: plan.pool_out,
            encryption_key: self.pos[who].viewing_pubkey,
            position_nonce: plan.pos_nonce,
            position_ciphertexts: plan.pos_ct,
            applied: plan.applied,
            requested: plan.requested + u128::from(wrong_binding),
            current_slot: self.slot,
        };
        if unauth && self.fault == Some(Fault::AcceptUnauthenticated) {
            report.output_authenticated = true;
            report.pool_output.ciphertexts[0][0] ^= 0xFF;
            report.applied = true;
        }
        let result = settle_supply_flow(
            &self.market,
            key(MARKET),
            &self.asset,
            &mut self.pool,
            key(POOL),
            &mut self.pos[who],
            pos_key(who),
            &mut intent,
            &report,
        );
        let out = match result {
            Err(_) => return Out::Reverted,
            Ok(SupplyFlowSettlement::Applied { is_withdraw }) => Out::Applied(is_withdraw),
            Ok(SupplyFlowSettlement::Zero) => Out::Zero,
            Ok(SupplyFlowSettlement::Failed(IntentStatus::Expired)) => Out::Expired,
            Ok(SupplyFlowSettlement::Failed(other)) => panic!("unexpected {other:?}"),
            Ok(SupplyFlowSettlement::Ignored) => Out::Ignored,
        };
        self.intents[index].intent = intent;
        match (out, self.fault) {
            (Out::Applied(_), Some(Fault::PositionNotStored)) => {
                self.pos[who].encrypted_state = before_pos;
            }
            (Out::Applied(_), Some(Fault::PositionGenerationSkipped)) => {
                self.pos[who].coordination.generation -= 1;
            }
            (Out::Zero, Some(Fault::ZeroWritesState)) => {
                self.pool.encrypted = report.pool_output;
            }
            _ => {}
        }
        out
    }

    fn apply(&mut self, action: Action) -> Out {
        match action {
            Action::Queue(who, withdraw, amt) => self.queue(who, withdraw, amt),
            Action::CbHonest(t) => match pick(self.intents.len(), t) {
                Some(i) => self.callback(i, false, false),
                None => Out::NoTarget,
            },
            Action::CbUnauth(t) => match pick(self.intents.len(), t) {
                Some(i) => self.callback(i, true, false),
                None => Out::NoTarget,
            },
            Action::CbWrongBinding(t) => match pick(self.intents.len(), t) {
                Some(i) => self.callback(i, false, true),
                None => Out::NoTarget,
            },
            Action::Abandon(t) => {
                let Some(i) = pick(self.intents.len(), t) else {
                    return Out::NoTarget;
                };
                let who = self.intents[i].who;
                let mut intent = self.intents[i].intent;
                match abandon_supply_flow(
                    &mut self.pool,
                    key(POOL),
                    &mut self.pos[who],
                    pos_key(who),
                    &mut intent,
                    self.slot,
                ) {
                    Ok(_) => {
                        self.intents[i].intent = intent;
                        Out::Abandoned
                    }
                    Err(_) => Out::Rejected,
                }
            }
            Action::Advance(n) => {
                self.slot += n;
                self.ts += n as i64;
                Out::Done
            }
            Action::BandRefresh => {
                self.asset.utilization_observation_slot = self.slot;
                Out::Done
            }
            Action::PauseSupply => {
                self.asset.pauses.flags ^= pause_flags::SUPPLY;
                Out::Done
            }
            Action::ClaimToggle => {
                if self.market.unsettled_recovery_claim_count == 0 {
                    record_absorb_accepted(&mut self.market).unwrap();
                } else {
                    record_recovery_claim_settled(&mut self.market).unwrap();
                }
                Out::Done
            }
            Action::Shield(who) => {
                if self.pos[who].coordination.lock.is_active() {
                    return Out::Rejected;
                }
                let (free, shares) = self.holdings(who);
                let nonce = self.fresh_nonce();
                self.pos[who].encrypted_state = seal_pos(free + 1_000, shares, nonce);
                self.pos[who].coordination.generation += 1;
                self.vault += 1_000;
                Out::Done
            }
        }
    }
}

// ---- The independent reference model ----------------------------------------

#[derive(Clone, Debug)]
struct MI {
    who: usize,
    withdraw: bool,
    req: u128,
    claims_clear: bool,
    accounted: u128,
    idx: u128,
    decision_expiry: u64,
    lock_expiry: u64,
    status: St,
    pool: [u128; 4],
    free: u64,
    shares: u128,
}

#[derive(Clone)]
struct M {
    pool: [u128; 4], // cash, supply shares, debt shares, reserves
    accounted: u128,
    idx: u128,
    slot: u64,
    free: [u64; 2],
    shares: [u128; 2],
    pool_gen: u64,
    pos_gen: [u64; 2],
    lock: Option<usize>,
    claims: u64,
    paused: bool,
    band_slot: u64,
    intents: Vec<MI>,
}

fn cdiv(a: u128, b: u128) -> u128 {
    a.div_ceil(b)
}

/// The requirement, restated: checkpoint first, then all-or-zero.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn m_flow(
    pool: [u128; 4],
    accounted: u128,
    idx: u128,
    free: u64,
    shares: u128,
    withdraw: bool,
    req: u128,
    claims_clear: bool,
) -> Option<([u128; 4], u64, u128)> {
    let [cash, ts, ds, res] = pool;
    let interest = cdiv(ds * idx, WAD) - cdiv(ds * accounted, WAD);
    let res2 = res + interest * 1_000 / 10_000; // launch reserve factor 10%
    let equity = cash + cdiv(ds * idx, WAD) - res2;
    if equity > MAX64 || cash > MAX64 || ts > MAX_SHARES || req == 0 || req > MAX64 {
        return None;
    }
    if withdraw {
        if ts == 0 || req > shares || req > ts {
            return None;
        }
        let claim = req * equity / ts;
        if claim == 0 || claim > cash || free as u128 + claim > MAX64 {
            return None;
        }
        Some((
            [cash - claim, ts - req, ds, res2],
            (free as u128 + claim) as u64,
            shares - req,
        ))
    } else {
        if req > free as u128 {
            return None;
        }
        let minted = if ts == 0 {
            if cash != 0 || ds != 0 || res2 != 0 || !claims_clear {
                return None;
            }
            req * 1_000
        } else {
            if equity == 0 {
                return None;
            }
            req * ts / equity
        };
        if minted == 0 || ts + minted > MAX_SHARES {
            return None;
        }
        Some((
            [cash + req, ts + minted, ds, res2],
            free - req as u64,
            shares + minted,
        ))
    }
}

impl M {
    fn new(variant: u8) -> Self {
        let (plain, holders) = world(variant);
        M {
            pool: [
                plain.cash,
                plain.total_supply_shares,
                plain.total_debt_shares,
                plain.reserves,
            ],
            accounted: WAD,
            idx: WAD,
            slot: START_SLOT,
            free: [holders[0].0, holders[1].0],
            shares: [holders[0].1, holders[1].1],
            pool_gen: 9,
            pos_gen: [3, 3],
            lock: None,
            claims: 0,
            paused: false,
            band_slot: START_SLOT - 10,
            intents: Vec::new(),
        }
    }

    fn release(&mut self, i: usize) {
        if self.lock == Some(i) {
            self.lock = None;
        }
    }

    fn apply(&mut self, action: Action, observed_index: u128) -> Out {
        match action {
            Action::Queue(who, withdraw, amt) => {
                let held = if withdraw {
                    self.shares[who]
                } else {
                    self.free[who] as u128
                };
                let req = amount_of(amt, held);
                let band_ok = self.band_slot != 0
                    && self.band_slot <= self.slot
                    && self.slot - self.band_slot <= MAX_BAND_AGE;
                if req == 0 || self.lock.is_some() || (!withdraw && (self.paused || !band_ok)) {
                    return Out::Rejected;
                }
                assert!(observed_index >= self.idx, "the index never decreases");
                self.idx = observed_index;
                self.lock = Some(self.intents.len());
                self.intents.push(MI {
                    who,
                    withdraw,
                    req,
                    claims_clear: self.claims == 0,
                    accounted: self.accounted,
                    idx: self.idx,
                    decision_expiry: self.slot + DECISION,
                    lock_expiry: self.slot + DECISION + LOCK_GRACE,
                    status: St::Queued,
                    pool: self.pool,
                    free: self.free[who],
                    shares: self.shares[who],
                });
                Out::Queued
            }
            Action::CbHonest(t) => self.callback(t, false, false),
            Action::CbUnauth(t) => self.callback(t, true, false),
            Action::CbWrongBinding(t) => self.callback(t, false, true),
            Action::Abandon(t) => {
                let Some(i) = pick(self.intents.len(), t) else {
                    return Out::NoTarget;
                };
                let it = &self.intents[i];
                if it.status != St::Queued || self.slot <= it.lock_expiry {
                    return Out::Rejected;
                }
                let who = it.who;
                self.intents[i].status = St::Abandoned;
                self.release(i);
                self.pool_gen += 1;
                self.pos_gen[who] += 1;
                Out::Abandoned
            }
            Action::Advance(n) => {
                self.slot += n;
                Out::Done
            }
            Action::BandRefresh => {
                self.band_slot = self.slot;
                Out::Done
            }
            Action::PauseSupply => {
                self.paused = !self.paused;
                Out::Done
            }
            Action::ClaimToggle => {
                self.claims = 1 - self.claims;
                Out::Done
            }
            Action::Shield(who) => {
                let locked = self.lock.is_some_and(|i| self.intents[i].who == who);
                if locked {
                    return Out::Rejected;
                }
                self.free[who] += 1_000;
                self.pos_gen[who] += 1;
                Out::Done
            }
        }
    }

    fn callback(&mut self, t: Target, unauth: bool, wrong_binding: bool) -> Out {
        let Some(i) = pick(self.intents.len(), t) else {
            return Out::NoTarget;
        };
        if unauth {
            return Out::Reverted;
        }
        let it = self.intents[i].clone();
        if it.status != St::Queued {
            return Out::Ignored;
        }
        if wrong_binding || (self.claims == 0) != it.claims_clear {
            return Out::Reverted;
        }
        if self.slot > it.decision_expiry {
            self.intents[i].status = St::Expired;
            self.release(i);
            return Out::Expired;
        }
        let flow = m_flow(
            it.pool,
            it.accounted,
            it.idx,
            it.free,
            it.shares,
            it.withdraw,
            it.req,
            it.claims_clear,
        );
        self.release(i);
        match flow {
            Some((pool, free, shares)) => {
                self.pool = pool;
                self.free[it.who] = free;
                self.shares[it.who] = shares;
                self.accounted = it.idx;
                self.pool_gen += 1;
                self.pos_gen[it.who] += 1;
                self.intents[i].status = St::Applied;
                Out::Applied(it.withdraw)
            }
            None => {
                self.intents[i].status = St::Zero;
                Out::Zero
            }
        }
    }
}

// ---- Views and checks -------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
struct View {
    pool: [u128; 4],
    free: [u64; 2],
    shares: [u128; 2],
    pool_gen: u64,
    pos_gen: [u64; 2],
    holder: Option<usize>,
    accounted: u128,
    idx: u128,
    claims: u64,
    statuses: Vec<St>,
}

fn status_of(s: IntentStatus) -> St {
    match s {
        IntentStatus::Queued => St::Queued,
        IntentStatus::Applied => St::Applied,
        IntentStatus::Zero => St::Zero,
        IntentStatus::Expired => St::Expired,
        IntentStatus::Abandoned => St::Abandoned,
        other => panic!("unexpected status {other:?}"),
    }
}

fn real_view(r: &Real) -> View {
    let p = open_pool(&r.pool.encrypted);
    let (fa, sa) = r.holdings(0);
    let (fb, sb) = r.holdings(1);
    let holder = r.intents.iter().position(|i| {
        i.intent.status == IntentStatus::Queued
            && i.intent.lock_owner().holds(&r.pool.coordination.lock)
    });
    View {
        pool: [
            p.cash,
            p.total_supply_shares,
            p.total_debt_shares,
            p.reserves,
        ],
        free: [fa, fb],
        shares: [sa, sb],
        pool_gen: r.pool.coordination.generation,
        pos_gen: [
            r.pos[0].coordination.generation,
            r.pos[1].coordination.generation,
        ],
        holder,
        accounted: r.pool.accounted_borrow_index,
        idx: r.asset.accounting_borrow_index,
        claims: r.market.unsettled_recovery_claim_count,
        statuses: r
            .intents
            .iter()
            .map(|i| status_of(i.intent.status))
            .collect(),
    }
}

fn model_view(m: &M) -> View {
    View {
        pool: m.pool,
        free: m.free,
        shares: m.shares,
        pool_gen: m.pool_gen,
        pos_gen: m.pos_gen,
        holder: m.lock,
        accounted: m.accounted,
        idx: m.idx,
        claims: m.claims,
        statuses: m.intents.iter().map(|i| i.status).collect(),
    }
}

fn plain(v: [u128; 4]) -> PoolPlain {
    PoolPlain {
        cash: v[0],
        total_supply_shares: v[1],
        total_debt_shares: v[2],
        reserves: v[3],
    }
}

fn equity_at(p: &PoolPlain, accounted: u128, index: u128) -> u128 {
    checkpoint(p, accounted, index, 1_000)
        .unwrap()
        .supplier_equity
}

fn check(
    before: &Real,
    after: &Real,
    action: Action,
    out: Out,
    seen: &mut Vec<(u8, u128)>,
) -> Result<(), String> {
    let (bv, av) = (real_view(before), real_view(after));
    // LEDGER
    let sum_free: u128 = av.free.iter().map(|f| *f as u128).sum();
    if after.vault != av.pool[0] + sum_free {
        return Err(format!(
            "LEDGER: vault {} != cash {} + free {}",
            after.vault, av.pool[0], sum_free
        ));
    }
    // LOCK
    for ri in &after.intents {
        let owner = ri.intent.lock_owner();
        let held = owner.holds(&after.pool.coordination.lock)
            || owner.holds(&after.pos[ri.who].coordination.lock);
        if ri.intent.status != IntentStatus::Queued && held {
            return Err(format!("LOCK: terminal intent still holds a Lock ({ri:?})"));
        }
        if ri.intent.status == IntentStatus::Queued
            && !(owner.holds(&after.pool.coordination.lock)
                && owner.holds(&after.pos[ri.who].coordination.lock))
        {
            return Err("LOCK: a queued intent lost one of its Locks".to_string());
        }
    }
    // NONCE
    let mut record = |kind: u8, was: u128, now: u128| -> Result<(), String> {
        if was != now {
            if seen.contains(&(kind, now)) {
                return Err(format!("NONCE: stored nonce {now} of kind {kind} repeats"));
            }
            seen.push((kind, now));
        }
        Ok(())
    };
    record(0, before.pool.encrypted.nonce, after.pool.encrypted.nonce)?;
    for who in 0..2 {
        record(
            1 + who as u8,
            before.pos[who].encrypted_state.nonce,
            after.pos[who].encrypted_state.nonce,
        )?;
    }
    let changed_pool = before.pool.encrypted != after.pool.encrypted;
    let changed_pos = |w: usize| before.pos[w].encrypted_state != after.pos[w].encrypted_state;
    if let Out::Applied(withdraw) = out {
        let ri = &after.intents[bv.holder.expect("an applied intent held the Locks")];
        let who = ri.who;
        // ATOMIC
        if !changed_pool || !changed_pos(who) {
            return Err(format!("ATOMIC: only one payload changed ({action:?})"));
        }
        if av.pool_gen != bv.pool_gen + 1 || av.pos_gen[who] != bv.pos_gen[who] + 1 {
            return Err(format!(
                "ATOMIC: generations did not both move ({action:?})"
            ));
        }
        if av.accounted != ri.intent.accounting_borrow_index {
            return Err("ATOMIC: accounted index is not the Intent's".to_string());
        }
        let (pb, pa) = (plain(bv.pool), plain(av.pool));
        let index = ri.intent.accounting_borrow_index;
        let eq_before = equity_at(&pb, bv.accounted, index);
        let eq_after = equity_at(&pa, index, index);
        let (ts_b, ts_a) = (pb.total_supply_shares, pa.total_supply_shares);
        // NAV
        if ts_b > 0 && ts_a > 0 && eq_after * ts_b < eq_before * ts_a {
            return Err(format!(
                "NAV: equity per share fell ({eq_before}/{ts_b} -> {eq_after}/{ts_a})"
            ));
        }
        if withdraw {
            let (claim, burned) = (pb.cash - pa.cash, ts_b - ts_a);
            if claim * ts_b > burned * eq_before {
                return Err("ROUNDING: claim above the pro-rata value".to_string());
            }
        } else {
            let (paid, minted) = (pa.cash - pb.cash, ts_a - ts_b);
            if ts_b > 0 && minted * eq_before > paid * ts_b {
                return Err("ROUNDING: minted shares worth more than paid".to_string());
            }
            if minted * eq_after > paid * ts_a {
                return Err("NOHIST: new shares are worth more than the amount paid".to_string());
            }
        }
    } else {
        let shielded = matches!(action, Action::Shield(_)) && out == Out::Done;
        let abandoned = out == Out::Abandoned;
        if changed_pool || av.accounted != bv.accounted {
            return Err(format!("NEUTRALITY: Pool payload changed on {out:?}"));
        }
        for who in 0..2 {
            let target = matches!(action, Action::Shield(w) if w == who);
            if changed_pos(who) && !(shielded && target) {
                return Err(format!("NEUTRALITY: Position {who} changed on {out:?}"));
            }
        }
        if !abandoned && !shielded && (av.pool_gen != bv.pool_gen || av.pos_gen != bv.pos_gen) {
            return Err(format!("NEUTRALITY: a generation moved on {out:?}"));
        }
    }
    Ok(())
}

fn step(
    real: &mut Real,
    model: &mut M,
    seen: &mut Vec<(u8, u128)>,
    action: Action,
    path: &[Action],
) -> Result<Out, String> {
    let before = real.clone();
    let out_real = real.apply(action);
    let observed = real.asset.accounting_borrow_index;
    let out_model = model.apply(action, observed);
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

fn play(
    actions: &[Action],
    variant: u8,
    fault: Option<Fault>,
) -> Result<(Real, M, Vec<Out>), String> {
    let mut real = Real::new(variant);
    real.fault = fault;
    let mut model = M::new(variant);
    let mut seen = vec![
        (0, real.pool.encrypted.nonce),
        (1, real.pos[0].encrypted_state.nonce),
        (2, real.pos[1].encrypted_state.nonce),
    ];
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

fn exhaustive(alphabet: &[Action], depth: usize, variant: u8) -> usize {
    fn go(
        alphabet: &[Action],
        depth: usize,
        real: &Real,
        model: &M,
        seen: &[(u8, u128)],
        path: &mut Vec<Action>,
        count: &mut usize,
    ) {
        if depth == 0 {
            return;
        }
        for &action in alphabet {
            let (mut r, mut m) = (real.clone(), model.clone());
            let mut s = seen.to_vec();
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
    let seen = vec![
        (0, real.pool.encrypted.nonce),
        (1, real.pos[0].encrypted_state.nonce),
        (2, real.pos[1].encrypted_state.nonce),
    ];
    let mut count = 0;
    go(
        alphabet,
        depth,
        &real,
        &M::new(variant),
        &seen,
        &mut Vec::new(),
        &mut count,
    );
    count
}

use Action::*;

const LIFECYCLE: [Action; 9] = [
    Queue(0, false, Amt::Quarter),
    Queue(0, true, Amt::Whole),
    Queue(1, false, Amt::Quarter),
    CbHonest(Target::Last),
    Advance(10),
    Advance(1_501),
    Advance(301),
    Abandon(Target::Last),
    Shield(0),
];

const ADVERSARIAL: [Action; 9] = [
    Queue(0, false, Amt::Whole),
    Queue(1, true, Amt::WholePlusOne),
    CbUnauth(Target::Last),
    CbWrongBinding(Target::Last),
    CbHonest(Target::First),
    CbHonest(Target::Last),
    ClaimToggle,
    Advance(1_501),
    Abandon(Target::Last),
];

const GOVERNANCE: [Action; 9] = [
    Queue(0, false, Amt::Quarter),
    Queue(1, true, Amt::Quarter),
    CbHonest(Target::Last),
    PauseSupply,
    BandRefresh,
    Advance(9_001),
    Advance(1_000),
    Shield(1),
    Abandon(Target::Prev),
];

fn full_alphabet() -> Vec<Action> {
    let mut all = Vec::new();
    for who in 0..2 {
        for withdraw in [false, true] {
            for amt in [
                Amt::Zero,
                Amt::One,
                Amt::Quarter,
                Amt::Whole,
                Amt::WholePlusOne,
            ] {
                all.push(Queue(who, withdraw, amt));
            }
        }
        all.push(Shield(who));
    }
    for t in [Target::Last, Target::First, Target::Prev] {
        all.extend([CbHonest(t), CbUnauth(t), CbWrongBinding(t), Abandon(t)]);
    }
    all.extend([
        Advance(10),
        Advance(301),
        Advance(1_000),
        Advance(1_501),
        Advance(9_001),
        BandRefresh,
        PauseSupply,
        ClaimToggle,
    ]);
    all
}

#[test]
fn prop_supply_exhaustive_lifecycle_sequences_up_to_length_4() {
    let mut total = 0;
    for variant in 0..5 {
        total += exhaustive(&LIFECYCLE, 4, variant);
    }
    assert_eq!(total, 5 * (9 + 81 + 729 + 6_561));
}

#[test]
fn prop_supply_exhaustive_adversarial_sequences_up_to_length_4() {
    let mut total = 0;
    for variant in 0..5 {
        total += exhaustive(&ADVERSARIAL, 4, variant);
    }
    assert_eq!(total, 5 * (9 + 81 + 729 + 6_561));
}

#[test]
fn prop_supply_exhaustive_governance_sequences_up_to_length_4() {
    let mut total = 0;
    for variant in 0..5 {
        total += exhaustive(&GOVERNANCE, 4, variant);
    }
    assert_eq!(total, 5 * (9 + 81 + 729 + 6_561));
}

#[test]
fn prop_supply_random_sequences_full_alphabet() {
    let alphabet = full_alphabet();
    let core = [
        Queue(0, false, Amt::Quarter),
        Queue(1, true, Amt::Quarter),
        Queue(0, true, Amt::Quarter),
        CbHonest(Target::Last),
        Advance(1_000),
        BandRefresh,
    ];
    let mut rng = Lcg(0x8800_0008);
    let (mut actions_run, mut supplies, mut withdrawals) = (0usize, 0usize, 0usize);
    for _ in 0..3_000 {
        let variant = (rng.next() % 5) as u8;
        let len = 1 + (rng.next() % 30) as usize;
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
                supplies += outs.iter().filter(|o| **o == Out::Applied(false)).count();
                withdrawals += outs.iter().filter(|o| **o == Out::Applied(true)).count();
            }
            Err(e) => panic!("{e}\nvariant {variant}, sequence {sequence:?}"),
        }
    }
    assert!(actions_run > 40_000, "{actions_run}");
    assert!(supplies > 300, "only {supplies} applied supplies");
    assert!(withdrawals > 150, "only {withdrawals} applied withdrawals");
}

// ---- Scripted scenarios -----------------------------------------------------

#[test]
fn first_supplier_then_last_withdrawal_round_trips_exactly() {
    let (real, model, outs) = play_ok(
        &[
            Queue(0, false, Amt::Whole),
            CbHonest(Target::Last),
            Queue(0, true, Amt::Whole),
            CbHonest(Target::Last),
        ],
        0,
    );
    assert_eq!(
        outs,
        [
            Out::Queued,
            Out::Applied(false),
            Out::Queued,
            Out::Applied(true)
        ]
    );
    let v = real_view(&real);
    assert_eq!(v, model_view(&model));
    assert_eq!(v.pool, [0, 0, 0, 0], "the Pool is canonically empty again");
    assert_eq!(v.free, [10_000, 10_000]);
}

#[test]
fn a_dirty_empty_pool_refuses_the_first_supplier_until_the_claim_count_and_residue_clear() {
    let (_, _, outs) = play_ok(&[Queue(0, false, Amt::Whole), CbHonest(Target::Last)], 2);
    assert_eq!(outs, [Out::Queued, Out::Zero]);
    // An unsettled claim blocks a canonically empty Pool: the flag is bound at
    // queue time, so toggling it before the callback reverts the callback.
    let (_, _, outs) = play_ok(
        &[
            ClaimToggle,
            Queue(0, false, Amt::Whole),
            CbHonest(Target::Last),
            ClaimToggle,
            Abandon(Target::Last),
            Advance(1_801),
            Abandon(Target::Last),
        ],
        0,
    );
    assert_eq!(outs[2], Out::Zero);
    assert_eq!(
        outs[4],
        Out::Rejected,
        "a terminal intent cannot be abandoned"
    );
}

#[test]
fn contention_expiry_and_late_callbacks() {
    let (real, model, outs) = play_ok(
        &[
            Queue(0, false, Amt::Quarter),
            Queue(1, false, Amt::Quarter), // the Pool is Locked
            Advance(1_501),
            CbHonest(Target::Last), // expired
            Queue(1, false, Amt::Quarter),
            Advance(1_801),
            Abandon(Target::Last),
            CbHonest(Target::Last), // late: ignored
        ],
        1,
    );
    assert_eq!(
        outs,
        [
            Out::Queued,
            Out::Rejected,
            Out::Done,
            Out::Expired,
            Out::Queued,
            Out::Done,
            Out::Abandoned,
            Out::Ignored
        ]
    );
    assert_eq!(real_view(&real), model_view(&model));
}

#[test]
fn a_stale_band_blocks_supply_but_not_withdrawal() {
    let (_, _, outs) = play_ok(
        &[
            Advance(9_001),
            Queue(0, false, Amt::Quarter),
            Queue(0, true, Amt::Quarter),
            CbHonest(Target::Last),
            BandRefresh,
            Queue(1, false, Amt::Quarter),
        ],
        1,
    );
    assert_eq!(
        outs,
        [
            Out::Done,
            Out::Rejected,
            Out::Queued,
            Out::Applied(true),
            Out::Done,
            Out::Queued
        ]
    );
}

#[test]
fn insufficient_liquidity_withdrawal_moves_zero() {
    // Supplier equity is 10,000 but Cash is 1,000: a full exit needs 10,000.
    let (real, _, outs) = play_ok(&[Queue(0, true, Amt::Whole), CbHonest(Target::Last)], 3);
    assert_eq!(outs, [Out::Queued, Out::Zero]);
    assert_eq!(real_view(&real).pool[0], 1_000);
    // A quarter of the shares is worth 2,500 and is still above the Cash.
    let (real, _, outs) = play_ok(&[Queue(0, true, Amt::Quarter), CbHonest(Target::Last)], 3);
    assert_eq!(outs[1], Out::Zero);
    assert_eq!(real_view(&real).free[0], 1_000);
    // A claim that rounds to zero is refused too.
    let (_, _, outs) = play_ok(&[Queue(0, true, Amt::One), CbHonest(Target::Last)], 3);
    assert_eq!(outs[1], Out::Zero);
}

#[test]
fn interest_before_a_supply_is_not_captured() {
    let (real, model, outs) = play_ok(
        &[
            Advance(1_000),
            Queue(1, false, Amt::Whole),
            CbHonest(Target::Last),
        ],
        1,
    );
    assert_eq!(outs[2], Out::Applied(false));
    let v = real_view(&real);
    assert_eq!(v, model_view(&model));
    // The 5,000 units left Free USDC and bought a share fraction that is worth
    // at most 5,000 at the applying index (the NOHIST check ran on this step).
    assert_eq!(v.free[1], 0);
    assert!(v.shares[1] > 400_000_000_000_000);
    assert!(real.asset.accounting_borrow_index > WAD, "interest accrued");
}
#[test]
fn dust_supply_mints_zero_shares_and_moves_nothing() {
    // Each share is worth 10,000 units: one unit mints floor(0.0001) = 0.
    let (real, model, outs) = play_ok(&[Queue(1, false, Amt::One), CbHonest(Target::Last)], 4);
    assert_eq!(outs, [Out::Queued, Out::Zero]);
    let v = real_view(&real);
    assert_eq!(v, model_view(&model));
    assert_eq!((v.free[1], v.shares[1]), (10_000, 0));
    // The whole 10,000 units mint exactly one share.
    let (real, _, outs) = play_ok(&[Queue(1, false, Amt::Whole), CbHonest(Target::Last)], 4);
    assert_eq!(outs[1], Out::Applied(false));
    assert_eq!(real_view(&real).shares[1], 1);
}

// ---- Negative controls ------------------------------------------------------

#[test]
fn supply_injected_faults_are_caught_by_the_named_invariant() {
    use Fault::*;
    let supply = [Queue(0, false, Amt::Quarter), CbHonest(Target::Last)];
    let accrued_supply = [
        Advance(1_501),
        Queue(0, false, Amt::Whole),
        CbHonest(Target::Last),
    ];
    let cases: [(Fault, &[Action], u8, &str); 8] = [
        (PositionNotStored, &supply, 1, "LEDGER"),
        (
            MintWithoutCheckpoint,
            &accrued_supply,
            1,
            "AGREEMENT|NOHIST|ROUNDING",
        ),
        (
            SkipCashCoverage,
            &[Queue(0, true, Amt::Whole), CbHonest(Target::Last)],
            3,
            "LEDGER",
        ),
        (
            ZeroWritesState,
            &[Queue(0, true, Amt::WholePlusOne), CbHonest(Target::Last)],
            1,
            "NEUTRALITY",
        ),
        (
            AcceptUnauthenticated,
            &[Queue(0, false, Amt::Quarter), CbUnauth(Target::Last)],
            1,
            "LEDGER",
        ),
        (MintRoundsUp, &supply, 1, "NAV"),
        (
            IgnoreClaimsFlag,
            &[
                ClaimToggle,
                Queue(0, false, Amt::Whole),
                CbHonest(Target::Last),
            ],
            0,
            "AGREEMENT",
        ),
        (PositionGenerationSkipped, &supply, 1, "ATOMIC"),
    ];
    for (fault, actions, variant, expected) in cases {
        assert!(
            play(actions, variant, None).is_ok(),
            "{fault:?}: the sequence must pass without the fault"
        );
        match play(actions, variant, Some(fault)) {
            Ok(_) => panic!("{fault:?} was not caught"),
            Err(e) => assert!(
                expected.split('|').any(|name| e.starts_with(name)),
                "{fault:?}: expected {expected}, got {e}"
            ),
        }
    }
}
