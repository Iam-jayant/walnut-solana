use anchor_lang::prelude::*;

use crate::operation::IntentError;
use crate::state::CoordinationError;

/// Program errors.
///
/// Variants are append-only: Anchor numbers them from 6000 in declaration order,
/// so reordering or inserting would renumber existing codes.
#[error_code]
pub enum ErrorCode {
    #[msg("The computation was aborted")]
    AbortedComputation,
    #[msg("The cluster is not set")]
    ClusterNotSet,
    #[msg("A protected account is already locked by another operation")]
    LockHeld,
    #[msg("The Lock has not expired, so the operation cannot be abandoned yet")]
    LockNotExpired,
    #[msg("The Operation Intent is not queued")]
    IntentNotQueued,
    #[msg("A protected account generation would overflow")]
    GenerationOverflow,
    #[msg("The same protected account was supplied more than once")]
    DuplicateProtectedAccount,
    #[msg("This actor may not drive that Operation Intent transition")]
    IntentWrongActor,
    #[msg("Decision Expiry has passed, so no value may be applied")]
    DecisionExpired,
    #[msg("Decision Expiry has not passed, so the Intent cannot be marked expired")]
    DecisionNotExpired,
    #[msg("The Operation Intent sequence was already used")]
    IntentSequenceReused,
    #[msg("The Operation Intent sequence is ahead of the owner counter")]
    IntentSequenceOutOfOrder,
    #[msg("The Operation Intent sequence counter would overflow")]
    IntentSequenceOverflow,
    #[msg("The sequence counter belongs to a different owner")]
    IntentCounterOwnerMismatch,
    #[msg("Rent and fees cannot be reclaimed before the Intent is terminal")]
    ReclaimBeforeTerminal,
    #[msg("The signer is not the recorded reclaim authority")]
    ReclaimNotAuthorized,
    #[msg("This kind of reclaim is not allowed for the Intent status")]
    ReclaimNotAllowedForStatus,
    #[msg("The reclaim authority must be the payer")]
    ReclaimAuthorityNotPayer,
    #[msg("Decision Expiry and Lock expiry must follow the queue slot in order")]
    InvalidIntentTiming,
    #[msg("A Position and its captured generation do not pair up")]
    IntentSnapshotMismatch,
}

impl From<CoordinationError> for ErrorCode {
    fn from(error: CoordinationError) -> Self {
        match error {
            CoordinationError::LockHeld => ErrorCode::LockHeld,
            CoordinationError::LockNotExpired => ErrorCode::LockNotExpired,
            CoordinationError::IntentNotQueued => ErrorCode::IntentNotQueued,
            CoordinationError::GenerationOverflow => ErrorCode::GenerationOverflow,
            CoordinationError::DuplicateAccount => ErrorCode::DuplicateProtectedAccount,
        }
    }
}

impl From<IntentError> for ErrorCode {
    fn from(error: IntentError) -> Self {
        match error {
            IntentError::NotQueued => ErrorCode::IntentNotQueued,
            IntentError::WrongActor => ErrorCode::IntentWrongActor,
            IntentError::DecisionExpired => ErrorCode::DecisionExpired,
            IntentError::DecisionNotExpired => ErrorCode::DecisionNotExpired,
            IntentError::LockNotExpired => ErrorCode::LockNotExpired,
            IntentError::SequenceReused => ErrorCode::IntentSequenceReused,
            IntentError::SequenceOutOfOrder => ErrorCode::IntentSequenceOutOfOrder,
            IntentError::SequenceOverflow => ErrorCode::IntentSequenceOverflow,
            IntentError::CounterOwnerMismatch => ErrorCode::IntentCounterOwnerMismatch,
            IntentError::ReclaimBeforeTerminal => ErrorCode::ReclaimBeforeTerminal,
            IntentError::ReclaimNotAuthorized => ErrorCode::ReclaimNotAuthorized,
            IntentError::ReclaimNotAllowedForStatus => ErrorCode::ReclaimNotAllowedForStatus,
            IntentError::ReclaimAuthorityNotPayer => ErrorCode::ReclaimAuthorityNotPayer,
            IntentError::InvalidTiming => ErrorCode::InvalidIntentTiming,
            IntentError::SnapshotMismatch => ErrorCode::IntentSnapshotMismatch,
            IntentError::Coordination(error) => ErrorCode::from(error),
        }
    }
}

/// Lets Anchor instructions propagate coordination failures with `?`.
impl From<CoordinationError> for anchor_lang::error::Error {
    fn from(error: CoordinationError) -> Self {
        ErrorCode::from(error).into()
    }
}

/// Lets Anchor instructions propagate Operation Intent failures with `?`.
impl From<IntentError> for anchor_lang::error::Error {
    fn from(error: IntentError) -> Self {
        ErrorCode::from(error).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Existing codes keep their numbers; new coordination codes append.
    #[test]
    fn error_numbers_are_append_only() {
        assert_eq!(u32::from(ErrorCode::AbortedComputation), 6000);
        assert_eq!(u32::from(ErrorCode::ClusterNotSet), 6001);
        assert_eq!(u32::from(ErrorCode::LockHeld), 6002);
        assert_eq!(u32::from(ErrorCode::LockNotExpired), 6003);
        assert_eq!(u32::from(ErrorCode::IntentNotQueued), 6004);
        assert_eq!(u32::from(ErrorCode::DuplicateProtectedAccount), 6006);
        // Task 3.2 appends after the Task 3.1 codes.
        assert_eq!(u32::from(ErrorCode::IntentWrongActor), 6007);
        assert_eq!(u32::from(ErrorCode::DecisionExpired), 6008);
        assert_eq!(u32::from(ErrorCode::DecisionNotExpired), 6009);
        assert_eq!(u32::from(ErrorCode::IntentSequenceReused), 6010);
        assert_eq!(u32::from(ErrorCode::IntentSequenceOutOfOrder), 6011);
        assert_eq!(u32::from(ErrorCode::IntentSequenceOverflow), 6012);
        assert_eq!(u32::from(ErrorCode::IntentCounterOwnerMismatch), 6013);
        assert_eq!(u32::from(ErrorCode::ReclaimBeforeTerminal), 6014);
        assert_eq!(u32::from(ErrorCode::ReclaimNotAuthorized), 6015);
        assert_eq!(u32::from(ErrorCode::ReclaimNotAllowedForStatus), 6016);
        assert_eq!(u32::from(ErrorCode::ReclaimAuthorityNotPayer), 6017);
        assert_eq!(u32::from(ErrorCode::InvalidIntentTiming), 6018);
        assert_eq!(u32::from(ErrorCode::IntentSnapshotMismatch), 6019);
    }

    #[test]
    fn coordination_errors_map_to_distinct_codes() {
        let all = [
            CoordinationError::LockHeld,
            CoordinationError::LockNotExpired,
            CoordinationError::IntentNotQueued,
            CoordinationError::GenerationOverflow,
            CoordinationError::DuplicateAccount,
        ];
        let codes: Vec<u32> = all
            .iter()
            .map(|error| u32::from(ErrorCode::from(*error)))
            .collect();
        for (index, code) in codes.iter().enumerate() {
            assert!(!codes[index + 1..].contains(code));
        }
    }

    #[test]
    fn intent_errors_map_to_distinct_codes() {
        let all = [
            IntentError::NotQueued,
            IntentError::WrongActor,
            IntentError::DecisionExpired,
            IntentError::DecisionNotExpired,
            IntentError::LockNotExpired,
            IntentError::SequenceReused,
            IntentError::SequenceOutOfOrder,
            IntentError::SequenceOverflow,
            IntentError::CounterOwnerMismatch,
            IntentError::ReclaimBeforeTerminal,
            IntentError::ReclaimNotAuthorized,
            IntentError::ReclaimNotAllowedForStatus,
            IntentError::ReclaimAuthorityNotPayer,
            IntentError::InvalidTiming,
            IntentError::SnapshotMismatch,
        ];
        let codes: Vec<u32> = all
            .iter()
            .map(|error| u32::from(ErrorCode::from(*error)))
            .collect();
        for (index, code) in codes.iter().enumerate() {
            assert!(!codes[index + 1..].contains(code));
        }
    }

    #[test]
    fn intent_errors_reuse_the_existing_codes_for_shared_conditions() {
        let code = |error: IntentError| u32::from(ErrorCode::from(error));
        assert_eq!(
            code(IntentError::NotQueued),
            u32::from(ErrorCode::IntentNotQueued)
        );
        assert_eq!(
            code(IntentError::LockNotExpired),
            u32::from(ErrorCode::LockNotExpired)
        );
        assert_eq!(
            code(IntentError::Coordination(CoordinationError::LockHeld)),
            u32::from(ErrorCode::LockHeld)
        );
    }
}
