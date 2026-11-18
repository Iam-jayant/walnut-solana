use anchor_lang::prelude::*;

use crate::market::MarketError;
use crate::operation::callback::CallbackError;
use crate::operation::IntentError;
use crate::oracle::OracleError;
use crate::state::pending_shield::PendingShieldError;
use crate::state::position::PositionError;
use crate::state::rotation::RotationError;
use crate::state::shield::ShieldError;
use crate::state::unshield::UnshieldError;
use crate::state::CoordinationError;

/// Program errors.
///
/// Variants are append-only: Anchor numbers them from 6000 in declaration order,
/// so reordering or inserting would renumber existing codes.
#[error_code]
#[derive(PartialEq, Eq)]
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
    #[msg("The callback output was not authenticated, so the Intent stays queued")]
    CallbackOutputNotAuthenticated,
    #[msg("The callback computation offset does not match the Operation Intent")]
    CallbackOffsetMismatch,
    #[msg("The callback was delivered for a different Operation Intent")]
    CallbackIntentMismatch,
    #[msg("The callback operation kind does not match the Operation Intent")]
    CallbackKindMismatch,
    #[msg("The callback accounts do not match the accounts commitment")]
    CallbackAccountsCommitmentMismatch,
    #[msg("The callback inputs do not match the inputs commitment")]
    CallbackInputsCommitmentMismatch,
    // ---- Task 4.1: market initialization ---------------------------------
    #[msg("Only the program upgrade authority may initialize the market")]
    InitializerNotUpgradeAuthority,
    #[msg("The program or program-data account is not the expected loader state")]
    InvalidProgramDataAccount,
    #[msg("The Market is already initialized")]
    MarketAlreadyInitialized,
    #[msg("The Asset State is already initialized")]
    AssetStateAlreadyInitialized,
    #[msg("The Pool State is already initialized")]
    PoolStateAlreadyInitialized,
    #[msg("The Recovery Position is already initialized")]
    RecoveryPositionAlreadyInitialized,
    #[msg("The vault authority is already initialized")]
    VaultAuthorityAlreadyInitialized,
    #[msg("The mint is not owned by the SPL Token program")]
    MintNotOwnedByTokenProgram,
    #[msg("The mint is not initialized")]
    MintNotInitialized,
    #[msg("The wSOL mint must be the native mint")]
    WsolMintMismatch,
    #[msg("The USDC mint must not be the native mint")]
    UsdcMintIsNative,
    #[msg("The USDC mint must use 6 decimals")]
    InvalidUsdcDecimals,
    #[msg("An authority key must not be the zero key")]
    InvalidAuthority,
    #[msg("The Pool State is not waiting for its encrypted zero state")]
    PoolStateNotInitializing,
    #[msg("The signer is not the market admin")]
    NotMarketAdmin,
    // ---- Task 4.2: roles --------------------------------------------------
    #[msg("The signer is not the pending admin")]
    NotPendingAdmin,
    #[msg("No admin transfer is pending")]
    NoPendingAdmin,
    #[msg("The signer is not the recorded upgrade authority")]
    NotUpgradeAuthority,
    #[msg("The signer is not the risk authority")]
    NotRiskAuthority,
    #[msg("The signer is not the treasury authority")]
    NotTreasuryAuthority,
    #[msg("The signer is not the guardian authority")]
    NotGuardianAuthority,
    #[msg("The key is one of the Market's own accounts and cannot hold a role")]
    InvalidRoleKey,
    #[msg("The new role key equals the current one")]
    RoleKeyUnchanged,
    #[msg("The key already holds another separated role")]
    RoleKeyNotDistinct,
    #[msg("The loader does not name this key as the program upgrade authority")]
    UpgradeAuthorityMismatch,
    #[msg("The Market is not active")]
    MarketNotActive,
    // ---- Task 4.3: pause controls (`market::pause`) -----------------------
    #[msg("This action is paused")]
    ActionPaused,
    #[msg("The pause mask is empty or names an undefined bit")]
    InvalidPauseFlags,
    #[msg("Repay and collateral addition cannot be paused")]
    PauseNotAllowed,
    #[msg("The guardian pause duration is zero or above the maximum")]
    InvalidPauseDuration,
    #[msg("The guardian must wait for the cooldown after an expired pause")]
    GuardianPauseCooldown,
    #[msg("No guardian pause is active")]
    NoActiveGuardianPause,
    #[msg("The renewed guardian expiry must be later than the current one")]
    InvalidPauseExpiry,
    // ---- Task 4.4: timelocked configuration (`market::config`) ------------
    #[msg("The proposal id is not the next proposal id")]
    ConfigProposalIdMismatch,
    #[msg("The proposed configuration equals the current one")]
    ConfigUnchanged,
    #[msg("A configuration parameter is outside its bounds")]
    ConfigOutOfBounds,
    #[msg("A configuration parameter moves more than its per-change limit")]
    ConfigMovementTooLarge,
    #[msg("The liquidation threshold, bonus, and target Health Factor violate the cross-bound")]
    LiquidationCrossBound,
    #[msg("The timelock delay is outside its bounds")]
    TimelockDelayOutOfBounds,
    #[msg("The timelock delay has not elapsed")]
    TimelockNotElapsed,
    #[msg("The configuration proposal's execution window has ended")]
    ConfigProposalExpired,
    #[msg("The configuration proposal is not pending")]
    ConfigProposalNotPending,
    #[msg("The configuration changed since the proposal was made")]
    ConfigProposalBaseChanged,
    #[msg("Only the risk authority or the guardian may cancel a proposal")]
    NotConfigCanceller,
    #[msg("A proposal id, time, or generation would overflow")]
    ConfigArithmeticOverflow,
    // ---- Task 6: Pyth oracle (`oracle`) -----------------------------------
    #[msg("The price account is not owned by the Pyth receiver program")]
    OracleWrongReceiverProgram,
    #[msg("The price account reports a different feed id than configured")]
    OracleFeedMismatch,
    #[msg("The price is older than the configured maximum age")]
    OracleStalePrice,
    #[msg("The price is not strictly positive")]
    OracleNonPositivePrice,
    #[msg("The price confidence interval is wider than allowed")]
    OracleConfidenceTooWide,
    #[msg("The price exponent is outside the representable window")]
    OracleExponentOutOfRange,
    #[msg("The normalized price is outside the committed price domain")]
    OraclePriceOutOfRange,
    #[msg("The spot price deviates from the EMA or last-good price by too much")]
    OracleDeviationTooHigh,
    #[msg("An oracle parameter is outside its bounds")]
    InvalidOracleParams,
    #[msg("A price publish time or the reference time is negative")]
    OracleNegativeTimestamp,
    #[msg("The oracle is in anomaly mode, so the operation is blocked")]
    OracleInAnomaly,
    #[msg("A price normalization would overflow")]
    OracleArithmeticOverflow,
    // ---- Task 4.5: caps and lifecycle counts (`market::caps`) -------------
    #[msg("The Position count cap is reached")]
    PositionCountCapReached,
    #[msg("Only cap decreases apply immediately; raises need the timelock")]
    CapChangeNotDecrease,
    #[msg("A public lifecycle count or reservation would overflow or underflow")]
    LifecycleCountOutOfRange,
    #[msg("The revealed debt transition contradicts the Position or the operation")]
    DebtTransitionMismatch,
    // ---- Task 5.1: Position initialization (`state::position`) ------------
    #[msg("The Viewing public key must not be the zero key")]
    InvalidViewingKey,
    #[msg("The Position is not initializing")]
    PositionNotInitializing,
    #[msg("A Position initialization attempt is still queued")]
    PositionInitAttemptQueued,
    #[msg("Only a Position whose initialization failed may retry it")]
    PositionNotRetryable,
    #[msg("Only a Position whose initialization failed, without debt, may close")]
    PositionNotClosable,
    #[msg("The Operation Intent is not this Position's queued initialization")]
    PositionInitIntentMismatch,
    // ---- Task 5.2: Pending Shield (`state::pending_shield`) ---------------
    #[msg("The shield amount must be nonzero")]
    PendingShieldZeroAmount,
    #[msg("The Pending Shield account already holds a shield")]
    PendingShieldAlreadyExists,
    #[msg("The shield sequence is not the Position's next shield sequence")]
    PendingShieldSequenceMismatch,
    #[msg("The Position shield sequence would overflow")]
    PendingShieldSequenceOverflow,
    #[msg("The Position is not the Pending Shield's Position or owner")]
    PendingShieldPositionMismatch,
    #[msg("The Position is not active")]
    PendingShieldPositionNotActive,
    #[msg("Only the Pending Shield owner may do this")]
    PendingShieldNotOwner,
    #[msg("The Pending Shield is not deposited")]
    PendingShieldNotDeposited,
    #[msg("The Pending Shield's active credit attempt is still queued")]
    PendingShieldAttemptQueued,
    #[msg("The Pending Shield's active credit attempt was applied")]
    PendingShieldAttemptApplied,
    #[msg("The Operation Intent is not the Pending Shield's active attempt")]
    PendingShieldActiveIntentMismatch,
    #[msg("The attempt Intent does not match the Pending Shield or its Position")]
    PendingShieldAttemptBindingMismatch,
    #[msg("The new attempt reuses the active attempt's Operation Intent")]
    PendingShieldAttemptReused,
    #[msg("The Pending Shield attempt sequence would overflow")]
    PendingShieldAttemptOverflow,
    // ---- Task 5.3: shield instructions (`state::shield`) ------------------
    #[msg("The mint is not the Market mint of the shield asset")]
    ShieldMintMismatch,
    #[msg("The vault is not the Market vault of the shield asset")]
    ShieldVaultMismatch,
    #[msg("The source token account must hold the asset mint and be owned by the signer")]
    ShieldSourceAccountInvalid,
    #[msg("The refund destination must hold the asset mint and be owned by the shield owner")]
    ShieldRefundDestinationInvalid,
    #[msg("The Operation Intent is not this Position's queued shield attempt")]
    ShieldIntentMismatch,
    #[msg("The Operation Intent is not the Pending Shield's current attempt")]
    ShieldAttemptNotCurrent,
    // ---- Task 5.4: unshield (`state::unshield`) ---------------------------
    #[msg("The unshield amount must be nonzero")]
    UnshieldZeroAmount,
    #[msg("Only the Position owner may unshield")]
    UnshieldNotOwner,
    #[msg("The Position is not active")]
    UnshieldPositionNotActive,
    #[msg("The mint is not the Market mint of the unshield asset")]
    UnshieldMintMismatch,
    #[msg("The vault is not the Market vault of the unshield asset")]
    UnshieldVaultMismatch,
    #[msg("The unshield destination must hold the asset mint and be owned by the Position owner")]
    UnshieldDestinationInvalid,
    #[msg("The Operation Intent is not this Position's queued unshield")]
    UnshieldIntentMismatch,
    #[msg("The approved unshield amount is neither the request nor zero")]
    UnshieldApprovalInvalid,
    // ---- Task 5.5: Viewing Key rotation (state::rotation) ---------------
    #[msg("Only the Position owner may rotate the Viewing Key")]
    RotationNotOwner,
    #[msg("The Position is not active")]
    RotationPositionNotActive,
    #[msg("The new Viewing public key equals the current one")]
    RotationKeyUnchanged,
    #[msg("The expected Viewing Key epochs do not match the Position")]
    RotationEpochMismatch,
    #[msg("The expected old Viewing public key does not match the Position")]
    RotationOldKeyMismatch,
    #[msg("The Viewing Key epoch cannot be incremented")]
    RotationEpochOverflow,
    #[msg("The Operation Intent is not this Position's queued rotation")]
    RotationIntentMismatch,
    // ---- Task 7 (fixed-point, rate model, utilization band) ----
    #[msg("A fixed-point product or sum overflowed")]
    FixedPointOverflow,
    #[msg("A fixed-point division by zero")]
    FixedPointDivisionByZero,
    #[msg("A fixed-point operand is outside its committed range")]
    FixedPointOutOfRange,
    #[msg("A fixed-point subtraction would underflow")]
    FixedPointUnderflow,
    #[msg("The interest rate, utilization, or band input is invalid")]
    InterestRateInvalid,
    #[msg("The target time is before the accounting Borrow Index timestamp")]
    BorrowIndexTimeRegression,
    #[msg("The elapsed time exceeds the maximum accrual interval")]
    BorrowIndexElapsedTooLong,
    #[msg("The utilization band observation is older than the maximum age")]
    UtilizationBandStale,
    #[msg("The utilization band has never been observed")]
    UtilizationBandNeverObserved,
    #[msg("The utilization band observation slot is in the future")]
    UtilizationBandFromTheFuture,
    #[msg("The revealed utilization band index is invalid")]
    UtilizationBandInvalid,
    #[msg("The Pool State is not active")]
    BandPoolNotActive,
    #[msg("The Operation Intent is not a queued utilization band update")]
    BandIntentMismatch,
    // ---- Task 7.4/7.5: Pool serialization and reserve withdrawal ----
    #[msg("The Pool is locked by another operation, so the accounting Borrow Index cannot change")]
    PoolLockedForIndexMutation,
    #[msg("The Pool State is not active")]
    PoolMutationPoolNotActive,
    #[msg("The Operation Intent is not this Pool's queued mutation")]
    PoolMutationIntentMismatch,
    #[msg("The Pool output nonce repeats the stored nonce")]
    PoolOutputNonceReused,
    #[msg("The reserve withdrawal amount must be nonzero")]
    ReserveWithdrawalZeroAmount,
    #[msg("The treasury USDC account is not configured")]
    TreasuryAccountNotSet,
    #[msg(
        "The treasury USDC account must hold the USDC mint and be owned by the treasury authority"
    )]
    TreasuryAccountInvalid,
    #[msg("The reserve withdrawal mint, vault, or token program is not the Market's")]
    ReserveWithdrawalAccountMismatch,
    #[msg("The approved reserve withdrawal is neither the request nor zero")]
    ReserveWithdrawalApprovalInvalid,
    #[msg("Only the Position owner can supply or withdraw supply")]
    SupplyNotOwner,
    #[msg("The Position is not Active")]
    SupplyPositionNotActive,
    #[msg("The supply or withdrawal request must be nonzero")]
    SupplyZeroRequest,
    #[msg("The Operation Intent is not this Position's queued supply or withdrawal")]
    SupplyIntentMismatch,
    #[msg("The unsettled Recovery Claim count would overflow")]
    ClaimCountOverflow,
    #[msg("The unsettled Recovery Claim count is already zero")]
    ClaimCountUnderflow,
    #[msg("Only the Position owner may run a position operation")]
    PositionOpNotOwner,
    #[msg("The Position is not Active")]
    PositionOpPositionNotActive,
    #[msg("The Operation Intent is not this Position's queued position operation")]
    PositionOpIntentMismatch,
    // walnut-lend appends the lending, liquidation and disclosure codes.
    #[msg("This instruction does not run that lending operation")]
    LendOpWrongEntry,
    #[msg("Unknown lending operation")]
    LendOpInvalid,
    #[msg("The lending operation amount must be nonzero")]
    LendOpZeroAmount,
    #[msg("The lending operation converts to zero shares")]
    LendOpZeroShares,
    #[msg("The supply would exceed the supply cap")]
    LendSupplyCapExceeded,
    #[msg("The borrow would exceed the total debt cap")]
    LendDebtCapExceeded,
    #[msg("The amount exceeds the per-asset cap")]
    LendAssetCapExceeded,
    #[msg("The vault does not hold enough unreserved USDC")]
    LendInsufficientLiquidity,
    #[msg("The price account is not the oracle configured on the LendingPool")]
    OracleAccountMismatch,
    #[msg("The price account is not a verified Pyth PriceUpdateV2")]
    OraclePriceAccountInvalid,
    #[msg("The LendingPool configuration is invalid")]
    LendPoolConfigInvalid,
    #[msg("A LendingPool total would go below zero")]
    LendPoolTotalsUnderflow,
    #[msg("A LendingPool total would overflow")]
    LendPoolTotalsOverflow,
    #[msg("The liquidation accounts are missing or do not match the Market")]
    LiquidationAccountsInvalid,
    #[msg("The liquidation amount is zero or outside the debt and vault bounds")]
    LiquidationAmountInvalid,
    #[msg("The disclosure mask selects unknown fields")]
    DisclosureMaskInvalid,
    #[msg("The Operation Intent is not this Position's queued disclosure")]
    DisclosureIntentMismatch,
    #[msg("The disclosure output is not sealed to the committed auditor key")]
    DisclosureAuditorMismatch,
    #[msg("The private operation code is not valid for this instruction")]
    PrivateOpInvalid,
    #[msg("The accrual timestamp is in the future, too old, or before the last accrual")]
    PrivateAccrualTimeInvalid,
    #[msg("The Operation Intent is not this private operation")]
    PrivateOpIntentMismatch,
    #[msg("The liquidator cannot liquidate its own Position")]
    PrivateLiquidationSelf,
    #[msg("The encrypted input key must not be all zero")]
    PrivateInputKeyInvalid,
}
impl From<crate::math::fixed::MathError> for ErrorCode {
    fn from(error: crate::math::fixed::MathError) -> Self {
        use crate::math::fixed::MathError;
        match error {
            MathError::Overflow => ErrorCode::FixedPointOverflow,
            MathError::DivisionByZero => ErrorCode::FixedPointDivisionByZero,
            MathError::OutOfRange => ErrorCode::FixedPointOutOfRange,
            MathError::Underflow => ErrorCode::FixedPointUnderflow,
            MathError::InvalidRate => ErrorCode::InterestRateInvalid,
            MathError::TimeRegression => ErrorCode::BorrowIndexTimeRegression,
            MathError::ElapsedTooLong => ErrorCode::BorrowIndexElapsedTooLong,
        }
    }
}
impl From<crate::math::rate::BandError> for ErrorCode {
    fn from(error: crate::math::rate::BandError) -> Self {
        use crate::math::rate::BandError;
        match error {
            BandError::NeverObserved => ErrorCode::UtilizationBandNeverObserved,
            BandError::Stale => ErrorCode::UtilizationBandStale,
            BandError::FromTheFuture => ErrorCode::UtilizationBandFromTheFuture,
        }
    }
}
impl From<crate::math::fixed::MathError> for anchor_lang::error::Error {
    fn from(error: crate::math::fixed::MathError) -> Self {
        ErrorCode::from(error).into()
    }
}
impl From<crate::math::rate::BandError> for anchor_lang::error::Error {
    fn from(error: crate::math::rate::BandError) -> Self {
        ErrorCode::from(error).into()
    }
}

impl From<ShieldError> for ErrorCode {
    fn from(error: ShieldError) -> Self {
        match error {
            ShieldError::MintMismatch => ErrorCode::ShieldMintMismatch,
            ShieldError::VaultMismatch => ErrorCode::ShieldVaultMismatch,
            ShieldError::SourceAccountInvalid => ErrorCode::ShieldSourceAccountInvalid,
            ShieldError::RefundDestinationInvalid => ErrorCode::ShieldRefundDestinationInvalid,
            ShieldError::IntentMismatch => ErrorCode::ShieldIntentMismatch,
            ShieldError::AttemptNotCurrent => ErrorCode::ShieldAttemptNotCurrent,
        }
    }
}

/// Lets Anchor instructions propagate shield failures with `?`.
impl From<ShieldError> for anchor_lang::error::Error {
    fn from(error: ShieldError) -> Self {
        ErrorCode::from(error).into()
    }
}

impl From<UnshieldError> for ErrorCode {
    fn from(error: UnshieldError) -> Self {
        match error {
            UnshieldError::ZeroAmount => ErrorCode::UnshieldZeroAmount,
            UnshieldError::NotOwner => ErrorCode::UnshieldNotOwner,
            UnshieldError::PositionNotActive => ErrorCode::UnshieldPositionNotActive,
            UnshieldError::MintMismatch => ErrorCode::UnshieldMintMismatch,
            UnshieldError::VaultMismatch => ErrorCode::UnshieldVaultMismatch,
            UnshieldError::DestinationInvalid => ErrorCode::UnshieldDestinationInvalid,
            UnshieldError::IntentMismatch => ErrorCode::UnshieldIntentMismatch,
            UnshieldError::ApprovalInvalid => ErrorCode::UnshieldApprovalInvalid,
        }
    }
}

/// Lets Anchor instructions propagate unshield failures with `?`.
impl From<UnshieldError> for anchor_lang::error::Error {
    fn from(error: UnshieldError) -> Self {
        ErrorCode::from(error).into()
    }
}

impl From<RotationError> for ErrorCode {
    fn from(error: RotationError) -> Self {
        match error {
            RotationError::NotOwner => ErrorCode::RotationNotOwner,
            RotationError::PositionNotActive => ErrorCode::RotationPositionNotActive,
            RotationError::KeyUnchanged => ErrorCode::RotationKeyUnchanged,
            RotationError::EpochMismatch => ErrorCode::RotationEpochMismatch,
            RotationError::OldKeyMismatch => ErrorCode::RotationOldKeyMismatch,
            RotationError::EpochOverflow => ErrorCode::RotationEpochOverflow,
            RotationError::IntentMismatch => ErrorCode::RotationIntentMismatch,
        }
    }
}

/// Lets Anchor instructions propagate rotation failures with `?`.
impl From<RotationError> for anchor_lang::error::Error {
    fn from(error: RotationError) -> Self {
        ErrorCode::from(error).into()
    }
}

impl From<PendingShieldError> for ErrorCode {
    fn from(error: PendingShieldError) -> Self {
        match error {
            PendingShieldError::ZeroAmount => ErrorCode::PendingShieldZeroAmount,
            PendingShieldError::AlreadyExists => ErrorCode::PendingShieldAlreadyExists,
            PendingShieldError::SequenceMismatch => ErrorCode::PendingShieldSequenceMismatch,
            PendingShieldError::SequenceOverflow => ErrorCode::PendingShieldSequenceOverflow,
            PendingShieldError::PositionMismatch => ErrorCode::PendingShieldPositionMismatch,
            PendingShieldError::PositionNotActive => ErrorCode::PendingShieldPositionNotActive,
            PendingShieldError::NotOwner => ErrorCode::PendingShieldNotOwner,
            PendingShieldError::NotDeposited => ErrorCode::PendingShieldNotDeposited,
            PendingShieldError::AttemptQueued => ErrorCode::PendingShieldAttemptQueued,
            PendingShieldError::AttemptApplied => ErrorCode::PendingShieldAttemptApplied,
            PendingShieldError::ActiveIntentMismatch => {
                ErrorCode::PendingShieldActiveIntentMismatch
            }
            PendingShieldError::AttemptBindingMismatch => {
                ErrorCode::PendingShieldAttemptBindingMismatch
            }
            PendingShieldError::AttemptReused => ErrorCode::PendingShieldAttemptReused,
            PendingShieldError::AttemptOverflow => ErrorCode::PendingShieldAttemptOverflow,
        }
    }
}

/// Lets Anchor instructions propagate Pending Shield failures with `?`.
impl From<PendingShieldError> for anchor_lang::error::Error {
    fn from(error: PendingShieldError) -> Self {
        ErrorCode::from(error).into()
    }
}

impl From<PositionError> for ErrorCode {
    fn from(error: PositionError) -> Self {
        match error {
            PositionError::InvalidViewingKey => ErrorCode::InvalidViewingKey,
            PositionError::NotInitializing => ErrorCode::PositionNotInitializing,
            PositionError::AttemptQueued => ErrorCode::PositionInitAttemptQueued,
            PositionError::NotRetryable => ErrorCode::PositionNotRetryable,
            PositionError::NotClosable => ErrorCode::PositionNotClosable,
            PositionError::IntentMismatch => ErrorCode::PositionInitIntentMismatch,
        }
    }
}

/// Lets Anchor instructions propagate Position lifecycle failures with `?`.
impl From<PositionError> for anchor_lang::error::Error {
    fn from(error: PositionError) -> Self {
        ErrorCode::from(error).into()
    }
}

impl From<MarketError> for ErrorCode {
    fn from(error: MarketError) -> Self {
        match error {
            MarketError::InitializerNotUpgradeAuthority => {
                ErrorCode::InitializerNotUpgradeAuthority
            }
            MarketError::InvalidProgramDataAccount => ErrorCode::InvalidProgramDataAccount,
            MarketError::MarketAlreadyInitialized => ErrorCode::MarketAlreadyInitialized,
            MarketError::AssetStateAlreadyInitialized => ErrorCode::AssetStateAlreadyInitialized,
            MarketError::PoolStateAlreadyInitialized => ErrorCode::PoolStateAlreadyInitialized,
            MarketError::RecoveryPositionAlreadyInitialized => {
                ErrorCode::RecoveryPositionAlreadyInitialized
            }
            MarketError::VaultAuthorityAlreadyInitialized => {
                ErrorCode::VaultAuthorityAlreadyInitialized
            }
            MarketError::MintNotOwnedByTokenProgram => ErrorCode::MintNotOwnedByTokenProgram,
            MarketError::MintNotInitialized => ErrorCode::MintNotInitialized,
            MarketError::WsolMintMismatch => ErrorCode::WsolMintMismatch,
            MarketError::UsdcMintIsNative => ErrorCode::UsdcMintIsNative,
            MarketError::InvalidUsdcDecimals => ErrorCode::InvalidUsdcDecimals,
            MarketError::InvalidAuthority => ErrorCode::InvalidAuthority,
            MarketError::PoolStateNotInitializing => ErrorCode::PoolStateNotInitializing,
            MarketError::NotMarketAdmin => ErrorCode::NotMarketAdmin,
            MarketError::NotPendingAdmin => ErrorCode::NotPendingAdmin,
            MarketError::NoPendingAdmin => ErrorCode::NoPendingAdmin,
            MarketError::NotUpgradeAuthority => ErrorCode::NotUpgradeAuthority,
            MarketError::NotRiskAuthority => ErrorCode::NotRiskAuthority,
            MarketError::NotTreasuryAuthority => ErrorCode::NotTreasuryAuthority,
            MarketError::NotGuardianAuthority => ErrorCode::NotGuardianAuthority,
            MarketError::InvalidRoleKey => ErrorCode::InvalidRoleKey,
            MarketError::RoleKeyUnchanged => ErrorCode::RoleKeyUnchanged,
            MarketError::RoleKeyNotDistinct => ErrorCode::RoleKeyNotDistinct,
            MarketError::UpgradeAuthorityMismatch => ErrorCode::UpgradeAuthorityMismatch,
            MarketError::MarketNotActive => ErrorCode::MarketNotActive,
            MarketError::ActionPaused => ErrorCode::ActionPaused,
            MarketError::InvalidPauseFlags => ErrorCode::InvalidPauseFlags,
            MarketError::PauseNotAllowed => ErrorCode::PauseNotAllowed,
            MarketError::InvalidPauseDuration => ErrorCode::InvalidPauseDuration,
            MarketError::GuardianPauseCooldown => ErrorCode::GuardianPauseCooldown,
            MarketError::NoActiveGuardianPause => ErrorCode::NoActiveGuardianPause,
            MarketError::InvalidPauseExpiry => ErrorCode::InvalidPauseExpiry,
            MarketError::ConfigProposalIdMismatch => ErrorCode::ConfigProposalIdMismatch,
            MarketError::ConfigUnchanged => ErrorCode::ConfigUnchanged,
            MarketError::ConfigOutOfBounds => ErrorCode::ConfigOutOfBounds,
            MarketError::ConfigMovementTooLarge => ErrorCode::ConfigMovementTooLarge,
            MarketError::LiquidationCrossBound => ErrorCode::LiquidationCrossBound,
            MarketError::TimelockDelayOutOfBounds => ErrorCode::TimelockDelayOutOfBounds,
            MarketError::TimelockNotElapsed => ErrorCode::TimelockNotElapsed,
            MarketError::ConfigProposalExpired => ErrorCode::ConfigProposalExpired,
            MarketError::ConfigProposalNotPending => ErrorCode::ConfigProposalNotPending,
            MarketError::ConfigProposalBaseChanged => ErrorCode::ConfigProposalBaseChanged,
            MarketError::NotConfigCanceller => ErrorCode::NotConfigCanceller,
            MarketError::ConfigArithmeticOverflow => ErrorCode::ConfigArithmeticOverflow,
            MarketError::PositionCountCapReached => ErrorCode::PositionCountCapReached,
            MarketError::CapChangeNotDecrease => ErrorCode::CapChangeNotDecrease,
            MarketError::LifecycleCountOutOfRange => ErrorCode::LifecycleCountOutOfRange,
            MarketError::DebtTransitionMismatch => ErrorCode::DebtTransitionMismatch,
        }
    }
}

/// Lets Anchor instructions propagate market validation failures with `?`.
impl From<MarketError> for anchor_lang::error::Error {
    fn from(error: MarketError) -> Self {
        ErrorCode::from(error).into()
    }
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

impl From<CallbackError> for ErrorCode {
    fn from(error: CallbackError) -> Self {
        match error {
            CallbackError::OffsetMismatch => ErrorCode::CallbackOffsetMismatch,
            CallbackError::IntentKeyMismatch => ErrorCode::CallbackIntentMismatch,
            CallbackError::IntentKindMismatch => ErrorCode::CallbackKindMismatch,
            CallbackError::AccountsCommitmentMismatch => {
                ErrorCode::CallbackAccountsCommitmentMismatch
            }
            CallbackError::InputsCommitmentMismatch => ErrorCode::CallbackInputsCommitmentMismatch,
            CallbackError::Intent(error) => ErrorCode::from(error),
        }
    }
}

/// Lets Anchor instructions propagate callback verification failures with `?`.
impl From<CallbackError> for anchor_lang::error::Error {
    fn from(error: CallbackError) -> Self {
        ErrorCode::from(error).into()
    }
}

impl From<OracleError> for ErrorCode {
    fn from(error: OracleError) -> Self {
        match error {
            OracleError::WrongReceiverProgram => ErrorCode::OracleWrongReceiverProgram,
            OracleError::FeedMismatch => ErrorCode::OracleFeedMismatch,
            OracleError::StalePrice => ErrorCode::OracleStalePrice,
            OracleError::NonPositivePrice => ErrorCode::OracleNonPositivePrice,
            OracleError::ConfidenceTooWide => ErrorCode::OracleConfidenceTooWide,
            OracleError::ExponentOutOfRange => ErrorCode::OracleExponentOutOfRange,
            OracleError::PriceOutOfRange => ErrorCode::OraclePriceOutOfRange,
            OracleError::DeviationTooHigh => ErrorCode::OracleDeviationTooHigh,
            OracleError::InvalidOracleParams => ErrorCode::InvalidOracleParams,
            OracleError::NegativeTimestamp => ErrorCode::OracleNegativeTimestamp,
            OracleError::OracleInAnomaly => ErrorCode::OracleInAnomaly,
            OracleError::ArithmeticOverflow => ErrorCode::OracleArithmeticOverflow,
        }
    }
}

/// Lets Anchor instructions propagate oracle validation failures with `?`.
impl From<OracleError> for anchor_lang::error::Error {
    fn from(error: OracleError) -> Self {
        ErrorCode::from(error).into()
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
        // Task 3.3 appends the callback verification codes.
        assert_eq!(u32::from(ErrorCode::CallbackOutputNotAuthenticated), 6020);
        assert_eq!(u32::from(ErrorCode::CallbackOffsetMismatch), 6021);
        assert_eq!(u32::from(ErrorCode::CallbackIntentMismatch), 6022);
        assert_eq!(u32::from(ErrorCode::CallbackKindMismatch), 6023);
        assert_eq!(
            u32::from(ErrorCode::CallbackAccountsCommitmentMismatch),
            6024
        );
        assert_eq!(u32::from(ErrorCode::CallbackInputsCommitmentMismatch), 6025);
        // Task 4.1 appends the market initialization codes.
        assert_eq!(u32::from(ErrorCode::InitializerNotUpgradeAuthority), 6026);
        assert_eq!(u32::from(ErrorCode::InvalidProgramDataAccount), 6027);
        assert_eq!(u32::from(ErrorCode::MarketAlreadyInitialized), 6028);
        assert_eq!(u32::from(ErrorCode::AssetStateAlreadyInitialized), 6029);
        assert_eq!(u32::from(ErrorCode::PoolStateAlreadyInitialized), 6030);
        assert_eq!(
            u32::from(ErrorCode::RecoveryPositionAlreadyInitialized),
            6031
        );
        assert_eq!(u32::from(ErrorCode::VaultAuthorityAlreadyInitialized), 6032);
        assert_eq!(u32::from(ErrorCode::MintNotOwnedByTokenProgram), 6033);
        assert_eq!(u32::from(ErrorCode::MintNotInitialized), 6034);
        assert_eq!(u32::from(ErrorCode::WsolMintMismatch), 6035);
        assert_eq!(u32::from(ErrorCode::UsdcMintIsNative), 6036);
        assert_eq!(u32::from(ErrorCode::InvalidUsdcDecimals), 6037);
        assert_eq!(u32::from(ErrorCode::InvalidAuthority), 6038);
        assert_eq!(u32::from(ErrorCode::PoolStateNotInitializing), 6039);
        assert_eq!(u32::from(ErrorCode::NotMarketAdmin), 6040);
        // Task 4.2 appends the role codes.
        assert_eq!(u32::from(ErrorCode::NotPendingAdmin), 6041);
        assert_eq!(u32::from(ErrorCode::NoPendingAdmin), 6042);
        assert_eq!(u32::from(ErrorCode::NotUpgradeAuthority), 6043);
        assert_eq!(u32::from(ErrorCode::NotRiskAuthority), 6044);
        assert_eq!(u32::from(ErrorCode::NotTreasuryAuthority), 6045);
        assert_eq!(u32::from(ErrorCode::NotGuardianAuthority), 6046);
        assert_eq!(u32::from(ErrorCode::InvalidRoleKey), 6047);
        assert_eq!(u32::from(ErrorCode::RoleKeyUnchanged), 6048);
        assert_eq!(u32::from(ErrorCode::RoleKeyNotDistinct), 6049);
        assert_eq!(u32::from(ErrorCode::UpgradeAuthorityMismatch), 6050);
        assert_eq!(u32::from(ErrorCode::MarketNotActive), 6051);
        assert_eq!(u32::from(ErrorCode::ActionPaused), 6052);
        assert_eq!(u32::from(ErrorCode::InvalidPauseFlags), 6053);
        assert_eq!(u32::from(ErrorCode::PauseNotAllowed), 6054);
        assert_eq!(u32::from(ErrorCode::InvalidPauseDuration), 6055);
        assert_eq!(u32::from(ErrorCode::GuardianPauseCooldown), 6056);
        assert_eq!(u32::from(ErrorCode::NoActiveGuardianPause), 6057);
        assert_eq!(u32::from(ErrorCode::InvalidPauseExpiry), 6058);
        // Task 4.4 appends the timelocked configuration codes.
        assert_eq!(u32::from(ErrorCode::ConfigProposalIdMismatch), 6059);
        assert_eq!(u32::from(ErrorCode::ConfigUnchanged), 6060);
        assert_eq!(u32::from(ErrorCode::ConfigOutOfBounds), 6061);
        assert_eq!(u32::from(ErrorCode::ConfigMovementTooLarge), 6062);
        assert_eq!(u32::from(ErrorCode::LiquidationCrossBound), 6063);
        assert_eq!(u32::from(ErrorCode::TimelockDelayOutOfBounds), 6064);
        assert_eq!(u32::from(ErrorCode::TimelockNotElapsed), 6065);
        assert_eq!(u32::from(ErrorCode::ConfigProposalExpired), 6066);
        assert_eq!(u32::from(ErrorCode::ConfigProposalNotPending), 6067);
        assert_eq!(u32::from(ErrorCode::ConfigProposalBaseChanged), 6068);
        assert_eq!(u32::from(ErrorCode::NotConfigCanceller), 6069);
        assert_eq!(u32::from(ErrorCode::ConfigArithmeticOverflow), 6070);
        // Task 6 appends the Pyth oracle codes.
        assert_eq!(u32::from(ErrorCode::OracleWrongReceiverProgram), 6071);
        assert_eq!(u32::from(ErrorCode::OracleFeedMismatch), 6072);
        assert_eq!(u32::from(ErrorCode::OracleStalePrice), 6073);
        assert_eq!(u32::from(ErrorCode::OracleNonPositivePrice), 6074);
        assert_eq!(u32::from(ErrorCode::OracleConfidenceTooWide), 6075);
        assert_eq!(u32::from(ErrorCode::OracleExponentOutOfRange), 6076);
        assert_eq!(u32::from(ErrorCode::OraclePriceOutOfRange), 6077);
        assert_eq!(u32::from(ErrorCode::OracleDeviationTooHigh), 6078);
        assert_eq!(u32::from(ErrorCode::InvalidOracleParams), 6079);
        assert_eq!(u32::from(ErrorCode::OracleNegativeTimestamp), 6080);
        assert_eq!(u32::from(ErrorCode::OracleInAnomaly), 6081);
        assert_eq!(u32::from(ErrorCode::OracleArithmeticOverflow), 6082);
        // Task 4.5 appends the cap and lifecycle-count codes.
        assert_eq!(u32::from(ErrorCode::PositionCountCapReached), 6083);
        assert_eq!(u32::from(ErrorCode::CapChangeNotDecrease), 6084);
        assert_eq!(u32::from(ErrorCode::LifecycleCountOutOfRange), 6085);
        assert_eq!(u32::from(ErrorCode::DebtTransitionMismatch), 6086);
        // Task 5.1 appends the Position initialization codes.
        assert_eq!(u32::from(ErrorCode::InvalidViewingKey), 6087);
        assert_eq!(u32::from(ErrorCode::PositionNotInitializing), 6088);
        assert_eq!(u32::from(ErrorCode::PositionInitAttemptQueued), 6089);
        assert_eq!(u32::from(ErrorCode::PositionNotRetryable), 6090);
        assert_eq!(u32::from(ErrorCode::PositionNotClosable), 6091);
        assert_eq!(u32::from(ErrorCode::PositionInitIntentMismatch), 6092);
        // Task 5.2 appends the Pending Shield codes.
        assert_eq!(u32::from(ErrorCode::PendingShieldZeroAmount), 6093);
        assert_eq!(u32::from(ErrorCode::PendingShieldAttemptOverflow), 6106);
        // Task 5.3 appends the shield instruction codes.
        assert_eq!(u32::from(ErrorCode::ShieldMintMismatch), 6107);
        assert_eq!(u32::from(ErrorCode::ShieldAttemptNotCurrent), 6112);
        // Task 5.4 appends the unshield codes.
        assert_eq!(u32::from(ErrorCode::UnshieldZeroAmount), 6113);
        assert_eq!(u32::from(ErrorCode::UnshieldApprovalInvalid), 6120);
        // Task 5.5 appends the rotation codes.
        assert_eq!(u32::from(ErrorCode::RotationNotOwner), 6121);
        assert_eq!(u32::from(ErrorCode::RotationIntentMismatch), 6127);
        // Task 7 appends the fixed-point, rate, and band codes.
        assert_eq!(u32::from(ErrorCode::FixedPointOverflow), 6128);
        assert_eq!(u32::from(ErrorCode::BandIntentMismatch), 6140);
        // Task 7.5 appends the reserve withdrawal codes (6145 to 6149).
        assert_eq!(u32::from(ErrorCode::ReserveWithdrawalApprovalInvalid), 6149);
        // Task 8 appends the supply codes and the claim-count codes.
        assert_eq!(u32::from(ErrorCode::SupplyNotOwner), 6150);
        assert_eq!(u32::from(ErrorCode::SupplyIntentMismatch), 6153);
        assert_eq!(u32::from(ErrorCode::ClaimCountOverflow), 6154);
        assert_eq!(u32::from(ErrorCode::ClaimCountUnderflow), 6155);
        // The walnut-diffn demo appends the position operation codes.
        assert_eq!(u32::from(ErrorCode::PositionOpNotOwner), 6156);
        assert_eq!(u32::from(ErrorCode::PositionOpIntentMismatch), 6158);
        // walnut-lend appends the lending, liquidation and disclosure codes.
        assert_eq!(u32::from(ErrorCode::LendOpWrongEntry), 6159);
        assert_eq!(u32::from(ErrorCode::OracleAccountMismatch), 6167);
        assert_eq!(u32::from(ErrorCode::LendPoolConfigInvalid), 6169);
        assert_eq!(u32::from(ErrorCode::DisclosureAuditorMismatch), 6176);
        // walnut-private (ADR-0004) appends the private-operation codes.
        assert_eq!(u32::from(ErrorCode::PrivateOpInvalid), 6177);
        assert_eq!(u32::from(ErrorCode::PrivateInputKeyInvalid), 6181);
    }

    #[test]
    fn shield_errors_map_to_distinct_codes() {
        let all = [
            ShieldError::MintMismatch,
            ShieldError::VaultMismatch,
            ShieldError::SourceAccountInvalid,
            ShieldError::RefundDestinationInvalid,
            ShieldError::IntentMismatch,
            ShieldError::AttemptNotCurrent,
        ];
        let codes: Vec<u32> = all
            .iter()
            .map(|error| u32::from(ErrorCode::from(*error)))
            .collect();
        let expected: Vec<u32> = (6107..=6112).collect();
        assert_eq!(codes, expected);
    }

    #[test]
    fn rotation_errors_map_to_distinct_codes() {
        let all = [
            RotationError::NotOwner,
            RotationError::PositionNotActive,
            RotationError::KeyUnchanged,
            RotationError::EpochMismatch,
            RotationError::OldKeyMismatch,
            RotationError::EpochOverflow,
            RotationError::IntentMismatch,
        ];
        let codes: Vec<u32> = all
            .iter()
            .map(|error| u32::from(ErrorCode::from(*error)))
            .collect();
        let expected: Vec<u32> = (6121..=6127).collect();
        assert_eq!(codes, expected);
    }

    #[test]
    fn unshield_errors_map_to_distinct_codes() {
        let all = [
            UnshieldError::ZeroAmount,
            UnshieldError::NotOwner,
            UnshieldError::PositionNotActive,
            UnshieldError::MintMismatch,
            UnshieldError::VaultMismatch,
            UnshieldError::DestinationInvalid,
            UnshieldError::IntentMismatch,
            UnshieldError::ApprovalInvalid,
        ];
        let codes: Vec<u32> = all
            .iter()
            .map(|error| u32::from(ErrorCode::from(*error)))
            .collect();
        for (index, code) in codes.iter().enumerate() {
            assert!(!codes[index + 1..].contains(code));
            assert!((6113..=6120).contains(code));
        }
    }

    #[test]
    fn pending_shield_errors_map_to_distinct_codes() {
        let all = [
            PendingShieldError::ZeroAmount,
            PendingShieldError::AlreadyExists,
            PendingShieldError::SequenceMismatch,
            PendingShieldError::SequenceOverflow,
            PendingShieldError::PositionMismatch,
            PendingShieldError::PositionNotActive,
            PendingShieldError::NotOwner,
            PendingShieldError::NotDeposited,
            PendingShieldError::AttemptQueued,
            PendingShieldError::AttemptApplied,
            PendingShieldError::ActiveIntentMismatch,
            PendingShieldError::AttemptBindingMismatch,
            PendingShieldError::AttemptReused,
            PendingShieldError::AttemptOverflow,
        ];
        let codes: Vec<u32> = all
            .iter()
            .map(|error| u32::from(ErrorCode::from(*error)))
            .collect();
        // Declaration order equals code order.
        let expected: Vec<u32> = (6093..=6106).collect();
        assert_eq!(codes, expected);
    }

    #[test]
    fn position_errors_map_to_distinct_codes() {
        let all = [
            PositionError::InvalidViewingKey,
            PositionError::NotInitializing,
            PositionError::AttemptQueued,
            PositionError::NotRetryable,
            PositionError::NotClosable,
            PositionError::IntentMismatch,
        ];
        let codes: Vec<u32> = all
            .iter()
            .map(|error| u32::from(ErrorCode::from(*error)))
            .collect();
        for (index, code) in codes.iter().enumerate() {
            assert!(!codes[index + 1..].contains(code));
            assert!((6087..=6092).contains(code));
        }
    }

    #[test]
    fn oracle_errors_map_to_distinct_codes() {
        let all = [
            OracleError::WrongReceiverProgram,
            OracleError::FeedMismatch,
            OracleError::StalePrice,
            OracleError::NonPositivePrice,
            OracleError::ConfidenceTooWide,
            OracleError::ExponentOutOfRange,
            OracleError::PriceOutOfRange,
            OracleError::DeviationTooHigh,
            OracleError::InvalidOracleParams,
            OracleError::NegativeTimestamp,
            OracleError::OracleInAnomaly,
            OracleError::ArithmeticOverflow,
        ];
        let codes: Vec<u32> = all
            .iter()
            .map(|error| u32::from(ErrorCode::from(*error)))
            .collect();
        for (index, code) in codes.iter().enumerate() {
            assert!(!codes[index + 1..].contains(code));
            assert!((6071..=6082).contains(code));
        }
    }

    #[test]
    fn market_errors_map_to_distinct_codes() {
        let all = [
            MarketError::InitializerNotUpgradeAuthority,
            MarketError::InvalidProgramDataAccount,
            MarketError::MarketAlreadyInitialized,
            MarketError::AssetStateAlreadyInitialized,
            MarketError::PoolStateAlreadyInitialized,
            MarketError::RecoveryPositionAlreadyInitialized,
            MarketError::VaultAuthorityAlreadyInitialized,
            MarketError::MintNotOwnedByTokenProgram,
            MarketError::MintNotInitialized,
            MarketError::WsolMintMismatch,
            MarketError::UsdcMintIsNative,
            MarketError::InvalidUsdcDecimals,
            MarketError::InvalidAuthority,
            MarketError::PoolStateNotInitializing,
            MarketError::NotMarketAdmin,
            MarketError::NotPendingAdmin,
            MarketError::NoPendingAdmin,
            MarketError::NotUpgradeAuthority,
            MarketError::NotRiskAuthority,
            MarketError::NotTreasuryAuthority,
            MarketError::NotGuardianAuthority,
            MarketError::InvalidRoleKey,
            MarketError::RoleKeyUnchanged,
            MarketError::RoleKeyNotDistinct,
            MarketError::UpgradeAuthorityMismatch,
            MarketError::MarketNotActive,
            MarketError::ActionPaused,
            MarketError::InvalidPauseFlags,
            MarketError::PauseNotAllowed,
            MarketError::InvalidPauseDuration,
            MarketError::GuardianPauseCooldown,
            MarketError::NoActiveGuardianPause,
            MarketError::InvalidPauseExpiry,
            MarketError::ConfigProposalIdMismatch,
            MarketError::ConfigUnchanged,
            MarketError::ConfigOutOfBounds,
            MarketError::ConfigMovementTooLarge,
            MarketError::LiquidationCrossBound,
            MarketError::TimelockDelayOutOfBounds,
            MarketError::TimelockNotElapsed,
            MarketError::ConfigProposalExpired,
            MarketError::ConfigProposalNotPending,
            MarketError::ConfigProposalBaseChanged,
            MarketError::NotConfigCanceller,
            MarketError::ConfigArithmeticOverflow,
            MarketError::PositionCountCapReached,
            MarketError::CapChangeNotDecrease,
            MarketError::LifecycleCountOutOfRange,
            MarketError::DebtTransitionMismatch,
        ];
        let codes: Vec<u32> = all
            .iter()
            .map(|error| u32::from(ErrorCode::from(*error)))
            .collect();
        for (index, code) in codes.iter().enumerate() {
            assert!(!codes[index + 1..].contains(code));
            // Tasks 4.1-4.4, then Task 4.5 after the Task 6 oracle codes.
            assert!((6026..=6070).contains(code) || (6083..=6086).contains(code));
        }
    }

    #[test]
    fn callback_errors_map_to_distinct_codes() {
        let all = [
            CallbackError::OffsetMismatch,
            CallbackError::IntentKeyMismatch,
            CallbackError::IntentKindMismatch,
            CallbackError::AccountsCommitmentMismatch,
            CallbackError::InputsCommitmentMismatch,
            CallbackError::Intent(IntentError::DecisionExpired),
        ];
        let codes: Vec<u32> = all
            .iter()
            .map(|error| u32::from(ErrorCode::from(*error)))
            .collect();
        for (index, code) in codes.iter().enumerate() {
            assert!(!codes[index + 1..].contains(code));
        }
        // Intent errors keep their existing codes.
        assert_eq!(
            codes[5],
            u32::from(ErrorCode::DecisionExpired),
            "wrapped Intent errors reuse the Task 3.2 code"
        );
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
