//! Supply and withdraw-supply, plaintext reference (Task 8).
//!
//! `encrypted-ixs/src/lib.rs::circuits::supply_flow_values` is the circuit
//! form of [`supply_flow`]; `packages/reference-model/src/supply.ts` mirrors it.
//!
//! ```text
//! (pool', equity) = checkpoint(pool)                      // FIRST, Task 7.3
//! supply   (empty Pool): shares = amount * INITIAL_SHARE_SCALE
//!                        requires Cash = Debt Shares = Reserves = 0 and
//!                        unsettled_recovery_claim_count = 0
//! supply   (otherwise) : shares = floor(amount * total_shares / equity)
//! withdraw             : claim  = floor(shares * equity / total_shares)
//! ```
//!
//! Everything is all-or-zero. A refused request returns the INPUT Pool and
//! Position unchanged (no checkpoint is committed on zero), exactly like the
//! reserve withdrawal.
//!
//! # Domain
//!
//! Products must fit `u128`, so the circuit works inside a committed domain:
//! `equity <= 2^64 - 1`, `cash <= 2^64 - 1`, `total_supply_shares <= 2^63`, and the
//! request `<= 2^64` (supply amount) or `<= 2^63` (shares to burn). Outside it
//! the operation is refused (zero), never wrapped.

use super::checkpoint::{checkpoint, PoolPlain};

/// Shares minted per USDC base unit by the first supplier of a canonically
/// empty Pool.
pub const INITIAL_SHARE_SCALE: u128 = 1_000;
/// Largest equity or Cash the supply flow accepts.
pub const MAX_FLOW_VALUE: u128 = u64::MAX as u128;
/// Largest total supply shares the supply flow accepts.
pub const MAX_FLOW_SHARES: u128 = 1 << 63;

/// The decrypted Position fields supply and withdrawal touch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SupplyPosition {
    /// Free USDC.
    pub free_usdc: u64,
    /// Supply shares.
    pub supply_shares: u128,
}

/// What one supply or withdrawal request asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupplyRequest {
    /// `true` for withdraw-supply.
    pub is_withdraw: bool,
    /// Supply: USDC base units. Withdraw: supply shares to burn.
    pub requested: u128,
    /// Public `supply_cap_usdc`.
    pub supply_cap: u64,
    /// Public `unsettled_recovery_claim_count == 0`.
    pub claims_clear: bool,
}

/// The outcome: the payloads to store and whether the request applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupplyOutcome {
    /// Pool payload to store (the input when not applied).
    pub pool: PoolPlain,
    /// Position fields to store (the input when not applied).
    pub position: SupplyPosition,
    /// Whether the request applied (`false` means Zero).
    pub applied: bool,
}

/// Supply or withdraw-supply. Pure; never fails: a refusal is `applied: false`.
pub fn supply_flow(
    pool: &PoolPlain,
    position: &SupplyPosition,
    accounted_index: u128,
    accounting_index: u128,
    reserve_factor_bps: u16,
    request: &SupplyRequest,
) -> SupplyOutcome {
    let refused = SupplyOutcome {
        pool: *pool,
        position: *position,
        applied: false,
    };
    let Ok(cp) = checkpoint(pool, accounted_index, accounting_index, reserve_factor_bps) else {
        return refused;
    };
    let equity = cp.supplier_equity;
    let p = cp.pool;
    if equity > MAX_FLOW_VALUE
        || p.cash > MAX_FLOW_VALUE
        || p.total_supply_shares > MAX_FLOW_SHARES
        || request.requested == 0
    {
        return refused;
    }
    let empty = p.total_supply_shares == 0;
    if request.is_withdraw {
        let shares = request.requested;
        if empty || shares > position.supply_shares || shares > p.total_supply_shares {
            return refused;
        }
        let claim = shares * equity / p.total_supply_shares;
        let new_free = position.free_usdc as u128 + claim;
        if claim == 0 || claim > p.cash || new_free > u64::MAX as u128 {
            return refused;
        }
        SupplyOutcome {
            pool: PoolPlain {
                cash: p.cash - claim,
                total_supply_shares: p.total_supply_shares - shares,
                ..p
            },
            position: SupplyPosition {
                free_usdc: new_free as u64,
                supply_shares: position.supply_shares - shares,
            },
            applied: true,
        }
    } else {
        let amount = request.requested;
        if amount > MAX_FLOW_VALUE
            || amount > position.free_usdc as u128
            || equity + amount > request.supply_cap as u128
        {
            return refused;
        }
        let shares = if empty {
            if p.cash != 0 || p.total_debt_shares != 0 || p.reserves != 0 || !request.claims_clear {
                return refused;
            }
            amount * INITIAL_SHARE_SCALE
        } else {
            if equity == 0 {
                return refused;
            }
            amount * p.total_supply_shares / equity
        };
        if shares == 0 || p.total_supply_shares + shares > MAX_FLOW_SHARES {
            return refused;
        }
        SupplyOutcome {
            pool: PoolPlain {
                cash: p.cash + amount,
                total_supply_shares: p.total_supply_shares + shares,
                ..p
            },
            position: SupplyPosition {
                free_usdc: position.free_usdc - amount as u64,
                supply_shares: position.supply_shares + shares,
            },
            applied: true,
        }
    }
}
