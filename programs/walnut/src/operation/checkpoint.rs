//! Task 3.5 framework checkpoint: an end-to-end model-based test of the
//! operation framework, and a proof that failed operations move no value.
//!
//! The framework (Tasks 3.1 to 3.3) has no financial state of its own, so this
//! module adds a small *model world* that gives "financial value" a meaning the
//! tests can check:
//!
//! * [`ValueLedger`] is a plain-integer ledger (one balance per protected
//!   account plus a public vault total). It is used only as a test oracle.
//! * [`ValueLedger::commit_value_effect`] is the only code that changes it. The
//!   driver calls it exactly when the production framework returns
//!   `CallbackOutcome::Applied`, the point where a production callback would
//!   write.
//!
//! # What is driven
//!
//! The [`System`] runs the real production code over [`Coordination`] fields:
//! queue goes through `seeds::reserve_intent` (which runs `validate_new`) and
//! `coordination::acquire_locks`; callbacks go through
//! `callback::classify_callback` and `callback::apply_decision_to_slots`;
//! timeout recovery goes through `callback::abandon_timeout`. The commitments
//! are the production SHA-256 commitments, recomputed by every callback.
//!
//! The [`Model`] is a deliberately naive reference state machine written from
//! the requirements, not from the production code. It uses its own status enum
//! and plain vectors, and it never calls production functions.
//!
//! After every action the [`Harness`] checks, in this order, that the real
//! system and the model produced the same [`Outcome`], that the named
//! invariants hold (see `Invariant` checks below), and that the two agree on
//! every Intent status, every account generation and Lock owner, the sequence
//! counter, the clock, the oracle generation and the whole ledger.
//!
//! # Action alphabet
//!
//! * `Queue(spec)`: reserve a fresh sequence and acquire every Lock in the
//!   footprint. Rejected for bad timing, a bad position snapshot, or a held
//!   Lock (a concurrent queue on a locked account).
//! * `QueueReplaySequence(spec)`: queue with the newest Intent's sequence again.
//! * `Retry(target)`: queue a NEW Intent for a terminal, non-Applied Intent.
//! * `Deliver { target, kind }`: a callback. Kinds: success nonzero, success
//!   zero, stale dependency, after Decision Expiry, authenticated static
//!   failure, unauthenticated output, wrong offset, wrong commitment, wrong
//!   Intent, duplicate, late.
//! * `Abandon { target, when }`: timeout recovery now (usually too early) or
//!   after advancing the clock past Lock expiry.
//! * `Advance(slots)`: move the clock.
//! * `BumpGeneration(account)`: an external write to a protected generation.
//!
//! # Invariants
//!
//! Each is one function that returns a message beginning with its name.
//!
//! * `FINANCIAL NEUTRALITY`: the ledger changes only on a transition to Applied,
//!   by exactly the modelled amount, and total value is conserved.
//! * `LOCK SAFETY`: every active Lock belongs to one Queued Intent whose
//!   footprint contains the account, and a Queued Intent holds its whole
//!   footprint.
//! * `GENERATION SAFETY`: generations never decrease and move by exactly one,
//!   only on Applied, Abandoned, or an external bump.
//! * `TERMINAL IMMUTABILITY`: an Intent leaves Queued exactly once, only the
//!   targeted Intent changes, and Applied happens at most once.
//! * `SEQUENCE COUNTER`: strictly increasing, one per queued Intent, never
//!   reused, with a fresh offset and key per Intent.
//! * `LATE CALLBACK`: a terminal Intent can never pass `callback_may_write`.
//! * `RETRY SAFETY`: a retry is a new Intent with a fresh sequence, offset and
//!   key, and the original it retries stays terminal and never becomes Applied.
//! * `OPERATION COUNTS`: released Locks and bumped generations match the outcome.
//! * `LIVENESS`: from any reachable state, abandoning every expired Intent
//!   unlocks every account and a new queue succeeds.

use std::collections::BTreeMap;

use anchor_lang::prelude::Pubkey;

use crate::lock::Lock;
use crate::operation::callback::{
    abandon_timeout, apply_decision_to_slots, classify_callback, CallbackContext, CallbackError,
    CallbackReport, CurrentDependencies,
};
use crate::operation::commitment::{
    account_commitment, input_commitment, AccountBinding, AccountRole, InputField, InputValue,
};
use crate::operation::intent::{IntentStatus, OperationIntent, OperationKind};
use crate::operation::lifecycle::CallbackOutcome;
use crate::operation::ownership::IntentError;
use crate::operation::seeds::{reserve_intent, OperationCounter};
use crate::state::coordination::{
    acquire_locks, callback_may_write, with_protected, Coordination, CoordinationSlot,
};
use crate::state::{CoordinationError, Slot};

// ---- Fixed world parameters --------------------------------------------------

const WAD: u128 = 1_000_000_000_000_000_000;
const TIMESTAMP: i64 = 1_700_000_000;
const INITIAL_BALANCE: u64 = 50;
const INITIAL_VAULT: u64 = 100;

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}

fn owner_key() -> Pubkey {
    key(0xA0)
}

fn payer_key() -> Pubkey {
    key(0xA1)
}

fn account_key(index: usize) -> Pubkey {
    key(1 + index as u8)
}

/// A cheap, unique Intent key per sequence. The real PDA derivation is covered
/// in `operation/seeds.rs`; deriving a program address per queue would make the
/// 20,000-sequence run slow.
fn intent_key(sequence: u64) -> Pubkey {
    let mut bytes = [0xA5u8; 32];
    bytes[..8].copy_from_slice(&sequence.to_le_bytes());
    Pubkey::new_from_array(bytes)
}

/// A fresh computation offset per sequence.
fn offset_for(sequence: u64) -> u64 {
    1_000 + 7 * sequence
}

fn has_bit(mask: u8, index: usize) -> bool {
    (mask >> index) & 1 == 1
}

/// The account indexes selected by `mask`.
fn members(mask: u8, accounts: usize) -> impl Iterator<Item = usize> {
    (0..accounts).filter(move |index| has_bit(mask, *index))
}

// ---- Layout, operations, and actions ----------------------------------------

/// How many protected accounts exist and which role each plays.
///
/// Two accounts are `[Position, Pool]`. Three are `[Position, SecondPosition,
/// Pool]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layout {
    accounts: usize,
}

impl Layout {
    const TWO: Layout = Layout { accounts: 2 };
    const THREE: Layout = Layout { accounts: 3 };

    fn role(self, account: usize) -> AccountRole {
        match (self.accounts, account) {
            (_, 0) => AccountRole::Position,
            (3, 1) => AccountRole::SecondPosition,
            _ => AccountRole::Pool,
        }
    }

    fn full_mask(self) -> u8 {
        (1u8 << self.accounts) - 1
    }

    /// The reference rule for a valid footprint: non-empty, inside the world,
    /// and a second Position needs a first.
    fn mask_is_valid(self, mask: u8) -> bool {
        let second_without_first = self.accounts == 3 && has_bit(mask, 1) && !has_bit(mask, 0);
        mask != 0 && mask & !self.full_mask() == 0 && !second_without_first
    }
}

/// The Decision Expiry and Lock expiry windows of an operation, in slots after
/// the queue slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Timing {
    /// Decision after 2, Lock after 4.
    Normal,
    /// Decision after 2, Lock after 2 (the two expiries coincide).
    Tight,
    /// Decision after 5, Lock after 9.
    Long,
    /// Decision after 0: rejected, the Intent could never apply.
    Invalid,
}

impl Timing {
    fn windows(self) -> (Slot, Slot) {
        match self {
            Timing::Normal => (2, 4),
            Timing::Tight => (2, 2),
            Timing::Long => (5, 9),
            Timing::Invalid => (0, 3),
        }
    }
}

/// Which way an Applied operation moves value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    /// Vault to the first footprint account.
    Credit,
    /// The first footprint account to the vault.
    Debit,
}

/// One model operation: its footprint, the amount it moves when Applied, and
/// its timing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OpSpec {
    mask: u8,
    amount: u64,
    direction: Direction,
    timing: Timing,
}

impl OpSpec {
    fn first_account(&self) -> usize {
        self.mask.trailing_zeros() as usize
    }
}

/// Which Intent an action addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Newest,
    Oldest,
    /// `index % number_of_intents`.
    At(u8),
}

impl Target {
    fn resolve(self, len: usize) -> Option<usize> {
        match self {
            Target::Newest => len.checked_sub(1),
            Target::Oldest => (len > 0).then_some(0),
            Target::At(index) => (len > 0).then(|| usize::from(index) % len),
        }
    }
}

/// What a callback delivery looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Delivery {
    /// Authenticated success approving a nonzero transition.
    Nonzero,
    /// Authenticated success approving zero.
    Zero,
    /// A dependency (the oracle generation) moved before the callback arrives.
    Stale,
    /// The clock first moves past this Intent's Decision Expiry.
    AfterDecisionExpiry,
    /// Authenticated static failure.
    StaticFailure,
    /// The signed output did not verify.
    Unauthenticated,
    /// The computation offset differs from the Intent's.
    BadOffset,
    /// The inputs differ from the committed inputs.
    BadCommitment,
    /// The callback was built for another Intent.
    WrongIntent,
    /// Replay of the callback that already resolved the Intent.
    Duplicate,
    /// A success callback for an Intent that is already terminal.
    Late,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AbandonWhen {
    /// At the current slot (usually too early).
    Now,
    /// After moving the clock one slot past Lock expiry.
    AfterLockExpiry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Queue(OpSpec),
    QueueReplaySequence(OpSpec),
    Retry(Target),
    Deliver { target: Target, kind: Delivery },
    Abandon { target: Target, when: AbandonWhen },
    Advance(Slot),
    BumpGeneration(u8),
}

fn deliver_to(target: Target, kind: Delivery) -> Action {
    Action::Deliver { target, kind }
}

// ---- Outcomes ----------------------------------------------------------------

/// The reference status of an Intent. A separate enum from the production
/// `IntentStatus` so the model shares no types with the code under test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum St {
    Queued,
    Applied,
    Zero,
    Failed,
    Stale,
    Expired,
    Abandoned,
}

impl From<IntentStatus> for St {
    fn from(status: IntentStatus) -> Self {
        match status {
            IntentStatus::Queued => St::Queued,
            IntentStatus::Applied => St::Applied,
            IntentStatus::Zero => St::Zero,
            IntentStatus::Failed => St::Failed,
            IntentStatus::Stale => St::Stale,
            IntentStatus::Expired => St::Expired,
            IntentStatus::Abandoned => St::Abandoned,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reject {
    Timing,
    Snapshot,
    SequenceReused,
    LockHeld,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CbReject {
    IntentKey,
    Offset,
    Inputs,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AbandonReject {
    NotQueued,
    NotExpired,
    Other,
}

/// What an action did. Both the real system and the model produce one, and they
/// must be equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    /// There was nothing for the action to act on.
    Skipped,
    Clock,
    Bumped(usize),
    Queued(usize),
    QueueRejected(Reject),
    /// The Intent left Queued through a callback.
    Resolved(usize, St),
    /// A callback for a terminal Intent: no change.
    Ignored,
    /// An unauthenticated callback for a Queued Intent: no change.
    LeftQueued,
    CallbackRejected(CbReject),
    Abandoned(usize),
    AbandonRejected(AbandonReject),
    /// The production code did something the model has no name for.
    Unexpected,
}

// ---- The value ledger --------------------------------------------------------

/// The value oracle: balances per protected account and a public vault.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ValueLedger {
    balances: Vec<u64>,
    vault: u64,
}

impl ValueLedger {
    fn new(accounts: usize) -> Self {
        Self {
            balances: vec![INITIAL_BALANCE; accounts],
            vault: INITIAL_VAULT,
        }
    }

    /// Balances in account order, then the vault.
    fn entries(&self) -> Vec<u64> {
        let mut entries = self.balances.clone();
        entries.push(self.vault);
        entries
    }

    /// The only mutation of the ledger. Called exactly when the framework
    /// returns Applied. Moves the amount, limited by what the source holds, and
    /// returns what moved.
    fn commit_value_effect(&mut self, spec: &OpSpec) -> u64 {
        let account = spec.first_account();
        match spec.direction {
            Direction::Credit => {
                let moved = spec.amount.min(self.vault);
                self.vault -= moved;
                self.balances[account] += moved;
                moved
            }
            Direction::Debit => {
                let moved = spec.amount.min(self.balances[account]);
                self.balances[account] -= moved;
                self.vault += moved;
                moved
            }
        }
    }
}

/// The ledger entries after one Applied operation, computed from the
/// specification alone (not through [`ValueLedger`]).
fn transfer_effect(before: &[u64], spec: &OpSpec, accounts: usize) -> Vec<u64> {
    let vault = accounts;
    let account = spec.first_account();
    let (source, destination) = match spec.direction {
        Direction::Credit => (vault, account),
        Direction::Debit => (account, vault),
    };
    let moved = spec.amount.min(before[source]);
    let mut after = before.to_vec();
    after[source] -= moved;
    after[destination] += moved;
    after
}

// ---- Faults (negative controls) ---------------------------------------------

/// A deliberately wrong driver. Each variant commits value on an outcome that
/// must move none, to show the neutrality invariant has teeth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    None,
    OnZero,
    OnStale,
    OnExpired,
    OnFailed,
    OnAbandon,
    OnIgnored,
    OnLeftQueued,
    OnRejected,
}

// ---- The real system ---------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct Record {
    intent: OperationIntent,
    spec: OpSpec,
    retry_of: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    generations: Vec<u64>,
    locks: Vec<Lock>,
    statuses: Vec<St>,
    next_sequence: u64,
    entries: Vec<u64>,
    slot: Slot,
}

/// The production framework driven over a model world.
#[derive(Clone)]
struct System {
    layout: Layout,
    accounts: Vec<Coordination>,
    counter: OperationCounter,
    records: Vec<Record>,
    slot: Slot,
    oracle_generation: u64,
    ledger: ValueLedger,
    initial_total: u64,
    applied_commits: Vec<u32>,
    fault: Fault,
    /// Locks released and generations bumped by the last callback or abandon.
    last_counts: (usize, usize),
}

fn slots_for(accounts: &mut [Coordination], mask: u8) -> Vec<CoordinationSlot<'_>> {
    accounts
        .iter_mut()
        .enumerate()
        .filter(|(index, _)| has_bit(mask, *index))
        .map(|(index, state)| CoordinationSlot::new(account_key(index), state))
        .collect()
}

fn captured_generation(intent: &OperationIntent, role: AccountRole) -> Option<u64> {
    match role {
        AccountRole::Position => intent.position_generation,
        AccountRole::SecondPosition => intent.second_position_generation,
        _ => intent.pool_generation,
    }
}

/// The ordered accounts a callback hashes, with the generations the Intent
/// captured.
fn bindings(layout: Layout, intent: &OperationIntent, spec: &OpSpec) -> Vec<AccountBinding> {
    let mut bindings = vec![AccountBinding {
        key: owner_key(),
        role: AccountRole::Owner,
        expected_owner: crate::ID,
        mint: None,
        authority: None,
        writable: false,
        generation: None,
    }];
    for index in members(spec.mask, layout.accounts) {
        let role = layout.role(index);
        bindings.push(AccountBinding {
            key: account_key(index),
            role,
            expected_owner: crate::ID,
            mint: None,
            authority: None,
            writable: true,
            generation: captured_generation(intent, role),
        });
    }
    bindings
}

fn input_fields(spec: &OpSpec, amount: u64) -> [InputField<'static>; 2] {
    [
        InputField::new(1, InputValue::U64(amount)),
        InputField::new(2, InputValue::U8(spec.direction as u8)),
    ]
}

impl System {
    fn new(layout: Layout, fault: Fault) -> Self {
        let ledger = ValueLedger::new(layout.accounts);
        Self {
            layout,
            accounts: vec![Coordination::default(); layout.accounts],
            counter: OperationCounter::new(owner_key(), 255),
            records: Vec::new(),
            slot: 0,
            oracle_generation: 1,
            initial_total: ledger.entries().iter().sum(),
            ledger,
            applied_commits: Vec::new(),
            fault,
            last_counts: (0, 0),
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            generations: self.accounts.iter().map(|c| c.generation).collect(),
            locks: self.accounts.iter().map(|c| c.lock).collect(),
            statuses: self
                .records
                .iter()
                .map(|r| St::from(r.intent.status))
                .collect(),
            next_sequence: self.counter.next_sequence,
            entries: self.ledger.entries(),
            slot: self.slot,
        }
    }

    fn apply(&mut self, action: Action) -> Outcome {
        self.last_counts = (0, 0);
        let len = self.records.len();
        match action {
            Action::Queue(spec) => self.queue(spec, None, None),
            Action::QueueReplaySequence(spec) => match self.counter.next_sequence.checked_sub(1) {
                None => Outcome::Skipped,
                Some(sequence) => self.queue(spec, Some(sequence), None),
            },
            Action::Retry(target) => match target.resolve(len) {
                None => Outcome::Skipped,
                Some(index) => {
                    let record = self.records[index];
                    let status = record.intent.status;
                    if status.is_terminal() && status != IntentStatus::Applied {
                        self.queue(record.spec, None, Some(index))
                    } else {
                        Outcome::Skipped
                    }
                }
            },
            Action::Deliver { target, kind } => match target.resolve(len) {
                None => Outcome::Skipped,
                Some(index) => self.deliver(index, kind),
            },
            Action::Abandon { target, when } => match target.resolve(len) {
                None => Outcome::Skipped,
                Some(index) => self.abandon(index, when),
            },
            Action::Advance(slots) => {
                self.slot += slots;
                Outcome::Clock
            }
            Action::BumpGeneration(account) => {
                let index = usize::from(account) % self.layout.accounts;
                self.accounts[index].generation += 1;
                Outcome::Bumped(index)
            }
        }
    }

    fn queue(&mut self, spec: OpSpec, sequence: Option<u64>, retry_of: Option<usize>) -> Outcome {
        let sequence = sequence.unwrap_or(self.counter.next_sequence);
        let (decision_window, lock_window) = spec.timing.windows();
        let mut intent = OperationIntent {
            key: intent_key(sequence),
            owner: owner_key(),
            payer: payer_key(),
            reclaim_authority: payer_key(),
            kind: OperationKind::Supply,
            sequence,
            computation_offset: offset_for(sequence),
            asset_generation: 1,
            oracle_generation: self.oracle_generation,
            rate_epoch_generation: 1,
            accounting_borrow_index: WAD,
            accounting_index_timestamp: TIMESTAMP,
            risk_borrow_index_at_expiry: WAD + 1,
            queued_slot: self.slot,
            decision_expiry_slot: self.slot + decision_window,
            lock_expiry_slot: self.slot + lock_window,
            ..OperationIntent::default()
        };
        for index in members(spec.mask, self.layout.accounts) {
            let generation = Some(self.accounts[index].generation);
            match self.layout.role(index) {
                AccountRole::Position => {
                    intent.position = Some(account_key(index));
                    intent.position_generation = generation;
                }
                AccountRole::SecondPosition => {
                    intent.second_position = Some(account_key(index));
                    intent.second_position_generation = generation;
                }
                _ => intent.pool_generation = generation,
            }
        }
        intent.accounts_commitment = account_commitment(&bindings(self.layout, &intent, &spec));
        intent.inputs_commitment = input_commitment(intent.kind, &input_fields(&spec, spec.amount));

        // The queue instruction is one transaction: the sequence is consumed
        // only if the Locks are also acquired.
        let mut counter = self.counter;
        if let Err(error) = reserve_intent(&mut counter, &intent) {
            return Outcome::QueueRejected(match error {
                IntentError::InvalidTiming => Reject::Timing,
                IntentError::SnapshotMismatch => Reject::Snapshot,
                IntentError::SequenceReused => Reject::SequenceReused,
                _ => Reject::Other,
            });
        }
        let acquired = {
            let mut slots = slots_for(&mut self.accounts, spec.mask);
            acquire_locks(intent.lock_owner(), intent.lock_expiry_slot, &mut slots)
        };
        if let Err(error) = acquired {
            return Outcome::QueueRejected(match error {
                CoordinationError::LockHeld => Reject::LockHeld,
                _ => Reject::Other,
            });
        }
        self.counter = counter;
        self.records.push(Record {
            intent,
            spec,
            retry_of,
        });
        self.applied_commits.push(0);
        Outcome::Queued(self.records.len() - 1)
    }

    fn current_dependencies(&self, spec: &OpSpec) -> CurrentDependencies {
        let mut current = CurrentDependencies {
            position_generation: None,
            second_position_generation: None,
            pool_generation: None,
            asset_generation: 1,
            oracle_generation: self.oracle_generation,
            rate_epoch_generation: 1,
            accounting_borrow_index: WAD,
            accounting_index_timestamp: TIMESTAMP,
        };
        for index in members(spec.mask, self.layout.accounts) {
            let generation = Some(self.accounts[index].generation);
            match self.layout.role(index) {
                AccountRole::Position => current.position_generation = generation,
                AccountRole::SecondPosition => current.second_position_generation = generation,
                _ => current.pool_generation = generation,
            }
        }
        current
    }

    fn deliver(&mut self, index: usize, kind: Delivery) -> Outcome {
        let record = self.records[index];
        let intent = record.intent;
        let terminal = intent.status.is_terminal();
        if matches!(kind, Delivery::Duplicate | Delivery::Late) && !terminal {
            return Outcome::Skipped;
        }
        match kind {
            Delivery::AfterDecisionExpiry => {
                self.slot = self.slot.max(intent.decision_expiry_slot + 1);
            }
            Delivery::Stale => self.oracle_generation += 1,
            _ => {}
        }
        let report = match kind {
            Delivery::Zero => CallbackReport::Success { nonzero: false },
            Delivery::StaticFailure => CallbackReport::StaticFailure,
            Delivery::Duplicate => match intent.status {
                IntentStatus::Zero => CallbackReport::Success { nonzero: false },
                IntentStatus::Failed => CallbackReport::StaticFailure,
                _ => CallbackReport::Success { nonzero: true },
            },
            _ => CallbackReport::Success { nonzero: true },
        };
        let account_bindings = bindings(self.layout, &intent, &record.spec);
        let amount = if kind == Delivery::BadCommitment {
            record.spec.amount + 1
        } else {
            record.spec.amount
        };
        let fields = input_fields(&record.spec, amount);
        let context = CallbackContext {
            output_authenticated: kind != Delivery::Unauthenticated,
            computation_offset: intent.computation_offset + u64::from(kind == Delivery::BadOffset),
            intent_key: if kind == Delivery::WrongIntent {
                key(0xEE)
            } else {
                intent.key
            },
            kind: intent.kind,
            accounts: &account_bindings,
            input_fields: &fields,
            current: self.current_dependencies(&record.spec),
            current_slot: self.slot,
            report,
        };
        let decision = match classify_callback(&intent, &context) {
            Ok(decision) => decision,
            Err(error) => {
                let reject = match error {
                    CallbackError::IntentKeyMismatch => CbReject::IntentKey,
                    CallbackError::OffsetMismatch => CbReject::Offset,
                    CallbackError::InputsCommitmentMismatch => CbReject::Inputs,
                    _ => CbReject::Other,
                };
                let outcome = Outcome::CallbackRejected(reject);
                self.inject_fault(index, outcome);
                return outcome;
            }
        };

        let mut working = intent;
        let result = {
            let mut slots = slots_for(&mut self.accounts, record.spec.mask);
            apply_decision_to_slots(&mut working, &mut slots, decision, self.slot)
        };
        let Ok(result) = result else {
            return Outcome::Unexpected;
        };
        self.records[index].intent = working;
        self.last_counts = (result.locks_released, result.generations_bumped);
        let outcome = match result.outcome {
            CallbackOutcome::Applied => {
                // The framework approved the transition: this is the moment a
                // production callback writes, so the ledger moves here and
                // nowhere else.
                self.ledger.commit_value_effect(&record.spec);
                self.applied_commits[index] += 1;
                Outcome::Resolved(index, St::Applied)
            }
            CallbackOutcome::Zero => Outcome::Resolved(index, St::Zero),
            CallbackOutcome::Failed => Outcome::Resolved(index, St::Failed),
            CallbackOutcome::Stale => Outcome::Resolved(index, St::Stale),
            CallbackOutcome::Expired => Outcome::Resolved(index, St::Expired),
            CallbackOutcome::TerminalNoop => Outcome::Ignored,
            CallbackOutcome::LeftQueuedForRecovery => Outcome::LeftQueued,
            CallbackOutcome::Abandoned => Outcome::Unexpected,
        };
        self.inject_fault(index, outcome);
        outcome
    }

    fn abandon(&mut self, index: usize, when: AbandonWhen) -> Outcome {
        let record = self.records[index];
        if when == AbandonWhen::AfterLockExpiry {
            self.slot = self.slot.max(record.intent.lock_expiry_slot + 1);
        }
        let slot = self.slot;
        let mut working = record.intent;
        let result = {
            let mut slots = slots_for(&mut self.accounts, record.spec.mask);
            with_protected(&mut slots, |accounts| {
                abandon_timeout(&mut working, accounts, slot)
            })
        };
        let outcome = match result {
            Ok(result) if result.outcome == CallbackOutcome::Abandoned => {
                self.records[index].intent = working;
                self.last_counts = (result.locks_released, result.generations_bumped);
                Outcome::Abandoned(index)
            }
            Ok(_) => Outcome::Unexpected,
            Err(CallbackError::Intent(IntentError::NotQueued)) => {
                Outcome::AbandonRejected(AbandonReject::NotQueued)
            }
            Err(CallbackError::Intent(IntentError::LockNotExpired)) => {
                Outcome::AbandonRejected(AbandonReject::NotExpired)
            }
            Err(_) => Outcome::AbandonRejected(AbandonReject::Other),
        };
        self.inject_fault(index, outcome);
        outcome
    }

    /// Applies the configured fault: commits value on an outcome that must move
    /// none.
    fn inject_fault(&mut self, index: usize, outcome: Outcome) {
        let commit = matches!(
            (self.fault, outcome),
            (Fault::OnZero, Outcome::Resolved(_, St::Zero))
                | (Fault::OnStale, Outcome::Resolved(_, St::Stale))
                | (Fault::OnExpired, Outcome::Resolved(_, St::Expired))
                | (Fault::OnFailed, Outcome::Resolved(_, St::Failed))
                | (Fault::OnAbandon, Outcome::Abandoned(_))
                | (Fault::OnIgnored, Outcome::Ignored)
                | (Fault::OnLeftQueued, Outcome::LeftQueued)
                | (Fault::OnRejected, Outcome::CallbackRejected(_))
        );
        if commit {
            self.ledger.commit_value_effect(&self.records[index].spec);
        }
    }
}

// ---- The reference model -----------------------------------------------------

#[derive(Clone, Debug)]
struct ModelIntent {
    spec: OpSpec,
    status: St,
    sequence: u64,
    offset: u64,
    retry_of: Option<usize>,
    captured: Vec<u64>,
    captured_oracle: u64,
    decision_expiry: Slot,
    lock_expiry: Slot,
}

/// The trivially-correct reference state machine.
#[derive(Clone, Debug)]
struct Model {
    layout: Layout,
    slot: Slot,
    oracle: u64,
    generations: Vec<u64>,
    /// The Intent holding each account's Lock.
    holder: Vec<Option<usize>>,
    next_sequence: u64,
    intents: Vec<ModelIntent>,
    entries: Vec<u64>,
}

impl Model {
    fn new(layout: Layout) -> Self {
        let mut entries = vec![INITIAL_BALANCE; layout.accounts];
        entries.push(INITIAL_VAULT);
        Self {
            layout,
            slot: 0,
            oracle: 1,
            generations: vec![0; layout.accounts],
            holder: vec![None; layout.accounts],
            next_sequence: 0,
            intents: Vec::new(),
            entries,
        }
    }

    fn apply(&mut self, action: Action) -> Outcome {
        let len = self.intents.len();
        match action {
            Action::Queue(spec) => self.queue(spec, None, None),
            Action::QueueReplaySequence(spec) => {
                if self.next_sequence == 0 {
                    Outcome::Skipped
                } else {
                    self.queue(spec, Some(self.next_sequence - 1), None)
                }
            }
            Action::Retry(target) => match target.resolve(len) {
                None => Outcome::Skipped,
                Some(index) => match self.intents[index].status {
                    St::Queued | St::Applied => Outcome::Skipped,
                    _ => {
                        let spec = self.intents[index].spec;
                        self.queue(spec, None, Some(index))
                    }
                },
            },
            Action::Deliver { target, kind } => match target.resolve(len) {
                None => Outcome::Skipped,
                Some(index) => self.deliver(index, kind),
            },
            Action::Abandon { target, when } => match target.resolve(len) {
                None => Outcome::Skipped,
                Some(index) => self.abandon(index, when),
            },
            Action::Advance(slots) => {
                self.slot += slots;
                Outcome::Clock
            }
            Action::BumpGeneration(account) => {
                let index = usize::from(account) % self.layout.accounts;
                self.generations[index] += 1;
                Outcome::Bumped(index)
            }
        }
    }

    fn queue(&mut self, spec: OpSpec, sequence: Option<u64>, retry_of: Option<usize>) -> Outcome {
        let (decision, lock) = spec.timing.windows();
        if decision == 0 || lock < decision {
            return Outcome::QueueRejected(Reject::Timing);
        }
        if !self.layout.mask_is_valid(spec.mask) {
            return Outcome::QueueRejected(Reject::Snapshot);
        }
        let sequence = sequence.unwrap_or(self.next_sequence);
        if sequence < self.next_sequence {
            return Outcome::QueueRejected(Reject::SequenceReused);
        }
        if members(spec.mask, self.layout.accounts).any(|i| self.holder[i].is_some()) {
            return Outcome::QueueRejected(Reject::LockHeld);
        }
        let index = self.intents.len();
        for account in members(spec.mask, self.layout.accounts) {
            self.holder[account] = Some(index);
        }
        self.next_sequence = sequence + 1;
        self.intents.push(ModelIntent {
            spec,
            status: St::Queued,
            sequence,
            offset: offset_for(sequence),
            retry_of,
            captured: self.generations.clone(),
            captured_oracle: self.oracle,
            decision_expiry: self.slot + decision,
            lock_expiry: self.slot + lock,
        });
        Outcome::Queued(index)
    }

    fn is_stale(&self, index: usize) -> bool {
        let intent = &self.intents[index];
        intent.captured_oracle != self.oracle
            || members(intent.spec.mask, self.layout.accounts)
                .any(|i| self.generations[i] != intent.captured[i])
    }

    /// Ends a Queued Intent: its Locks are released, and for Applied and
    /// Abandoned every held account's generation moves by one.
    fn finish(&mut self, index: usize, status: St) {
        let spec = self.intents[index].spec;
        for account in members(spec.mask, self.layout.accounts) {
            if self.holder[account] == Some(index) {
                self.holder[account] = None;
                if matches!(status, St::Applied | St::Abandoned) {
                    self.generations[account] += 1;
                }
            }
        }
        if status == St::Applied {
            self.entries = transfer_effect(&self.entries, &spec, self.layout.accounts);
        }
        self.intents[index].status = status;
    }

    fn deliver(&mut self, index: usize, kind: Delivery) -> Outcome {
        let terminal = self.intents[index].status != St::Queued;
        if matches!(kind, Delivery::Duplicate | Delivery::Late) && !terminal {
            return Outcome::Skipped;
        }
        match kind {
            Delivery::AfterDecisionExpiry => {
                self.slot = self.slot.max(self.intents[index].decision_expiry + 1);
            }
            Delivery::Stale => self.oracle += 1,
            _ => {}
        }
        // The order of these checks is the documented verification order.
        if kind == Delivery::Unauthenticated {
            return if terminal {
                Outcome::Ignored
            } else {
                Outcome::LeftQueued
            };
        }
        if kind == Delivery::WrongIntent {
            return Outcome::CallbackRejected(CbReject::IntentKey);
        }
        if terminal {
            return Outcome::Ignored;
        }
        if kind == Delivery::BadOffset {
            return Outcome::CallbackRejected(CbReject::Offset);
        }
        if kind == Delivery::BadCommitment {
            return Outcome::CallbackRejected(CbReject::Inputs);
        }
        let status = if kind == Delivery::StaticFailure {
            St::Failed
        } else if self.is_stale(index) {
            St::Stale
        } else if self.slot > self.intents[index].decision_expiry {
            St::Expired
        } else if kind == Delivery::Zero {
            St::Zero
        } else {
            St::Applied
        };
        self.finish(index, status);
        Outcome::Resolved(index, status)
    }

    fn abandon(&mut self, index: usize, when: AbandonWhen) -> Outcome {
        if when == AbandonWhen::AfterLockExpiry {
            self.slot = self.slot.max(self.intents[index].lock_expiry + 1);
        }
        if self.intents[index].status != St::Queued {
            return Outcome::AbandonRejected(AbandonReject::NotQueued);
        }
        if self.slot <= self.intents[index].lock_expiry {
            return Outcome::AbandonRejected(AbandonReject::NotExpired);
        }
        self.finish(index, St::Abandoned);
        Outcome::Abandoned(index)
    }
}

// ---- Invariants --------------------------------------------------------------

struct Step<'a> {
    before: &'a Snapshot,
    after: &'a Snapshot,
    action: Action,
    outcome: Outcome,
    real: &'a System,
}

fn footprint_size(real: &System, index: usize) -> usize {
    members(real.records[index].spec.mask, real.layout.accounts).count()
}

/// The ledger changes only on a transition to Applied, by exactly the modelled
/// amount; total value is conserved; Applied happens at most once per Intent.
fn financial_neutrality(step: &Step<'_>) -> Result<(), String> {
    let expected_total = step.real.initial_total;
    for entries in [&step.before.entries, &step.after.entries] {
        if entries.iter().sum::<u64>() != expected_total {
            return Err(format!(
                "FINANCIAL NEUTRALITY: total value {entries:?} is not {expected_total}"
            ));
        }
    }
    match step.outcome {
        Outcome::Resolved(index, St::Applied) => {
            let spec = step.real.records[index].spec;
            let expected = transfer_effect(&step.before.entries, &spec, step.real.layout.accounts);
            if step.after.entries != expected {
                return Err(format!(
                    "FINANCIAL NEUTRALITY: Applied moved {:?} -> {:?}, expected {expected:?}",
                    step.before.entries, step.after.entries
                ));
            }
        }
        other => {
            if step.after.entries != step.before.entries {
                return Err(format!(
                    "FINANCIAL NEUTRALITY: {other:?} moved value {:?} -> {:?}",
                    step.before.entries, step.after.entries
                ));
            }
        }
    }
    for (index, commits) in step.real.applied_commits.iter().enumerate() {
        let applied = step.after.statuses[index] == St::Applied;
        if *commits > 1 || (*commits == 1) != applied {
            return Err(format!(
                "FINANCIAL NEUTRALITY: Intent {index} committed value {commits} times \
                 with status {:?}",
                step.after.statuses[index]
            ));
        }
    }
    Ok(())
}

/// Every active Lock belongs to one Queued Intent whose footprint contains the
/// account, and every Queued Intent holds its whole footprint.
fn lock_safety(step: &Step<'_>) -> Result<(), String> {
    let real = step.real;
    for (account, state) in real.accounts.iter().enumerate() {
        if !state.lock.is_active() {
            continue;
        }
        let owners: Vec<usize> = real
            .records
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                state
                    .lock
                    .matches(&r.intent.key, r.intent.computation_offset)
            })
            .map(|(index, _)| index)
            .collect();
        let [owner] = owners[..] else {
            return Err(format!(
                "LOCK SAFETY: account {account} Lock has {} owners",
                owners.len()
            ));
        };
        let record = &real.records[owner];
        if record.intent.status != IntentStatus::Queued {
            return Err(format!(
                "LOCK SAFETY: account {account} is locked by terminal Intent {owner} ({:?})",
                record.intent.status
            ));
        }
        if !has_bit(record.spec.mask, account) {
            return Err(format!(
                "LOCK SAFETY: Intent {owner} holds account {account} outside its footprint"
            ));
        }
        if state.lock.expiry_slot != record.intent.lock_expiry_slot {
            return Err(format!(
                "LOCK SAFETY: account {account} Lock expiry differs from Intent {owner}"
            ));
        }
    }
    for (index, record) in real.records.iter().enumerate() {
        if record.intent.status != IntentStatus::Queued {
            continue;
        }
        for account in members(record.spec.mask, real.layout.accounts) {
            let lock = &real.accounts[account].lock;
            if !lock.matches(&record.intent.key, record.intent.computation_offset) {
                return Err(format!(
                    "LOCK SAFETY: queued Intent {index} does not hold account {account}"
                ));
            }
        }
    }
    Ok(())
}

/// Generations never decrease, move by exactly one, and only on Applied,
/// Abandoned, or an external bump of that account.
fn generation_safety(step: &Step<'_>) -> Result<(), String> {
    let accounts = step.real.layout.accounts;
    let mut expected: Vec<u64> = step.before.generations.clone();
    match step.outcome {
        Outcome::Resolved(index, St::Applied) | Outcome::Abandoned(index) => {
            for account in members(step.real.records[index].spec.mask, accounts) {
                expected[account] += 1;
            }
        }
        Outcome::Bumped(account) => expected[account] += 1,
        _ => {}
    }
    for (account, ((before, after), want)) in step
        .before
        .generations
        .iter()
        .zip(&step.after.generations)
        .zip(&expected)
        .enumerate()
    {
        if after < before {
            return Err(format!(
                "GENERATION SAFETY: account {account} went backwards {before} -> {after}"
            ));
        }
        if after != want {
            return Err(format!(
                "GENERATION SAFETY: account {account} is {after}, expected {want} after {:?}",
                step.outcome
            ));
        }
    }
    Ok(())
}

/// An Intent leaves Queued exactly once and only the targeted Intent changes.
fn terminal_immutability(step: &Step<'_>) -> Result<(), String> {
    let before = &step.before.statuses;
    let after = &step.after.statuses;
    let created = usize::from(matches!(step.outcome, Outcome::Queued(_)));
    if after.len() != before.len() + created {
        return Err(format!(
            "TERMINAL IMMUTABILITY: {} Intents became {} after {:?}",
            before.len(),
            after.len(),
            step.outcome
        ));
    }
    let changed: Vec<usize> = before
        .iter()
        .zip(after)
        .enumerate()
        .filter(|(_, (b, a))| b != a)
        .map(|(index, _)| index)
        .collect();
    let expected: Option<(usize, St)> = match step.outcome {
        Outcome::Resolved(index, status) => Some((index, status)),
        Outcome::Abandoned(index) => Some((index, St::Abandoned)),
        _ => None,
    };
    match (expected, changed.as_slice()) {
        (None, []) => Ok(()),
        (Some((index, status)), [changed_index])
            if *changed_index == index && before[index] == St::Queued && after[index] == status =>
        {
            Ok(())
        }
        _ => Err(format!(
            "TERMINAL IMMUTABILITY: changed Intents {changed:?} after {:?} ({before:?} -> {after:?})",
            step.outcome
        )),
    }
}

/// One sequence per queued Intent, strictly increasing, with fresh offsets and
/// keys.
fn sequence_counter(step: &Step<'_>) -> Result<(), String> {
    let queued = matches!(step.outcome, Outcome::Queued(_));
    if step.after.next_sequence != step.before.next_sequence + u64::from(queued) {
        return Err(format!(
            "SEQUENCE COUNTER: {} -> {} after {:?}",
            step.before.next_sequence, step.after.next_sequence, step.outcome
        ));
    }
    let records = &step.real.records;
    for (index, record) in records.iter().enumerate() {
        if record.intent.sequence != index as u64 {
            return Err(format!(
                "SEQUENCE COUNTER: Intent {index} has sequence {}",
                record.intent.sequence
            ));
        }
        for other in &records[..index] {
            if other.intent.computation_offset == record.intent.computation_offset
                || other.intent.key == record.intent.key
            {
                return Err(format!(
                    "SEQUENCE COUNTER: Intent {index} reuses an offset or key"
                ));
            }
        }
    }
    if step.after.next_sequence != records.len() as u64 {
        return Err("SEQUENCE COUNTER: counter differs from the number of Intents".to_string());
    }
    Ok(())
}

/// A terminal Intent can never pass `callback_may_write`; a Queued one can
/// exactly while its captured generations are still current.
fn late_callback_guard(step: &Step<'_>) -> Result<(), String> {
    let real = step.real;
    for (index, record) in real.records.iter().enumerate() {
        let pairs: Vec<(&Coordination, u64)> = members(record.spec.mask, real.layout.accounts)
            .map(|account| {
                let role = real.layout.role(account);
                let captured = captured_generation(&record.intent, role).unwrap_or(u64::MAX);
                (&real.accounts[account], captured)
            })
            .collect();
        let may_write = callback_may_write(record.intent.lock_owner(), &pairs);
        let current = pairs
            .iter()
            .all(|(state, captured)| state.generation == *captured);
        let expected = record.intent.status == IntentStatus::Queued && current;
        if may_write != expected {
            return Err(format!(
                "LATE CALLBACK: Intent {index} ({:?}) may_write={may_write}, expected {expected}",
                record.intent.status
            ));
        }
    }
    Ok(())
}

/// A retry is a new Intent with a fresh sequence, offset and key. The original
/// it retries stays terminal and never becomes Applied.
fn retry_safety(step: &Step<'_>) -> Result<(), String> {
    let records = &step.real.records;
    if let (Action::Retry(_), Outcome::Queued(new)) = (step.action, step.outcome) {
        let Some(original) = records[new].retry_of else {
            return Err("RETRY SAFETY: a queued retry has no original".to_string());
        };
        let (old, fresh) = (&records[original], &records[new]);
        if !(fresh.intent.sequence > old.intent.sequence
            && fresh.intent.computation_offset != old.intent.computation_offset
            && fresh.intent.key != old.intent.key
            && fresh.spec == old.spec
            && old.intent.status.is_terminal()
            && old.intent.status != IntentStatus::Applied)
        {
            return Err(format!(
                "RETRY SAFETY: retry {new} of {original} is not fresh or the original was {:?}",
                old.intent.status
            ));
        }
    }
    // The original of any retry stays terminal and never becomes Applied, so a
    // retry cannot be the second application of the same operation. Two retries
    // of one original are two independent new operations, each of which may
    // apply once (`financial_neutrality` checks that).
    for (index, record) in records.iter().enumerate() {
        if let Some(original) = record.retry_of {
            let status = records[original].intent.status;
            if !status.is_terminal() || status == IntentStatus::Applied || original >= index {
                return Err(format!(
                    "RETRY SAFETY: Intent {original} is {status:?} but Intent {index} retries it"
                ));
            }
        }
    }
    Ok(())
}

/// Released Locks and bumped generations match the outcome.
fn operation_counts(step: &Step<'_>) -> Result<(), String> {
    let expected = match step.outcome {
        Outcome::Resolved(index, St::Applied) | Outcome::Abandoned(index) => {
            let size = footprint_size(step.real, index);
            (size, size)
        }
        Outcome::Resolved(index, _) => (footprint_size(step.real, index), 0),
        _ => (0, 0),
    };
    if step.real.last_counts != expected {
        return Err(format!(
            "OPERATION COUNTS: released/bumped {:?}, expected {expected:?} after {:?}",
            step.real.last_counts, step.outcome
        ));
    }
    Ok(())
}

fn check_invariants(step: &Step<'_>) -> Result<(), String> {
    financial_neutrality(step)?;
    lock_safety(step)?;
    generation_safety(step)?;
    terminal_immutability(step)?;
    sequence_counter(step)?;
    late_callback_guard(step)?;
    retry_safety(step)?;
    operation_counts(step)
}

/// The real system and the model agree on every observable.
fn agree(real: &System, model: &Model) -> Result<(), String> {
    if real.slot != model.slot || real.oracle_generation != model.oracle {
        return Err(format!(
            "MODEL: clock/oracle {}/{} vs {}/{}",
            real.slot, real.oracle_generation, model.slot, model.oracle
        ));
    }
    if real.counter.next_sequence != model.next_sequence {
        return Err(format!(
            "MODEL: sequence counter {} vs {}",
            real.counter.next_sequence, model.next_sequence
        ));
    }
    if real.ledger.entries() != model.entries {
        return Err(format!(
            "MODEL: ledger {:?} vs {:?}",
            real.ledger.entries(),
            model.entries
        ));
    }
    if real.records.len() != model.intents.len() {
        return Err("MODEL: Intent counts differ".to_string());
    }
    for (index, (record, expected)) in real.records.iter().zip(&model.intents).enumerate() {
        let same = St::from(record.intent.status) == expected.status
            && record.intent.sequence == expected.sequence
            && record.intent.computation_offset == expected.offset
            && record.intent.lock_expiry_slot == expected.lock_expiry
            && record.intent.decision_expiry_slot == expected.decision_expiry
            && record.retry_of == expected.retry_of
            && record.spec == expected.spec;
        if !same {
            return Err(format!(
                "MODEL: Intent {index} is {record:?}, model has {expected:?}"
            ));
        }
    }
    for (account, state) in real.accounts.iter().enumerate() {
        if state.generation != model.generations[account] {
            return Err(format!(
                "MODEL: account {account} generation {} vs {}",
                state.generation, model.generations[account]
            ));
        }
        let holder = if state.lock.is_active() {
            real.records.iter().position(|r| {
                state
                    .lock
                    .matches(&r.intent.key, r.intent.computation_offset)
            })
        } else {
            None
        };
        if holder != model.holder[account] || (state.lock.is_active() && holder.is_none()) {
            return Err(format!(
                "MODEL: account {account} Lock owner {holder:?} vs {:?}",
                model.holder[account]
            ));
        }
    }
    Ok(())
}

// ---- Harness -----------------------------------------------------------------

/// The real system and the model, stepped together.
#[derive(Clone)]
struct Harness {
    real: System,
    model: Model,
}

impl Harness {
    fn new(layout: Layout) -> Self {
        Self::with_fault(layout, Fault::None)
    }

    fn with_fault(layout: Layout, fault: Fault) -> Self {
        Self {
            real: System::new(layout, fault),
            model: Model::new(layout),
        }
    }

    /// Runs one action on both sides and checks everything.
    fn step(&mut self, action: Action) -> Result<Outcome, String> {
        let before = self.real.snapshot();
        let outcome = self.real.apply(action);
        let expected = self.model.apply(action);
        if outcome != expected {
            return Err(format!(
                "MODEL: {action:?} gave {outcome:?}, the model expected {expected:?}"
            ));
        }
        let after = self.real.snapshot();
        check_invariants(&Step {
            before: &before,
            after: &after,
            action,
            outcome,
            real: &self.real,
        })?;
        agree(&self.real, &self.model)?;
        Ok(outcome)
    }

    fn run(&mut self, actions: &[Action]) -> Result<Vec<Outcome>, String> {
        let mut outcomes = Vec::new();
        for (position, action) in actions.iter().enumerate() {
            let outcome = self
                .step(*action)
                .map_err(|error| format!("{error}\n  at step {position} of {actions:?}"))?;
            outcomes.push(outcome);
        }
        Ok(outcomes)
    }

    /// From this state, expiring and abandoning every Queued Intent unlocks
    /// every account, and a new operation over all accounts can be queued.
    fn check_liveness(&self) -> Result<(), String> {
        let mut probe = self.clone();
        let queued: Vec<usize> = probe
            .real
            .records
            .iter()
            .enumerate()
            .filter(|(_, r)| r.intent.status == IntentStatus::Queued)
            .map(|(index, _)| index)
            .collect();
        let horizon = queued
            .iter()
            .map(|index| probe.real.records[*index].intent.lock_expiry_slot)
            .max();
        if let Some(horizon) = horizon {
            if probe.real.slot <= horizon {
                probe.step(Action::Advance(horizon + 1 - probe.real.slot))?;
            }
        }
        for index in queued {
            // Sequences are at most a few dozen Intents long, so `At` can name
            // every Intent exactly.
            let target = Target::At(index as u8);
            if index > 255 || target.resolve(probe.real.records.len()) != Some(index) {
                return Err("LIVENESS: target does not address the Intent".to_string());
            }
            let abandon = Action::Abandon {
                target,
                when: AbandonWhen::Now,
            };
            let outcome = probe.step(abandon)?;
            if outcome != Outcome::Abandoned(index) {
                return Err(format!(
                    "LIVENESS: expired Intent {index} could not be abandoned: {outcome:?}"
                ));
            }
        }
        if probe.real.accounts.iter().any(|c| c.lock.is_active()) {
            return Err("LIVENESS: an account is still locked after abandonment".to_string());
        }
        let spec = OpSpec {
            mask: probe.real.layout.full_mask(),
            amount: 7,
            direction: Direction::Credit,
            timing: Timing::Normal,
        };
        match probe.step(Action::Queue(spec))? {
            Outcome::Queued(_) => Ok(()),
            other => Err(format!("LIVENESS: queueing after recovery gave {other:?}")),
        }
    }
}

// ---- Coverage ----------------------------------------------------------------

/// Counts every distinct outcome seen, so a run cannot pass vacuously.
#[derive(Default)]
struct Coverage {
    counts: BTreeMap<&'static str, u64>,
}

impl Coverage {
    fn record(&mut self, outcome: Outcome, real: &System) {
        let retried = |index: usize| real.records[index].retry_of.is_some();
        let label = match outcome {
            Outcome::Skipped => "skipped",
            Outcome::Clock => "clock",
            Outcome::Bumped(_) => "external_bump",
            Outcome::Queued(index) if retried(index) => "retry_queued",
            Outcome::Queued(_) => "queued",
            Outcome::QueueRejected(Reject::Timing) => "queue_rejected_timing",
            Outcome::QueueRejected(Reject::Snapshot) => "queue_rejected_snapshot",
            Outcome::QueueRejected(Reject::SequenceReused) => "queue_rejected_sequence_reused",
            Outcome::QueueRejected(Reject::LockHeld) => "queue_rejected_lock_held",
            Outcome::QueueRejected(Reject::Other) => "queue_rejected_other",
            Outcome::Resolved(index, St::Applied) if retried(index) => "retry_applied",
            Outcome::Resolved(_, St::Applied) => "applied",
            Outcome::Resolved(_, St::Zero) => "zero",
            Outcome::Resolved(_, St::Stale) => "stale",
            Outcome::Resolved(_, St::Expired) => "expired",
            Outcome::Resolved(_, St::Failed) => "failed",
            Outcome::Resolved(..) => "resolved_other",
            Outcome::Ignored => "ignored_terminal_callback",
            Outcome::LeftQueued => "left_queued",
            Outcome::CallbackRejected(_) => "callback_rejected",
            Outcome::Abandoned(_) => "abandoned",
            Outcome::AbandonRejected(AbandonReject::NotExpired) => "abandon_too_early",
            Outcome::AbandonRejected(AbandonReject::NotQueued) => "abandon_not_queued",
            Outcome::AbandonRejected(AbandonReject::Other) => "abandon_rejected_other",
            Outcome::Unexpected => "unexpected",
        };
        *self.counts.entry(label).or_default() += 1;
    }

    fn require(&self, labels: &[&str]) {
        for label in labels {
            assert!(
                self.counts.get(label).copied().unwrap_or(0) > 0,
                "outcome `{label}` never occurred; counts: {:?}",
                self.counts
            );
        }
    }

    fn assert_none(&self, labels: &[&str]) {
        for label in labels {
            assert!(
                self.counts.get(label).copied().unwrap_or(0) == 0,
                "outcome `{label}` must not occur; counts: {:?}",
                self.counts
            );
        }
    }
}

// ---- Runners -----------------------------------------------------------------

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

fn random_spec(rng: &mut Lcg, layout: Layout) -> OpSpec {
    // Masks 2 and 6 in the three-account world are invalid snapshots on purpose.
    let mask = 1 + rng.below(u64::from(layout.full_mask())) as u8;
    let amount = [7, 30, 80][rng.below(3) as usize];
    let direction = if rng.below(2) == 0 {
        Direction::Credit
    } else {
        Direction::Debit
    };
    let timing = match rng.below(8) {
        4 => Timing::Tight,
        5 => Timing::Long,
        7 => Timing::Invalid,
        _ => Timing::Normal,
    };
    OpSpec {
        mask,
        amount,
        direction,
        timing,
    }
}

fn random_target(rng: &mut Lcg) -> Target {
    match rng.below(4) {
        0 => Target::Oldest,
        1 => Target::At(rng.below(256) as u8),
        _ => Target::Newest,
    }
}

fn random_action(rng: &mut Lcg, layout: Layout) -> Action {
    let target = random_target(rng);
    let abandon = |when| Action::Abandon { target, when };
    match rng.below(100) {
        0..=17 => Action::Queue(random_spec(rng, layout)),
        18..=19 => Action::QueueReplaySequence(random_spec(rng, layout)),
        20..=27 => Action::Retry(target),
        28..=40 => deliver_to(target, Delivery::Nonzero),
        41..=45 => deliver_to(target, Delivery::Zero),
        46..=49 => deliver_to(target, Delivery::Stale),
        50..=53 => deliver_to(target, Delivery::AfterDecisionExpiry),
        54..=57 => deliver_to(target, Delivery::StaticFailure),
        58..=60 => deliver_to(target, Delivery::Unauthenticated),
        61..=63 => deliver_to(target, Delivery::BadOffset),
        64..=65 => deliver_to(target, Delivery::BadCommitment),
        66..=67 => deliver_to(target, Delivery::WrongIntent),
        68..=71 => deliver_to(target, Delivery::Duplicate),
        72..=74 => deliver_to(target, Delivery::Late),
        75..=79 => abandon(AbandonWhen::Now),
        80..=84 => abandon(AbandonWhen::AfterLockExpiry),
        85..=94 => Action::Advance(1 + rng.below(5)),
        _ => Action::BumpGeneration(rng.below(8) as u8),
    }
}

/// Runs `actions` from a fresh world, recording coverage and checking liveness
/// at the end.
fn run_checked(
    layout: Layout,
    actions: &[Action],
    coverage: &mut Coverage,
) -> Result<Harness, String> {
    let mut harness = Harness::new(layout);
    for (position, action) in actions.iter().enumerate() {
        let outcome = harness
            .step(*action)
            .map_err(|error| format!("{error}\n  at step {position} of {actions:?}"))?;
        coverage.record(outcome, &harness.real);
    }
    harness
        .check_liveness()
        .map_err(|error| format!("{error}\n  after {actions:?}"))?;
    Ok(harness)
}

struct ExploreStats {
    nodes: u64,
}

/// Depth-first enumeration of every action sequence up to `depth` over
/// `alphabet`. Each node is one action applied to a clone of its parent.
fn explore(
    parent: &Harness,
    alphabet: &[Action],
    depth: usize,
    path: &mut Vec<Action>,
    coverage: &mut Coverage,
    stats: &mut ExploreStats,
) -> Result<(), String> {
    for action in alphabet {
        let mut child = parent.clone();
        path.push(*action);
        let outcome = child
            .step(*action)
            .map_err(|error| format!("{error}\n  at path {path:?}"))?;
        coverage.record(outcome, &child.real);
        stats.nodes += 1;
        // Liveness runs at every leaf. It steps a clone, so it cannot disturb
        // the enumeration.
        if depth == 1 {
            child
                .check_liveness()
                .map_err(|error| format!("{error}\n  at path {path:?}"))?;
        } else {
            explore(&child, alphabet, depth - 1, path, coverage, stats)?;
        }
        path.pop();
    }
    Ok(())
}

fn exhaustive(layout: Layout, alphabet: &[Action], depth: usize) -> (Coverage, u64) {
    let mut coverage = Coverage::default();
    let mut stats = ExploreStats { nodes: 0 };
    let mut path = Vec::new();
    explore(
        &Harness::new(layout),
        alphabet,
        depth,
        &mut path,
        &mut coverage,
        &mut stats,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let expected: u64 = (1..=depth as u32)
        .map(|length| (alphabet.len() as u64).pow(length))
        .sum();
    assert_eq!(stats.nodes, expected, "every sequence must be enumerated");
    (coverage, stats.nodes)
}

// ---- Tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const CREDIT_BOTH: OpSpec = OpSpec {
        mask: 0b11,
        amount: 30,
        direction: Direction::Credit,
        timing: Timing::Normal,
    };

    fn queue_both() -> Action {
        Action::Queue(CREDIT_BOTH)
    }

    fn newest(kind: Delivery) -> Action {
        deliver_to(Target::Newest, kind)
    }

    fn abandon_now(target: Target) -> Action {
        Action::Abandon {
            target,
            when: AbandonWhen::Now,
        }
    }

    fn initial_entries(layout: Layout) -> Vec<u64> {
        Harness::new(layout).real.ledger.entries()
    }

    /// Runs `actions`, panicking with the failure and the trace.
    fn run(harness: &mut Harness, actions: &[Action]) -> Vec<Outcome> {
        harness
            .run(actions)
            .unwrap_or_else(|error| panic!("{error}"))
    }

    // ---- Targeted checkpoint tests -----------------------------------------

    /// **Validates: Requirements 3.4, 3.6** Applied moves exactly the modelled
    /// amount, and a duplicate or late callback cannot move it again.
    #[test]
    fn checkpoint_applied_moves_exactly_the_modelled_value_once() {
        let mut harness = Harness::new(Layout::TWO);
        let outcomes = run(&mut harness, &[queue_both(), newest(Delivery::Nonzero)]);
        assert_eq!(outcomes[1], Outcome::Resolved(0, St::Applied));
        // Credit 30 from the vault to account 0.
        assert_eq!(harness.real.ledger.entries(), vec![80, 50, 70]);

        let outcomes = run(
            &mut harness,
            &[newest(Delivery::Duplicate), newest(Delivery::Late)],
        );
        assert_eq!(outcomes, vec![Outcome::Ignored, Outcome::Ignored]);
        assert_eq!(harness.real.ledger.entries(), vec![80, 50, 70]);
        assert_eq!(harness.real.applied_commits, vec![1]);
    }

    /// The scripts that leave one Intent in each non-Applied terminal state.
    fn terminal_scripts() -> Vec<(&'static str, Vec<Action>, Outcome)> {
        vec![
            (
                "zero",
                vec![queue_both(), newest(Delivery::Zero)],
                Outcome::Resolved(0, St::Zero),
            ),
            (
                "stale",
                vec![queue_both(), newest(Delivery::Stale)],
                Outcome::Resolved(0, St::Stale),
            ),
            (
                "expired",
                vec![queue_both(), Action::Advance(3), newest(Delivery::Nonzero)],
                Outcome::Resolved(0, St::Expired),
            ),
            (
                "failed",
                vec![queue_both(), newest(Delivery::StaticFailure)],
                Outcome::Resolved(0, St::Failed),
            ),
            (
                "abandoned",
                vec![
                    queue_both(),
                    Action::Advance(5),
                    abandon_now(Target::Newest),
                ],
                Outcome::Abandoned(0),
            ),
        ]
    }

    /// **Validates: Requirements 3.4, 3.6, 3.8** Zero, Stale, Expired, Failed and
    /// Abandoned each move no value, and neither does a late callback after them.
    #[test]
    fn checkpoint_every_non_applied_terminal_outcome_moves_no_value() {
        for (name, script, last) in terminal_scripts() {
            let mut harness = Harness::new(Layout::TWO);
            let outcomes = run(&mut harness, &script);
            assert_eq!(outcomes.last(), Some(&last), "{name}");
            assert_eq!(
                harness.real.ledger.entries(),
                initial_entries(Layout::TWO),
                "{name} moved value"
            );
            let late = run(&mut harness, &[newest(Delivery::Late)]);
            assert_eq!(late, vec![Outcome::Ignored], "{name}");
            assert_eq!(
                harness.real.ledger.entries(),
                initial_entries(Layout::TWO),
                "{name}: a late callback moved value"
            );
            harness.check_liveness().unwrap();
        }
    }

    /// **Validates: Requirements 3.6, 3.9** Callbacks that change nothing leave
    /// the Intent Queued and every account as it was.
    #[test]
    fn checkpoint_rejected_and_unauthenticated_callbacks_change_nothing() {
        for kind in [
            Delivery::Unauthenticated,
            Delivery::BadOffset,
            Delivery::BadCommitment,
            Delivery::WrongIntent,
        ] {
            let mut harness = Harness::new(Layout::TWO);
            run(&mut harness, &[queue_both()]);
            let before = harness.real.snapshot();
            let outcome = run(&mut harness, &[newest(kind)])[0];
            let expected = match kind {
                Delivery::Unauthenticated => Outcome::LeftQueued,
                Delivery::BadOffset => Outcome::CallbackRejected(CbReject::Offset),
                Delivery::BadCommitment => Outcome::CallbackRejected(CbReject::Inputs),
                _ => Outcome::CallbackRejected(CbReject::IntentKey),
            };
            assert_eq!(outcome, expected);
            assert_eq!(harness.real.snapshot(), before, "{kind:?} changed state");
            assert_eq!(harness.real.records[0].intent.status, IntentStatus::Queued);
        }
    }

    /// **Validates: Requirements 3.1, 3.7** A queue that fails changes nothing,
    /// including the sequence counter.
    #[test]
    fn checkpoint_failed_queues_change_nothing() {
        let mut harness = Harness::new(Layout::THREE);
        run(&mut harness, &[queue_both()]);
        let before = harness.real.snapshot();

        let invalid_snapshot = OpSpec {
            mask: 0b010,
            ..CREDIT_BOTH
        };
        let invalid_timing = OpSpec {
            timing: Timing::Invalid,
            mask: 0b100,
            ..CREDIT_BOTH
        };
        let held = OpSpec {
            mask: 0b101,
            ..CREDIT_BOTH
        };
        let outcomes = run(
            &mut harness,
            &[
                Action::Queue(invalid_snapshot),
                Action::Queue(invalid_timing),
                Action::Queue(held),
                Action::QueueReplaySequence(OpSpec {
                    mask: 0b100,
                    ..CREDIT_BOTH
                }),
            ],
        );
        assert_eq!(
            outcomes,
            vec![
                Outcome::QueueRejected(Reject::Snapshot),
                Outcome::QueueRejected(Reject::Timing),
                Outcome::QueueRejected(Reject::LockHeld),
                Outcome::QueueRejected(Reject::SequenceReused),
            ]
        );
        assert_eq!(harness.real.snapshot(), before);
    }

    /// **Validates: Requirements 3.1, 3.7** Two queues over one account: the
    /// second is rejected and the first Intent's Locks are untouched.
    #[test]
    fn checkpoint_concurrent_queue_on_a_locked_account_is_rejected() {
        let mut harness = Harness::new(Layout::TWO);
        let account_zero_only = OpSpec {
            mask: 0b01,
            ..CREDIT_BOTH
        };
        let outcomes = run(
            &mut harness,
            &[queue_both(), Action::Queue(account_zero_only)],
        );
        assert_eq!(outcomes[1], Outcome::QueueRejected(Reject::LockHeld));
        assert_eq!(harness.real.counter.next_sequence, 1);
        assert!(harness.real.accounts[0]
            .lock
            .matches(&harness.real.records[0].intent.key, offset_for(0)));
    }

    /// **Validates: Requirements 3.6, 3.8** After abandonment a late callback
    /// cannot write, even while a newer Intent holds the same Locks.
    #[test]
    fn checkpoint_late_callback_after_abandonment_cannot_write() {
        let mut harness = Harness::new(Layout::TWO);
        let outcomes = run(
            &mut harness,
            &[
                queue_both(),
                abandon_now(Target::Newest), // too early
                Action::Advance(5),
                abandon_now(Target::Newest),
                queue_both(), // a new writer takes the same Locks
                deliver_to(Target::Oldest, Delivery::Late),
                deliver_to(Target::Oldest, Delivery::Nonzero),
            ],
        );
        assert_eq!(
            outcomes[1],
            Outcome::AbandonRejected(AbandonReject::NotExpired)
        );
        assert_eq!(outcomes[3], Outcome::Abandoned(0));
        assert_eq!(outcomes[5], Outcome::Ignored);
        assert_eq!(outcomes[6], Outcome::Ignored);
        assert_eq!(harness.real.ledger.entries(), initial_entries(Layout::TWO));
        // The abandoned Intent bumped both generations, and the newer Intent
        // still owns both Locks.
        assert_eq!(harness.real.accounts[0].generation, 1);
        assert!(harness.real.accounts[1]
            .lock
            .matches(&harness.real.records[1].intent.key, offset_for(1)));
        // The newer Intent can still apply.
        let applied = run(
            &mut harness,
            &[deliver_to(Target::Newest, Delivery::Nonzero)],
        );
        assert_eq!(applied[0], Outcome::Resolved(1, St::Applied));
        assert_eq!(harness.real.ledger.entries(), vec![80, 50, 70]);
    }

    /// **Validates: Requirements 3.4, 3.6, 3.8** A retry after Failed, Stale,
    /// Expired, Zero or Abandoned uses a fresh sequence and offset, can be
    /// Applied, and the original can never be Applied afterward.
    #[test]
    fn checkpoint_retry_after_each_non_applied_outcome_applies_exactly_once() {
        for (name, mut script, _) in terminal_scripts() {
            script.push(Action::Retry(Target::Newest));
            script.push(newest(Delivery::Nonzero));
            script.push(deliver_to(Target::Oldest, Delivery::Late));
            script.push(deliver_to(Target::Oldest, Delivery::Nonzero));
            script.push(newest(Delivery::Duplicate));

            let mut harness = Harness::new(Layout::TWO);
            let outcomes = run(&mut harness, &script);
            let n = outcomes.len();
            assert_eq!(outcomes[n - 5], Outcome::Queued(1), "{name}");
            assert_eq!(outcomes[n - 4], Outcome::Resolved(1, St::Applied), "{name}");
            assert_eq!(outcomes[n - 3], Outcome::Ignored, "{name}");
            assert_eq!(outcomes[n - 2], Outcome::Ignored, "{name}");
            assert_eq!(outcomes[n - 1], Outcome::Ignored, "{name}");

            let records = &harness.real.records;
            assert_ne!(records[1].intent.sequence, records[0].intent.sequence);
            assert_ne!(
                records[1].intent.computation_offset,
                records[0].intent.computation_offset
            );
            assert_ne!(records[0].intent.status, IntentStatus::Applied, "{name}");
            assert_eq!(records[1].retry_of, Some(0));
            // Value moved once, for the retry only.
            assert_eq!(harness.real.ledger.entries(), vec![80, 50, 70], "{name}");
        }
    }

    /// A retry of an Intent that is still Queued or already Applied is not a
    /// retry: it is skipped.
    #[test]
    fn checkpoint_retry_requires_a_terminal_non_applied_intent() {
        let mut harness = Harness::new(Layout::TWO);
        let outcomes = run(
            &mut harness,
            &[
                queue_both(),
                Action::Retry(Target::Newest),
                newest(Delivery::Nonzero),
                Action::Retry(Target::Newest),
            ],
        );
        assert_eq!(outcomes[1], Outcome::Skipped);
        assert_eq!(outcomes[3], Outcome::Skipped);
        assert_eq!(harness.real.records.len(), 1);
    }

    /// **Validates: Requirements 3.8** An external generation write makes the
    /// callback Stale rather than Applied, with no value moved.
    #[test]
    fn checkpoint_external_generation_change_makes_the_callback_stale() {
        let mut harness = Harness::new(Layout::TWO);
        let outcomes = run(
            &mut harness,
            &[
                queue_both(),
                Action::BumpGeneration(1),
                newest(Delivery::Nonzero),
            ],
        );
        assert_eq!(outcomes[2], Outcome::Resolved(0, St::Stale));
        assert_eq!(harness.real.ledger.entries(), initial_entries(Layout::TWO));
        assert_eq!(harness.real.accounts[0].generation, 0);
        assert_eq!(harness.real.accounts[1].generation, 1);
    }

    /// **Validates: Requirements 3.8** From a state with several expired Intents
    /// on disjoint accounts, abandonment frees everything and a new queue works.
    #[test]
    fn checkpoint_expired_locks_never_leave_an_account_permanently_locked() {
        let mut harness = Harness::new(Layout::THREE);
        let spec = |mask| {
            Action::Queue(OpSpec {
                mask,
                timing: Timing::Long,
                ..CREDIT_BOTH
            })
        };
        let outcomes = run(&mut harness, &[spec(0b001), spec(0b010 | 0b001)]);
        assert_eq!(outcomes[1], Outcome::QueueRejected(Reject::LockHeld));
        run(&mut harness, &[spec(0b100)]);
        harness.check_liveness().unwrap();
        // Nothing was abandoned on the checked state: liveness ran on a clone.
        assert!(harness.real.accounts[0].lock.is_active());
    }

    /// **Validates: Requirements 3.1** Sequences and offsets are strictly
    /// increasing and never reused across queues, retries and failed queues.
    #[test]
    fn checkpoint_sequence_counter_is_strictly_increasing_and_never_reused() {
        let mut harness = Harness::new(Layout::TWO);
        run(
            &mut harness,
            &[
                queue_both(),
                Action::Queue(CREDIT_BOTH), // held
                Action::QueueReplaySequence(CREDIT_BOTH),
                newest(Delivery::StaticFailure),
                Action::Retry(Target::Newest),
                newest(Delivery::Zero),
                Action::Retry(Target::Newest),
            ],
        );
        let sequences: Vec<u64> = harness
            .real
            .records
            .iter()
            .map(|r| r.intent.sequence)
            .collect();
        assert_eq!(sequences, vec![0, 1, 2]);
        assert_eq!(harness.real.counter.next_sequence, 3);
    }

    /// Negative control: a driver that commits value on an outcome that must
    /// move none is caught by the neutrality invariant itself, and the same
    /// scripts pass when the driver is correct.
    #[test]
    fn checkpoint_injected_value_faults_are_caught() {
        let scripts: Vec<(Fault, Vec<Action>)> = vec![
            (Fault::OnZero, vec![queue_both(), newest(Delivery::Zero)]),
            (Fault::OnStale, vec![queue_both(), newest(Delivery::Stale)]),
            (
                Fault::OnExpired,
                vec![queue_both(), Action::Advance(3), newest(Delivery::Nonzero)],
            ),
            (
                Fault::OnFailed,
                vec![queue_both(), newest(Delivery::StaticFailure)],
            ),
            (
                Fault::OnAbandon,
                vec![
                    queue_both(),
                    Action::Advance(5),
                    abandon_now(Target::Newest),
                ],
            ),
            (
                Fault::OnIgnored,
                vec![
                    queue_both(),
                    newest(Delivery::Nonzero),
                    newest(Delivery::Duplicate),
                ],
            ),
            (
                Fault::OnLeftQueued,
                vec![queue_both(), newest(Delivery::Unauthenticated)],
            ),
            (
                Fault::OnRejected,
                vec![queue_both(), newest(Delivery::BadOffset)],
            ),
        ];
        for (fault, script) in scripts {
            let error = Harness::with_fault(Layout::TWO, fault)
                .run(&script)
                .expect_err("the fault must be detected");
            assert!(
                error.starts_with("FINANCIAL NEUTRALITY"),
                "{fault:?} was reported as: {error}"
            );
            Harness::new(Layout::TWO)
                .run(&script)
                .unwrap_or_else(|error| {
                    panic!("{fault:?} script failed on the real driver: {error}")
                });
        }
    }

    // ---- Property tests ------------------------------------------------------

    fn core_alphabet() -> Vec<Action> {
        let spec = |mask, amount, direction, timing| OpSpec {
            mask,
            amount,
            direction,
            timing,
        };
        vec![
            Action::Queue(spec(0b01, 80, Direction::Debit, Timing::Tight)),
            Action::Queue(spec(0b10, 30, Direction::Credit, Timing::Normal)),
            newest(Delivery::Nonzero),
            newest(Delivery::Zero),
            newest(Delivery::StaticFailure),
            newest(Delivery::Stale),
            Action::Advance(3),
            abandon_now(Target::Newest),
            Action::Retry(Target::Newest),
            deliver_to(Target::Oldest, Delivery::Nonzero),
        ]
    }

    fn adversarial_alphabet() -> Vec<Action> {
        vec![
            queue_both(),
            newest(Delivery::Nonzero),
            newest(Delivery::Unauthenticated),
            newest(Delivery::BadCommitment),
            newest(Delivery::Late),
            newest(Delivery::Duplicate),
            Action::BumpGeneration(0),
            Action::Advance(3),
            Action::Abandon {
                target: Target::Newest,
                when: AbandonWhen::AfterLockExpiry,
            },
            Action::Retry(Target::Newest),
        ]
    }

    /// **Validates: Requirements 3.4, 3.5, 3.6, 3.8** Every sequence of up to 5
    /// actions over the core lifecycle alphabet (queue on disjoint accounts,
    /// success, zero, failure, stale, clock, abandon, retry) with two accounts:
    /// the real system matches the model, every invariant holds, and failed
    /// operations move no value.
    #[test]
    fn prop_exhaustive_core_lifecycle_sequences_up_to_length_5() {
        let alphabet = core_alphabet();
        let (coverage, nodes) = exhaustive(Layout::TWO, &alphabet, 5);
        println!(
            "core alphabet: {} actions, {nodes} sequences, outcomes {:?}",
            alphabet.len(),
            coverage.counts
        );
        coverage.require(&[
            "queued",
            "applied",
            "zero",
            "stale",
            "expired",
            "failed",
            "abandoned",
            "ignored_terminal_callback",
            "queue_rejected_lock_held",
            "abandon_too_early",
            "retry_queued",
            "retry_applied",
        ]);
        coverage.assert_none(&["unexpected", "queue_rejected_other"]);
    }

    /// **Validates: Requirements 3.5, 3.6, 3.8** Every sequence of up to 5
    /// actions over the adversarial callback alphabet (unauthenticated, wrong
    /// commitment, duplicate, late, external generation write, abandonment after
    /// expiry, retry) with two accounts.
    #[test]
    fn prop_exhaustive_adversarial_callback_sequences_up_to_length_5() {
        let alphabet = adversarial_alphabet();
        let (coverage, nodes) = exhaustive(Layout::TWO, &alphabet, 5);
        println!(
            "adversarial alphabet: {} actions, {nodes} sequences, outcomes {:?}",
            alphabet.len(),
            coverage.counts
        );
        coverage.require(&[
            "applied",
            "stale",
            "expired",
            "abandoned",
            "left_queued",
            "callback_rejected",
            "ignored_terminal_callback",
            "external_bump",
            "retry_applied",
        ]);
        coverage.assert_none(&["unexpected", "queue_rejected_other"]);
    }

    /// **Validates: Requirements 3.1, 3.4, 3.5, 3.6, 3.7, 3.8** At least 20,000
    /// deterministic random sequences of up to 40 actions over the full
    /// alphabet with three accounts.
    #[test]
    fn prop_random_sequences_full_alphabet_three_accounts() {
        const SEQUENCES: u32 = 20_000;
        let mut rng = Lcg(0x3_5C4E_C4B0);
        let mut coverage = Coverage::default();
        let mut steps = 0usize;
        for _ in 0..SEQUENCES {
            let length = 1 + rng.below(40) as usize;
            let actions: Vec<Action> = (0..length)
                .map(|_| random_action(&mut rng, Layout::THREE))
                .collect();
            steps += actions.len();
            run_checked(Layout::THREE, &actions, &mut coverage)
                .unwrap_or_else(|error| panic!("{error}"));
        }
        println!(
            "random: {SEQUENCES} sequences, {steps} actions, outcomes {:?}",
            coverage.counts
        );
        coverage.require(&[
            "queued",
            "applied",
            "zero",
            "stale",
            "expired",
            "failed",
            "abandoned",
            "ignored_terminal_callback",
            "left_queued",
            "callback_rejected",
            "queue_rejected_timing",
            "queue_rejected_snapshot",
            "queue_rejected_sequence_reused",
            "queue_rejected_lock_held",
            "abandon_too_early",
            "abandon_not_queued",
            "retry_queued",
            "retry_applied",
            "external_bump",
        ]);
        coverage.assert_none(&[
            "unexpected",
            "queue_rejected_other",
            "abandon_rejected_other",
        ]);
    }
}
