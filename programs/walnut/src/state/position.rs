//! Production Position initialization lifecycle (Task 5.1; Requirements 3.1-3.6,
//! 4.10, 4.11, 5.1-5.3, 5.10).
//!
//! Pure Rust rules behind `initialize_position`, `retry_position_init`, the
//! `init_position_state` callback, `abandon_position_init`, and
//! `close_position`. The Anchor handlers in `lib.rs` and
//! `instructions/position.rs` only load accounts and call these functions, so
//! every transition is unit- and property-tested on the host.
//!
//! # Address
//!
//! ```text
//! Position = PDA[ "position" | market | owner ]
//! ```
//!
//! One Position per owner per Market. The spike `SpikePosition`
//! (`"spike_position" | owner`) and the Recovery Position
//! (`"recovery_position" | market`) use other seeds.
//!
//! # State machine
//!
//! | From           | Event                                   | To             | Count  |
//! |----------------|-----------------------------------------|----------------|--------|
//! | (no account)   | `initialize_position` (owner)           | `Initializing` | +1     |
//! | `Initializing` | callback `Applied`                      | `Active`       |        |
//! | `Initializing` | callback `Stale`/`Expired`/`Failed`/`Zero` | `InitFailed` |    |
//! | `Initializing` | `abandon_position_init` after Lock expiry | `InitFailed` |     |
//! | `InitFailed`   | `retry_position_init` (owner)           | `Initializing` |        |
//! | `InitFailed`   | `close_position` (owner)                | (closed)       | -1     |
//!
//! A callback for a terminal Intent changes nothing. An unauthenticated output
//! is rejected, so the Intent stays `Queued` and the Lock stays held until
//! abandonment. Retry and close require the Lock to be free, so neither can run
//! while an attempt is queued. `Active` Positions cannot close yet: proving an
//! all-zero encrypted state needs a computation (deferred).
//!
//! The count changes when the PDA is created and when it is closed, so
//! `position_count` always equals the number of open user Positions.
//!
//! # Intent binding
//!
//! The Intent captures the Position generation. `accounts_commitment` covers,
//! in order: owner (signer and payer), Market, Position. `inputs_commitment`
//! covers the Viewing public key, the Viewing Key epoch, and the input nonce.
//! The callback rebuilds the inputs from the output: the output encryption key
//! must equal the registered key, and the input nonce is the output nonce - 1
//! (ADR-0003). Any mismatch rejects the callback.

use anchor_lang::prelude::Pubkey;

use super::accounts::{EncryptedPositionPayload, Position, PositionStatus};
use super::coordination::{acquire_locks, CoordinationSlot};
use super::Slot;
use crate::errors::ErrorCode;
use crate::market::caps::CountChange;
use crate::market::{on_position_closed, on_position_initialized, Market, MarketStatus};
use crate::operation::callback::{
    apply_decision_to_slots, classify_callback, CallbackContext, CallbackDecision, CallbackReport,
    CurrentDependencies,
};
use crate::operation::commitment::{
    account_commitment, input_commitment, AccountBinding, AccountRole, InputField, InputValue,
};
use crate::operation::seeds::reserve_intent;
use crate::operation::{IntentStatus, OperationCounter, OperationIntent, OperationKind};

/// Seed of a production Position PDA (followed by the market and the owner).
pub const POSITION_ACCOUNT_SEED: &[u8] = b"position";

/// Decision Expiry distance of a Position initialization, in slots. The
/// computation moves no value, so it uses a fixed window rather than the
/// Asset State risk parameter.
pub const POSITION_INIT_DECISION_SLOTS: u64 = 1_500;

/// Slots between Decision Expiry and Lock expiry. After Lock expiry anyone may
/// abandon the attempt.
pub const POSITION_INIT_LOCK_GRACE_SLOTS: u64 = 300;

/// Input field id: the Viewing public key (`Bytes32`). Append-only.
pub const INPUT_VIEWING_PUBKEY: u8 = 1;
/// Input field id: the Viewing Key epoch (`U64`). Append-only.
pub const INPUT_VIEWING_KEY_EPOCH: u8 = 2;
/// Input field id: the `Shared` input nonce (`U128`). Append-only.
pub const INPUT_NONCE: u8 = 3;

/// Position lifecycle failures. Mapped onto program errors in `errors.rs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionError {
    /// The Viewing public key is all zero.
    InvalidViewingKey,
    /// The Position is not `Initializing`.
    NotInitializing,
    /// An initialization attempt is still queued (the Lock is held).
    AttemptQueued,
    /// Only an `InitFailed` Position may retry initialization.
    NotRetryable,
    /// Only an `InitFailed` Position without debt may close.
    NotClosable,
    /// The Operation Intent is not this Position's queued initialization.
    IntentMismatch,
}

/// Derives the Position address and bump for `market` and `owner`.
pub fn derive_position_address(
    market: &Pubkey,
    owner: &Pubkey,
    program_id: &Pubkey,
) -> (Pubkey, u8) {
    let (address, bump) = anchor_lang::derive_program_address(
        &[POSITION_ACCOUNT_SEED, market.as_ref(), owner.as_ref()],
        &program_id.to_bytes(),
    );
    (Pubkey::new_from_array(address), bump)
}

/// Rejects an all-zero Viewing public key.
pub fn check_viewing_key(viewing_pubkey: &[u8; 32]) -> Result<(), PositionError> {
    if viewing_pubkey.iter().all(|byte| *byte == 0) {
        return Err(PositionError::InvalidViewingKey);
    }
    Ok(())
}

impl Position {
    /// A new user Position in `Initializing` with the Viewing Key registered
    /// at epoch zero and an all-zero payload.
    pub fn initializing(
        owner: Pubkey,
        market: Pubkey,
        bump: u8,
        viewing_pubkey: [u8; 32],
    ) -> Result<Self, PositionError> {
        check_viewing_key(&viewing_pubkey)?;
        let mut position = Self::blank(owner, market, bump);
        position.status = PositionStatus::Initializing;
        position.viewing_pubkey = viewing_pubkey;
        Ok(position)
    }

    /// Retry is allowed only from `InitFailed` with the Lock free.
    pub fn check_retry(&self) -> Result<(), PositionError> {
        if self.coordination.lock.is_active() {
            return Err(PositionError::AttemptQueued);
        }
        if self.status != PositionStatus::InitFailed {
            return Err(PositionError::NotRetryable);
        }
        Ok(())
    }

    /// Close is allowed only from `InitFailed` with the Lock free and no debt.
    pub fn check_close(&self) -> Result<(), PositionError> {
        if self.coordination.lock.is_active() {
            return Err(PositionError::AttemptQueued);
        }
        if self.status != PositionStatus::InitFailed || self.has_debt {
            return Err(PositionError::NotClosable);
        }
        Ok(())
    }
}

/// The ordered accounts bound by a Position initialization Intent.
pub fn position_init_bindings(
    owner: Pubkey,
    market: Pubkey,
    position: Pubkey,
    position_generation: Option<u64>,
) -> [AccountBinding; 3] {
    let program = crate::ID;
    [
        AccountBinding {
            key: owner,
            role: AccountRole::Owner,
            expected_owner: anchor_lang::system_program::ID,
            mint: None,
            authority: None,
            writable: true,
            generation: None,
        },
        AccountBinding {
            key: market,
            role: AccountRole::Market,
            expected_owner: program,
            mint: None,
            authority: None,
            // The computation never writes the Market; `position_count`
            // changes only at create and close.
            writable: false,
            generation: None,
        },
        AccountBinding {
            key: position,
            role: AccountRole::Position,
            expected_owner: program,
            mint: None,
            authority: Some(owner),
            writable: true,
            generation: position_generation,
        },
    ]
}

/// The ordered inputs bound by a Position initialization Intent.
pub fn position_init_inputs(
    viewing_pubkey: [u8; 32],
    viewing_key_epoch: u64,
    nonce: u128,
) -> [InputField<'static>; 3] {
    [
        InputField::new(INPUT_VIEWING_PUBKEY, InputValue::Bytes32(viewing_pubkey)),
        InputField::new(INPUT_VIEWING_KEY_EPOCH, InputValue::U64(viewing_key_epoch)),
        InputField::new(INPUT_NONCE, InputValue::U128(nonce)),
    ]
}

/// One queue request (initialize or retry).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PositionInitRequest {
    /// The Operation Intent PDA address.
    pub intent_key: Pubkey,
    /// The Position PDA address.
    pub position_key: Pubkey,
    /// The owner's next Intent sequence.
    pub sequence: u64,
    /// The Arcium computation offset.
    pub computation_offset: u64,
    /// The `Shared` input nonce chosen by the client.
    pub nonce: u128,
    /// The current slot.
    pub queued_slot: Slot,
}

/// Builds the Intent for `request` against `position` (owner = payer).
pub fn build_position_init_intent(
    position: &Position,
    request: &PositionInitRequest,
) -> Result<OperationIntent, ErrorCode> {
    let decision_expiry_slot = request
        .queued_slot
        .checked_add(POSITION_INIT_DECISION_SLOTS)
        .ok_or(ErrorCode::InvalidIntentTiming)?;
    let lock_expiry_slot = decision_expiry_slot
        .checked_add(POSITION_INIT_LOCK_GRACE_SLOTS)
        .ok_or(ErrorCode::InvalidIntentTiming)?;
    let generation = Some(position.coordination.generation);
    let bindings = position_init_bindings(
        position.owner,
        position.market,
        request.position_key,
        generation,
    );
    let inputs = position_init_inputs(
        position.viewing_pubkey,
        position.viewing_key_epoch,
        request.nonce,
    );
    Ok(OperationIntent {
        key: request.intent_key,
        owner: position.owner,
        payer: position.owner,
        reclaim_authority: position.owner,
        kind: OperationKind::PositionInit,
        status: IntentStatus::Queued,
        sequence: request.sequence,
        computation_offset: request.computation_offset,
        accounts_commitment: account_commitment(&bindings),
        inputs_commitment: input_commitment(OperationKind::PositionInit, &inputs),
        queued_slot: request.queued_slot,
        decision_expiry_slot,
        lock_expiry_slot,
        position: Some(request.position_key),
        position_generation: generation,
        ..OperationIntent::default()
    })
}

/// Shared queue path: builds and reserves the Intent and acquires the
/// Position Lock. The Position must be `Initializing`. Atomic: on error the
/// Position and the counter are unchanged.
fn queue_attempt(
    position: &mut Position,
    counter: &mut OperationCounter,
    request: &PositionInitRequest,
) -> Result<OperationIntent, ErrorCode> {
    if position.status != PositionStatus::Initializing {
        return Err(PositionError::NotInitializing.into());
    }
    let intent = build_position_init_intent(position, request)?;
    let mut next_counter = *counter;
    reserve_intent(&mut next_counter, &intent)?;
    acquire_locks(
        intent.lock_owner(),
        intent.lock_expiry_slot,
        &mut [CoordinationSlot::new(
            request.position_key,
            &mut position.coordination,
        )],
    )?;
    *counter = next_counter;
    Ok(intent)
}

/// Result of [`initialize_position`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitializedPosition {
    /// The new Position, `Initializing` and Locked by the Intent.
    pub intent: OperationIntent,
    /// The `position_count` change.
    pub count: CountChange,
}

/// The identity of a Position being created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NewPosition {
    /// The owner (signer and payer).
    pub owner: Pubkey,
    /// The Market PDA.
    pub market: Pubkey,
    /// The Position PDA bump.
    pub bump: u8,
    /// The owner's Viewing public key (epoch zero).
    pub viewing_pubkey: [u8; 32],
}

/// `initialize_position`: validates the Viewing Key, enforces the Position
/// count cap (incrementing `position_count`), writes `position` as a fresh
/// `Initializing` Position, and queues the first attempt. Atomic: on error
/// the Market, the Position, and the counter are unchanged.
pub fn initialize_position(
    market: &mut Market,
    position: &mut Position,
    counter: &mut OperationCounter,
    new: &NewPosition,
    request: &PositionInitRequest,
) -> Result<InitializedPosition, ErrorCode> {
    if market.status != MarketStatus::Active {
        return Err(ErrorCode::MarketNotActive);
    }
    let mut fresh = Position::initializing(new.owner, new.market, new.bump, new.viewing_pubkey)?;
    let count = on_position_initialized(market)?;
    match queue_attempt(&mut fresh, counter, request) {
        Ok(intent) => {
            *position = fresh;
            Ok(InitializedPosition { intent, count })
        }
        Err(error) => {
            market.position_count = count.old_count;
            Err(error)
        }
    }
}

/// `retry_position_init`: from `InitFailed` with the Lock free, returns to
/// `Initializing` and queues a new attempt under a new Intent. Atomic.
pub fn retry_position_init(
    position: &mut Position,
    counter: &mut OperationCounter,
    request: &PositionInitRequest,
) -> Result<OperationIntent, ErrorCode> {
    position.check_retry()?;
    position.status = PositionStatus::Initializing;
    let result = queue_attempt(position, counter, request);
    if result.is_err() {
        position.status = PositionStatus::InitFailed;
    }
    result
}

/// What the `init_position_state` callback observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PositionInitCallback {
    /// Whether Arcium `verify_output` succeeded.
    pub output_authenticated: bool,
    /// The computation offset the callback's computation account belongs to.
    pub computation_offset: u64,
    /// The Operation Intent account the callback received.
    pub intent_key: Pubkey,
    /// The output's encryption key (must be the registered Viewing Key).
    pub encryption_key: [u8; 32],
    /// The output nonce (input nonce + 1).
    pub output_nonce: u128,
    /// The output ciphertexts in `PositionSecret` order.
    pub ciphertexts: [[u8; 32]; 5],
    /// The current slot.
    pub current_slot: Slot,
}

/// What the callback did to the Position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionInitSettlement {
    /// `Applied`: the zero state is stored and the Position is `Active`.
    Activated,
    /// The attempt ended with this terminal status; the Position is
    /// `InitFailed` and stores nothing.
    Failed(IntentStatus),
    /// The Intent was already terminal; nothing changed.
    Ignored,
}

/// Verifies and applies an `init_position_state` callback (Task 3.3 helpers).
///
/// Unauthenticated output and commitment, offset, or binding mismatches are
/// errors, so the transaction reverts and the Intent stays `Queued`. A
/// terminal Intent is `Ignored`. Otherwise the decision releases the Lock (and
/// bumps the generation for `Applied`) and moves the Position to `Active` or
/// `InitFailed`. Atomic on error.
pub fn settle_position_init(
    position: &mut Position,
    position_key: Pubkey,
    intent: &mut OperationIntent,
    callback: &PositionInitCallback,
) -> Result<PositionInitSettlement, ErrorCode> {
    let bindings = position_init_bindings(
        position.owner,
        position.market,
        position_key,
        intent.position_generation,
    );
    let inputs = position_init_inputs(
        callback.encryption_key,
        position.viewing_key_epoch,
        callback.output_nonce.wrapping_sub(1),
    );
    let context = CallbackContext {
        output_authenticated: callback.output_authenticated,
        computation_offset: callback.computation_offset,
        intent_key: callback.intent_key,
        kind: OperationKind::PositionInit,
        accounts: &bindings,
        input_fields: &inputs,
        current: CurrentDependencies {
            position_generation: Some(position.coordination.generation),
            ..CurrentDependencies::captured_by(intent)
        },
        current_slot: callback.current_slot,
        report: CallbackReport::Success { nonzero: true },
    };
    let decision = classify_callback(intent, &context)?;
    match decision {
        CallbackDecision::LeaveQueued => return Err(ErrorCode::CallbackOutputNotAuthenticated),
        CallbackDecision::Ignore => return Ok(PositionInitSettlement::Ignored),
        _ => {}
    }
    if position.status != PositionStatus::Initializing {
        return Err(PositionError::NotInitializing.into());
    }
    if !intent.lock_owner().holds(&position.coordination.lock) {
        return Err(PositionError::IntentMismatch.into());
    }
    let result = apply_decision_to_slots(
        intent,
        &mut [CoordinationSlot::new(
            position_key,
            &mut position.coordination,
        )],
        decision,
        callback.current_slot,
    )?;
    if result.status == IntentStatus::Applied {
        position.encrypted_state = EncryptedPositionPayload {
            nonce: callback.output_nonce,
            ciphertexts: callback.ciphertexts,
        };
        position.status = PositionStatus::Active;
        Ok(PositionInitSettlement::Activated)
    } else {
        position.status = PositionStatus::InitFailed;
        Ok(PositionInitSettlement::Failed(result.status))
    }
}

/// `abandon_position_init`: permissionless timeout recovery of a queued
/// attempt after Lock expiry. Bumps the Position generation, releases the
/// Lock, marks the Intent `Abandoned`, and moves the Position to
/// `InitFailed`. Atomic. Returns the number of Locks cleared.
pub fn abandon_position_init(
    position: &mut Position,
    position_key: Pubkey,
    intent: &mut OperationIntent,
    current_slot: Slot,
) -> Result<usize, ErrorCode> {
    if intent.kind != OperationKind::PositionInit || intent.position != Some(position_key) {
        return Err(PositionError::IntentMismatch.into());
    }
    if intent.status.is_terminal() {
        return Err(ErrorCode::IntentNotQueued);
    }
    if position.status != PositionStatus::Initializing {
        return Err(PositionError::NotInitializing.into());
    }
    if !intent.lock_owner().holds(&position.coordination.lock) {
        return Err(PositionError::IntentMismatch.into());
    }
    let cleared = crate::operation::abandon_intent(
        intent,
        current_slot,
        &mut [CoordinationSlot::new(
            position_key,
            &mut position.coordination,
        )],
    )?;
    position.status = PositionStatus::InitFailed;
    Ok(cleared)
}

/// `close_position`: checks the close rule and decrements `position_count`.
/// The handler then closes the account to the owner. Atomic.
pub fn close_position(market: &mut Market, position: &Position) -> Result<CountChange, ErrorCode> {
    position.check_close()?;
    on_position_closed(market).map_err(ErrorCode::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::MarketError;
    use crate::operation::derive_intent_address;

    const OWNER: Pubkey = Pubkey::new_from_array([0x11; 32]);
    const MARKET_KEY: Pubkey = Pubkey::new_from_array([0x22; 32]);
    const VIEWING: [u8; 32] = [0x33; 32];

    fn key(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    fn market(cap: u64) -> Market {
        Market {
            layout_version: 1,
            status: MarketStatus::Active,
            bump: 0xFE,
            vault_authority_bump: 0xFD,
            admin: key(1),
            pending_admin: Pubkey::default(),
            upgrade_authority: key(3),
            risk_authority: key(4),
            treasury_authority: key(5),
            guardian_authority: key(6),
            asset_state: key(0x41),
            pool_state: key(0x42),
            recovery_position: key(0x43),
            vault_authority: key(0x44),
            wsol_mint: key(0x45),
            usdc_mint: key(0x46),
            wsol_vault: key(0x47),
            usdc_vault: key(0x48),
            treasury_usdc_account: Pubkey::default(),
            position_count: 0,
            debt_position_count: 0,
            unsettled_recovery_claim_count: 0,
            position_count_cap: cap,
            debt_position_count_cap: 0,
            initialized_slot: 1,
            debt_position_reservations: 0,
            reserved: [0; 120],
        }
    }

    fn position_key(owner: &Pubkey) -> Pubkey {
        derive_position_address(&MARKET_KEY, owner, &crate::ID).0
    }

    fn request(owner: &Pubkey, sequence: u64, offset: u64, slot: Slot) -> PositionInitRequest {
        PositionInitRequest {
            intent_key: derive_intent_address(owner, sequence, &crate::ID).0,
            position_key: position_key(owner),
            sequence,
            computation_offset: offset,
            nonce: 0x0102_0304_0506_0708_090A_0B0C_0D0E_0F10 + u128::from(sequence),
            queued_slot: slot,
        }
    }

    fn new(owner: Pubkey, bump: u8, viewing_pubkey: [u8; 32]) -> NewPosition {
        NewPosition {
            owner,
            market: MARKET_KEY,
            bump,
            viewing_pubkey,
        }
    }

    struct World {
        market: Market,
        position: Position,
        counter: OperationCounter,
        intent: OperationIntent,
        request: PositionInitRequest,
    }

    fn initialized(slot: Slot) -> World {
        let mut market = market(10);
        let mut position = Position::blank(Pubkey::default(), Pubkey::default(), 0);
        let mut counter = OperationCounter::new(OWNER, 7);
        let request = request(&OWNER, 0, 77, slot);
        let done = initialize_position(
            &mut market,
            &mut position,
            &mut counter,
            &new(OWNER, 5, VIEWING),
            &request,
        )
        .unwrap();
        World {
            market,
            position,
            counter,
            intent: done.intent,
            request,
        }
    }

    fn honest(world: &World, slot: Slot) -> PositionInitCallback {
        PositionInitCallback {
            output_authenticated: true,
            computation_offset: world.request.computation_offset,
            intent_key: world.request.intent_key,
            encryption_key: VIEWING,
            output_nonce: world.request.nonce + 1,
            ciphertexts: [[0x99; 32]; 5],
            current_slot: slot,
        }
    }

    fn code(error: ErrorCode) -> u32 {
        u32::from(error)
    }

    #[test]
    fn seed_is_distinct_and_addresses_are_per_owner_and_market() {
        assert_eq!(POSITION_ACCOUNT_SEED, b"position");
        let a = derive_position_address(&MARKET_KEY, &OWNER, &crate::ID).0;
        assert_ne!(
            a,
            derive_position_address(&MARKET_KEY, &key(9), &crate::ID).0
        );
        assert_ne!(a, derive_position_address(&key(9), &OWNER, &crate::ID).0);
        assert_ne!(
            a,
            crate::market::derive_recovery_position_address(&MARKET_KEY, &crate::ID).0
        );
        let spike =
            Pubkey::find_program_address(&[b"spike_position", OWNER.as_ref()], &crate::ID).0;
        assert_ne!(a, spike);
    }

    #[test]
    fn viewing_key_must_be_nonzero() {
        assert_eq!(
            check_viewing_key(&[0; 32]),
            Err(PositionError::InvalidViewingKey)
        );
        let mut one = [0; 32];
        one[31] = 1;
        assert_eq!(check_viewing_key(&one), Ok(()));
        assert!(Position::initializing(OWNER, MARKET_KEY, 1, [0; 32]).is_err());
    }

    #[test]
    fn initialize_registers_the_key_at_epoch_zero_and_locks() {
        let w = initialized(100);
        assert_eq!(w.position.status, PositionStatus::Initializing);
        assert_eq!(w.position.owner, OWNER);
        assert_eq!(w.position.market, MARKET_KEY);
        assert_eq!(w.position.viewing_pubkey, VIEWING);
        assert_eq!(w.position.viewing_key_epoch, 0);
        assert_eq!(w.position.bump, 5);
        assert!(!w.position.has_debt);
        assert_eq!(
            w.position.encrypted_state,
            EncryptedPositionPayload::default()
        );
        assert!(w.intent.lock_owner().holds(&w.position.coordination.lock));
        assert_eq!(w.position.coordination.lock.expiry_slot, 100 + 1_500 + 300);
        assert_eq!(w.market.position_count, 1);
        assert_eq!(w.counter.next_sequence, 1);
        assert_eq!(w.intent.kind, OperationKind::PositionInit);
        assert_eq!(w.intent.status, IntentStatus::Queued);
        assert_eq!(w.intent.payer, OWNER);
        assert_eq!(w.intent.reclaim_authority, OWNER);
        assert_eq!(w.intent.position, Some(w.request.position_key));
        assert_eq!(w.intent.position_generation, Some(0));
        assert_eq!(w.intent.decision_expiry_slot, 1_600);
        assert_eq!(w.intent.lock_expiry_slot, 1_900);
        assert_eq!(w.intent.validate_new(), Ok(()));
    }

    #[test]
    fn initialize_fails_atomically() {
        // Count cap.
        let mut m = market(0);
        let mut position = Position::blank(Pubkey::default(), Pubkey::default(), 0);
        let mut counter = OperationCounter::new(OWNER, 7);
        let r = request(&OWNER, 0, 1, 10);
        let ok = new(OWNER, 1, VIEWING);
        let err = initialize_position(&mut m, &mut position, &mut counter, &ok, &r).unwrap_err();
        assert_eq!(code(err), code(ErrorCode::PositionCountCapReached));
        assert_eq!(m.position_count, 0);
        assert_eq!(counter.next_sequence, 0);
        assert_eq!(position.status, PositionStatus::Uninitialized);

        // Wrong sequence: the count increment is rolled back.
        let mut m = market(5);
        let r = request(&OWNER, 3, 1, 10);
        let err = initialize_position(&mut m, &mut position, &mut counter, &ok, &r).unwrap_err();
        assert_eq!(code(err), code(ErrorCode::IntentSequenceOutOfOrder));
        assert_eq!(m.position_count, 0);
        assert_eq!(position.status, PositionStatus::Uninitialized);

        // Zero key and inactive market.
        let r = request(&OWNER, 0, 1, 10);
        let zero = new(OWNER, 1, [0; 32]);
        let err = initialize_position(&mut m, &mut position, &mut counter, &zero, &r).unwrap_err();
        assert_eq!(code(err), code(ErrorCode::InvalidViewingKey));
        m.status = MarketStatus::Uninitialized;
        let err = initialize_position(&mut m, &mut position, &mut counter, &ok, &r).unwrap_err();
        assert_eq!(code(err), code(ErrorCode::MarketNotActive));
        assert_eq!(m.position_count, 0);
    }

    #[test]
    fn applied_callback_stores_the_payload_and_activates() {
        let mut w = initialized(100);
        let cb = honest(&w, 200);
        let out = settle_position_init(&mut w.position, w.request.position_key, &mut w.intent, &cb)
            .unwrap();
        assert_eq!(out, PositionInitSettlement::Activated);
        assert_eq!(w.position.status, PositionStatus::Active);
        assert_eq!(w.position.encrypted_state.nonce, w.request.nonce + 1);
        assert_eq!(w.position.encrypted_state.ciphertexts, [[0x99; 32]; 5]);
        assert_eq!(w.position.coordination.generation, 1);
        assert!(!w.position.coordination.lock.is_active());
        assert_eq!(w.intent.status, IntentStatus::Applied);

        // Duplicate callback: ignored, nothing changes.
        let before = (w.position.coordination, w.position.encrypted_state);
        let again = PositionInitCallback {
            ciphertexts: [[0x55; 32]; 5],
            ..cb
        };
        let out = settle_position_init(
            &mut w.position,
            w.request.position_key,
            &mut w.intent,
            &again,
        )
        .unwrap();
        assert_eq!(out, PositionInitSettlement::Ignored);
        assert_eq!(
            (w.position.coordination, w.position.encrypted_state),
            before
        );
        // Active cannot retry or close.
        assert_eq!(w.position.check_retry(), Err(PositionError::NotRetryable));
        assert_eq!(w.position.check_close(), Err(PositionError::NotClosable));
    }

    #[test]
    fn expired_callback_fails_recoverably() {
        let mut w = initialized(100);
        let cb = honest(&w, 1_601);
        let out = settle_position_init(&mut w.position, w.request.position_key, &mut w.intent, &cb)
            .unwrap();
        assert_eq!(out, PositionInitSettlement::Failed(IntentStatus::Expired));
        assert_eq!(w.position.status, PositionStatus::InitFailed);
        assert_eq!(
            w.position.encrypted_state,
            EncryptedPositionPayload::default()
        );
        assert_eq!(w.position.coordination.generation, 0);
        assert!(!w.position.coordination.lock.is_active());
    }

    #[test]
    fn invalid_callbacks_leave_the_attempt_queued() {
        let w = initialized(100);
        let base = honest(&w, 200);
        let cases = [
            (
                PositionInitCallback {
                    output_authenticated: false,
                    ..base
                },
                ErrorCode::CallbackOutputNotAuthenticated,
            ),
            (
                PositionInitCallback {
                    computation_offset: 78,
                    ..base
                },
                ErrorCode::CallbackOffsetMismatch,
            ),
            (
                PositionInitCallback {
                    intent_key: key(0xEE),
                    ..base
                },
                ErrorCode::CallbackIntentMismatch,
            ),
            (
                PositionInitCallback {
                    encryption_key: [0x34; 32],
                    ..base
                },
                ErrorCode::CallbackInputsCommitmentMismatch,
            ),
            (
                PositionInitCallback {
                    output_nonce: w.request.nonce,
                    ..base
                },
                ErrorCode::CallbackInputsCommitmentMismatch,
            ),
        ];
        for (cb, expected) in cases {
            let mut position = clone_position(&w.position);
            let mut intent = w.intent;
            let err = settle_position_init(&mut position, w.request.position_key, &mut intent, &cb)
                .unwrap_err();
            assert_eq!(code(err), code(expected));
            assert_eq!(intent, w.intent);
            assert_eq!(position.coordination, w.position.coordination);
            assert_eq!(position.status, PositionStatus::Initializing);
        }
        // Another Position's key changes the accounts commitment.
        let mut position = clone_position(&w.position);
        let mut intent = w.intent;
        let err = settle_position_init(&mut position, key(0x77), &mut intent, &base).unwrap_err();
        assert_eq!(
            code(err),
            code(ErrorCode::CallbackAccountsCommitmentMismatch)
        );
    }

    #[test]
    fn retry_is_blocked_while_queued_and_uses_a_fresh_intent() {
        let mut w = initialized(100);
        assert_eq!(w.position.check_retry(), Err(PositionError::AttemptQueued));
        assert_eq!(w.position.check_close(), Err(PositionError::AttemptQueued));
        let r1 = request(&OWNER, 1, 88, 150);
        let err = retry_position_init(&mut w.position, &mut w.counter, &r1).unwrap_err();
        assert_eq!(code(err), code(ErrorCode::PositionInitAttemptQueued));

        // Abandon only after Lock expiry.
        let err = abandon_position_init(
            &mut w.position,
            w.request.position_key,
            &mut w.intent,
            1_900,
        )
        .unwrap_err();
        assert_eq!(code(err), code(ErrorCode::LockNotExpired));
        let cleared = abandon_position_init(
            &mut w.position,
            w.request.position_key,
            &mut w.intent,
            1_901,
        )
        .unwrap();
        assert_eq!(cleared, 1);
        assert_eq!(w.intent.status, IntentStatus::Abandoned);
        assert_eq!(w.position.status, PositionStatus::InitFailed);
        assert_eq!(w.position.coordination.generation, 1);

        // A late callback for the abandoned attempt is ignored.
        let late = honest(&w, 1_950);
        let mut intent = w.intent;
        assert_eq!(
            settle_position_init(&mut w.position, w.request.position_key, &mut intent, &late)
                .unwrap(),
            PositionInitSettlement::Ignored
        );
        assert_eq!(w.position.status, PositionStatus::InitFailed);

        // Retry: new sequence, new Intent address, new generation captured.
        let r1 = request(&OWNER, 1, 88, 2_000);
        let intent2 = retry_position_init(&mut w.position, &mut w.counter, &r1).unwrap();
        assert_ne!(intent2.key, w.intent.key);
        assert_eq!(intent2.position_generation, Some(1));
        assert_eq!(w.counter.next_sequence, 2);
        assert_eq!(w.position.status, PositionStatus::Initializing);
        assert_eq!(w.market.position_count, 1, "retry does not count again");

        // Reusing the old sequence is rejected and leaves InitFailed intact.
        let mut failed = clone_position(&w.position);
        failed.status = PositionStatus::InitFailed;
        failed.coordination = Default::default();
        let mut counter = w.counter;
        let err = retry_position_init(&mut failed, &mut counter, &request(&OWNER, 0, 9, 2_000))
            .unwrap_err();
        assert_eq!(code(err), code(ErrorCode::IntentSequenceReused));
        assert_eq!(failed.status, PositionStatus::InitFailed);
        assert!(!failed.coordination.lock.is_active());
    }

    #[test]
    fn close_only_from_init_failed_and_decrements_the_count() {
        let mut w = initialized(100);
        let err = close_position(&mut w.market, &w.position).unwrap_err();
        assert_eq!(code(err), code(ErrorCode::PositionInitAttemptQueued));
        abandon_position_init(
            &mut w.position,
            w.request.position_key,
            &mut w.intent,
            2_000,
        )
        .unwrap();
        let change = close_position(&mut w.market, &w.position).unwrap();
        assert_eq!((change.old_count, change.new_count), (1, 0));
        assert_eq!(w.market.position_count, 0);
        // Debt blocks close; an empty count cannot underflow.
        let mut indebted = clone_position(&w.position);
        indebted.has_debt = true;
        let err = close_position(&mut w.market, &indebted).unwrap_err();
        assert_eq!(code(err), code(ErrorCode::PositionNotClosable));
        let err = close_position(&mut w.market, &w.position).unwrap_err();
        assert_eq!(code(err), code(ErrorCode::LifecycleCountOutOfRange));
        assert_eq!(
            on_position_closed(&mut market(1)),
            Err(MarketError::LifecycleCountOutOfRange)
        );
    }

    #[test]
    fn abandon_requires_this_positions_queued_init_intent() {
        let w = initialized(100);
        let mut position = clone_position(&w.position);
        let mut other = OperationIntent {
            kind: OperationKind::Shield,
            ..w.intent
        };
        let err = abandon_position_init(&mut position, w.request.position_key, &mut other, 5_000)
            .unwrap_err();
        assert_eq!(code(err), code(ErrorCode::PositionInitIntentMismatch));
        let mut intent = w.intent;
        let err = abandon_position_init(&mut position, key(0x77), &mut intent, 5_000).unwrap_err();
        assert_eq!(code(err), code(ErrorCode::PositionInitIntentMismatch));
        let mut wrong_offset = OperationIntent {
            computation_offset: 1,
            ..w.intent
        };
        let err = abandon_position_init(
            &mut position,
            w.request.position_key,
            &mut wrong_offset,
            5_000,
        )
        .unwrap_err();
        assert_eq!(code(err), code(ErrorCode::PositionInitIntentMismatch));
        assert_eq!(position.coordination, w.position.coordination);
    }

    fn clone_position(p: &Position) -> Position {
        Position {
            owner: p.owner,
            coordination: p.coordination,
            bump: p.bump,
            has_debt: p.has_debt,
            status: p.status,
            market: p.market,
            viewing_pubkey: p.viewing_pubkey,
            viewing_key_epoch: p.viewing_key_epoch,
            frozen: p.frozen,
            freeze_epoch: p.freeze_epoch,
            frozen_since_slot: p.frozen_since_slot,
            next_disclosure_seq: p.next_disclosure_seq,
            encrypted_state: p.encrypted_state,
            next_shield_seq: p.next_shield_seq,
            reserved: p.reserved,
        }
    }

    // ---- Property tests -------------------------------------------------------

    /// Deterministic linear-congruential sampler (no external crate).
    struct Lcg(u64);
    impl Lcg {
        fn sample(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 11
        }

        fn below(&mut self, n: u64) -> u64 {
            self.sample() % n
        }
    }

    /// Reference model of one owner's Position.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Model {
        Absent,
        Initializing,
        Active,
        InitFailed,
    }

    struct OwnerWorld {
        owner: Pubkey,
        model: Model,
        position: Position,
        counter: OperationCounter,
        intent: Option<OperationIntent>,
        request: Option<PositionInitRequest>,
    }

    /// Random sequences of init, callbacks (honest, expired, invalid, duplicate),
    /// abandonment, retry, and close over several owners. Checks after every
    /// step that only the model's transitions happen, `position_count` equals
    /// the number of open Positions and never passes the cap, retry and close
    /// are refused while an attempt is queued, and only `Active` stores state.
    #[test]
    fn prop_position_init_state_machine_matches_the_model() {
        for seed in 0..64u64 {
            let mut rng = Lcg(0x5EED_5100 ^ seed);
            let cap = 1 + rng.below(4);
            let mut m = market(cap);
            let mut slot: Slot = 1_000;
            let mut owners: Vec<OwnerWorld> = (0..5u8)
                .map(|n| OwnerWorld {
                    owner: key(0x60 + n),
                    model: Model::Absent,
                    position: Position::blank(Pubkey::default(), Pubkey::default(), 0),
                    counter: OperationCounter::new(key(0x60 + n), 1),
                    intent: None,
                    request: None,
                })
                .collect();
            let mut offset = 1u64;

            for _ in 0..200 {
                slot += 1 + rng.below(400);
                offset += 1;
                let index = rng.below(owners.len() as u64) as usize;
                let open_before = owners.iter().filter(|o| o.model != Model::Absent).count() as u64;
                let o = &mut owners[index];
                let pkey = derive_position_address(&MARKET_KEY, &o.owner, &crate::ID).0;
                let sequence = o.counter.next_sequence;
                let req = PositionInitRequest {
                    intent_key: derive_intent_address(&o.owner, sequence, &crate::ID).0,
                    position_key: pkey,
                    sequence,
                    computation_offset: offset,
                    nonce: u128::from(rng.sample()) << 32 | u128::from(offset),
                    queued_slot: slot,
                };
                let before_status = o.position.status;
                match rng.below(6) {
                    // initialize
                    0 => {
                        if o.model != Model::Absent {
                            continue; // the PDA exists: Anchor `init` rejects a duplicate
                        }
                        let result = initialize_position(
                            &mut m,
                            &mut o.position,
                            &mut o.counter,
                            &new(o.owner, 1, VIEWING),
                            &req,
                        );
                        if open_before >= cap {
                            assert_eq!(
                                code(result.unwrap_err()),
                                code(ErrorCode::PositionCountCapReached)
                            );
                            assert_eq!(o.position.status, before_status);
                        } else {
                            let done = result.unwrap();
                            o.model = Model::Initializing;
                            o.intent = Some(done.intent);
                            o.request = Some(req);
                        }
                    }
                    // callback: honest, possibly late
                    1 | 2 => {
                        let (Some(mut intent), Some(r)) = (o.intent, o.request) else {
                            continue;
                        };
                        let cb = PositionInitCallback {
                            output_authenticated: rng.below(8) != 0,
                            computation_offset: r.computation_offset,
                            intent_key: r.intent_key,
                            encryption_key: VIEWING,
                            output_nonce: r.nonce.wrapping_add(1),
                            ciphertexts: [[0x42; 32]; 5],
                            current_slot: slot,
                        };
                        let result = settle_position_init(&mut o.position, pkey, &mut intent, &cb);
                        match result {
                            Err(_) => {
                                assert!(!cb.output_authenticated);
                                assert_eq!(o.position.status, before_status);
                            }
                            Ok(PositionInitSettlement::Ignored) => {
                                assert_ne!(o.model, Model::Initializing);
                            }
                            Ok(PositionInitSettlement::Activated) => {
                                assert_eq!(o.model, Model::Initializing);
                                assert!(slot <= r.queued_slot + POSITION_INIT_DECISION_SLOTS);
                                o.model = Model::Active;
                            }
                            Ok(PositionInitSettlement::Failed(status)) => {
                                assert_eq!(o.model, Model::Initializing);
                                assert_eq!(status, IntentStatus::Expired);
                                o.model = Model::InitFailed;
                            }
                        }
                        o.intent = Some(intent);
                    }
                    // abandon
                    3 => {
                        let Some(mut intent) = o.intent else { continue };
                        let result =
                            abandon_position_init(&mut o.position, pkey, &mut intent, slot);
                        let expired = slot > intent.lock_expiry_slot;
                        match (o.model, expired) {
                            (Model::Initializing, true) => {
                                result.unwrap();
                                o.model = Model::InitFailed;
                            }
                            _ => {
                                assert!(result.is_err());
                                assert_eq!(o.position.status, before_status);
                            }
                        }
                        o.intent = Some(intent);
                    }
                    // retry
                    4 => {
                        if o.model == Model::Absent {
                            continue;
                        }
                        let result = retry_position_init(&mut o.position, &mut o.counter, &req);
                        match o.model {
                            Model::InitFailed => {
                                let intent = result.unwrap();
                                o.model = Model::Initializing;
                                o.intent = Some(intent);
                                o.request = Some(req);
                            }
                            Model::Initializing => {
                                assert_eq!(
                                    code(result.unwrap_err()),
                                    code(ErrorCode::PositionInitAttemptQueued)
                                );
                            }
                            _ => {
                                assert_eq!(
                                    code(result.unwrap_err()),
                                    code(ErrorCode::PositionNotRetryable)
                                );
                            }
                        }
                    }
                    // close
                    _ => {
                        if o.model == Model::Absent {
                            continue;
                        }
                        let result = close_position(&mut m, &o.position);
                        if o.model == Model::InitFailed {
                            result.unwrap();
                            o.model = Model::Absent;
                            o.position = Position::blank(Pubkey::default(), Pubkey::default(), 0);
                            o.intent = None;
                            o.request = None;
                        } else {
                            assert!(result.is_err());
                        }
                    }
                }

                // Invariants.
                let open = owners.iter().filter(|o| o.model != Model::Absent).count() as u64;
                assert_eq!(m.position_count, open, "count tracks open Positions");
                assert!(m.position_count <= cap);
                for o in &owners {
                    let expected = match o.model {
                        Model::Absent => PositionStatus::Uninitialized,
                        Model::Initializing => PositionStatus::Initializing,
                        Model::Active => PositionStatus::Active,
                        Model::InitFailed => PositionStatus::InitFailed,
                    };
                    assert_eq!(o.position.status, expected);
                    let locked = o.position.coordination.lock.is_active();
                    assert_eq!(locked, o.model == Model::Initializing);
                    let stored = o.position.encrypted_state != EncryptedPositionPayload::default();
                    assert_eq!(stored, o.model == Model::Active);
                    if o.model != Model::Absent {
                        assert_eq!(o.position.viewing_key_epoch, 0);
                        assert_eq!(o.position.viewing_pubkey, VIEWING);
                    }
                }
            }
        }
    }
}
