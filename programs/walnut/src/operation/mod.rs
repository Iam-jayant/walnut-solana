//! Operation Intents and their lifecycle.
//!
//! Task 2.1 scaffolding: the Operation Intent record, its one-way terminal
//! status machine, and the handlers for the four lifecycle outcomes (success,
//! static failure, expiry, abandonment) plus the queue path. Task 3.2 promotes
//! the Intent to a production PDA and adds:
//!
//! * [`seeds`]: the unique, non-replayable address scheme and the per-owner
//!   sequence counter;
//! * [`commitment`] (with [`sha256`]): the canonical account and input
//!   commitments;
//! * [`ownership`]: exact transition ownership, Decision Expiry and Lock-expiry
//!   timing, and payer and reclaim ownership.
//!
//! Task 3.3 adds [`callback`]: the callback verification order, the decision
//! classifier, and the helpers that apply a decision or abandon on timeout.
//!
//! No financial logic lives here.

pub mod callback;
pub mod commitment;
pub mod intent;
pub mod lifecycle;
pub mod ownership;
pub mod seeds;
pub mod sha256;

pub use callback::{
    abandon_timeout, apply_decision, classify_callback, verify_and_classify, CallbackContext,
    CallbackDecision, CallbackError, CallbackReport, CallbackResult, CurrentDependencies,
    StaleReason,
};
pub use commitment::{
    account_commitment, input_commitment, AccountBinding, AccountRole, InputField, InputValue,
};
pub use intent::{IntentStatus, OperationIntent, OperationKind};
pub use lifecycle::{
    abandon, on_callback_tx_failure, on_static_failure, on_success, queue, CallbackOutcome,
};
pub use ownership::{
    abandon_intent, can_reclaim, check_reclaim, Actor, IntentError, ReclaimKind, Transition,
};
pub use seeds::{
    derive_counter_address, derive_intent_address, reserve_intent, IntentSeeds, OperationCounter,
    OPERATION_COUNTER_SEED, OPERATION_INTENT_SEED,
};

// The event-ordering property tests (Task 3.1) drive the shared Lock and
// generation rules, through both the lifecycle handlers and the Anchor adapter,
// over every ordering of the coordination events.
#[cfg(test)]
mod ordering;

// The failure-matrix tests (Task 2.4) assert the state-machine behaviour of
// every coordination/callback invariant in the Requirement 2.4 matrix. They are
// pure Rust so they run in CI without a localnet or MPC round trip.
#[cfg(test)]
mod failure_matrix;

// The Task 3.5 framework checkpoint: a model-based test that drives the real
// queue, callback, failure, expiry, abandonment, and retry code against a
// reference state machine and proves failed operations move no value.
#[cfg(test)]
mod checkpoint;
