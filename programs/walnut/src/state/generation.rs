//! Account generations.
//!
//! A generation is a monotonically increasing account revision. A callback may
//! commit only against the generations captured by its Operation Intent; a
//! generation changes whenever accepted state changes or an old operation is
//! abandoned. (Requirements 3.5, 3.8.)

use super::CoordinationError;

/// A monotonically increasing account revision counter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Generation(pub u64);

impl Generation {
    /// Creates a generation at an explicit value.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the current revision value.
    pub const fn value(self) -> u64 {
        self.0
    }

    /// Advances to the next revision.
    ///
    /// Increments by exactly one so the counter stays strictly increasing.
    /// Returns [`CoordinationError::GenerationOverflow`] instead of wrapping,
    /// which would silently let a stale callback alias a future generation.
    pub fn bump(&mut self) -> Result<(), CoordinationError> {
        self.0 = self
            .0
            .checked_add(1)
            .ok_or(CoordinationError::GenerationOverflow)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_increments_by_one() {
        let mut g = Generation::default();
        assert_eq!(g.value(), 0);
        g.bump().unwrap();
        assert_eq!(g.value(), 1);
        g.bump().unwrap();
        assert_eq!(g.value(), 2);
    }

    #[test]
    fn bump_at_max_overflows() {
        let mut g = Generation::new(u64::MAX);
        assert_eq!(g.bump(), Err(CoordinationError::GenerationOverflow));
        // The value is unchanged on overflow.
        assert_eq!(g.value(), u64::MAX);
    }

    /// **Validates: Requirements 3.5, 3.8** — generations only ever increase,
    /// by exactly one per bump, until they saturate the domain.
    #[test]
    fn prop_generation_strictly_increases() {
        let mut g = Generation::new(0);
        let mut previous = g.value();
        for expected in 1..10_000u64 {
            g.bump().unwrap();
            assert!(g.value() > previous);
            assert_eq!(g.value(), previous + 1);
            assert_eq!(g.value(), expected);
            previous = g.value();
        }
    }
}
