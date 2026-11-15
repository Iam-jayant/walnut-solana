//! Operation Intents and their lifecycle.
//!
//! Task 2.1 scaffolding: the Operation Intent record, its one-way terminal
//! status machine, and the handlers for the four lifecycle outcomes (success,
//! static failure, expiry, abandonment) plus the queue path. No financial
//! logic lives here.

pub mod intent;
pub mod lifecycle;

pub use intent::{IntentStatus, OperationIntent, OperationKind};
pub use lifecycle::{abandon, on_static_failure, on_success, queue, CallbackOutcome};
