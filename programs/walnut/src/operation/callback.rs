//! Callback verification and decision helpers (Task 3.3; Requirements 3.4,
//! 3.5, 3.6, 3.8, 3.9).
//!
//! Production callbacks call these helpers. They are pure Rust and take no
//! Anchor account types, so the whole verification order and every outcome run
//! in CI without a localnet or an MPC round trip. The Arcium call itself
//! (`SignedComputationOutputs::verify_output`) stays in the instruction: the
//! instruction passes the result in as [`CallbackContext::output_authenticated`].
//!
//! # Verification order
//!
//! [`classify_callback`] follows the design's success-callback ordering. It
//! reads the Intent and the callback context and changes nothing.
//!
//! | # | Check | Result when it fails |
//! |---|-------|----------------------|
//! | 1 | output authenticated | `LeaveQueued` (no state change) |
//! | 2 | Intent key and kind match the callback ([`verify_intent_binding`]) | error |
//! | 3 | Intent status is `Queued` | `Ignore` (no state change) |
//! | 4 | computation offset matches ([`verify_offset`]) | error |
//! | 5 | accounts and inputs commitments match ([`verify_commitments`]) | error |
//! | 6 | static failure report | `Failed` |
//! | 7 | generations, accounting index, rate epoch ([`verify_dependencies`]) | `Stale` |
//! | 8 | slot is at or before Decision Expiry | `Expired` |
//! | 9 | approval is zero | `Zero` |
//! | 10 | otherwise | `Applied` |
//!
//! Dependencies are checked before expiry (step 7 before step 8), as in the
//! design ("if dependencies changed, mark `Stale`; if Decision Expiry passed,
//! mark `Expired`") and in `lifecycle::on_success`. A callback whose dependency
//! changed and whose Decision Expiry also passed is therefore `Stale`.
//!
//! A commitment mismatch is an error, not `Stale`. A mismatch means the
//! callback was built from other accounts or inputs than the ones queued. If it
//! resolved the Intent as `Stale`, anyone holding a public signed output could
//! cancel a live operation by pairing it with the wrong accounts. Rejecting
//! leaves the Intent `Queued`, so the honest callback can still arrive, and
//! timeout recovery covers the case where it never does. A *captured generation
//! that moved on* is a different condition: the callback recomputes
//! `accounts_commitment` from the generations the Intent **captured** (never the
//! current ones), and [`verify_dependencies`] compares the current generations
//! separately, so movement shows up as `Stale`.
//!
//! An unauthenticated output (step 1) is `LeaveQueued` whatever the Intent's
//! status, because nothing in it can be trusted. A callback for a terminal
//! Intent (step 3) is `Ignore` even if its offset or commitments differ, since
//! it cannot change anything and must not block rent reclaim or the Arcium
//! computation's finalization.
//!
//! # Decision table
//!
//! [`apply_decision`] drives `lifecycle::on_success` / `on_static_failure`
//! after the Task 3.2 transition checks ([`OperationIntent::check_transition`]
//! with the verified-callback actor). [`abandon_timeout`] is the separate
//! permissionless path.
//!
//! | Decision | Intent status | Generations | Locks owned by the Intent |
//! |----------|---------------|-------------|---------------------------|
//! | `Applied` | `Applied` | bumped | released |
//! | `Zero` | `Zero` | unchanged | released |
//! | `Stale` | `Stale` | unchanged | released |
//! | `Expired` | `Expired` | unchanged | released |
//! | `Failed` | `Failed` | unchanged | released |
//! | timeout abandon | `Abandoned` | bumped first | released |
//! | `Ignore` | unchanged (terminal) | unchanged | unchanged |
//! | `LeaveQueued` | unchanged (`Queued`) | unchanged | still held |
//!
//! Locks held by any other Intent are never touched.
//!
//! # Hash seam
//!
//! The commitments are plain SHA-256 from the pure-Rust implementation in
//! `operation/sha256.rs`, because Anchor 1.0.2 removed `solana_program::hash`.
//! That implementation costs more compute units than the `sol_sha256` syscall.
//! [`CommitmentHasher`] is the seam for replacing it: once a dependency change
//! is approved, a syscall-backed hasher (`solana-sha256-hasher`, already in
//! `Cargo.lock`) implements the trait and the callbacks call
//! [`verify_commitments_with`] with it. Nothing here depends on how the hash is
//! computed: verification only compares the recomputed commitment with the
//! stored one, and the output bytes must stay identical so stored Intents and
//! SDK-built commitments remain valid.

use anchor_lang::prelude::Pubkey;

use super::commitment::{account_commitment, input_commitment, AccountBinding, InputField};
use super::intent::{IntentStatus, OperationIntent, OperationKind};
use super::lifecycle::{abandon, on_static_failure, on_success, CallbackOutcome};
use super::ownership::{Actor, IntentError, Transition};
use crate::errors::ErrorCode;
use crate::state::coordination::{with_protected, CoordinationSlot};
use crate::state::{CoordinationError, LockOwner, ProtectedAccount, Slot};

// ---- Errors and reasons ------------------------------------------------------

/// A callback that must be rejected. The transaction reverts, the Intent stays
/// `Queued`, and timeout recovery remains available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackError {
    /// The callback's computation offset is not the Intent's.
    OffsetMismatch,
    /// The callback was delivered for a different Operation Intent.
    IntentKeyMismatch,
    /// The callback's operation kind is not the Intent's.
    IntentKindMismatch,
    /// The accounts recomputed from the callback differ from the commitment.
    AccountsCommitmentMismatch,
    /// The inputs recomputed from the callback differ from the commitment.
    InputsCommitmentMismatch,
    /// A transition-ownership, timing, or coordination failure.
    Intent(IntentError),
}

impl From<IntentError> for CallbackError {
    fn from(error: IntentError) -> Self {
        CallbackError::Intent(error)
    }
}

impl From<CoordinationError> for CallbackError {
    fn from(error: CoordinationError) -> Self {
        CallbackError::Intent(IntentError::from(error))
    }
}

/// The first captured dependency that no longer matches. Carried by
/// [`CallbackDecision::Stale`] so events and logs can say why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaleReason {
    /// The primary Position generation changed.
    PositionGeneration,
    /// The second Position generation changed.
    SecondPositionGeneration,
    /// The Pool generation changed.
    PoolGeneration,
    /// The Asset State generation changed.
    AssetGeneration,
    /// The oracle generation changed.
    OracleGeneration,
    /// The rate-epoch generation changed.
    RateEpoch,
    /// The accounting Borrow Index changed.
    AccountingIndex,
    /// The accounting index timestamp changed.
    AccountingIndexTimestamp,
}

impl StaleReason {
    /// Every reason, in the order [`verify_dependencies`] checks them.
    pub const ALL: [StaleReason; 8] = [
        StaleReason::PositionGeneration,
        StaleReason::SecondPositionGeneration,
        StaleReason::PoolGeneration,
        StaleReason::AssetGeneration,
        StaleReason::OracleGeneration,
        StaleReason::RateEpoch,
        StaleReason::AccountingIndex,
        StaleReason::AccountingIndexTimestamp,
    ];
}

// ---- Callback context --------------------------------------------------------

/// The current values of every dependency the Intent captured, read from the
/// accounts the callback received.
///
/// Optional generations mirror the Intent: a dependency the Intent did not
/// capture must be absent here, and one it captured must be present.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CurrentDependencies {
    /// Current generation of the primary Position, if any.
    pub position_generation: Option<u64>,
    /// Current generation of the second Position, if any.
    pub second_position_generation: Option<u64>,
    /// Current Pool generation, if the Pool is a dependency.
    pub pool_generation: Option<u64>,
    /// Current Asset State generation.
    pub asset_generation: u64,
    /// Current oracle generation.
    pub oracle_generation: u64,
    /// Current rate-epoch generation.
    pub rate_epoch_generation: u64,
    /// Current accounting Borrow Index.
    pub accounting_borrow_index: u128,
    /// Current accounting index timestamp.
    pub accounting_index_timestamp: i64,
}

impl CurrentDependencies {
    /// The values `intent` captured. Equal to the current state exactly when
    /// nothing a callback depends on has moved.
    pub fn captured_by(intent: &OperationIntent) -> Self {
        Self {
            position_generation: intent.position_generation,
            second_position_generation: intent.second_position_generation,
            pool_generation: intent.pool_generation,
            asset_generation: intent.asset_generation,
            oracle_generation: intent.oracle_generation,
            rate_epoch_generation: intent.rate_epoch_generation,
            accounting_borrow_index: intent.accounting_borrow_index,
            accounting_index_timestamp: intent.accounting_index_timestamp,
        }
    }
}

/// What the Arcium callback reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackReport {
    /// The computation succeeded. `nonzero` is whether it approves value.
    Success {
        /// Whether the approved transition is nonzero.
        nonzero: bool,
    },
    /// An authenticated static failure (circuit failure, `OutputTooLarge`).
    StaticFailure,
}

/// Everything a callback must check, gathered from its actual accounts and
/// inputs by the Anchor instruction.
#[derive(Clone, Copy, Debug)]
pub struct CallbackContext<'a> {
    /// `true` only if Arcium `verify_output` succeeded (or, for a static
    /// failure, the failure claim was authenticated). The instruction sets it.
    pub output_authenticated: bool,
    /// The computation offset the callback was delivered for.
    pub computation_offset: u64,
    /// The address of the Operation Intent account the callback received.
    pub intent_key: Pubkey,
    /// The operation kind of the callback instruction.
    pub kind: OperationKind,
    /// The ordered accounts as the callback sees them, with the generations the
    /// Intent **captured** (not the current ones). Hashed into
    /// `accounts_commitment`.
    pub accounts: &'a [AccountBinding],
    /// The ordered immutable inputs as the callback sees them. Hashed into
    /// `inputs_commitment`.
    pub input_fields: &'a [InputField<'a>],
    /// The current dependency values.
    pub current: CurrentDependencies,
    /// The current slot.
    pub current_slot: Slot,
    /// What the callback reports.
    pub report: CallbackReport,
}

/// What a callback does to Walnut state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackDecision {
    /// Terminal Intent: return success and change nothing.
    Ignore,
    /// Unauthenticated output: change nothing; the Intent stays `Queued`.
    LeaveQueued,
    /// Authenticated, current, nonzero: apply and bump generations.
    Applied,
    /// Authenticated, current, approved zero: apply nothing.
    Zero,
    /// A captured dependency changed: apply nothing.
    Stale(StaleReason),
    /// Authenticated but past Decision Expiry: apply nothing.
    Expired,
    /// Authenticated static failure: apply nothing.
    Failed,
}

// ---- Pure checks -------------------------------------------------------------

/// Verifies the callback's computation offset.
pub fn verify_offset(
    intent: &OperationIntent,
    computation_offset: u64,
) -> Result<(), CallbackError> {
    if intent.computation_offset == computation_offset {
        Ok(())
    } else {
        Err(CallbackError::OffsetMismatch)
    }
}

/// Verifies the callback was delivered for this Intent and operation kind.
pub fn verify_intent_binding(
    intent: &OperationIntent,
    intent_key: &Pubkey,
    kind: OperationKind,
) -> Result<(), CallbackError> {
    if intent.key != *intent_key {
        return Err(CallbackError::IntentKeyMismatch);
    }
    if intent.kind != kind {
        return Err(CallbackError::IntentKindMismatch);
    }
    Ok(())
}

/// Computes the two commitments. See the module docs ("Hash seam").
pub trait CommitmentHasher {
    /// The `accounts_commitment` over the ordered accounts.
    fn accounts_commitment(&self, accounts: &[AccountBinding]) -> [u8; 32];
    /// The `inputs_commitment` over the operation kind and ordered inputs.
    fn inputs_commitment(&self, kind: OperationKind, fields: &[InputField<'_>]) -> [u8; 32];
}

/// The current hasher: the canonical commitments over pure-Rust SHA-256.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sha256Commitments;

impl CommitmentHasher for Sha256Commitments {
    fn accounts_commitment(&self, accounts: &[AccountBinding]) -> [u8; 32] {
        account_commitment(accounts)
    }

    fn inputs_commitment(&self, kind: OperationKind, fields: &[InputField<'_>]) -> [u8; 32] {
        input_commitment(kind, fields)
    }
}

/// Recomputes both commitments with `hasher` from the callback's actual
/// accounts and inputs and compares them with the Intent's.
///
/// The accounts commitment is compared first. Verification only compares the
/// recomputed value with the stored one, so it does not depend on which hash
/// the hasher uses.
pub fn verify_commitments_with<H: CommitmentHasher + ?Sized>(
    hasher: &H,
    intent: &OperationIntent,
    accounts: &[AccountBinding],
    fields: &[InputField<'_>],
) -> Result<(), CallbackError> {
    if hasher.accounts_commitment(accounts) != intent.accounts_commitment {
        return Err(CallbackError::AccountsCommitmentMismatch);
    }
    if hasher.inputs_commitment(intent.kind, fields) != intent.inputs_commitment {
        return Err(CallbackError::InputsCommitmentMismatch);
    }
    Ok(())
}

/// [`verify_commitments_with`] using the current SHA-256 commitments.
pub fn verify_commitments(
    intent: &OperationIntent,
    accounts: &[AccountBinding],
    fields: &[InputField<'_>],
) -> Result<(), CallbackError> {
    verify_commitments_with(&Sha256Commitments, intent, accounts, fields)
}

/// Compares every captured generation, the accounting index and timestamp, and
/// the rate epoch with the current values. Returns the first mismatch.
pub fn verify_dependencies(
    intent: &OperationIntent,
    current: &CurrentDependencies,
) -> Result<(), StaleReason> {
    if intent.position_generation != current.position_generation {
        return Err(StaleReason::PositionGeneration);
    }
    if intent.second_position_generation != current.second_position_generation {
        return Err(StaleReason::SecondPositionGeneration);
    }
    if intent.pool_generation != current.pool_generation {
        return Err(StaleReason::PoolGeneration);
    }
    if intent.asset_generation != current.asset_generation {
        return Err(StaleReason::AssetGeneration);
    }
    if intent.oracle_generation != current.oracle_generation {
        return Err(StaleReason::OracleGeneration);
    }
    if intent.rate_epoch_generation != current.rate_epoch_generation {
        return Err(StaleReason::RateEpoch);
    }
    if intent.accounting_borrow_index != current.accounting_borrow_index {
        return Err(StaleReason::AccountingIndex);
    }
    if intent.accounting_index_timestamp != current.accounting_index_timestamp {
        return Err(StaleReason::AccountingIndexTimestamp);
    }
    Ok(())
}

// ---- Classification ----------------------------------------------------------

/// Classifies a callback. Reads only; see the module docs for the order.
///
/// Returns a [`CallbackDecision`] for every outcome that is a legal resolution
/// (including the two no-ops) and a [`CallbackError`] when the callback must be
/// rejected.
pub fn classify_callback(
    intent: &OperationIntent,
    ctx: &CallbackContext<'_>,
) -> Result<CallbackDecision, CallbackError> {
    if !ctx.output_authenticated {
        return Ok(CallbackDecision::LeaveQueued);
    }
    verify_intent_binding(intent, &ctx.intent_key, ctx.kind)?;
    if intent.status.is_terminal() {
        return Ok(CallbackDecision::Ignore);
    }
    verify_offset(intent, ctx.computation_offset)?;
    verify_commitments(intent, ctx.accounts, ctx.input_fields)?;

    let nonzero = match ctx.report {
        CallbackReport::StaticFailure => return Ok(CallbackDecision::Failed),
        CallbackReport::Success { nonzero } => nonzero,
    };
    if let Err(reason) = verify_dependencies(intent, &ctx.current) {
        return Ok(CallbackDecision::Stale(reason));
    }
    if intent.is_past_decision_expiry(ctx.current_slot) {
        return Ok(CallbackDecision::Expired);
    }
    Ok(if nonzero {
        CallbackDecision::Applied
    } else {
        CallbackDecision::Zero
    })
}

/// The thin Anchor-facing wrapper around [`classify_callback`].
///
/// Errors convert into program errors with `?`. `LeaveQueued` becomes
/// [`ErrorCode::CallbackOutputNotAuthenticated`]: the callback transaction
/// reverts, so nothing persists and the Intent stays `Queued` for timeout
/// recovery, exactly like any other failing callback transaction
/// (`lifecycle::on_callback_tx_failure`). Every other decision is returned for
/// the instruction to apply.
pub fn verify_and_classify(
    intent: &OperationIntent,
    ctx: &CallbackContext<'_>,
) -> anchor_lang::Result<CallbackDecision> {
    match classify_callback(intent, ctx)? {
        CallbackDecision::LeaveQueued => Err(ErrorCode::CallbackOutputNotAuthenticated.into()),
        decision => Ok(decision),
    }
}

// ---- Applying a decision -----------------------------------------------------

/// What a decision did, for the Anchor adapter and SDK to map to events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallbackResult {
    /// The lifecycle outcome.
    pub outcome: CallbackOutcome,
    /// The Intent status afterwards.
    pub status: IntentStatus,
    /// How many Locks owned by the Intent were released.
    pub locks_released: usize,
    /// How many protected-account generations were bumped.
    pub generations_bumped: usize,
}

fn held_by(owner: LockOwner, accounts: &[&mut ProtectedAccount]) -> usize {
    accounts
        .iter()
        .filter(|account| owner.holds(&account.lock))
        .count()
}

/// Runs `step` and reports what it changed.
fn measured(
    intent: &mut OperationIntent,
    accounts: &mut [&mut ProtectedAccount],
    step: impl FnOnce(
        &mut OperationIntent,
        &mut [&mut ProtectedAccount],
    ) -> Result<CallbackOutcome, CallbackError>,
) -> Result<CallbackResult, CallbackError> {
    let owner = intent.lock_owner();
    let held_before = held_by(owner, accounts);
    let generations: Vec<u64> = accounts
        .iter()
        .map(|account| account.generation.value())
        .collect();

    let outcome = step(intent, accounts)?;

    let generations_bumped = accounts
        .iter()
        .zip(&generations)
        .filter(|(account, before)| account.generation.value() != **before)
        .count();
    Ok(CallbackResult {
        outcome,
        status: intent.status,
        locks_released: held_before - held_by(owner, accounts),
        generations_bumped,
    })
}

/// Applies a decision from [`classify_callback`].
///
/// Each state-changing decision first passes the Task 3.2 ownership and timing
/// check for the verified-callback actor, then runs the matching lifecycle
/// handler, so a decision that the clock or the Intent's status does not allow
/// is rejected with the Intent and every account unchanged. For example,
/// `Applied` after Decision Expiry fails with `DecisionExpired`.
///
/// A terminal Intent never changes: every decision returns
/// [`CallbackOutcome::TerminalNoop`]. `Ignore` and `LeaveQueued` change
/// nothing.
pub fn apply_decision(
    intent: &mut OperationIntent,
    accounts: &mut [&mut ProtectedAccount],
    decision: CallbackDecision,
    current_slot: Slot,
) -> Result<CallbackResult, CallbackError> {
    measured(intent, accounts, |intent, accounts| {
        if intent.status.is_terminal() {
            return Ok(CallbackOutcome::TerminalNoop);
        }
        let allow = |intent: &OperationIntent, transition| {
            intent.check_transition(transition, Actor::VerifiedCallback, current_slot)
        };
        let outcome = match decision {
            CallbackDecision::Ignore => CallbackOutcome::TerminalNoop,
            CallbackDecision::LeaveQueued => CallbackOutcome::LeftQueuedForRecovery,
            CallbackDecision::Applied => {
                allow(intent, Transition::Applied)?;
                on_success(intent, accounts, current_slot, true, true)?
            }
            CallbackDecision::Zero => {
                allow(intent, Transition::Zero)?;
                on_success(intent, accounts, current_slot, true, false)?
            }
            CallbackDecision::Stale(_) => {
                allow(intent, Transition::Stale)?;
                on_success(intent, accounts, current_slot, false, true)?
            }
            CallbackDecision::Expired => {
                allow(intent, Transition::Expired)?;
                on_success(intent, accounts, current_slot, true, true)?
            }
            CallbackDecision::Failed => {
                allow(intent, Transition::Failed)?;
                on_static_failure(intent, accounts)?
            }
        };
        Ok(outcome)
    })
}

/// [`apply_decision`] over the on-chain [`Coordination`](crate::state::Coordination)
/// fields of loaded accounts.
///
/// It copies the fields out, applies the decision, and writes them back only on
/// success, so a rejected decision leaves every account unchanged. Duplicate
/// accounts are rejected.
pub fn apply_decision_to_slots(
    intent: &mut OperationIntent,
    slots: &mut [CoordinationSlot<'_>],
    decision: CallbackDecision,
    current_slot: Slot,
) -> Result<CallbackResult, CallbackError> {
    with_protected(slots, |accounts| {
        apply_decision(intent, accounts, decision, current_slot)
    })
}

/// Permissionless timeout recovery (`Abandoned`).
///
/// Requires the Intent to be `Queued` and every Lock it owns to be past its
/// expiry slot. It bumps every still-matching generation before it clears the
/// Locks, so a late computation cannot commit against a generation it captured.
/// Any failure leaves the Intent and every account unchanged.
pub fn abandon_timeout(
    intent: &mut OperationIntent,
    accounts: &mut [&mut ProtectedAccount],
    current_slot: Slot,
) -> Result<CallbackResult, CallbackError> {
    measured(intent, accounts, |intent, accounts| {
        intent.check_transition(Transition::Abandoned, Actor::Signer, current_slot)?;
        Ok(abandon(intent, accounts, current_slot)?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::commitment::{AccountRole, InputValue};
    use crate::operation::lifecycle::queue;
    use crate::state::coordination::Coordination;
    use crate::state::Generation;

    fn key(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    const STATUSES: [IntentStatus; 6] = [
        IntentStatus::Applied,
        IntentStatus::Zero,
        IntentStatus::Failed,
        IntentStatus::Stale,
        IntentStatus::Expired,
        IntentStatus::Abandoned,
    ];

    /// Number of accounts the Intent locks. A fourth account is locked by
    /// another operation and must never be touched.
    const FOOTPRINT: usize = 3;

    fn base_bindings() -> Vec<AccountBinding> {
        let binding = |n: u8, role: AccountRole, generation: Option<u64>| AccountBinding {
            key: key(n),
            role,
            expected_owner: key(200),
            mint: None,
            authority: None,
            writable: generation.is_some(),
            generation,
        };
        vec![
            binding(100, AccountRole::Owner, None),
            binding(1, AccountRole::Position, Some(3)),
            binding(2, AccountRole::SecondPosition, Some(5)),
            binding(3, AccountRole::Pool, Some(9)),
        ]
    }

    const BASE_AMOUNT: u64 = 1_000;

    fn base_cipher() -> Vec<u8> {
        (1..=16).collect()
    }

    fn fields<'a>(amount: u64, destination: &Pubkey, cipher: &'a [u8]) -> Vec<InputField<'a>> {
        vec![
            InputField::new(1, InputValue::U64(amount)),
            InputField::new(2, InputValue::Key(*destination)),
            InputField::new(3, InputValue::Bytes(cipher)),
        ]
    }

    fn base_intent() -> OperationIntent {
        let cipher = base_cipher();
        OperationIntent {
            key: key(42),
            owner: key(100),
            payer: key(101),
            reclaim_authority: key(101),
            kind: OperationKind::Borrow,
            computation_offset: 7,
            asset_generation: 11,
            oracle_generation: 12,
            rate_epoch_generation: 13,
            accounting_borrow_index: 1_000_000_000_000_000_000,
            accounting_index_timestamp: 1_700_000_000,
            risk_borrow_index_at_expiry: 1_000_000_000_000_000_001,
            queued_slot: 10,
            decision_expiry_slot: 100,
            lock_expiry_slot: 200,
            position: Some(key(1)),
            position_generation: Some(3),
            second_position: Some(key(2)),
            second_position_generation: Some(5),
            pool_generation: Some(9),
            accounts_commitment: account_commitment(&base_bindings()),
            inputs_commitment: input_commitment(
                OperationKind::Borrow,
                &fields(BASE_AMOUNT, &key(50), &cipher),
            ),
            ..OperationIntent::default()
        }
    }

    /// A queued Intent with its Locks held, plus the callback that resolves it.
    #[derive(Clone)]
    struct Scenario {
        intent: OperationIntent,
        accounts: [ProtectedAccount; FOOTPRINT + 1],
        authenticated: bool,
        offset: u64,
        intent_key: Pubkey,
        kind: OperationKind,
        bindings: Vec<AccountBinding>,
        amount: u64,
        destination: Pubkey,
        cipher: Vec<u8>,
        current: CurrentDependencies,
        slot: Slot,
        report: CallbackReport,
    }

    impl Scenario {
        fn new() -> Self {
            let intent = base_intent();
            let mut accounts = [
                ProtectedAccount::new(key(1)),
                ProtectedAccount::new(key(2)),
                ProtectedAccount::new(key(3)),
                ProtectedAccount::new(key(4)),
            ];
            accounts[0].generation = Generation::new(3);
            accounts[1].generation = Generation::new(5);
            accounts[2].generation = Generation::new(9);
            accounts[3].generation = Generation::new(20);
            {
                let [a, b, c, other] = &mut accounts;
                queue(&intent, &mut [a, b, c], intent.lock_expiry_slot).unwrap();
                other.lock.acquire(key(77), 99, 500).unwrap();
            }
            Self {
                intent,
                accounts,
                authenticated: true,
                offset: intent.computation_offset,
                intent_key: intent.key,
                kind: intent.kind,
                bindings: base_bindings(),
                amount: BASE_AMOUNT,
                destination: key(50),
                cipher: base_cipher(),
                current: CurrentDependencies::captured_by(&intent),
                slot: 50,
                report: CallbackReport::Success { nonzero: true },
            }
        }

        fn classify(&self) -> Result<CallbackDecision, CallbackError> {
            let input_fields = fields(self.amount, &self.destination, &self.cipher);
            let ctx = CallbackContext {
                output_authenticated: self.authenticated,
                computation_offset: self.offset,
                intent_key: self.intent_key,
                kind: self.kind,
                accounts: &self.bindings,
                input_fields: &input_fields,
                current: self.current,
                current_slot: self.slot,
                report: self.report,
            };
            classify_callback(&self.intent, &ctx)
        }

        fn apply(&mut self, decision: CallbackDecision) -> Result<CallbackResult, CallbackError> {
            let [a, b, c, d] = &mut self.accounts;
            apply_decision(&mut self.intent, &mut [a, b, c, d], decision, self.slot)
        }

        fn abandon(&mut self, slot: Slot) -> Result<CallbackResult, CallbackError> {
            let [a, b, c, d] = &mut self.accounts;
            abandon_timeout(&mut self.intent, &mut [a, b, c, d], slot)
        }

        fn generations(&self) -> [u64; FOOTPRINT + 1] {
            std::array::from_fn(|index| self.accounts[index].generation.value())
        }

        fn footprint_locked(&self) -> usize {
            let owner = self.intent.lock_owner();
            self.accounts[..FOOTPRINT]
                .iter()
                .filter(|account| owner.holds(&account.lock))
                .count()
        }

        /// The unrelated account is locked by another operation, untouched.
        fn other_untouched(&self) -> bool {
            let other = &self.accounts[FOOTPRINT];
            other.lock.matches(&key(77), 99) && other.generation.value() == 20
        }
    }

    /// Every way one thing about a callback can differ from what was queued.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Mutation {
        Offset,
        IntentKey,
        Kind,
        AccountKey(usize),
        AccountGeneration,
        InputByte,
        InputAmount,
        InputDestination,
        Dependency(StaleReason),
    }

    fn all_mutations() -> Vec<Mutation> {
        let mut all = vec![Mutation::Offset, Mutation::IntentKey, Mutation::Kind];
        all.extend((0..4).map(Mutation::AccountKey));
        all.extend([
            Mutation::AccountGeneration,
            Mutation::InputByte,
            Mutation::InputAmount,
            Mutation::InputDestination,
        ]);
        all.extend(StaleReason::ALL.map(Mutation::Dependency));
        all
    }

    fn mutate(s: &mut Scenario, mutation: Mutation) {
        match mutation {
            Mutation::Offset => s.offset += 1,
            Mutation::IntentKey => s.intent_key = key(0xEE),
            Mutation::Kind => s.kind = OperationKind::Repay,
            Mutation::AccountKey(index) => s.bindings[index].key = key(0xEE),
            Mutation::AccountGeneration => {
                let generation = s.bindings[3].generation.unwrap();
                s.bindings[3].generation = Some(generation + 1);
            }
            Mutation::InputByte => s.cipher[5] ^= 0x01,
            Mutation::InputAmount => s.amount += 1,
            Mutation::InputDestination => s.destination = key(0xEE),
            Mutation::Dependency(reason) => {
                let c = &mut s.current;
                match reason {
                    StaleReason::PositionGeneration => {
                        c.position_generation = c.position_generation.map(|g| g + 1)
                    }
                    StaleReason::SecondPositionGeneration => {
                        c.second_position_generation = c.second_position_generation.map(|g| g + 1)
                    }
                    StaleReason::PoolGeneration => {
                        c.pool_generation = c.pool_generation.map(|g| g + 1)
                    }
                    StaleReason::AssetGeneration => c.asset_generation += 1,
                    StaleReason::OracleGeneration => c.oracle_generation += 1,
                    StaleReason::RateEpoch => c.rate_epoch_generation += 1,
                    StaleReason::AccountingIndex => c.accounting_borrow_index += 1,
                    StaleReason::AccountingIndexTimestamp => c.accounting_index_timestamp += 1,
                }
            }
        }
    }

    fn is_dependency(mutation: Mutation) -> bool {
        matches!(mutation, Mutation::Dependency(_))
    }

    // ---- Fixture sanity -----------------------------------------------------

    #[test]
    fn the_fixture_is_a_valid_queued_intent_with_locks_held() {
        let s = Scenario::new();
        assert_eq!(s.intent.validate_new(), Ok(()));
        assert_eq!(s.footprint_locked(), FOOTPRINT);
        assert!(s.other_untouched());
        assert_eq!(s.classify(), Ok(CallbackDecision::Applied));
    }

    // ---- Decision branches ---------------------------------------------------

    #[test]
    fn success_with_nonzero_approval_is_applied() {
        let mut s = Scenario::new();
        assert_eq!(s.classify(), Ok(CallbackDecision::Applied));
        let result = s.apply(CallbackDecision::Applied).unwrap();
        assert_eq!(result.outcome, CallbackOutcome::Applied);
        assert_eq!(result.status, IntentStatus::Applied);
        assert_eq!(result.locks_released, 3);
        assert_eq!(result.generations_bumped, 3);
        assert_eq!(s.generations(), [4, 6, 10, 20]);
        assert_eq!(s.footprint_locked(), 0);
        assert!(s.other_untouched());
    }

    #[test]
    fn success_with_zero_approval_is_zero_and_bumps_nothing() {
        let mut s = Scenario::new();
        s.report = CallbackReport::Success { nonzero: false };
        assert_eq!(s.classify(), Ok(CallbackDecision::Zero));
        let result = s.apply(CallbackDecision::Zero).unwrap();
        assert_eq!(result.outcome, CallbackOutcome::Zero);
        assert_eq!(result.status, IntentStatus::Zero);
        assert_eq!((result.locks_released, result.generations_bumped), (3, 0));
        assert_eq!(s.generations(), [3, 5, 9, 20]);
        assert!(s.other_untouched());
    }

    #[test]
    fn authenticated_static_failure_is_failed() {
        let mut s = Scenario::new();
        s.report = CallbackReport::StaticFailure;
        assert_eq!(s.classify(), Ok(CallbackDecision::Failed));
        let result = s.apply(CallbackDecision::Failed).unwrap();
        assert_eq!(result.outcome, CallbackOutcome::Failed);
        assert_eq!(result.status, IntentStatus::Failed);
        assert_eq!((result.locks_released, result.generations_bumped), (3, 0));
        assert_eq!(s.generations(), [3, 5, 9, 20]);
        assert!(s.other_untouched());
    }

    #[test]
    fn a_static_failure_resolves_failed_even_when_dependencies_moved_or_expired() {
        let mut s = Scenario::new();
        s.report = CallbackReport::StaticFailure;
        mutate(&mut s, Mutation::Dependency(StaleReason::PoolGeneration));
        s.slot = 500;
        assert_eq!(s.classify(), Ok(CallbackDecision::Failed));
    }

    #[test]
    fn a_static_failure_still_verifies_context_and_commitments() {
        for mutation in [
            Mutation::Offset,
            Mutation::IntentKey,
            Mutation::AccountKey(1),
            Mutation::InputByte,
        ] {
            let mut s = Scenario::new();
            s.report = CallbackReport::StaticFailure;
            mutate(&mut s, mutation);
            assert!(s.classify().is_err(), "{mutation:?}");
        }
    }

    #[test]
    fn each_changed_dependency_is_stale_with_its_reason() {
        for reason in StaleReason::ALL {
            let mut s = Scenario::new();
            mutate(&mut s, Mutation::Dependency(reason));
            let decision = s.classify().unwrap();
            assert_eq!(decision, CallbackDecision::Stale(reason));

            let result = s.apply(decision).unwrap();
            assert_eq!(result.outcome, CallbackOutcome::Stale);
            assert_eq!(result.status, IntentStatus::Stale);
            assert_eq!((result.locks_released, result.generations_bumped), (3, 0));
            assert_eq!(s.generations(), [3, 5, 9, 20]);
            assert!(s.other_untouched());
        }
    }

    #[test]
    fn a_dependency_present_on_one_side_only_is_stale() {
        let mut s = Scenario::new();
        s.current.pool_generation = None;
        assert_eq!(
            s.classify(),
            Ok(CallbackDecision::Stale(StaleReason::PoolGeneration))
        );

        let mut s = Scenario::new();
        s.intent.second_position = None;
        s.intent.second_position_generation = None;
        s.current.second_position_generation = Some(5);
        assert_eq!(
            s.classify(),
            Ok(CallbackDecision::Stale(
                StaleReason::SecondPositionGeneration
            ))
        );
    }

    #[test]
    fn a_callback_past_decision_expiry_is_expired() {
        let mut s = Scenario::new();
        s.slot = 101;
        assert_eq!(s.classify(), Ok(CallbackDecision::Expired));
        let result = s.apply(CallbackDecision::Expired).unwrap();
        assert_eq!(result.outcome, CallbackOutcome::Expired);
        assert_eq!(result.status, IntentStatus::Expired);
        assert_eq!((result.locks_released, result.generations_bumped), (3, 0));
        assert_eq!(s.generations(), [3, 5, 9, 20]);
        assert!(s.other_untouched());
    }

    #[test]
    fn decision_expiry_is_inclusive() {
        let mut s = Scenario::new();
        s.slot = 100;
        assert_eq!(s.classify(), Ok(CallbackDecision::Applied));
        s.slot = 101;
        assert_eq!(s.classify(), Ok(CallbackDecision::Expired));
    }

    #[test]
    fn stale_takes_precedence_over_expired() {
        let mut s = Scenario::new();
        s.slot = 500;
        mutate(&mut s, Mutation::Dependency(StaleReason::RateEpoch));
        assert_eq!(
            s.classify(),
            Ok(CallbackDecision::Stale(StaleReason::RateEpoch))
        );
    }

    #[test]
    fn expired_applies_to_zero_approvals_too() {
        let mut s = Scenario::new();
        s.report = CallbackReport::Success { nonzero: false };
        s.slot = 101;
        assert_eq!(s.classify(), Ok(CallbackDecision::Expired));
    }

    // ---- Unauthenticated output ----------------------------------------------

    #[test]
    fn unauthenticated_output_leaves_the_intent_queued() {
        let mut s = Scenario::new();
        s.authenticated = false;
        assert_eq!(s.classify(), Ok(CallbackDecision::LeaveQueued));

        let before = (s.intent, s.accounts);
        let result = s.apply(CallbackDecision::LeaveQueued).unwrap();
        assert_eq!(result.outcome, CallbackOutcome::LeftQueuedForRecovery);
        assert_eq!(result.status, IntentStatus::Queued);
        assert_eq!((result.locks_released, result.generations_bumped), (0, 0));
        assert_eq!((s.intent, s.accounts), before);
        assert_eq!(s.footprint_locked(), 3);
    }

    #[test]
    fn unauthenticated_output_is_never_inspected() {
        let mut s = Scenario::new();
        s.authenticated = false;
        for mutation in all_mutations() {
            mutate(&mut s, mutation);
        }
        s.slot = 10_000;
        assert_eq!(s.classify(), Ok(CallbackDecision::LeaveQueued));
    }

    #[test]
    fn unauthenticated_output_recovers_through_timeout_abandonment() {
        let mut s = Scenario::new();
        s.authenticated = false;
        s.apply(CallbackDecision::LeaveQueued).unwrap();
        let result = s.abandon(201).unwrap();
        assert_eq!(result.outcome, CallbackOutcome::Abandoned);
        assert_eq!(s.intent.status, IntentStatus::Abandoned);
    }

    // ---- Terminal Intents -----------------------------------------------------

    #[test]
    fn a_callback_for_a_terminal_intent_is_ignored_without_any_change() {
        for status in STATUSES {
            let mut s = Scenario::new();
            s.intent.status = status;
            assert_eq!(s.classify(), Ok(CallbackDecision::Ignore), "{status:?}");

            let before = (s.intent, s.accounts);
            for decision in [
                CallbackDecision::Ignore,
                CallbackDecision::Applied,
                CallbackDecision::Zero,
                CallbackDecision::Failed,
                CallbackDecision::Expired,
                CallbackDecision::Stale(StaleReason::RateEpoch),
                CallbackDecision::LeaveQueued,
            ] {
                let result = s.apply(decision).unwrap();
                assert_eq!(result.outcome, CallbackOutcome::TerminalNoop);
                assert_eq!(result.status, status);
                assert_eq!((result.locks_released, result.generations_bumped), (0, 0));
                assert_eq!((s.intent, s.accounts), before);
            }
        }
    }

    #[test]
    fn a_terminal_intent_ignores_a_mismatched_offset_or_commitment() {
        let mut s = Scenario::new();
        s.intent.status = IntentStatus::Applied;
        mutate(&mut s, Mutation::Offset);
        mutate(&mut s, Mutation::InputByte);
        assert_eq!(s.classify(), Ok(CallbackDecision::Ignore));
    }

    #[test]
    fn a_wrong_intent_is_rejected_even_when_terminal() {
        let mut s = Scenario::new();
        s.intent.status = IntentStatus::Applied;
        mutate(&mut s, Mutation::IntentKey);
        assert_eq!(s.classify(), Err(CallbackError::IntentKeyMismatch));
    }

    #[test]
    fn a_duplicate_callback_after_applied_changes_nothing() {
        let mut s = Scenario::new();
        let first = s.classify().unwrap();
        s.apply(first).unwrap();
        let after_first = (s.intent, s.accounts);
        assert_eq!(s.generations(), [4, 6, 10, 20]);

        let second = s.classify().unwrap();
        assert_eq!(second, CallbackDecision::Ignore);
        let result = s.apply(second).unwrap();
        assert_eq!(result.outcome, CallbackOutcome::TerminalNoop);
        assert_eq!((s.intent, s.accounts), after_first);
        assert_eq!(s.generations(), [4, 6, 10, 20]);
    }

    #[test]
    fn a_late_callback_after_abandonment_changes_nothing() {
        let mut s = Scenario::new();
        s.abandon(201).unwrap();
        assert_eq!(s.intent.status, IntentStatus::Abandoned);
        let after_abandon = (s.intent, s.accounts);
        assert_eq!(s.generations(), [4, 6, 10, 20]);

        // The old computation finishes late, with the generations it captured.
        s.slot = 50;
        let decision = s.classify().unwrap();
        assert_eq!(decision, CallbackDecision::Ignore);
        let result = s.apply(decision).unwrap();
        assert_eq!(result.outcome, CallbackOutcome::TerminalNoop);
        assert_eq!((s.intent, s.accounts), after_abandon);
    }

    // ---- Rejections -----------------------------------------------------------

    #[test]
    fn a_wrong_computation_offset_is_rejected() {
        let mut s = Scenario::new();
        mutate(&mut s, Mutation::Offset);
        assert_eq!(s.classify(), Err(CallbackError::OffsetMismatch));
        assert_eq!(
            verify_offset(&s.intent, s.intent.computation_offset),
            Ok(())
        );
        assert_eq!(
            verify_offset(&s.intent, s.offset),
            Err(CallbackError::OffsetMismatch)
        );
    }

    #[test]
    fn a_wrong_intent_or_kind_is_rejected() {
        let mut s = Scenario::new();
        mutate(&mut s, Mutation::IntentKey);
        assert_eq!(s.classify(), Err(CallbackError::IntentKeyMismatch));

        let mut s = Scenario::new();
        mutate(&mut s, Mutation::Kind);
        assert_eq!(s.classify(), Err(CallbackError::IntentKindMismatch));

        let intent = base_intent();
        assert_eq!(
            verify_intent_binding(&intent, &intent.key, intent.kind),
            Ok(())
        );
        assert_eq!(
            verify_intent_binding(&intent, &key(1), intent.kind),
            Err(CallbackError::IntentKeyMismatch)
        );
    }

    #[test]
    fn substituting_any_one_account_is_rejected() {
        for index in 0..4 {
            let mut s = Scenario::new();
            mutate(&mut s, Mutation::AccountKey(index));
            assert_eq!(
                s.classify(),
                Err(CallbackError::AccountsCommitmentMismatch),
                "account {index}"
            );
        }
    }

    #[test]
    fn reordering_accounts_is_rejected() {
        let mut s = Scenario::new();
        s.bindings.swap(1, 2);
        assert_eq!(s.classify(), Err(CallbackError::AccountsCommitmentMismatch));
    }

    #[test]
    fn a_callback_built_from_current_generations_does_not_match_the_commitment() {
        // The callback must hash the generations the Intent captured. Hashing a
        // moved generation is a commitment mismatch, not a quiet pass.
        let mut s = Scenario::new();
        mutate(&mut s, Mutation::AccountGeneration);
        assert_eq!(s.classify(), Err(CallbackError::AccountsCommitmentMismatch));
    }

    #[test]
    fn substituting_one_input_byte_is_rejected() {
        for index in 0..16 {
            let mut s = Scenario::new();
            s.cipher[index] ^= 0x80;
            assert_eq!(
                s.classify(),
                Err(CallbackError::InputsCommitmentMismatch),
                "byte {index}"
            );
        }
        for mutation in [Mutation::InputAmount, Mutation::InputDestination] {
            let mut s = Scenario::new();
            mutate(&mut s, mutation);
            assert_eq!(s.classify(), Err(CallbackError::InputsCommitmentMismatch));
        }
    }

    #[test]
    fn a_commitment_mismatch_is_rejected_not_stale() {
        // Rejected callbacks resolve nothing: the Intent can still complete.
        let mut s = Scenario::new();
        mutate(&mut s, Mutation::InputByte);
        assert!(s.classify().is_err());
        assert_eq!(s.intent.status, IntentStatus::Queued);
        assert_eq!(s.footprint_locked(), 3);
        // The honest callback still applies afterwards.
        let honest = Scenario {
            cipher: base_cipher(),
            ..s
        };
        assert_eq!(honest.classify(), Ok(CallbackDecision::Applied));
    }

    // ---- apply_decision guards -------------------------------------------------

    #[test]
    fn applied_after_decision_expiry_is_refused_without_change() {
        let mut s = Scenario::new();
        s.slot = 101;
        let before = (s.intent, s.accounts);
        for decision in [CallbackDecision::Applied, CallbackDecision::Zero] {
            assert_eq!(
                s.apply(decision),
                Err(CallbackError::Intent(IntentError::DecisionExpired))
            );
            assert_eq!((s.intent, s.accounts), before);
        }
    }

    #[test]
    fn expired_before_decision_expiry_is_refused_without_change() {
        let mut s = Scenario::new();
        let before = (s.intent, s.accounts);
        assert_eq!(
            s.apply(CallbackDecision::Expired),
            Err(CallbackError::Intent(IntentError::DecisionNotExpired))
        );
        assert_eq!((s.intent, s.accounts), before);
    }

    #[test]
    fn a_generation_overflow_aborts_applied_without_change() {
        let mut s = Scenario::new();
        s.accounts[1].generation = Generation::new(u64::MAX);
        let before = (s.intent, s.accounts);
        assert_eq!(
            s.apply(CallbackDecision::Applied),
            Err(CallbackError::Intent(IntentError::Coordination(
                CoordinationError::GenerationOverflow
            )))
        );
        assert_eq!((s.intent, s.accounts), before);
    }

    #[test]
    fn apply_decision_to_slots_matches_the_pure_path() {
        let mut s = Scenario::new();
        let mut pure = s.clone();
        let mut coords: Vec<Coordination> = s.accounts[..FOOTPRINT]
            .iter()
            .map(Coordination::from)
            .collect();

        let expected = pure.apply(CallbackDecision::Applied).unwrap();
        let [a, b, c] = &mut coords[..] else {
            unreachable!()
        };
        let result = apply_decision_to_slots(
            &mut s.intent,
            &mut [
                CoordinationSlot::new(key(1), a),
                CoordinationSlot::new(key(2), b),
                CoordinationSlot::new(key(3), c),
            ],
            CallbackDecision::Applied,
            50,
        )
        .unwrap();

        assert_eq!(result, expected);
        assert_eq!(s.intent, pure.intent);
        for (coord, account) in coords.iter().zip(&pure.accounts) {
            assert_eq!(*coord, Coordination::from(account));
        }
    }

    #[test]
    fn apply_decision_to_slots_rejects_duplicates_and_keeps_state() {
        let mut s = Scenario::new();
        let mut a = Coordination::from(&s.accounts[0]);
        let mut again = a;
        let before = (s.intent, a, again);
        let result = apply_decision_to_slots(
            &mut s.intent,
            &mut [
                CoordinationSlot::new(key(1), &mut a),
                CoordinationSlot::new(key(1), &mut again),
            ],
            CallbackDecision::Applied,
            50,
        );
        assert_eq!(
            result,
            Err(CallbackError::Intent(IntentError::Coordination(
                CoordinationError::DuplicateAccount
            )))
        );
        assert_eq!((s.intent, a, again), before);
    }

    // ---- Timeout abandonment ---------------------------------------------------

    #[test]
    fn abandonment_requires_lock_expiry() {
        let mut s = Scenario::new();
        let before = (s.intent, s.accounts);
        assert_eq!(
            s.abandon(200),
            Err(CallbackError::Intent(IntentError::LockNotExpired))
        );
        assert_eq!((s.intent, s.accounts), before);
    }

    #[test]
    fn abandonment_bumps_generations_and_releases_only_its_locks() {
        let mut s = Scenario::new();
        let result = s.abandon(201).unwrap();
        assert_eq!(result.outcome, CallbackOutcome::Abandoned);
        assert_eq!(result.status, IntentStatus::Abandoned);
        assert_eq!((result.locks_released, result.generations_bumped), (3, 3));
        assert_eq!(s.generations(), [4, 6, 10, 20]);
        assert_eq!(s.footprint_locked(), 0);
        assert!(s.other_untouched());
    }

    #[test]
    fn a_terminal_intent_cannot_be_abandoned() {
        let mut s = Scenario::new();
        let decision = s.classify().unwrap();
        s.apply(decision).unwrap();
        let before = (s.intent, s.accounts);
        assert_eq!(
            s.abandon(999),
            Err(CallbackError::Intent(IntentError::NotQueued))
        );
        assert_eq!((s.intent, s.accounts), before);
    }

    // ---- Anchor wrapper --------------------------------------------------------

    fn error_number(error: anchor_lang::error::Error) -> u32 {
        match error {
            anchor_lang::error::Error::AnchorError(error) => error.error_code_number,
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn verify_and_classify_maps_each_result() {
        fn context<'a>(s: &'a Scenario, input_fields: &'a [InputField<'a>]) -> CallbackContext<'a> {
            CallbackContext {
                output_authenticated: s.authenticated,
                computation_offset: s.offset,
                intent_key: s.intent_key,
                kind: s.kind,
                accounts: &s.bindings,
                input_fields,
                current: s.current,
                current_slot: s.slot,
                report: s.report,
            }
        }

        let s = Scenario::new();
        let cipher = base_cipher();
        let input_fields = fields(BASE_AMOUNT, &key(50), &cipher);

        assert_eq!(
            verify_and_classify(&s.intent, &context(&s, &input_fields)).unwrap(),
            CallbackDecision::Applied
        );

        // Unauthenticated output reverts the callback, leaving the Intent queued.
        let unauthenticated = Scenario {
            authenticated: false,
            ..s.clone()
        };
        let error = verify_and_classify(
            &unauthenticated.intent,
            &context(&unauthenticated, &input_fields),
        )
        .unwrap_err();
        assert_eq!(
            error_number(error),
            u32::from(ErrorCode::CallbackOutputNotAuthenticated)
        );

        let wrong_offset = Scenario {
            offset: 99,
            ..s.clone()
        };
        let error =
            verify_and_classify(&wrong_offset.intent, &context(&wrong_offset, &input_fields))
                .unwrap_err();
        assert_eq!(
            error_number(error),
            u32::from(ErrorCode::CallbackOffsetMismatch)
        );

        // A terminal Intent succeeds with `Ignore`.
        let mut terminal = s.clone();
        terminal.intent.status = IntentStatus::Failed;
        assert_eq!(
            verify_and_classify(&terminal.intent, &context(&terminal, &input_fields)).unwrap(),
            CallbackDecision::Ignore
        );
    }

    // ---- Hash seam ----------------------------------------------------------------

    /// A deliberately different, non-cryptographic "hash": it only has to be a
    /// deterministic function of its input for the comparison to work.
    struct FoldHasher(u8);

    impl FoldHasher {
        fn fold(&self, bytes: &[u8]) -> [u8; 32] {
            let mut out = [self.0; 32];
            for (index, byte) in bytes.iter().enumerate() {
                out[index % 32] = out[index % 32].wrapping_mul(31).wrapping_add(*byte);
            }
            out
        }
    }

    impl CommitmentHasher for FoldHasher {
        fn accounts_commitment(&self, accounts: &[AccountBinding]) -> [u8; 32] {
            let mut bytes = Vec::new();
            for account in accounts {
                bytes.extend_from_slice(&account.key.to_bytes());
                bytes.push(account.role.tag());
                bytes.extend_from_slice(&account.generation.unwrap_or(0).to_le_bytes());
            }
            self.fold(&bytes)
        }

        fn inputs_commitment(&self, kind: OperationKind, fields: &[InputField<'_>]) -> [u8; 32] {
            let mut bytes = vec![kind.tag()];
            for field in fields {
                bytes.push(field.id);
                match field.value {
                    InputValue::U64(value) => bytes.extend_from_slice(&value.to_le_bytes()),
                    InputValue::Key(value) => bytes.extend_from_slice(&value.to_bytes()),
                    InputValue::Bytes(value) => bytes.extend_from_slice(value),
                    _ => {}
                }
            }
            self.fold(&bytes)
        }
    }

    #[test]
    fn verify_commitments_does_not_depend_on_the_hash_implementation() {
        let hasher = FoldHasher(9);
        let cipher = base_cipher();
        let input_fields = fields(BASE_AMOUNT, &key(50), &cipher);
        let bindings = base_bindings();

        // An Intent that stores commitments from a different hash still verifies.
        let mut intent = base_intent();
        intent.accounts_commitment = hasher.accounts_commitment(&bindings);
        intent.inputs_commitment = hasher.inputs_commitment(intent.kind, &input_fields);
        assert_eq!(
            verify_commitments_with(&hasher, &intent, &bindings, &input_fields),
            Ok(())
        );
        // The same stored values do not verify under the SHA-256 hasher, so the
        // comparison is purely recomputed-versus-stored.
        assert_eq!(
            verify_commitments(&intent, &bindings, &input_fields),
            Err(CallbackError::AccountsCommitmentMismatch)
        );

        // Substitution is still caught under the other hash.
        let mut substituted = bindings.clone();
        substituted[2].key = key(0xEE);
        assert_eq!(
            verify_commitments_with(&hasher, &intent, &substituted, &input_fields),
            Err(CallbackError::AccountsCommitmentMismatch)
        );
        let mut tampered = cipher.clone();
        tampered[0] ^= 1;
        let tampered_fields = fields(BASE_AMOUNT, &key(50), &tampered);
        assert_eq!(
            verify_commitments_with(&hasher, &intent, &bindings, &tampered_fields),
            Err(CallbackError::InputsCommitmentMismatch)
        );
    }

    #[test]
    fn the_default_hasher_is_the_canonical_commitment() {
        let cipher = base_cipher();
        let input_fields = fields(BASE_AMOUNT, &key(50), &cipher);
        let bindings = base_bindings();
        assert_eq!(
            Sha256Commitments.accounts_commitment(&bindings),
            account_commitment(&bindings)
        );
        assert_eq!(
            Sha256Commitments.inputs_commitment(OperationKind::Borrow, &input_fields),
            input_commitment(OperationKind::Borrow, &input_fields)
        );
        assert_eq!(
            verify_commitments(&base_intent(), &bindings, &input_fields),
            Ok(())
        );
    }

    // ---- Property tests (run via `cargo test -- prop_`) ---------------------------

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

        fn below(&mut self, bound: usize) -> usize {
            (self.sample() % bound as u64) as usize
        }

        fn chance(&mut self) -> bool {
            self.sample().is_multiple_of(2)
        }
    }

    fn random_report(rng: &mut Lcg) -> CallbackReport {
        match rng.below(3) {
            0 => CallbackReport::StaticFailure,
            1 => CallbackReport::Success { nonzero: false },
            _ => CallbackReport::Success { nonzero: true },
        }
    }

    fn random_slot(rng: &mut Lcg) -> Slot {
        rng.sample() % 300
    }

    /// A scenario with every field randomly perturbed by up to a few mutations.
    fn perturbed(rng: &mut Lcg) -> Scenario {
        let mut s = Scenario::new();
        let mutations = all_mutations();
        for _ in 0..rng.below(4) {
            mutate(&mut s, mutations[rng.below(mutations.len())]);
        }
        s.slot = random_slot(rng);
        s.report = random_report(rng);
        s
    }

    /// **Validates: Requirements 3.6, 3.9** â€” an unauthenticated output never
    /// changes any state, whatever else is wrong with the callback.
    #[test]
    fn prop_unauthenticated_output_never_changes_state() {
        let mut rng = Lcg(0x3303_0001);
        for _ in 0..3_000 {
            let mut s = perturbed(&mut rng);
            s.authenticated = false;
            if rng.chance() {
                s.intent.status = STATUSES[rng.below(STATUSES.len())];
            }
            assert_eq!(s.classify(), Ok(CallbackDecision::LeaveQueued));

            let before = (s.intent, s.accounts);
            let result = s.apply(CallbackDecision::LeaveQueued).unwrap();
            assert_eq!((s.intent, s.accounts), before);
            assert_eq!((result.locks_released, result.generations_bumped), (0, 0));
        }
    }

    /// **Validates: Requirements 3.4, 3.6** â€” a terminal Intent never changes
    /// state, whatever decision is applied to it.
    #[test]
    fn prop_a_terminal_intent_never_changes_state() {
        let mut rng = Lcg(0x3303_0002);
        let decisions = [
            CallbackDecision::Ignore,
            CallbackDecision::LeaveQueued,
            CallbackDecision::Applied,
            CallbackDecision::Zero,
            CallbackDecision::Stale(StaleReason::AssetGeneration),
            CallbackDecision::Expired,
            CallbackDecision::Failed,
        ];
        for _ in 0..3_000 {
            let mut s = Scenario::new();
            // Only mutations after the Intent binding checks may differ.
            let mutations = all_mutations();
            for _ in 0..rng.below(4) {
                let mutation = mutations[rng.below(mutations.len())];
                if !matches!(mutation, Mutation::IntentKey | Mutation::Kind) {
                    mutate(&mut s, mutation);
                }
            }
            s.slot = random_slot(&mut rng);
            s.report = random_report(&mut rng);
            s.intent.status = STATUSES[rng.below(STATUSES.len())];
            assert_eq!(s.classify(), Ok(CallbackDecision::Ignore));

            let before = (s.intent, s.accounts);
            let decision = decisions[rng.below(decisions.len())];
            let result = s.apply(decision).unwrap();
            assert_eq!(result.outcome, CallbackOutcome::TerminalNoop);
            assert_eq!((s.intent, s.accounts), before);
            assert!(s.abandon(random_slot(&mut rng)).is_err());
            assert_eq!((s.intent, s.accounts), before);
        }
    }

    /// **Validates: Requirements 3.5, 3.6** â€” any single-field substitution is
    /// rejected or resolves stale. It is never applied or approved.
    #[test]
    fn prop_a_single_field_substitution_is_never_applied() {
        let mut rng = Lcg(0x3303_0003);
        let mutations = all_mutations();
        for _ in 0..3_000 {
            let mutation = mutations[rng.below(mutations.len())];
            let mut s = Scenario::new();
            s.slot = random_slot(&mut rng);
            s.report = random_report(&mut rng);
            mutate(&mut s, mutation);

            let decision = s.classify();
            assert!(
                !matches!(
                    decision,
                    Ok(CallbackDecision::Applied | CallbackDecision::Zero)
                ),
                "{mutation:?} -> {decision:?}"
            );
            if is_dependency(mutation) {
                // A moved dependency is stale for a success and `Failed` for a
                // static failure, which writes nothing either way.
                match s.report {
                    CallbackReport::StaticFailure => {
                        assert_eq!(decision, Ok(CallbackDecision::Failed))
                    }
                    CallbackReport::Success { .. } => {
                        assert!(matches!(decision, Ok(CallbackDecision::Stale(_))))
                    }
                }
            } else {
                assert!(decision.is_err(), "{mutation:?} -> {decision:?}");
            }
        }
    }

    /// Which checks pass in a generated callback.
    #[derive(Clone, Copy, Debug)]
    struct Flags {
        authenticated: bool,
        queued: bool,
        key_ok: bool,
        kind_ok: bool,
        offset_ok: bool,
        accounts_ok: bool,
        inputs_ok: bool,
        dependencies_ok: bool,
        in_time: bool,
        success: bool,
        nonzero: bool,
    }

    /// **Validates: Requirements 3.4, 3.5, 3.6** â€” `Applied` happens only when
    /// every check passes, and the classifier agrees with an independent oracle
    /// on every combination.
    #[test]
    fn prop_applied_only_when_every_check_passes() {
        let mut rng = Lcg(0x3303_0004);
        for _ in 0..6_000 {
            let flags = Flags {
                authenticated: rng.below(4) != 0,
                queued: rng.below(4) != 0,
                key_ok: rng.below(4) != 0,
                kind_ok: rng.below(4) != 0,
                offset_ok: rng.below(4) != 0,
                accounts_ok: rng.below(4) != 0,
                inputs_ok: rng.below(4) != 0,
                dependencies_ok: rng.below(4) != 0,
                in_time: rng.below(4) != 0,
                success: rng.below(4) != 0,
                nonzero: rng.below(4) != 0,
            };
            let mut s = Scenario::new();
            s.authenticated = flags.authenticated;
            if !flags.queued {
                s.intent.status = STATUSES[rng.below(STATUSES.len())];
            }
            if !flags.key_ok {
                mutate(&mut s, Mutation::IntentKey);
            }
            if !flags.kind_ok {
                mutate(&mut s, Mutation::Kind);
            }
            if !flags.offset_ok {
                mutate(&mut s, Mutation::Offset);
            }
            if !flags.accounts_ok {
                mutate(&mut s, Mutation::AccountKey(rng.below(4)));
            }
            if !flags.inputs_ok {
                mutate(&mut s, Mutation::InputByte);
            }
            let mut stale_reason = None;
            if !flags.dependencies_ok {
                let reason = StaleReason::ALL[rng.below(8)];
                stale_reason = Some(reason);
                mutate(&mut s, Mutation::Dependency(reason));
            }
            s.slot = if flags.in_time {
                100
            } else {
                101 + rng.sample() % 50
            };
            s.report = if flags.success {
                CallbackReport::Success {
                    nonzero: flags.nonzero,
                }
            } else {
                CallbackReport::StaticFailure
            };

            // The oracle, written as a flat decision list.
            let expected: Result<CallbackDecision, ()> = if !flags.authenticated {
                Ok(CallbackDecision::LeaveQueued)
            } else if !flags.key_ok || !flags.kind_ok {
                Err(())
            } else if !flags.queued {
                Ok(CallbackDecision::Ignore)
            } else if !flags.offset_ok || !flags.accounts_ok || !flags.inputs_ok {
                Err(())
            } else if !flags.success {
                Ok(CallbackDecision::Failed)
            } else if let Some(reason) = stale_reason {
                Ok(CallbackDecision::Stale(reason))
            } else if !flags.in_time {
                Ok(CallbackDecision::Expired)
            } else if flags.nonzero {
                Ok(CallbackDecision::Applied)
            } else {
                Ok(CallbackDecision::Zero)
            };
            let actual = s.classify();
            assert_eq!(actual.map_err(|_| ()), expected, "{flags:?}");

            let every_check_passes = flags.authenticated
                && flags.queued
                && flags.key_ok
                && flags.kind_ok
                && flags.offset_ok
                && flags.accounts_ok
                && flags.inputs_ok
                && flags.dependencies_ok
                && flags.in_time
                && flags.success
                && flags.nonzero;
            assert_eq!(
                actual == Ok(CallbackDecision::Applied),
                every_check_passes,
                "{flags:?}"
            );
        }
    }

    /// **Validates: Requirements 3.6, 3.7, 3.8** â€” after any decision, every
    /// Lock the Intent owns is either still held (`LeaveQueued`) or released,
    /// generations bump only on `Applied` and `Abandoned`, terminal states are
    /// write-once, and another operation's Lock is never touched.
    #[test]
    fn prop_locks_and_generations_follow_the_decision_table() {
        let mut rng = Lcg(0x3303_0005);
        for _ in 0..3_000 {
            let mut s = Scenario::new();
            let mut previous = s.generations();

            for _ in 0..1 + rng.below(4) {
                let before = (s.intent, s.accounts);
                let was_terminal = s.intent.status.is_terminal();

                // Either a callback (possibly perturbed) or a timeout abandon.
                let result = if rng.below(4) == 0 {
                    s.abandon(random_slot(&mut rng) + 100)
                } else {
                    let mut attempt = perturbed(&mut rng);
                    attempt.authenticated = rng.below(5) != 0;
                    attempt.intent = s.intent;
                    attempt.accounts = s.accounts;
                    // Perturbed *callback* fields only: copy them over.
                    let decision = attempt.classify();
                    let result = match decision {
                        Ok(decision) => attempt.apply(decision),
                        Err(error) => Err(error),
                    };
                    s.intent = attempt.intent;
                    s.accounts = attempt.accounts;
                    result
                };

                match result {
                    Err(_) => {
                        assert_eq!((s.intent, s.accounts), before, "an error changes nothing");
                    }
                    Ok(result) => {
                        let bumps = s.generations();
                        let bumped_accounts = (0..FOOTPRINT)
                            .filter(|index| bumps[*index] != previous[*index])
                            .count();
                        assert_eq!(bumped_accounts, result.generations_bumped);
                        assert_eq!(
                            s.footprint_locked() + result.locks_released,
                            before_held(&before)
                        );

                        match result.outcome {
                            CallbackOutcome::Applied | CallbackOutcome::Abandoned => {
                                assert!(!was_terminal);
                                assert_eq!(result.generations_bumped, FOOTPRINT);
                                assert_eq!(s.footprint_locked(), 0);
                            }
                            CallbackOutcome::Zero
                            | CallbackOutcome::Stale
                            | CallbackOutcome::Expired
                            | CallbackOutcome::Failed => {
                                assert!(!was_terminal);
                                assert_eq!(result.generations_bumped, 0);
                                assert_eq!(s.footprint_locked(), 0);
                            }
                            CallbackOutcome::LeftQueuedForRecovery => {
                                assert_eq!((s.intent, s.accounts), before);
                                assert_eq!(s.footprint_locked(), before_held(&before));
                            }
                            CallbackOutcome::TerminalNoop => {
                                assert_eq!((s.intent, s.accounts), before);
                            }
                        }
                    }
                }

                // Generations never decrease, a terminal status never changes,
                // and the unrelated Lock is never touched.
                let now = s.generations();
                for (after, earlier) in now.iter().zip(&previous) {
                    assert!(after >= earlier);
                }
                if was_terminal {
                    assert_eq!(s.intent.status, before.0.status);
                    assert_eq!(s.intent, before.0);
                }
                assert!(s.other_untouched());
                previous = now;
            }
        }
    }

    fn before_held(before: &(OperationIntent, [ProtectedAccount; FOOTPRINT + 1])) -> usize {
        let owner = before.0.lock_owner();
        before.1[..FOOTPRINT]
            .iter()
            .filter(|account| owner.holds(&account.lock))
            .count()
    }
}
