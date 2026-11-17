//! Production instruction account structs and handlers.
//!
//! Arcium queue, callback, and computation-definition account structs stay in
//! `lib.rs` next to the `#[arcium_program]` module, as the Arcium macros
//! expect. Plain Anchor instructions live here, one file per instruction, and
//! are glob re-exported from the crate root so the `#[program]` macro finds
//! their generated client modules.

pub mod caps;
pub mod config;
pub mod initialize_market;
pub mod pause;
pub mod position;
pub mod reserve_withdrawal;
pub mod roles;
pub mod rotation;
pub mod shield;
pub mod unshield;
