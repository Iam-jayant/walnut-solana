//! Canonical fixed-point primitives (Task 7.1).
//!
//! One definition of WAD multiplication and division, with explicit floor and
//! ceil variants and checked conversions between the `u64`, `u128`, and WAD
//! domains of the range table in [`super`]. Nothing here saturates or wraps:
//! every function returns `Result` and an out-of-range input or result is an
//! error.
//!
//! # Rounding conventions
//!
//! | Quantity | Rounding | Why |
//! | -------- | -------- | --- |
//! | debt units from shares (`shares × index / WAD`) | ceil | the borrower never owes less than the shares represent |
//! | shares from a borrowed amount (`amount × WAD / index`) | ceil | the Pool never issues fewer debt shares than the amount needs |
//! | supply shares minted, withdrawal amount paid | floor | against the supplier / for the Pool |
//! | reserve allocation of interest | floor | the remainder stays with suppliers |
//! | per-second rate | ceil | interest never rounds to zero in the Pool's disfavor |
//!
//! # Three implementations, one algorithm
//!
//! * This module is the program-side definition. [`mul_div_floor`] and
//!   [`mul_div_ceil`] form `a × b` in `u128` and fail with
//!   [`MathError::Overflow`] when it does not fit. The range table
//!   (`MAX_SHARES × MAX_BORROW_INDEX = 10^37 < 2^127`) keeps every
//!   share-versus-index product inside that domain.
//! * `encrypted-ixs/src/lib.rs` (`circuits::mul_div_floor`, `mul_div_ceil`,
//!   `mul_wad_*`, `div_wad_*`) carries the same formulas in plain `u128`
//!   (Arcis has no 256-bit type and no `Result`). The circuit range-checks its
//!   operands against the same constants first and falls back to a flagged
//!   "not ok" outcome instead of overflowing. Division by a *constant* WAD
//!   keeps the circuit cost low.
//! * `packages/reference-model/src/fixed.ts` mirrors it in `bigint`.
//!
//! All three read the golden vectors in
//! `packages/reference-model/vectors/fixed.vectors`.
//!
//! [`mul_wad_floor_wide`] and [`mul_wad_ceil_wide`] are the program-side
//! variants for *public* index accrual, where the operands are not bounded by
//! the share range (`index × growth` can exceed `u128` before the final
//! division). They split both operands at `WAD` so the exact result is formed
//! without a 256-bit type; they agree with the narrow functions whenever the
//! narrow product fits.

use super::{MAX_BORROW_INDEX, MAX_SHARES, WAD};

/// A fixed-point failure. Maps onto program errors in `errors.rs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathError {
    /// An intermediate product or sum does not fit `u128`, or the result does
    /// not fit the requested type.
    Overflow,
    /// A divisor was zero.
    DivisionByZero,
    /// An operand is outside its committed domain (share count above
    /// `MAX_SHARES`, index outside `[WAD, MAX_BORROW_INDEX]`).
    OutOfRange,
    /// A subtraction would go below zero.
    Underflow,
    /// The rate, utilization, or band input is invalid.
    InvalidRate,
    /// The target time is before the accounting timestamp.
    TimeRegression,
    /// The elapsed time exceeds `MAX_ELAPSED_SECONDS`.
    ElapsedTooLong,
}

/// `floor(a × b / d)`. Fails with [`MathError::DivisionByZero`] for `d == 0`
/// and [`MathError::Overflow`] when `a × b` does not fit `u128`.
pub const fn mul_div_floor(a: u128, b: u128, d: u128) -> Result<u128, MathError> {
    if d == 0 {
        return Err(MathError::DivisionByZero);
    }
    match a.checked_mul(b) {
        Some(product) => Ok(product / d),
        None => Err(MathError::Overflow),
    }
}

/// `ceil(a × b / d)`. Same failures as [`mul_div_floor`].
pub const fn mul_div_ceil(a: u128, b: u128, d: u128) -> Result<u128, MathError> {
    if d == 0 {
        return Err(MathError::DivisionByZero);
    }
    match a.checked_mul(b) {
        Some(product) => {
            let quotient = product / d;
            if product % d == 0 {
                Ok(quotient)
            } else {
                // `quotient < product / d + 1 <= u128::MAX` because `d >= 2`
                // whenever the remainder is nonzero.
                Ok(quotient + 1)
            }
        }
        None => Err(MathError::Overflow),
    }
}

/// `floor(a × b / WAD)`.
pub const fn mul_wad_floor(a: u128, b: u128) -> Result<u128, MathError> {
    mul_div_floor(a, b, WAD)
}

/// `ceil(a × b / WAD)`.
pub const fn mul_wad_ceil(a: u128, b: u128) -> Result<u128, MathError> {
    mul_div_ceil(a, b, WAD)
}

/// `floor(a × WAD / b)`.
pub const fn div_wad_floor(a: u128, b: u128) -> Result<u128, MathError> {
    mul_div_floor(a, WAD, b)
}

/// `ceil(a × WAD / b)`.
pub const fn div_wad_ceil(a: u128, b: u128) -> Result<u128, MathError> {
    mul_div_ceil(a, WAD, b)
}

/// Exact `(floor, has_remainder)` of `a × b / WAD` without a 256-bit type.
///
/// With `a = ah·WAD + al` and `b = bh·WAD + bl` (`al, bl < WAD`):
///
/// ```text
/// a·b / WAD = ah·bh·WAD + ah·bl + al·bh + al·bl / WAD
/// ```
///
/// The first three terms are integers; only `al·bl` (< 10^36) is divided.
const fn mul_wad_wide(a: u128, b: u128) -> Result<(u128, bool), MathError> {
    let (ah, al) = (a / WAD, a % WAD);
    let (bh, bl) = (b / WAD, b % WAD);
    let Some(hh) = ah.checked_mul(bh) else {
        return Err(MathError::Overflow);
    };
    let Some(hh) = hh.checked_mul(WAD) else {
        return Err(MathError::Overflow);
    };
    let (Some(x), Some(y)) = (ah.checked_mul(bl), al.checked_mul(bh)) else {
        return Err(MathError::Overflow);
    };
    let low = al * bl; // < WAD^2 = 10^36, fits.

    let Some(sum) = hh.checked_add(x) else {
        return Err(MathError::Overflow);
    };
    let Some(sum) = sum.checked_add(y) else {
        return Err(MathError::Overflow);
    };
    let Some(sum) = sum.checked_add(low / WAD) else {
        return Err(MathError::Overflow);
    };
    Ok((sum, low % WAD != 0))
}

/// `floor(a × b / WAD)` for operands whose product may exceed `u128`; fails
/// only if the *result* does not fit.
pub const fn mul_wad_floor_wide(a: u128, b: u128) -> Result<u128, MathError> {
    match mul_wad_wide(a, b) {
        Ok((value, _)) => Ok(value),
        Err(error) => Err(error),
    }
}

/// `ceil(a × b / WAD)` for operands whose product may exceed `u128`.
pub const fn mul_wad_ceil_wide(a: u128, b: u128) -> Result<u128, MathError> {
    match mul_wad_wide(a, b) {
        Ok((value, false)) => Ok(value),
        Ok((value, true)) => match value.checked_add(1) {
            Some(rounded) => Ok(rounded),
            None => Err(MathError::Overflow),
        },
        Err(error) => Err(error),
    }
}

// ---- Domain checks and conversions ------------------------------------------

/// Checks `index` is a valid Borrow Index: `WAD <= index <= MAX_BORROW_INDEX`.
pub const fn check_index(index: u128) -> Result<u128, MathError> {
    if index < WAD || index > MAX_BORROW_INDEX {
        Err(MathError::OutOfRange)
    } else {
        Ok(index)
    }
}

/// Checks `shares` is within the committed share range.
pub const fn check_shares(shares: u128) -> Result<u128, MathError> {
    if shares > MAX_SHARES {
        Err(MathError::OutOfRange)
    } else {
        Ok(shares)
    }
}

/// `u128 -> u64`, failing instead of truncating.
pub const fn to_u64(value: u128) -> Result<u64, MathError> {
    if value > u64::MAX as u128 {
        Err(MathError::Overflow)
    } else {
        Ok(value as u64)
    }
}

/// `u64 -> u128` (lossless; named for symmetry at call sites).
pub const fn from_u64(value: u64) -> u128 {
    value as u128
}

/// `u128 -> i64`, failing instead of truncating (timestamps).
pub const fn to_i64(value: u128) -> Result<i64, MathError> {
    if value > i64::MAX as u128 {
        Err(MathError::Overflow)
    } else {
        Ok(value as i64)
    }
}

/// Debt units owed by `shares` at `index`: `ceil(shares × index / WAD)`.
/// Operands are domain-checked first.
pub const fn debt_from_shares(shares: u128, index: u128) -> Result<u128, MathError> {
    if let Err(error) = check_shares(shares) {
        return Err(error);
    }
    if let Err(error) = check_index(index) {
        return Err(error);
    }
    mul_wad_ceil(shares, index)
}

/// Debt shares issued for `amount` borrowed at `index`:
/// `ceil(amount × WAD / index)`.
pub const fn shares_from_amount_ceil(amount: u128, index: u128) -> Result<u128, MathError> {
    if let Err(error) = check_index(index) {
        return Err(error);
    }
    div_wad_ceil(amount, index)
}

/// Debt shares for `amount` at `index`, rounded down (used for the greatest
/// share count a partial repayment may burn).
pub const fn shares_from_amount_floor(amount: u128, index: u128) -> Result<u128, MathError> {
    if let Err(error) = check_index(index) {
        return Err(error);
    }
    div_wad_floor(amount, index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Lcg;
    use crate::math::{MAX_BORROW_INDEX, MAX_SHARES};

    const VECTORS: &str =
        include_str!("../../../../packages/reference-model/vectors/fixed.vectors");

    fn parse(word: &str) -> Result<u128, MathError> {
        // Expected values are decimal numbers or an error name.
        word.parse::<u128>().map_err(|_| match word {
            "overflow" => MathError::Overflow,
            "div0" => MathError::DivisionByZero,
            "range" => MathError::OutOfRange,
            other => panic!("unknown expectation {other}"),
        })
    }

    #[test]
    fn golden_vectors_hold() {
        let mut checked = 0;
        for line in VECTORS.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let w: Vec<&str> = line.split_whitespace().collect();
            let n = |i: usize| w[i].parse::<u128>().unwrap();
            let (got, expected) = match w[0] {
                "mul_div_floor" => (mul_div_floor(n(1), n(2), n(3)), parse(w[4])),
                "mul_div_ceil" => (mul_div_ceil(n(1), n(2), n(3)), parse(w[4])),
                "mul_wad_floor" => (mul_wad_floor(n(1), n(2)), parse(w[3])),
                "mul_wad_ceil" => (mul_wad_ceil(n(1), n(2)), parse(w[3])),
                "div_wad_floor" => (div_wad_floor(n(1), n(2)), parse(w[3])),
                "div_wad_ceil" => (div_wad_ceil(n(1), n(2)), parse(w[3])),
                "mul_wad_floor_wide" => (mul_wad_floor_wide(n(1), n(2)), parse(w[3])),
                "mul_wad_ceil_wide" => (mul_wad_ceil_wide(n(1), n(2)), parse(w[3])),
                "debt_from_shares" => (debt_from_shares(n(1), n(2)), parse(w[3])),
                "shares_from_amount_ceil" => (shares_from_amount_ceil(n(1), n(2)), parse(w[3])),
                "shares_from_amount_floor" => (shares_from_amount_floor(n(1), n(2)), parse(w[3])),
                _ => continue,
            };
            assert_eq!(got, expected, "vector: {line}");
            checked += 1;
        }
        assert!(checked >= 30, "only {checked} fixed-point vectors ran");
    }

    #[test]
    fn floor_and_ceil_differ_by_at_most_one() {
        assert_eq!(mul_wad_floor(3, WAD / 2), Ok(1));
        assert_eq!(mul_wad_ceil(3, WAD / 2), Ok(2));
        assert_eq!(mul_wad_floor(WAD, WAD), Ok(WAD));
        assert_eq!(mul_wad_ceil(WAD, WAD), Ok(WAD));
        assert_eq!(div_wad_ceil(1, 3 * WAD), Ok(1));
        assert_eq!(div_wad_floor(1, 3 * WAD), Ok(0));
    }

    #[test]
    fn errors_are_reported_not_saturated() {
        assert_eq!(mul_div_floor(1, 1, 0), Err(MathError::DivisionByZero));
        assert_eq!(mul_div_ceil(u128::MAX, 2, 3), Err(MathError::Overflow));
        assert_eq!(to_u64(u64::MAX as u128 + 1), Err(MathError::Overflow));
        assert_eq!(to_u64(7), Ok(7));
        assert_eq!(check_index(WAD - 1), Err(MathError::OutOfRange));
        assert_eq!(
            check_index(MAX_BORROW_INDEX + 1),
            Err(MathError::OutOfRange)
        );
        assert_eq!(
            debt_from_shares(MAX_SHARES + 1, WAD),
            Err(MathError::OutOfRange)
        );
        assert_eq!(
            debt_from_shares(MAX_SHARES, MAX_BORROW_INDEX),
            Ok(MAX_SHARES * 1_000)
        );
    }

    /// **Validates: Requirements 7.1** (checked fixed-point): the wide variant
    /// agrees with the narrow one whenever the narrow product fits.
    #[test]
    fn prop_wide_agrees_with_narrow_when_the_product_fits() {
        let mut rng = Lcg(0x7101);
        for _ in 0..4_000 {
            let a = rng.u128_below(MAX_SHARES * 1_000);
            let b = rng.u128_below(MAX_BORROW_INDEX + 1);
            if a.checked_mul(b).is_some() {
                assert_eq!(mul_wad_floor(a, b), mul_wad_floor_wide(a, b));
                assert_eq!(mul_wad_ceil(a, b), mul_wad_ceil_wide(a, b));
            } else {
                // The wide variant still returns the exact value when it fits.
                assert!(mul_wad_floor_wide(a, b).is_ok());
            }
        }
    }

    #[test]
    fn prop_ceil_is_floor_or_floor_plus_one() {
        let mut rng = Lcg(0x7102);
        for _ in 0..4_000 {
            let a = rng.u128_below(MAX_SHARES + 1);
            let b = WAD + rng.u128_below(MAX_BORROW_INDEX - WAD + 1);
            let floor = mul_wad_floor(a, b).unwrap();
            let ceil = mul_wad_ceil(a, b).unwrap();
            assert!(ceil == floor || ceil == floor + 1);
            assert_eq!(ceil == floor, (a * b).is_multiple_of(WAD));
        }
    }

    #[test]
    fn prop_share_round_trip_never_favors_the_borrower() {
        let mut rng = Lcg(0x7103);
        for _ in 0..4_000 {
            let amount = rng.u128_below(u64::MAX as u128 + 1);
            let index = WAD + rng.u128_below(MAX_BORROW_INDEX - WAD + 1);
            // Borrow `amount`: shares round up, so the booked debt is at least
            // the amount borrowed.
            let shares = shares_from_amount_ceil(amount, index).unwrap();
            let booked = mul_wad_ceil(shares, index).unwrap();
            assert!(booked >= amount);
        }
    }
}
