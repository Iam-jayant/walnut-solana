//! First-supplier gate and the public Recovery Claim count (Task 8.1;
//! Requirements 7.13, 8.2, 8.3; design "Supply").
//!
//! When total Supply Shares is zero, the next supplier may only enter a
//! canonically empty Pool: encrypted Cash, Debt Shares, and Reserves are zero
//! (checked by the circuit, [`crate::math::supply::is_canonical_empty`]) and
//! the Market's public `unsettled_recovery_claim_count` is zero (checked here
//! and passed to the circuit as the public `claims_clear` flag). A failed
//! condition returns Zero and moves no value. The initial share scale applies
//! only in that state.
//!
//! # Where the count lives
//!
//! The count is the Market's public field (design: "The Market stores ...
//! public `unsettled_recovery_claim_count`"), so no Pool layout change is
//! needed. It changes only through the two hooks below. Task 12 calls them
//! from the same account write that:
//!
//! * [`record_absorb_accepted`]: the accepted Absorb callback (it runs under
//!   the Pool Lock, so no supply can be queued against a stale flag);
//! * [`record_recovery_claim_settled`]: the callback that fully settles a
//!   Recovery Claim.
//!
//! Both are atomic: an overflow or underflow returns an error and leaves the
//! count unchanged. A supply queued while the count is nonzero binds the
//! `claims_clear = false` flag in its Intent, so even a count that changes
//! before the callback resolves it `Stale`, never `Applied`.
//!
//! # Reopening an inconsistent Pool
//!
//! A Pool with no Supply Shares but residual Cash, Debt Shares, or Reserves
//! cannot take a first supplier. Reserves are withdrawn by the treasury
//! (`reserve_withdrawal`), Debt Shares are repaid or absorbed, Recovery Claims
//! are settled, and rounding dust is reclassified as Reserves by the separately
//! reviewed [`super::recovery`] operation. Supply never reopens by a shortcut.

use crate::errors::ErrorCode;
use crate::market::Market;

/// The public `claims_clear` flag a supply binds: no Recovery Claim is
/// unsettled.
pub fn claims_clear(market: &Market) -> bool {
    market.unsettled_recovery_claim_count == 0
}

/// Absorb accepted: one more unsettled Recovery Claim. Returns the new count.
/// Atomic.
pub fn record_absorb_accepted(market: &mut Market) -> Result<u64, ErrorCode> {
    let next = market
        .unsettled_recovery_claim_count
        .checked_add(1)
        .ok_or(ErrorCode::ClaimCountOverflow)?;
    market.unsettled_recovery_claim_count = next;
    Ok(next)
}

/// A Recovery Claim fully settled: one fewer unsettled claim. Returns the new
/// count. Atomic.
pub fn record_recovery_claim_settled(market: &mut Market) -> Result<u64, ErrorCode> {
    let next = market
        .unsettled_recovery_claim_count
        .checked_sub(1)
        .ok_or(ErrorCode::ClaimCountUnderflow)?;
    market.unsettled_recovery_claim_count = next;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::checkpoint::PoolPlain;
    use crate::math::supply::{supply_flow, SupplyPosition, SupplyRequest};
    use crate::math::{Lcg, WAD};
    use crate::state::test_fixtures::market;

    fn first_supply(claims_clear: bool) -> bool {
        let out = supply_flow(
            &PoolPlain::default(),
            &SupplyPosition {
                free_usdc: 100,
                supply_shares: 0,
            },
            WAD,
            WAD,
            1_000,
            &SupplyRequest {
                is_withdraw: false,
                requested: 100,
                supply_cap: u64::MAX,
                claims_clear,
            },
        );
        out.applied
    }

    #[test]
    fn absorb_and_settlement_move_the_count_by_one() {
        let mut m = market();
        assert!(claims_clear(&m));
        assert_eq!(record_absorb_accepted(&mut m), Ok(1));
        assert_eq!(record_absorb_accepted(&mut m), Ok(2));
        assert!(!claims_clear(&m));
        assert_eq!(record_recovery_claim_settled(&mut m), Ok(1));
        assert_eq!(record_recovery_claim_settled(&mut m), Ok(0));
        assert!(claims_clear(&m));
    }

    #[test]
    fn underflow_and_overflow_leave_the_count_unchanged() {
        let mut m = market();
        assert_eq!(
            record_recovery_claim_settled(&mut m),
            Err(ErrorCode::ClaimCountUnderflow)
        );
        assert_eq!(m.unsettled_recovery_claim_count, 0);
        m.unsettled_recovery_claim_count = u64::MAX;
        assert_eq!(
            record_absorb_accepted(&mut m),
            Err(ErrorCode::ClaimCountOverflow)
        );
        assert_eq!(m.unsettled_recovery_claim_count, u64::MAX);
    }

    #[test]
    fn an_unsettled_claim_closes_supply_until_it_settles() {
        let mut m = market();
        assert!(first_supply(claims_clear(&m)));
        record_absorb_accepted(&mut m).unwrap();
        assert!(!first_supply(claims_clear(&m)));
        record_recovery_claim_settled(&mut m).unwrap();
        assert!(first_supply(claims_clear(&m)));
    }

    #[test]
    fn prop_the_count_tracks_absorbs_minus_settlements_and_gates_supply() {
        let mut rng = Lcg(0x8101);
        let mut m = market();
        let mut model: u64 = 0;
        for _ in 0..4_000 {
            if rng.next().is_multiple_of(2) {
                record_absorb_accepted(&mut m).unwrap();
                model += 1;
            } else {
                match record_recovery_claim_settled(&mut m) {
                    Ok(_) => model -= 1,
                    Err(e) => {
                        assert_eq!(model, 0);
                        assert_eq!(e, ErrorCode::ClaimCountUnderflow);
                    }
                }
            }
            assert_eq!(m.unsettled_recovery_claim_count, model);
            assert_eq!(first_supply(claims_clear(&m)), model == 0);
        }
    }
}
