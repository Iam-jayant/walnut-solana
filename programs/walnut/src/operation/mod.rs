//! Operation Intents and their lifecycle.
//!
//! Task 2.1 scaffolding: the Operation Intent record, its one-way terminal
//! status machine, and the handlers for the four lifecycle outcomes (success,
//! static failure, expiry, abandonment) plus the queue path. No financial
//! logic lives here.

pub mod intent;
pub mod lifecycle;

pub use intent::{IntentStatus, OperationIntent, OperationKind};
pub use lifecycle::{
    abandon, on_callback_tx_failure, on_static_failure, on_success, queue, CallbackOutcome,
};

// The failure-matrix tests (Task 2.4) assert the state-machine behaviour of
// every coordination/callback invariant in the Requirement 2.4 matrix. They are
// pure Rust so they run in CI without a localnet or MPC round trip.
#[cfg(test)]
mod failure_matrix;
