//! Task 5.6 balance-layer checkpoint: a model-based proof of the Position,
//! shield, unshield, and key-rotation rules.
//!
//! Tasks 5.1 to 5.5 own the pure rules; none of them holds a plaintext balance
//! because the balances are encrypted. This module gives the encrypted
//! payload a *test oracle*: a reversible test cipher ([`seal`] / [`unseal`])
//! lets the harness play the MPC circuit (decrypt the stored payload, compute
//! the new balances, re-seal them under the derived output nonce) and lets
//! every check read the stored payload back as plain integers.
//!
//! # Two sides, compared after every action
//!
//! * [`Real`] runs the REAL production functions the instructions call
//!   (`initialize_position`, `queue_shield`, `settle_shield_credit`,
//!   `retry_shield`, `refund_shield`, `queue_unshield`, `settle_unshield`,
//!   `queue_rotation`, `settle_rotation`, the abandon functions, and the
//!   pause functions). It also performs the token movements the instruction
//!   handlers would (`wallet -> vault` on a queued shield, `vault -> wallet`
//!   on a refund or an approved unshield), at exactly the points the handlers
//!   do.
//! * [`Model`] is a naive reference written from the requirements. It uses no
//!   production function. It keeps plaintext free balances per asset, both
//!   vault totals, the Pending Shields, the Intents, the Lock, the generation,
//!   the pause state, and the slot.
//!
//! [`Harness::step`] runs one [`Act`] on both, checks the named invariants on
//! the real side, then requires the two to agree on the [`Outcome`] and on the
//! whole [`Snap`]shot.
//!
//! # Action alphabet
//!
//! `InitPosition`, `RetryInit`, `Shield(asset, amount)` (amounts include 0,
//! 1, and the `u64` edge), `ShieldOverwrite` (a shield into an occupied
//! Pending Shield account), `RetryShield`, `RefundShield`, `Unshield(asset,
//! within | exact balance | balance + 1 | huge)`, `Rotate(key)`, `RotateStale`,
//! `Callback(target, variant)` with variants honest, unauthenticated (this is
//! also how an Arcium computation failure arrives), wrong key, wrong output
//! nonce, wrong offset and targets current, first, previous (the honest
//! callback of a terminal Intent is a duplicate, a late callback after
//! abandonment, or an old-attempt callback; after `ExternalBump` or `Advance`
//! it resolves `Stale` or `Expired`), `Abandon`, durable and guardian pause
//! and unpause of the Shield and Unshield bits, `GuardianClear`, `Advance`,
//! and `ExternalBump`.
//!
//! # Invariants
//!
//! Each is one function whose error message starts with its name.
//!
//! * `PENDING SHIELD EXCLUSIVITY`
//! * `VAULT CONSERVATION`
//! * `OVERSIZED UNSHIELD RETURNS ZERO`
//! * `FAILURE AND LATE CALLBACK NEUTRALITY`
//! * `BALANCE BACKING`
//! * `ROTATION`
//! * `NONCE UNIQUENESS`
//! * `PAUSE AND RESCUE`
//! * `LOCK SAFETY`
//!
//! `AGREEMENT` is the real-versus-reference comparison. Each invariant has a
//! negative control: [`Fault`] breaks the real side in one specific way and
//! the matching test asserts that the named check reports it.

use std::collections::BTreeSet;

use anchor_lang::prelude::Pubkey;

use super::accounts::{EncryptedPositionPayload, Position, PositionStatus};
use super::nonce::output_nonce;
use super::pending_shield::{NewPendingShield, PendingShield, PendingShieldStatus, ShieldAsset};
use super::position::{
    abandon_position_init, initialize_position, retry_position_init, settle_position_init,
    NewPosition, PositionInitCallback, PositionInitRequest, PositionInitSettlement,
};
use super::rotation::{
    abandon_rotation, queue_rotation, settle_rotation, RotationReport, RotationRequest,
    RotationSettlement,
};
use super::shield::{
    abandon_shield, asset_mint, asset_vault, queue_shield, refund_shield, retry_shield,
    settle_shield_credit, ShieldCallback, ShieldRequest, ShieldSettlement, TokenAccountView,
};
use super::unshield::{
    abandon_unshield, queue_unshield, settle_unshield, UnshieldReport, UnshieldRequest,
    UnshieldSettlement,
};
use super::Slot;
use crate::errors::ErrorCode;
use crate::market::pause::{apply_durable_pause, apply_guardian_clear, apply_guardian_pause};
use crate::market::{
    is_paused, pause_flags, Market, MarketError, MarketStatus, PauseCategory, PauseState,
};
use crate::operation::{IntentStatus, OperationCounter, OperationIntent, OperationKind};

// ---- Fixed world ------------------------------------------------------------

const OWNER: Pubkey = Pubkey::new_from_array([0x11; 32]);
const MARKET_KEY: Pubkey = Pubkey::new_from_array([0x22; 32]);
const POSITION_KEY: Pubkey = Pubkey::new_from_array([0x23; 32]);
const SOURCE: Pubkey = Pubkey::new_from_array([0x24; 32]);
const DEST: Pubkey = Pubkey::new_from_array([0x25; 32]);

/// The owner wallet starts with this many base units of each asset, far more
/// than any run can shield, so a shield is never limited by the wallet.
const WALLET_START: u128 = 1 << 100;

/// The reference model's own timing constants (Decision Expiry and Lock
/// grace), written from the design, not read from the production constants.
const MODEL_DECISION: u64 = 1_500;
const MODEL_GRACE: u64 = 300;
/// The reference model's own guardian cooldown.
const MODEL_COOLDOWN: u64 = 9_000;

/// The initial Viewing Key id (`[1; 32]`).
const KEY_INITIAL: u8 = 1;

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}

fn market() -> Market {
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
        position_count_cap: 10,
        debt_position_count_cap: 0,
        initialized_slot: 1,
        debt_position_reservations: 0,
        reserved: [0; 120],
    }
}

fn intent_key(sequence: u64) -> Pubkey {
    let mut bytes = [0x60; 32];
    bytes[..8].copy_from_slice(&sequence.to_le_bytes());
    Pubkey::new_from_array(bytes)
}

fn shield_key(sequence: u64) -> Pubkey {
    let mut bytes = [0x50; 32];
    bytes[..8].copy_from_slice(&sequence.to_le_bytes());
    Pubkey::new_from_array(bytes)
}

fn offset_for(sequence: u64) -> u64 {
    9_000 + sequence
}

fn asset_index(asset: ShieldAsset) -> usize {
    match asset {
        ShieldAsset::Sol => 0,
        ShieldAsset::Usdc => 1,
    }
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

// ---- The test cipher (the oracle for the encrypted payload) ------------------

/// Marker bytes that follow each sealed value. A wrong key or nonce yields a
/// different pad, so the marker fails with probability `1 - 2^-192`, which
/// models the authenticated rejection of a wrong Viewing Key.
const MARKER: u8 = 0xC7;

/// The splitmix64 finalizer, a cheap bijective mixer.
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A keystream block for `(viewing_key, nonce, index)`. It is not a secure
/// cipher; it only has to make a different key or nonce produce a different
/// block, which models the authenticated rejection of a wrong Viewing Key.
fn pad(viewing_key: &[u8; 32], nonce: u128, index: usize) -> [u8; 32] {
    let mut state = mix(0x9E37_79B9_7F4A_7C15 ^ index as u64);
    for chunk in viewing_key.chunks(8) {
        let mut word = [0u8; 8];
        word.copy_from_slice(chunk);
        state = mix(state ^ u64::from_le_bytes(word));
    }
    state = mix(state ^ nonce as u64);
    state = mix(state ^ (nonce >> 64) as u64);
    let mut block = [0u8; 32];
    for part in block.chunks_mut(8) {
        state = mix(state.wrapping_add(0x9E37_79B9_7F4A_7C15));
        part.copy_from_slice(&state.to_le_bytes());
    }
    block
}
/// Seals five plain values under `(viewing_key, nonce)`.
fn seal(viewing_key: &[u8; 32], nonce: u128, values: [u64; 5]) -> [[u8; 32]; 5] {
    let mut out = [[0u8; 32]; 5];
    for (index, value) in values.iter().enumerate() {
        let mut plain = [MARKER; 32];
        plain[..8].copy_from_slice(&value.to_le_bytes());
        let pad = pad(viewing_key, nonce, index);
        for byte in 0..32 {
            out[index][byte] = plain[byte] ^ pad[byte];
        }
    }
    out
}

/// Opens a payload. `None` when the key or nonce is wrong.
fn unseal(viewing_key: &[u8; 32], nonce: u128, sealed: &[[u8; 32]; 5]) -> Option<[u64; 5]> {
    let mut values = [0u64; 5];
    for index in 0..5 {
        let pad = pad(viewing_key, nonce, index);
        let mut plain = [0u8; 32];
        for byte in 0..32 {
            plain[byte] = sealed[index][byte] ^ pad[byte];
        }
        if plain[8..].iter().any(|b| *b != MARKER) {
            return None;
        }
        let mut low = [0u8; 8];
        low.copy_from_slice(&plain[..8]);
        values[index] = u64::from_le_bytes(low);
    }
    Some(values)
}

// ---- Actions and outcomes ----------------------------------------------------

/// How an unshield amount is chosen, resolved against the model balance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UAmt {
    Exact(u64),
    /// Half the balance, at least 1.
    Within,
    /// Exactly the balance.
    Balance,
    /// The balance plus one (saturating).
    BalancePlusOne,
    /// `u64::MAX`.
    Huge,
}

fn resolve_amount(amount: UAmt, balance: u64) -> u64 {
    match amount {
        UAmt::Exact(v) => v,
        UAmt::Within => (balance / 2).max(1),
        UAmt::Balance => balance,
        UAmt::BalancePlusOne => balance.saturating_add(1),
        UAmt::Huge => u64::MAX,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sel {
    Last,
    First,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Cur,
    First,
    Prev,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Variant {
    Honest,
    Unauth,
    WrongKey,
    WrongNonce,
    WrongOffset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    InitPosition,
    RetryInit,
    Shield(ShieldAsset, u64),
    ShieldOverwrite(Sel),
    RetryShield(Sel),
    RefundShield(Sel),
    Unshield(ShieldAsset, UAmt),
    Rotate(u8),
    RotateStale,
    Callback(Target, Variant),
    Abandon,
    PauseShield,
    UnpauseShield,
    PauseUnshield,
    UnpauseUnshield,
    GuardianPause(u16, u64),
    GuardianClear,
    Advance(u64),
    ExternalBump,
}

/// Why an action was rejected, coarsely. Only the classes the invariants
/// care about are distinguished.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Why {
    Paused,
    Cooldown,
    /// Unauthenticated callback output (also an Arcium computation failure).
    Unauth,
    /// Callback binding mismatch (key, nonce, offset, accounts).
    Binding,
    /// Any other rule: state, amount, Lock held, authority, expectations.
    Invalid,
}

/// The observable result of one action. Both sides must produce the same one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Outcome {
    /// A queue, retry, pause, or clock action succeeded.
    Done,
    /// The action does not apply in this state; nothing was called.
    Skipped,
    /// A callback or abandonment resolved the Intent `Applied` (a credit, an
    /// activation, or a rotation).
    Applied,
    /// The circuit approved zero: a credit that would overflow, or an
    /// oversized unshield.
    Zero,
    Stale,
    Expired,
    /// A callback for a terminal Intent: nothing changed.
    Ignored,
    Abandoned,
    /// A refund returned this many tokens to the source account.
    Refund(u64),
    /// An unshield was approved and this many tokens left the vault.
    Approved(u64),
    Rejected(Why),
}

fn status_outcome(status: IntentStatus) -> Outcome {
    match status {
        IntentStatus::Zero => Outcome::Zero,
        IntentStatus::Stale => Outcome::Stale,
        IntentStatus::Expired => Outcome::Expired,
        other => panic!("unexpected callback status {other:?}"),
    }
}

fn why(error: ErrorCode) -> Why {
    if u32::from(error) == u32::from(ErrorCode::ActionPaused) {
        Why::Paused
    } else {
        Why::Invalid
    }
}

fn callback_why(error: ErrorCode) -> Why {
    if u32::from(error) == u32::from(ErrorCode::CallbackOutputNotAuthenticated) {
        Why::Unauth
    } else {
        Why::Binding
    }
}

fn market_why(error: MarketError) -> Why {
    if error == MarketError::GuardianPauseCooldown {
        Why::Cooldown
    } else {
        Why::Invalid
    }
}

fn pick(target: Target, len: usize) -> Option<usize> {
    match target {
        Target::Cur => len.checked_sub(1),
        Target::First => (len > 0).then_some(0),
        Target::Prev => match len {
            0 => None,
            1 => Some(0),
            n => Some(n - 2),
        },
    }
}

fn pick_sel(sel: Sel, len: usize) -> Option<usize> {
    match sel {
        Sel::Last => len.checked_sub(1),
        Sel::First => (len > 0).then_some(0),
    }
}

// ---- Snapshots ---------------------------------------------------------------

/// Everything the real side and the reference model must agree on.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Snap {
    slot: Slot,
    status: PositionStatus,
    key: u8,
    epoch: u64,
    generation: u64,
    /// Index of the Intent holding the Position Lock.
    lock: Option<usize>,
    /// Free `[sol, usdc]` balances; `Some` only while `Active`.
    balances: Option<[u64; 2]>,
    intents: Vec<(OperationKind, IntentStatus)>,
    /// `(asset, amount, status, attempt, active Intent index)` per shield.
    shields: Vec<(ShieldAsset, u64, PendingShieldStatus, u64, usize)>,
    vault: [u128; 2],
    wallet: [u128; 2],
    out: [u128; 2],
    blocked: (bool, bool),
    next_shield_seq: u64,
    next_sequence: u64,
}

/// The value-bearing state the neutrality checks compare.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Fin {
    payload: EncryptedPositionPayload,
    key: [u8; 32],
    epoch: u64,
    balances: Option<[u64; 5]>,
    undecryptable: bool,
    vault: [u128; 2],
    wallet: [u128; 2],
    out: [u128; 2],
    shields: Vec<(PendingShieldStatus, ShieldAsset, u64, u64, usize)>,
    intents: Vec<IntentStatus>,
}

impl Fin {
    /// Equality of the value-bearing fields (not the Intent statuses).
    fn same_value(&self, other: &Fin) -> bool {
        self.payload == other.payload
            && self.key == other.key
            && self.epoch == other.epoch
            && self.balances == other.balances
            && self.vault == other.vault
            && self.wallet == other.wallet
            && self.out == other.out
            && self.shields == other.shields
    }
}

// ---- The real side -----------------------------------------------------------

/// What the simulated circuit sealed for one queued attempt.
#[derive(Clone, Copy, Debug)]
struct Out {
    key: [u8; 32],
    nonce: u128,
    cts: [[u8; 32]; 5],
    /// Shield: the credit fits in `u64`.
    credited: bool,
    /// Unshield: the approved amount (the request or zero).
    approved: u64,
}

#[derive(Clone, Copy, Debug)]
struct Rec {
    intent: OperationIntent,
    shield: Option<usize>,
    asset: ShieldAsset,
    amount: u64,
    out: Out,
}

#[derive(Clone, Copy, Debug)]
struct RShield {
    shield: PendingShield,
    /// Index of the active attempt in `Real::recs`.
    active: usize,
}

/// A deliberate defect in the real side, used by the negative controls to
/// prove that each invariant check can fail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    None,
    /// A refund pays the owner but leaves the vault untouched.
    RefundKeepsVault,
    /// A `Zero` callback still writes a balance.
    ZeroWrites,
    /// A callback for a terminal Intent still writes a balance.
    IgnoredWrites,
    /// A refund marks the Pending Shield `Credited`.
    RefundFlipsToCredited,
    /// An oversized unshield wipes the balance instead of approving zero.
    OversizedDrains,
    /// The refund handler wrongly consults the Shield pause.
    RefundBlockedByPause,
    /// A rotation forgets to increment the epoch.
    RotationKeepsEpoch,
    /// A rotation changes the key without re-sealing the payload.
    RotationSwapsKeyOnly,
    /// Every output is recorded under the same nonce.
    NonceReuse,
    /// A late callback for an abandoned Intent moves a token.
    LateCallbackMovesTokens,
}

struct Real {
    market: Market,
    pauses: PauseState,
    position: Position,
    counter: OperationCounter,
    recs: Vec<Rec>,
    shields: Vec<RShield>,
    vault: [u128; 2],
    wallet: [u128; 2],
    out: [u128; 2],
    slot: Slot,
    init_count: u64,
    /// The Intent the last callback targeted.
    target: Option<usize>,
    fault: Fault,
    /// Defects the real side noticed in its own bookkeeping.
    violations: Vec<String>,
}

impl Clone for Real {
    fn clone(&self) -> Self {
        Self {
            market: market_clone(&self.market),
            pauses: self.pauses,
            position: clone_position(&self.position),
            counter: self.counter,
            recs: self.recs.clone(),
            shields: self.shields.clone(),
            vault: self.vault,
            wallet: self.wallet,
            out: self.out,
            slot: self.slot,
            init_count: self.init_count,
            target: self.target,
            fault: self.fault,
            violations: self.violations.clone(),
        }
    }
}

fn market_clone(m: &Market) -> Market {
    m.clone()
}

impl Real {
    fn new(fault: Fault) -> Self {
        Self {
            market: market(),
            pauses: PauseState::default(),
            position: Position::blank(OWNER, MARKET_KEY, 0xFC),
            counter: OperationCounter::new(OWNER, 0xFA),
            recs: Vec::new(),
            shields: Vec::new(),
            vault: [0; 2],
            wallet: [WALLET_START; 2],
            out: [0; 2],
            slot: 10,
            init_count: 0,
            target: None,
            fault,
            violations: Vec::new(),
        }
    }

    /// The stored payload read with the stored key.
    fn decode(&self) -> Option<[u64; 5]> {
        unseal(
            &self.position.viewing_pubkey,
            self.position.encrypted_state.nonce,
            &self.position.encrypted_state.ciphertexts,
        )
    }

    /// The balances the circuit reads: zero until the Position is `Active`.
    fn plain(&self) -> [u64; 5] {
        if self.position.status == PositionStatus::Active {
            self.decode().unwrap_or([0; 5])
        } else {
            [0; 5]
        }
    }

    /// Replaces the stored payload (under the stored key and nonce) with the
    /// balances `edit` produces. Only the fault injections use it.
    fn corrupt(&mut self, edit: impl Fn(&mut [u64; 5])) {
        let mut values = self.plain();
        edit(&mut values);
        let state = &mut self.position.encrypted_state;
        state.ciphertexts = seal(&self.position.viewing_pubkey, state.nonce, values);
    }

    fn fin(&self) -> Fin {
        let active = self.position.status == PositionStatus::Active;
        let decoded = if active { self.decode() } else { None };
        Fin {
            payload: self.position.encrypted_state,
            key: self.position.viewing_pubkey,
            epoch: self.position.viewing_key_epoch,
            balances: decoded,
            undecryptable: active && decoded.is_none(),
            vault: self.vault,
            wallet: self.wallet,
            out: self.out,
            shields: self
                .shields
                .iter()
                .map(|s| {
                    (
                        s.shield.status,
                        s.shield.asset,
                        s.shield.amount,
                        s.shield.attempt_sequence,
                        s.active,
                    )
                })
                .collect(),
            intents: self.recs.iter().map(|r| r.intent.status).collect(),
        }
    }

    fn snap(&self) -> Snap {
        let p = &self.position;
        let uniform = p.viewing_pubkey.iter().all(|b| *b == p.viewing_pubkey[0]);
        let lock = if p.coordination.lock.is_active() {
            Some(
                self.recs
                    .iter()
                    .position(|r| {
                        r.intent.status == IntentStatus::Queued
                            && r.intent.lock_owner().holds(&p.coordination.lock)
                    })
                    .unwrap_or(usize::MAX),
            )
        } else {
            None
        };
        let balances = if p.status == PositionStatus::Active {
            self.decode().map(|b| [b[0], b[1]])
        } else {
            None
        };
        Snap {
            slot: self.slot,
            status: p.status,
            key: if uniform { p.viewing_pubkey[0] } else { 0xFF },
            epoch: p.viewing_key_epoch,
            generation: p.coordination.generation,
            lock,
            balances,
            intents: self
                .recs
                .iter()
                .map(|r| (r.intent.kind, r.intent.status))
                .collect(),
            shields: self
                .shields
                .iter()
                .map(|s| {
                    (
                        s.shield.asset,
                        s.shield.amount,
                        s.shield.status,
                        s.shield.attempt_sequence,
                        s.active,
                    )
                })
                .collect(),
            vault: self.vault,
            wallet: self.wallet,
            out: self.out,
            blocked: (
                is_paused(&self.pauses, PauseCategory::Shield, self.slot),
                is_paused(&self.pauses, PauseCategory::Unshield, self.slot),
            ),
            next_shield_seq: p.next_shield_seq,
            next_sequence: self.counter.next_sequence,
        }
    }

    fn flag(&mut self, message: String) {
        self.violations.push(message);
    }

    fn move_to_vault(&mut self, asset: ShieldAsset, amount: u64) {
        let a = asset_index(asset);
        let amount = u128::from(amount);
        match self.wallet[a].checked_sub(amount) {
            Some(rest) => self.wallet[a] = rest,
            None => self.flag("VAULT CONSERVATION: wallet underflow".into()),
        }
        self.vault[a] += amount;
    }

    fn move_from_vault(&mut self, asset: ShieldAsset, amount: u64) {
        let a = asset_index(asset);
        let amount = u128::from(amount);
        match self.vault[a].checked_sub(amount) {
            Some(rest) => self.vault[a] = rest,
            None => self.flag(format!(
                "VAULT CONSERVATION: asset {a}: transfer of {amount} exceeds vault {}",
                self.vault[a]
            )),
        }
        self.wallet[a] += amount;
    }

    // -- circuit simulations ---------------------------------------------------

    fn shield_output(&self, sequence: u64, asset: ShieldAsset, amount: u64) -> Out {
        let old = self.plain();
        let a = asset_index(asset);
        let (new, credited) = match old[a].checked_add(amount) {
            Some(sum) => {
                let mut values = old;
                values[a] = sum;
                (values, true)
            }
            None => (old, false),
        };
        let viewing = self.position.viewing_pubkey;
        let nonce = output_nonce(&POSITION_KEY, sequence);
        Out {
            key: viewing,
            nonce,
            cts: seal(&viewing, nonce, new),
            credited,
            approved: 0,
        }
    }

    fn unshield_output(&self, sequence: u64, asset: ShieldAsset, amount: u64) -> Out {
        let old = self.plain();
        let a = asset_index(asset);
        let approved = if amount <= old[a] { amount } else { 0 };
        let mut new = old;
        new[a] -= approved;
        let viewing = self.position.viewing_pubkey;
        let nonce = output_nonce(&POSITION_KEY, sequence);
        Out {
            key: viewing,
            nonce,
            cts: seal(&viewing, nonce, new),
            credited: false,
            approved,
        }
    }

    fn rotation_output(&self, sequence: u64, new_key: u8) -> Out {
        let viewing = [new_key; 32];
        let nonce = output_nonce(&POSITION_KEY, sequence);
        Out {
            key: viewing,
            nonce,
            cts: seal(&viewing, nonce, self.plain()),
            credited: false,
            approved: 0,
        }
    }

    // -- queue paths -----------------------------------------------------------

    fn init_request(&self) -> PositionInitRequest {
        let sequence = self.counter.next_sequence;
        PositionInitRequest {
            intent_key: intent_key(sequence),
            position_key: POSITION_KEY,
            sequence,
            computation_offset: offset_for(sequence),
            // The client-chosen input nonce: distinct per attempt.
            nonce: 0x1000_0000 + 16 * u128::from(self.init_count + 1),
            queued_slot: self.slot,
        }
    }

    fn init_output(&self, request: &PositionInitRequest) -> Out {
        let viewing = self.position.viewing_pubkey;
        let nonce = request.nonce + 1;
        Out {
            key: viewing,
            nonce,
            cts: seal(&viewing, nonce, [0; 5]),
            credited: false,
            approved: 0,
        }
    }

    fn push_rec(&mut self, intent: OperationIntent, shield: Option<usize>, out: Out) {
        let (asset, amount) = match shield {
            Some(s) => (self.shields[s].shield.asset, self.shields[s].shield.amount),
            None => (ShieldAsset::Sol, 0),
        };
        self.recs.push(Rec {
            intent,
            shield,
            asset,
            amount,
            out,
        });
    }

    fn init_position(&mut self) -> Outcome {
        if self.position.status != PositionStatus::Uninitialized {
            return Outcome::Skipped;
        }
        let request = self.init_request();
        let new = NewPosition {
            owner: OWNER,
            market: MARKET_KEY,
            bump: 0xFC,
            viewing_pubkey: [KEY_INITIAL; 32],
        };
        match initialize_position(
            &mut self.market,
            &mut self.position,
            &mut self.counter,
            &new,
            &request,
        ) {
            Ok(done) => {
                self.init_count += 1;
                let out = self.init_output(&request);
                self.push_rec(done.intent, None, out);
                Outcome::Done
            }
            Err(error) => Outcome::Rejected(why(error)),
        }
    }

    fn retry_init(&mut self) -> Outcome {
        let request = self.init_request();
        match retry_position_init(&mut self.position, &mut self.counter, &request) {
            Ok(intent) => {
                self.init_count += 1;
                let out = self.init_output(&request);
                self.push_rec(intent, None, out);
                Outcome::Done
            }
            Err(error) => Outcome::Rejected(why(error)),
        }
    }

    fn shield_request(&self) -> ShieldRequest {
        let sequence = self.counter.next_sequence;
        ShieldRequest {
            intent_key: intent_key(sequence),
            sequence,
            computation_offset: offset_for(sequence),
            queued_slot: self.slot,
        }
    }

    fn new_shield(&self, asset: ShieldAsset, amount: u64) -> NewPendingShield {
        NewPendingShield {
            owner: OWNER,
            position: POSITION_KEY,
            source_token_account: SOURCE,
            asset,
            amount,
            sequence: self.position.next_shield_seq,
            bump: 0xFB,
        }
    }

    fn shield(&mut self, asset: ShieldAsset, amount: u64) -> Outcome {
        let request = self.shield_request();
        let new = self.new_shield(asset, amount);
        let mut shield = PendingShield::default();
        let result = queue_shield(
            &self.market,
            MARKET_KEY,
            &self.pauses,
            &mut self.position,
            POSITION_KEY,
            &mut self.counter,
            &mut shield,
            shield_key(new.sequence),
            &new,
            &request,
        );
        match result {
            Ok(intent) => {
                // The handler transfers into the vault, then queues. The
                // transfer is part of the same transaction.
                self.move_to_vault(asset, amount);
                let out = self.shield_output(request.sequence, asset, amount);
                self.shields.push(RShield {
                    shield,
                    active: self.recs.len(),
                });
                let index = self.shields.len() - 1;
                self.push_rec(intent, Some(index), out);
                Outcome::Done
            }
            Err(error) => Outcome::Rejected(why(error)),
        }
    }

    fn shield_overwrite(&mut self, sel: Sel) -> Outcome {
        let Some(i) = pick_sel(sel, self.shields.len()) else {
            return Outcome::Skipped;
        };
        let request = self.shield_request();
        let new = self.new_shield(ShieldAsset::Sol, 5);
        let mut occupied = self.shields[i].shield;
        let result = queue_shield(
            &self.market,
            MARKET_KEY,
            &self.pauses,
            &mut self.position,
            POSITION_KEY,
            &mut self.counter,
            &mut occupied,
            shield_key(self.shields[i].shield.sequence),
            &new,
            &request,
        );
        match result {
            Ok(_) => {
                self.flag(
                    "PENDING SHIELD EXCLUSIVITY: a shield overwrote an occupied account".into(),
                );
                Outcome::Done
            }
            Err(error) => {
                // A rejected queue leaves the account unchanged.
                self.shields[i].shield = occupied;
                Outcome::Rejected(why(error))
            }
        }
    }

    fn retry_shield(&mut self, sel: Sel) -> Outcome {
        let Some(i) = pick_sel(sel, self.shields.len()) else {
            return Outcome::Skipped;
        };
        let request = self.shield_request();
        let prior = self.recs[self.shields[i].active].intent;
        let mut shield = self.shields[i].shield;
        let shield_address = shield_key(shield.sequence);
        let result = retry_shield(
            &self.market,
            MARKET_KEY,
            &mut self.position,
            POSITION_KEY,
            &mut self.counter,
            &mut shield,
            shield_address,
            &OWNER,
            &prior,
            &request,
        );
        match result {
            Ok((intent, attempt)) => {
                if attempt != shield.attempt_sequence {
                    self.flag("PENDING SHIELD EXCLUSIVITY: attempt sequence mismatch".into());
                }
                let out = self.shield_output(request.sequence, shield.asset, shield.amount);
                self.shields[i].shield = shield;
                self.shields[i].active = self.recs.len();
                self.push_rec(intent, Some(i), out);
                Outcome::Done
            }
            Err(error) => {
                // Atomic: the Pending Shield is unchanged on error.
                if shield != self.shields[i].shield {
                    self.flag(
                        "PENDING SHIELD EXCLUSIVITY: a rejected retry changed the shield".into(),
                    );
                }
                Outcome::Rejected(why(error))
            }
        }
    }

    fn refund_shield(&mut self, sel: Sel) -> Outcome {
        let Some(i) = pick_sel(sel, self.shields.len()) else {
            return Outcome::Skipped;
        };
        if self.fault == Fault::RefundBlockedByPause
            && is_paused(&self.pauses, PauseCategory::Shield, self.slot)
        {
            return Outcome::Rejected(Why::Paused);
        }
        let active = self.recs[self.shields[i].active].intent;
        let asset = self.shields[i].shield.asset;
        let mint = asset_mint(&self.market, asset);
        let vault = asset_vault(&self.market, asset);
        let destination = TokenAccountView {
            key: SOURCE,
            mint,
            owner: OWNER,
        };
        let mut shield = self.shields[i].shield;
        match refund_shield(
            &self.market,
            &mut shield,
            &OWNER,
            &active,
            &mint,
            &vault,
            &destination,
        ) {
            Ok(refund) => {
                if refund.destination != SOURCE || refund.asset != asset {
                    self.flag("VAULT CONSERVATION: refund went to the wrong account".into());
                }
                self.shields[i].shield = shield;
                if self.fault == Fault::RefundKeepsVault {
                    self.wallet[asset_index(asset)] += u128::from(refund.amount);
                } else {
                    self.move_from_vault(asset, refund.amount);
                }
                if self.fault == Fault::RefundFlipsToCredited {
                    self.shields[i].shield.status = PendingShieldStatus::Credited;
                }
                Outcome::Refund(refund.amount)
            }
            Err(error) => Outcome::Rejected(why(error)),
        }
    }

    fn unshield(&mut self, asset: ShieldAsset, amount: u64) -> Outcome {
        let sequence = self.counter.next_sequence;
        let request = UnshieldRequest {
            intent_key: intent_key(sequence),
            sequence,
            computation_offset: offset_for(sequence),
            queued_slot: self.slot,
            asset,
            amount,
            destination: DEST,
        };
        let destination = TokenAccountView {
            key: DEST,
            mint: asset_mint(&self.market, asset),
            owner: OWNER,
        };
        match queue_unshield(
            &self.market,
            MARKET_KEY,
            &self.pauses,
            &mut self.position,
            POSITION_KEY,
            &mut self.counter,
            &OWNER,
            &destination,
            &request,
        ) {
            Ok(intent) => {
                let out = self.unshield_output(sequence, asset, amount);
                self.recs.push(Rec {
                    intent,
                    shield: None,
                    asset,
                    amount,
                    out,
                });
                Outcome::Done
            }
            Err(error) => Outcome::Rejected(why(error)),
        }
    }

    fn rotate(&mut self, new_key: u8, stale: bool) -> Outcome {
        let sequence = self.counter.next_sequence;
        let epoch = self.position.viewing_key_epoch;
        let skew = u64::from(stale);
        let new_key = if stale { 0x77 } else { new_key };
        let request = RotationRequest {
            intent_key: intent_key(sequence),
            sequence,
            computation_offset: offset_for(sequence),
            queued_slot: self.slot,
            expected_old_epoch: epoch + skew,
            expected_new_epoch: epoch + 1 + skew,
            expected_old_pubkey: self.position.viewing_pubkey,
            new_viewing_pubkey: [new_key; 32],
        };
        match queue_rotation(
            &mut self.position,
            POSITION_KEY,
            &mut self.counter,
            &OWNER,
            &request,
        ) {
            Ok(intent) => {
                let out = self.rotation_output(sequence, new_key);
                self.recs.push(Rec {
                    intent,
                    shield: None,
                    asset: ShieldAsset::Sol,
                    amount: 0,
                    out,
                });
                Outcome::Done
            }
            Err(error) => Outcome::Rejected(why(error)),
        }
    }

    // -- callbacks and abandonment ----------------------------------------------

    fn deliver(&mut self, target: Target, variant: Variant) -> Outcome {
        let Some(i) = pick(target, self.recs.len()) else {
            return Outcome::Skipped;
        };
        self.target = Some(i);
        let rec = self.recs[i];
        let mut intent = rec.intent;
        let authenticated = variant != Variant::Unauth;
        let offset = intent.computation_offset + u64::from(variant == Variant::WrongOffset);
        let encryption_key = if variant == Variant::WrongKey {
            [0xEE; 32]
        } else {
            rec.out.key
        };
        let nonce = rec.out.nonce + if variant == Variant::WrongNonce { 7 } else { 0 };
        let slot = self.slot;
        let outcome = match intent.kind {
            OperationKind::PositionInit => {
                let callback = PositionInitCallback {
                    output_authenticated: authenticated,
                    computation_offset: offset,
                    intent_key: intent.key,
                    encryption_key,
                    output_nonce: nonce,
                    ciphertexts: rec.out.cts,
                    current_slot: slot,
                };
                match settle_position_init(&mut self.position, POSITION_KEY, &mut intent, &callback)
                {
                    Ok(PositionInitSettlement::Activated) => Outcome::Applied,
                    Ok(PositionInitSettlement::Failed(status)) => status_outcome(status),
                    Ok(PositionInitSettlement::Ignored) => Outcome::Ignored,
                    Err(error) => Outcome::Rejected(callback_why(error)),
                }
            }
            OperationKind::Shield => {
                let s = rec.shield.expect("a shield Intent has a Pending Shield");
                let mut shield = self.shields[s].shield;
                let shield_address = shield_key(shield.sequence);
                let callback = ShieldCallback {
                    output_authenticated: authenticated,
                    computation_offset: offset,
                    intent_key: intent.key,
                    encryption_key,
                    output_nonce: nonce,
                    ciphertexts: rec.out.cts,
                    credited: rec.out.credited,
                    current_slot: slot,
                };
                let result = settle_shield_credit(
                    &self.market,
                    MARKET_KEY,
                    &mut self.position,
                    POSITION_KEY,
                    &mut shield,
                    shield_address,
                    &mut intent,
                    &callback,
                );
                self.shields[s].shield = shield;
                match result {
                    Ok(ShieldSettlement::Credited { asset, amount }) => {
                        if asset != rec.asset || amount != rec.amount {
                            self.flag(
                                "PENDING SHIELD EXCLUSIVITY: credited the wrong value".into(),
                            );
                        }
                        Outcome::Applied
                    }
                    Ok(ShieldSettlement::Failed(status)) => status_outcome(status),
                    Ok(ShieldSettlement::Ignored) => Outcome::Ignored,
                    Err(error) => Outcome::Rejected(callback_why(error)),
                }
            }
            OperationKind::Unshield => {
                let report = UnshieldReport {
                    output_authenticated: authenticated,
                    computation_offset: offset,
                    intent_key: intent.key,
                    encryption_key,
                    output_nonce: nonce,
                    ciphertexts: rec.out.cts,
                    approved_amount: rec.out.approved,
                    requested_amount: rec.amount,
                    destination: TokenAccountView {
                        key: DEST,
                        mint: asset_mint(&self.market, rec.asset),
                        owner: OWNER,
                    },
                    mint: asset_mint(&self.market, rec.asset),
                    vault: asset_vault(&self.market, rec.asset),
                    vault_authority: self.market.vault_authority,
                    token_program: anchor_spl::token::ID,
                    current_slot: slot,
                };
                match settle_unshield(
                    &self.market,
                    MARKET_KEY,
                    &mut self.position,
                    POSITION_KEY,
                    &mut intent,
                    &report,
                ) {
                    Ok(UnshieldSettlement::Approved {
                        asset,
                        amount,
                        destination,
                    }) => {
                        if destination != DEST || asset != rec.asset {
                            self.flag("VAULT CONSERVATION: unshield to the wrong account".into());
                        }
                        // The handler transfers exactly the approved amount.
                        self.move_from_vault(asset, amount);
                        self.out[asset_index(asset)] += u128::from(amount);
                        Outcome::Approved(amount)
                    }
                    Ok(UnshieldSettlement::Zero { .. }) => Outcome::Zero,
                    Ok(UnshieldSettlement::Failed(status)) => status_outcome(status),
                    Ok(UnshieldSettlement::Ignored) => Outcome::Ignored,
                    Err(error) => Outcome::Rejected(callback_why(error)),
                }
            }
            OperationKind::ViewingKeyRotation => {
                let report = RotationReport {
                    output_authenticated: authenticated,
                    computation_offset: offset,
                    intent_key: intent.key,
                    encryption_key,
                    output_nonce: nonce,
                    ciphertexts: rec.out.cts,
                    current_slot: slot,
                };
                match settle_rotation(&mut self.position, POSITION_KEY, &mut intent, &report) {
                    Ok(RotationSettlement::Rotated { .. }) => Outcome::Applied,
                    Ok(RotationSettlement::Failed(status)) => status_outcome(status),
                    Ok(RotationSettlement::Ignored) => Outcome::Ignored,
                    Err(error) => Outcome::Rejected(callback_why(error)),
                }
            }
            other => panic!("unexpected Intent kind {other:?}"),
        };
        self.recs[i].intent = intent;
        outcome
    }

    fn abandon(&mut self) -> Outcome {
        let Some(i) = self.recs.len().checked_sub(1) else {
            return Outcome::Skipped;
        };
        let rec = self.recs[i];
        let mut intent = rec.intent;
        let slot = self.slot;
        let result = match intent.kind {
            OperationKind::PositionInit => {
                abandon_position_init(&mut self.position, POSITION_KEY, &mut intent, slot)
            }
            OperationKind::Shield => {
                let s = rec.shield.expect("a shield Intent has a Pending Shield");
                abandon_shield(
                    &mut self.position,
                    POSITION_KEY,
                    &self.shields[s].shield,
                    &mut intent,
                    slot,
                )
            }
            OperationKind::Unshield => {
                abandon_unshield(&mut self.position, POSITION_KEY, &mut intent, slot)
            }
            OperationKind::ViewingKeyRotation => {
                abandon_rotation(&mut self.position, POSITION_KEY, &mut intent, slot)
            }
            other => panic!("unexpected Intent kind {other:?}"),
        };
        self.recs[i].intent = intent;
        match result {
            Ok(_) => Outcome::Abandoned,
            Err(error) => Outcome::Rejected(why(error)),
        }
    }

    // -- pause and clock ---------------------------------------------------------

    fn durable(&mut self, flags: u16, paused: bool) -> Outcome {
        match apply_durable_pause(&mut self.pauses, flags, paused) {
            Ok(()) => Outcome::Done,
            Err(error) => Outcome::Rejected(market_why(error)),
        }
    }

    fn step(&mut self, act: &Act) -> Outcome {
        match *act {
            Act::InitPosition => self.init_position(),
            Act::RetryInit => self.retry_init(),
            Act::Shield(asset, amount) => self.shield(asset, amount),
            Act::ShieldOverwrite(sel) => self.shield_overwrite(sel),
            Act::RetryShield(sel) => self.retry_shield(sel),
            Act::RefundShield(sel) => self.refund_shield(sel),
            Act::Unshield(asset, UAmt::Exact(amount)) => self.unshield(asset, amount),
            Act::Unshield(..) => panic!("unresolved unshield amount"),
            Act::Rotate(k) => self.rotate(k, false),
            Act::RotateStale => self.rotate(0x77, true),
            Act::Callback(target, variant) => {
                let outcome = self.deliver(target, variant);
                self.after_callback(outcome);
                outcome
            }
            Act::Abandon => self.abandon(),
            Act::PauseShield => self.durable(pause_flags::SHIELD, true),
            Act::UnpauseShield => self.durable(pause_flags::SHIELD, false),
            Act::PauseUnshield => self.durable(pause_flags::UNSHIELD, true),
            Act::UnpauseUnshield => self.durable(pause_flags::UNSHIELD, false),
            Act::GuardianPause(flags, duration) => {
                match apply_guardian_pause(&mut self.pauses, flags, duration, self.slot) {
                    Ok(()) => Outcome::Done,
                    Err(error) => Outcome::Rejected(market_why(error)),
                }
            }
            Act::GuardianClear => match apply_guardian_clear(&mut self.pauses) {
                Ok(()) => Outcome::Done,
                Err(error) => Outcome::Rejected(market_why(error)),
            },
            Act::Advance(slots) => {
                self.slot += slots;
                Outcome::Done
            }
            Act::ExternalBump => {
                if self.position.status == PositionStatus::Uninitialized {
                    return Outcome::Skipped;
                }
                self.position.coordination.generation += 1;
                Outcome::Done
            }
        }
    }

    /// Fault injections that follow a callback.
    fn after_callback(&mut self, outcome: Outcome) {
        match (self.fault, outcome) {
            (Fault::ZeroWrites, Outcome::Zero) | (Fault::IgnoredWrites, Outcome::Ignored) => {
                self.corrupt(|values| values[0] = values[0].wrapping_add(1));
            }
            (Fault::OversizedDrains, Outcome::Zero) => {
                if self.target.map(|t| self.recs[t].intent.kind) == Some(OperationKind::Unshield) {
                    self.corrupt(|values| values[0] = 0);
                }
            }
            (Fault::RotationKeepsEpoch, Outcome::Applied) => {
                if self.target.map(|t| self.recs[t].intent.kind)
                    == Some(OperationKind::ViewingKeyRotation)
                {
                    self.position.viewing_key_epoch -= 1;
                }
            }
            (Fault::RotationSwapsKeyOnly, Outcome::Applied) => {
                if self.target.map(|t| self.recs[t].intent.kind)
                    == Some(OperationKind::ViewingKeyRotation)
                {
                    self.position.viewing_pubkey = [0x09; 32];
                }
            }
            (Fault::LateCallbackMovesTokens, Outcome::Ignored) => {
                self.vault[0] += 1;
            }
            _ => {}
        }
    }
}

// ---- The reference model -------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct MIntent {
    kind: OperationKind,
    status: IntentStatus,
    generation: u64,
    decision_expiry: u64,
    lock_expiry: u64,
    shield: Option<usize>,
    asset: ShieldAsset,
    amount: u64,
    new_key: u8,
}

#[derive(Clone, Copy, Debug)]
struct MShield {
    asset: ShieldAsset,
    amount: u64,
    status: PendingShieldStatus,
    attempt: u64,
    active: usize,
}

/// The naive plaintext reference. It never calls a production function.
#[derive(Clone, Debug)]
struct Model {
    slot: u64,
    generation: u64,
    status: PositionStatus,
    key: u8,
    epoch: u64,
    free: [u64; 2],
    lock: Option<usize>,
    intents: Vec<MIntent>,
    shields: Vec<MShield>,
    vault: [u128; 2],
    wallet: [u128; 2],
    out: [u128; 2],
    durable: u16,
    guardian: u16,
    guardian_expiry: u64,
}

impl Model {
    fn new() -> Self {
        Self {
            slot: 10,
            generation: 0,
            status: PositionStatus::Uninitialized,
            key: 0,
            epoch: 0,
            free: [0; 2],
            lock: None,
            intents: Vec::new(),
            shields: Vec::new(),
            vault: [0; 2],
            wallet: [WALLET_START; 2],
            out: [0; 2],
            durable: 0,
            guardian: 0,
            guardian_expiry: 0,
        }
    }

    fn blocked(&self, bit: u16) -> bool {
        let guardian = if self.guardian != 0 && self.slot < self.guardian_expiry {
            self.guardian
        } else {
            0
        };
        (self.durable | guardian) & bit != 0
    }

    fn queue(
        &mut self,
        kind: OperationKind,
        shield: Option<usize>,
        asset: ShieldAsset,
        amount: u64,
        new_key: u8,
    ) -> usize {
        self.intents.push(MIntent {
            kind,
            status: IntentStatus::Queued,
            generation: self.generation,
            decision_expiry: self.slot + MODEL_DECISION,
            lock_expiry: self.slot + MODEL_DECISION + MODEL_GRACE,
            shield,
            asset,
            amount,
            new_key,
        });
        let index = self.intents.len() - 1;
        self.lock = Some(index);
        index
    }

    fn active(&self, s: usize) -> &MIntent {
        &self.intents[self.shields[s].active]
    }

    fn step(&mut self, act: &Act) -> Outcome {
        match *act {
            Act::InitPosition => {
                if self.status != PositionStatus::Uninitialized {
                    return Outcome::Skipped;
                }
                self.status = PositionStatus::Initializing;
                self.key = KEY_INITIAL;
                self.generation = 0;
                self.queue(OperationKind::PositionInit, None, ShieldAsset::Sol, 0, 0);
                Outcome::Done
            }
            Act::RetryInit => {
                if self.status == PositionStatus::InitFailed && self.lock.is_none() {
                    self.status = PositionStatus::Initializing;
                    self.queue(OperationKind::PositionInit, None, ShieldAsset::Sol, 0, 0);
                    Outcome::Done
                } else {
                    Outcome::Rejected(Why::Invalid)
                }
            }
            Act::Shield(asset, amount) => {
                if self.blocked(pause_flags::SHIELD) {
                    return Outcome::Rejected(Why::Paused);
                }
                if amount == 0 || self.status != PositionStatus::Active || self.lock.is_some() {
                    return Outcome::Rejected(Why::Invalid);
                }
                let a = asset_index(asset);
                self.wallet[a] -= u128::from(amount);
                self.vault[a] += u128::from(amount);
                let index = self.shields.len();
                let intent = self.queue(OperationKind::Shield, Some(index), asset, amount, 0);
                self.shields.push(MShield {
                    asset,
                    amount,
                    status: PendingShieldStatus::Deposited,
                    attempt: 1,
                    active: intent,
                });
                Outcome::Done
            }
            Act::ShieldOverwrite(sel) => {
                if pick_sel(sel, self.shields.len()).is_none() {
                    return Outcome::Skipped;
                }
                if self.blocked(pause_flags::SHIELD) {
                    Outcome::Rejected(Why::Paused)
                } else {
                    Outcome::Rejected(Why::Invalid)
                }
            }
            Act::RetryShield(sel) => {
                let Some(s) = pick_sel(sel, self.shields.len()) else {
                    return Outcome::Skipped;
                };
                let usable = self.shields[s].status == PendingShieldStatus::Deposited
                    && self.active(s).status != IntentStatus::Queued
                    && self.status == PositionStatus::Active
                    && self.lock.is_none();
                if !usable {
                    return Outcome::Rejected(Why::Invalid);
                }
                let (asset, amount) = (self.shields[s].asset, self.shields[s].amount);
                let intent = self.queue(OperationKind::Shield, Some(s), asset, amount, 0);
                self.shields[s].attempt += 1;
                self.shields[s].active = intent;
                Outcome::Done
            }
            Act::RefundShield(sel) => {
                let Some(s) = pick_sel(sel, self.shields.len()) else {
                    return Outcome::Skipped;
                };
                let usable = self.shields[s].status == PendingShieldStatus::Deposited
                    && self.active(s).status != IntentStatus::Queued;
                if !usable {
                    return Outcome::Rejected(Why::Invalid);
                }
                let (asset, amount) = (self.shields[s].asset, self.shields[s].amount);
                let a = asset_index(asset);
                self.shields[s].status = PendingShieldStatus::Refunded;
                self.vault[a] -= u128::from(amount);
                self.wallet[a] += u128::from(amount);
                Outcome::Refund(amount)
            }
            Act::Unshield(asset, UAmt::Exact(amount)) => {
                if self.blocked(pause_flags::UNSHIELD) {
                    return Outcome::Rejected(Why::Paused);
                }
                if amount == 0 || self.status != PositionStatus::Active || self.lock.is_some() {
                    return Outcome::Rejected(Why::Invalid);
                }
                self.queue(OperationKind::Unshield, None, asset, amount, 0);
                Outcome::Done
            }
            Act::Unshield(..) => panic!("unresolved unshield amount"),
            Act::Rotate(k) => {
                if self.status != PositionStatus::Active
                    || self.lock.is_some()
                    || k == 0
                    || k == self.key
                {
                    return Outcome::Rejected(Why::Invalid);
                }
                self.queue(
                    OperationKind::ViewingKeyRotation,
                    None,
                    ShieldAsset::Sol,
                    0,
                    k,
                );
                Outcome::Done
            }
            Act::RotateStale => Outcome::Rejected(Why::Invalid),
            Act::Callback(target, variant) => self.callback(target, variant),
            Act::Abandon => {
                let Some(i) = self.intents.len().checked_sub(1) else {
                    return Outcome::Skipped;
                };
                let intent = self.intents[i];
                if intent.status != IntentStatus::Queued || self.slot <= intent.lock_expiry {
                    return Outcome::Rejected(Why::Invalid);
                }
                self.intents[i].status = IntentStatus::Abandoned;
                self.generation += 1;
                self.lock = None;
                if intent.kind == OperationKind::PositionInit {
                    self.status = PositionStatus::InitFailed;
                }
                Outcome::Abandoned
            }
            Act::PauseShield => {
                self.durable |= pause_flags::SHIELD;
                Outcome::Done
            }
            Act::UnpauseShield => {
                self.durable &= !pause_flags::SHIELD;
                Outcome::Done
            }
            Act::PauseUnshield => {
                self.durable |= pause_flags::UNSHIELD;
                Outcome::Done
            }
            Act::UnpauseUnshield => {
                self.durable &= !pause_flags::UNSHIELD;
                Outcome::Done
            }
            Act::GuardianPause(flags, duration) => {
                let active = self.guardian != 0 && self.slot < self.guardian_expiry;
                if active {
                    self.guardian |= flags;
                    Outcome::Done
                } else if self.guardian_expiry != 0
                    && self.slot < self.guardian_expiry + MODEL_COOLDOWN
                {
                    Outcome::Rejected(Why::Cooldown)
                } else {
                    self.guardian = flags;
                    self.guardian_expiry = self.slot + duration;
                    Outcome::Done
                }
            }
            Act::GuardianClear => {
                if self.guardian == 0 && self.guardian_expiry == 0 {
                    Outcome::Rejected(Why::Invalid)
                } else {
                    self.guardian = 0;
                    self.guardian_expiry = 0;
                    Outcome::Done
                }
            }
            Act::Advance(slots) => {
                self.slot += slots;
                Outcome::Done
            }
            Act::ExternalBump => {
                if self.status == PositionStatus::Uninitialized {
                    return Outcome::Skipped;
                }
                self.generation += 1;
                Outcome::Done
            }
        }
    }

    fn callback(&mut self, target: Target, variant: Variant) -> Outcome {
        let Some(i) = pick(target, self.intents.len()) else {
            return Outcome::Skipped;
        };
        if variant == Variant::Unauth {
            return Outcome::Rejected(Why::Unauth);
        }
        let intent = self.intents[i];
        if intent.status != IntentStatus::Queued {
            return Outcome::Ignored;
        }
        if variant != Variant::Honest {
            return Outcome::Rejected(Why::Binding);
        }
        // Dependencies are checked before Decision Expiry, then the circuit's
        // answer decides between `Applied` and `Zero`.
        let status = if intent.generation != self.generation {
            IntentStatus::Stale
        } else if self.slot > intent.decision_expiry {
            IntentStatus::Expired
        } else {
            let a = asset_index(intent.asset);
            let approves = match intent.kind {
                OperationKind::Shield => self.free[a].checked_add(intent.amount).is_some(),
                OperationKind::Unshield => intent.amount <= self.free[a],
                _ => true,
            };
            if approves {
                IntentStatus::Applied
            } else {
                IntentStatus::Zero
            }
        };
        self.intents[i].status = status;
        self.lock = None;
        match status {
            IntentStatus::Stale | IntentStatus::Expired | IntentStatus::Zero => {
                if intent.kind == OperationKind::PositionInit && status != IntentStatus::Zero {
                    self.status = PositionStatus::InitFailed;
                }
                match status {
                    IntentStatus::Stale => Outcome::Stale,
                    IntentStatus::Expired => Outcome::Expired,
                    _ => Outcome::Zero,
                }
            }
            _ => {
                self.generation += 1;
                let a = asset_index(intent.asset);
                match intent.kind {
                    OperationKind::PositionInit => {
                        self.status = PositionStatus::Active;
                        Outcome::Applied
                    }
                    OperationKind::Shield => {
                        let s = intent.shield.expect("shield");
                        self.free[a] += intent.amount;
                        self.shields[s].status = PendingShieldStatus::Credited;
                        Outcome::Applied
                    }
                    OperationKind::Unshield => {
                        self.free[a] -= intent.amount;
                        self.vault[a] -= u128::from(intent.amount);
                        self.wallet[a] += u128::from(intent.amount);
                        self.out[a] += u128::from(intent.amount);
                        Outcome::Approved(intent.amount)
                    }
                    OperationKind::ViewingKeyRotation => {
                        self.key = intent.new_key;
                        self.epoch += 1;
                        Outcome::Applied
                    }
                    other => panic!("unexpected Intent kind {other:?}"),
                }
            }
        }
    }

    fn snap(&self) -> Snap {
        Snap {
            slot: self.slot,
            status: self.status,
            key: self.key,
            epoch: self.epoch,
            generation: self.generation,
            lock: self.lock,
            balances: (self.status == PositionStatus::Active).then_some(self.free),
            intents: self.intents.iter().map(|i| (i.kind, i.status)).collect(),
            shields: self
                .shields
                .iter()
                .map(|s| (s.asset, s.amount, s.status, s.attempt, s.active))
                .collect(),
            vault: self.vault,
            wallet: self.wallet,
            out: self.out,
            blocked: (
                self.blocked(pause_flags::SHIELD),
                self.blocked(pause_flags::UNSHIELD),
            ),
            next_shield_seq: self.shields.len() as u64,
            next_sequence: self.intents.len() as u64,
        }
    }
}

// ---- The harness and the invariants ----------------------------------------------

/// Counts of every outcome seen, for the coverage guards.
#[derive(Clone, Debug, Default)]
struct Coverage {
    counts: std::collections::BTreeMap<String, u64>,
}

impl Coverage {
    fn add(&mut self, outcome: Outcome) {
        let name = match outcome {
            Outcome::Refund(_) => "Refund".to_string(),
            Outcome::Approved(_) => "Approved".to_string(),
            other => format!("{other:?}"),
        };
        *self.counts.entry(name).or_default() += 1;
    }

    fn get(&self, name: &str) -> u64 {
        self.counts.get(name).copied().unwrap_or(0)
    }
}

#[derive(Clone)]
struct Harness {
    real: Real,
    model: Model,
    nonces: BTreeSet<([u8; 32], u128)>,
}

impl Harness {
    fn new(fault: Fault) -> Self {
        Self {
            real: Real::new(fault),
            model: Model::new(),
            nonces: BTreeSet::new(),
        }
    }

    fn resolve(&self, act: Act) -> Act {
        match act {
            Act::Unshield(asset, amount) => {
                let balance = self.model.free[asset_index(asset)];
                Act::Unshield(asset, UAmt::Exact(resolve_amount(amount, balance)))
            }
            other => other,
        }
    }

    /// Runs one action on both sides and checks everything.
    fn step(&mut self, act: Act) -> Result<Outcome, String> {
        let act = self.resolve(act);
        let before = self.real.fin();
        let known = self.real.recs.len();
        self.real.target = None;
        let outcome = self.real.step(&act);
        if let Some(message) = self.real.violations.first() {
            return Err(format!("{message} (after {act:?})"));
        }
        let after = self.real.fin();
        self.check_nonces(known)
            .map_err(|m| format!("{m} (after {act:?})"))?;
        self.check_exclusivity(&act, outcome, &before, &after)
            .and_then(|()| self.check_vault())
            .and_then(|()| self.check_oversized(outcome, &before, &after))
            .and_then(|()| self.check_neutrality(&act, outcome, &before, &after))
            .and_then(|()| self.check_backing(&after))
            .and_then(|()| self.check_rotation(&act, outcome, &before, &after))
            .and_then(|()| self.check_pause(&act, outcome))
            .and_then(|()| self.check_lock())
            .map_err(|m| format!("{m} (after {act:?}, outcome {outcome:?})"))?;
        let expected = self.model.step(&act);
        if expected != outcome {
            return Err(format!(
                "AGREEMENT: {act:?}: production {outcome:?}, reference {expected:?}"
            ));
        }
        let (real, model) = (self.real.snap(), self.model.snap());
        if real != model {
            return Err(format!(
                "AGREEMENT: state differs after {act:?}:\n production {real:?}\n reference  {model:?}"
            ));
        }
        Ok(outcome)
    }

    fn check_nonces(&mut self, known: usize) -> Result<(), String> {
        for index in known..self.real.recs.len() {
            let out = self.real.recs[index].out;
            let nonce = if self.real.fault == Fault::NonceReuse {
                1
            } else {
                out.nonce
            };
            if !self.nonces.insert((out.key, nonce)) {
                return Err(format!(
                    "NONCE UNIQUENESS: output {index} repeats a (key, nonce) pair"
                ));
            }
        }
        Ok(())
    }

    /// Each Pending Shield ends credited XOR refunded XOR still `Deposited`.
    fn check_exclusivity(
        &self,
        act: &Act,
        outcome: Outcome,
        before: &Fin,
        after: &Fin,
    ) -> Result<(), String> {
        let name = "PENDING SHIELD EXCLUSIVITY";
        let shields = &self.real.shields;
        if shields.len() < before.shields.len()
            || shields.len() > before.shields.len() + 1
            || (shields.len() > before.shields.len()
                && (!matches!(act, Act::Shield(..)) || outcome != Outcome::Done))
        {
            return Err(format!(
                "{name}: the set of Pending Shields changed illegally"
            ));
        }
        let credited_target = match (act, outcome) {
            (Act::Callback(..), Outcome::Applied) => {
                self.real.target.and_then(|t| self.real.recs[t].shield)
            }
            _ => None,
        };
        for (i, old) in before.shields.iter().enumerate() {
            let new = &after.shields[i];
            let changed = old.0 != new.0;
            let allowed = !changed
                || (old.0 == PendingShieldStatus::Deposited
                    && new.0 == PendingShieldStatus::Credited
                    && credited_target == Some(i))
                || (old.0 == PendingShieldStatus::Deposited
                    && new.0 == PendingShieldStatus::Refunded
                    && matches!(act, Act::RefundShield(_))
                    && matches!(outcome, Outcome::Refund(_)));
            if !allowed {
                return Err(format!(
                    "{name}: shield {i} went {:?} -> {:?} on {act:?}",
                    old.0, new.0
                ));
            }
            if old.0.is_final() && (old != new) {
                return Err(format!("{name}: final shield {i} changed"));
            }
            if (old.1, old.2) != (new.1, new.2) {
                return Err(format!("{name}: shield {i} asset or amount changed"));
            }
        }
        // Sequences are unique and dense: a new shield takes the next one and
        // no account is overwritten.
        for (i, s) in shields.iter().enumerate() {
            if s.shield.sequence != i as u64
                || s.shield.status == PendingShieldStatus::Uninitialized
            {
                return Err(format!(
                    "{name}: shield {i} has sequence {}",
                    s.shield.sequence
                ));
            }
            if s.shield.amount == 0 {
                return Err(format!("{name}: shield {i} has a zero amount"));
            }
        }
        if self.real.position.next_shield_seq != shields.len() as u64 {
            return Err(format!("{name}: next_shield_seq is not the shield count"));
        }
        // At most one queued attempt per shield, and it is the active one.
        for (i, s) in shields.iter().enumerate() {
            let queued: Vec<usize> = self
                .real
                .recs
                .iter()
                .enumerate()
                .filter(|(_, r)| r.shield == Some(i) && r.intent.status == IntentStatus::Queued)
                .map(|(j, _)| j)
                .collect();
            if queued.len() > 1 || queued.first().is_some_and(|j| *j != s.active) {
                return Err(format!("{name}: shield {i} has queued attempts {queued:?}"));
            }
            if s.shield.attempt_sequence
                != self
                    .real
                    .recs
                    .iter()
                    .filter(|r| r.shield == Some(i))
                    .count() as u64
            {
                return Err(format!("{name}: shield {i} attempt count mismatch"));
            }
            // A Credited shield's active attempt is the one that applied.
            if s.shield.status == PendingShieldStatus::Credited
                && self.real.recs[s.active].intent.status != IntentStatus::Applied
            {
                return Err(format!(
                    "{name}: shield {i} credited without an Applied attempt"
                ));
            }
            if s.shield.status == PendingShieldStatus::Refunded
                && self.real.recs[s.active].intent.status == IntentStatus::Applied
            {
                return Err(format!(
                    "{name}: shield {i} refunded after an Applied attempt"
                ));
            }
        }
        Ok(())
    }

    /// For each asset the vault holds exactly the credited plus the deposited
    /// shields minus the approved unshields, and no token is created or lost.
    fn check_vault(&self) -> Result<(), String> {
        for asset in [ShieldAsset::Sol, ShieldAsset::Usdc] {
            let a = asset_index(asset);
            let (mut credited, mut deposited) = (0u128, 0u128);
            for s in &self.real.shields {
                if s.shield.asset == asset {
                    match s.shield.status {
                        PendingShieldStatus::Credited => credited += u128::from(s.shield.amount),
                        PendingShieldStatus::Deposited => deposited += u128::from(s.shield.amount),
                        _ => {}
                    }
                }
            }
            let expected = (credited + deposited).checked_sub(self.real.out[a]);
            if expected != Some(self.real.vault[a]) {
                return Err(format!(
                    "VAULT CONSERVATION: asset {a}: vault {} != credited {credited} + deposited {deposited} - unshielded {}",
                    self.real.vault[a], self.real.out[a]
                ));
            }
            if self.real.wallet[a] + self.real.vault[a] != WALLET_START {
                return Err(format!(
                    "VAULT CONSERVATION: asset {a}: tokens were created or destroyed (wallet {} + vault {})",
                    self.real.wallet[a], self.real.vault[a]
                ));
            }
        }
        Ok(())
    }

    /// An unshield above the free balance approves zero and changes nothing.
    /// An unshield of exactly the balance leaves exactly zero.
    fn check_oversized(&self, outcome: Outcome, before: &Fin, after: &Fin) -> Result<(), String> {
        let name = "OVERSIZED UNSHIELD RETURNS ZERO";
        let Some(target) = self.real.target else {
            return Ok(());
        };
        let rec = &self.real.recs[target];
        if rec.intent.kind != OperationKind::Unshield {
            return Ok(());
        }
        let a = asset_index(rec.asset);
        let balance = before.balances.map_or(0, |b| b[a]);
        match outcome {
            Outcome::Approved(amount) => {
                if amount != rec.amount || rec.amount > balance {
                    return Err(format!(
                        "{name}: approved {amount} of a request for {} with balance {balance}",
                        rec.amount
                    ));
                }
                let rest = after.balances.map(|b| b[a]);
                if rest != Some(balance - amount) {
                    return Err(format!(
                        "{name}: balance {balance} - {amount} left {rest:?}"
                    ));
                }
                if rec.amount == balance && rest != Some(0) {
                    return Err(format!(
                        "{name}: unshielding the whole balance left {rest:?}"
                    ));
                }
                Ok(())
            }
            Outcome::Zero => {
                if rec.amount <= balance {
                    return Err(format!(
                        "{name}: zero approved for {} within balance {balance}",
                        rec.amount
                    ));
                }
                if !before.same_value(after) {
                    return Err(format!(
                        "{name}: an oversized request ({} > {balance}) changed the balance or moved tokens",
                        rec.amount
                    ));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Failed, stale, expired, zero, ignored, rejected, and abandoned paths
    /// change no balance, key, epoch, or vault. Terminal Intents never change.
    fn check_neutrality(
        &self,
        act: &Act,
        outcome: Outcome,
        before: &Fin,
        after: &Fin,
    ) -> Result<(), String> {
        let name = "FAILURE AND LATE CALLBACK NEUTRALITY";
        for (i, old) in before.intents.iter().enumerate() {
            if *old != IntentStatus::Queued && *old != after.intents[i] {
                return Err(format!(
                    "{name}: terminal Intent {i} changed ({old:?} -> {:?})",
                    after.intents[i]
                ));
            }
        }
        match outcome {
            Outcome::Ignored
            | Outcome::Rejected(_)
            | Outcome::Skipped
            | Outcome::Stale
            | Outcome::Expired
            | Outcome::Zero
            | Outcome::Abandoned => {
                if !before.same_value(after) {
                    return Err(format!(
                        "{name}: {outcome:?} changed a balance, key, epoch, payload, vault or shield"
                    ));
                }
            }
            Outcome::Done | Outcome::Refund(_) => {
                if before.payload != after.payload
                    || (before.key != after.key && !matches!(act, Act::InitPosition))
                    || before.epoch != after.epoch
                    || before.balances != after.balances
                {
                    return Err(format!("{name}: {act:?} changed the Position payload"));
                }
            }
            Outcome::Applied => {
                if before.vault != after.vault
                    || before.wallet != after.wallet
                    || before.out != after.out
                {
                    return Err(format!(
                        "{name}: an applied credit or rotation moved tokens"
                    ));
                }
            }
            Outcome::Approved(_) => {}
        }
        Ok(())
    }

    /// The decoded free balance of each asset equals the credited shields
    /// minus the approved unshields, so the encrypted claim is fully backed.
    fn check_backing(&self, after: &Fin) -> Result<(), String> {
        if after.undecryptable {
            return Ok(());
        }
        let Some(balances) = after.balances else {
            return Ok(());
        };
        for asset in [ShieldAsset::Sol, ShieldAsset::Usdc] {
            let a = asset_index(asset);
            let credited: u128 = self
                .real
                .shields
                .iter()
                .filter(|s| {
                    s.shield.asset == asset && s.shield.status == PendingShieldStatus::Credited
                })
                .map(|s| u128::from(s.shield.amount))
                .sum();
            if u128::from(balances[a]) + self.real.out[a] != credited {
                return Err(format!(
                    "BALANCE BACKING: asset {a}: balance {} + unshielded {} != credited {credited}",
                    balances[a], self.real.out[a]
                ));
            }
        }
        if balances[2..].iter().any(|v| *v != 0) {
            return Err("BALANCE BACKING: an unused payload field is not zero".into());
        }
        Ok(())
    }

    /// Epoch moves only on an applied rotation, by one; key and payload
    /// change together; balances survive; the old key can no longer read.
    fn check_rotation(
        &self,
        act: &Act,
        outcome: Outcome,
        before: &Fin,
        after: &Fin,
    ) -> Result<(), String> {
        let name = "ROTATION";
        let rotated = outcome == Outcome::Applied
            && self.real.target.is_some_and(|t| {
                self.real.recs[t].intent.kind == OperationKind::ViewingKeyRotation
            });
        let expected_epoch = before.epoch + u64::from(rotated);
        if after.epoch != expected_epoch {
            return Err(format!(
                "{name}: epoch {} -> {} (rotated: {rotated})",
                before.epoch, after.epoch
            ));
        }
        if !rotated && !matches!(act, Act::InitPosition) && after.key != before.key {
            return Err(format!(
                "{name}: the key changed without an applied rotation"
            ));
        }
        if after.undecryptable {
            return Err(format!(
                "{name}: the stored payload does not open under the stored key (key/payload not atomic)"
            ));
        }
        if rotated {
            if after.balances != before.balances {
                return Err(format!("{name}: a rotation changed the balances"));
            }
            if after.key == before.key {
                return Err(format!("{name}: a rotation kept the key"));
            }
            if unseal(
                &before.key,
                self.real.position.encrypted_state.nonce,
                &self.real.position.encrypted_state.ciphertexts,
            )
            .is_some()
            {
                return Err(format!("{name}: the previous key still opens the payload"));
            }
        }
        Ok(())
    }

    /// A pause blocks only NEW shield and unshield queueing. Everything else
    /// stays available, and an expired guardian pause blocks nothing.
    fn check_pause(&self, act: &Act, outcome: Outcome) -> Result<(), String> {
        let name = "PAUSE AND RESCUE";
        let slot = self.real.slot;
        let shield_paused = is_paused(&self.real.pauses, PauseCategory::Shield, slot);
        let unshield_paused = is_paused(&self.real.pauses, PauseCategory::Unshield, slot);
        let queues_shield = matches!(act, Act::Shield(..) | Act::ShieldOverwrite(_));
        let queues_unshield = matches!(act, Act::Unshield(..));
        if outcome == Outcome::Rejected(Why::Paused) {
            let allowed = (queues_shield && shield_paused) || (queues_unshield && unshield_paused);
            if !allowed {
                return Err(format!(
                    "{name}: {act:?} was blocked by a pause that must not block it"
                ));
            }
        }
        if queues_shield
            && shield_paused
            && outcome != Outcome::Skipped
            && outcome != Outcome::Rejected(Why::Paused)
        {
            return Err(format!("{name}: a paused Shield queued"));
        }
        if queues_unshield && unshield_paused && outcome != Outcome::Rejected(Why::Paused) {
            return Err(format!("{name}: a paused Unshield queued"));
        }
        // An expired guardian pause alone never blocks.
        let pauses = &self.real.pauses;
        if slot >= pauses.guardian_expiry_slot
            && pauses.flags & pause_flags::SHIELD == 0
            && shield_paused
        {
            return Err(format!(
                "{name}: an expired guardian pause still blocks Shield"
            ));
        }
        Ok(())
    }

    fn check_lock(&self) -> Result<(), String> {
        let queued = self
            .real
            .recs
            .iter()
            .filter(|r| r.intent.status == IntentStatus::Queued)
            .count();
        let locked = self.real.position.coordination.lock.is_active();
        if queued > 1 || (queued == 1) != locked {
            return Err(format!(
                "LOCK SAFETY: {queued} queued Intents with Lock active = {locked}"
            ));
        }
        Ok(())
    }
}

// ---- Drivers -------------------------------------------------------------------

/// Deterministic linear-congruential sampler (no external crate).
struct Lcg(u64);

impl Lcg {
    fn sample(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, n: u64) -> u64 {
        self.sample() % n
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len() as u64) as usize]
    }
}

const SOL: ShieldAsset = ShieldAsset::Sol;
const USDC: ShieldAsset = ShieldAsset::Usdc;
const AMOUNTS: [u64; 9] = [0, 1, 7, 100, 500, 1_000, u64::MAX, u64::MAX - 1, 1 << 63];

/// A state-aware action that is usually valid, so the long random runs reach
/// the deep states (credits, approved unshields, refunds, rotations).
fn guided_act(rng: &mut Lcg, model: &Model) -> Option<Act> {
    let asset = if rng.below(2) == 0 { SOL } else { USDC };
    let amounts = [1, 7, 100, 500, 1_000, u64::MAX, u64::MAX - 1, 1 << 63];
    let unshield_amounts = [
        UAmt::Within,
        UAmt::Within,
        UAmt::Balance,
        UAmt::BalancePlusOne,
        UAmt::Huge,
        UAmt::Exact(1),
    ];
    match (model.status, model.lock) {
        (PositionStatus::Uninitialized, _) => Some(Act::InitPosition),
        (PositionStatus::InitFailed, None) => Some(Act::RetryInit),
        (_, Some(_)) => Some(match rng.below(10) {
            0..=4 => Act::Callback(Target::Cur, Variant::Honest),
            5 => Act::Advance(rng.pick(&[400, 1_501, 2_000])),
            6 => Act::Callback(
                rng.pick(&[Target::Cur, Target::First, Target::Prev]),
                rng.pick(&[
                    Variant::Unauth,
                    Variant::WrongKey,
                    Variant::WrongNonce,
                    Variant::WrongOffset,
                    Variant::Honest,
                ]),
            ),
            7 => Act::ExternalBump,
            8 => Act::Abandon,
            _ => Act::Advance(2_000),
        }),
        (PositionStatus::Active, None) => Some(match rng.below(12) {
            0..=3 => Act::Shield(asset, rng.pick(&amounts)),
            4..=7 => Act::Unshield(asset, rng.pick(&unshield_amounts)),
            8 => Act::Rotate(1 + rng.below(3) as u8),
            9 => Act::RefundShield(if rng.below(3) == 0 {
                Sel::First
            } else {
                Sel::Last
            }),
            10 => Act::RetryShield(if rng.below(3) == 0 {
                Sel::First
            } else {
                Sel::Last
            }),
            _ => Act::Advance(rng.pick(&[10, 2_000])),
        }),
        _ => None,
    }
}

fn random_act(rng: &mut Lcg, model: &Model) -> Act {
    if rng.below(10) < 6 {
        if let Some(act) = guided_act(rng, model) {
            return act;
        }
    }
    uniform_act(rng)
}
fn uniform_act(rng: &mut Lcg) -> Act {
    let asset = if rng.below(2) == 0 { SOL } else { USDC };
    let sel = if rng.below(3) == 0 {
        Sel::First
    } else {
        Sel::Last
    };
    let unshield_amounts = [
        UAmt::Within,
        UAmt::Balance,
        UAmt::BalancePlusOne,
        UAmt::Huge,
        UAmt::Exact(1),
        UAmt::Exact(50),
    ];
    match rng.below(39) {
        0 => Act::InitPosition,
        1 => Act::RetryInit,
        2..=7 => Act::Callback(Target::Cur, Variant::Honest),
        8 => Act::Callback(Target::Cur, Variant::Unauth),
        9..=10 => Act::Callback(
            rng.pick(&[Target::Cur, Target::First, Target::Prev]),
            rng.pick(&[
                Variant::Honest,
                Variant::Unauth,
                Variant::WrongKey,
                Variant::WrongNonce,
                Variant::WrongOffset,
            ]),
        ),
        11..=15 => Act::Shield(asset, rng.pick(&AMOUNTS)),
        16 => Act::ShieldOverwrite(sel),
        17 => Act::RetryShield(sel),
        18..=20 => Act::RefundShield(sel),
        21..=23 => Act::Unshield(asset, rng.pick(&unshield_amounts)),
        24 => Act::Rotate(rng.below(4) as u8),
        25 => Act::RotateStale,
        26..=27 => Act::Abandon,
        28..=31 => Act::Advance(rng.pick(&[10, 400, 1_501, 2_000])),
        32 => Act::ExternalBump,
        33 => Act::PauseShield,
        34 => Act::UnpauseShield,
        35 => Act::PauseUnshield,
        36 => Act::UnpauseUnshield,
        37 => Act::GuardianPause(
            rng.pick(&[
                pause_flags::SHIELD,
                pause_flags::UNSHIELD,
                pause_flags::SHIELD | pause_flags::UNSHIELD,
            ]),
            rng.pick(&[50, 300, 2_000]),
        ),
        _ => Act::GuardianClear,
    }
}

/// Runs `acts` on a fresh harness; the first failure carries the sequence.
fn run(fault: Fault, acts: &[Act]) -> Result<(Harness, Vec<Outcome>), String> {
    let mut harness = Harness::new(fault);
    let mut outcomes = Vec::new();
    for (index, act) in acts.iter().enumerate() {
        match harness.step(*act) {
            Ok(outcome) => outcomes.push(outcome),
            Err(message) => return Err(format!("step {index}: {message}\n sequence: {acts:?}")),
        }
    }
    Ok((harness, outcomes))
}

/// Every sequence of up to `depth` actions over `alphabet`, depth first, so
/// each prefix runs once. Returns the number of actions executed.
fn explore(
    harness: &Harness,
    alphabet: &[Act],
    depth: usize,
    path: &mut Vec<Act>,
    coverage: &mut Coverage,
) -> u64 {
    let mut executed = 0;
    for act in alphabet {
        let mut next = harness.clone();
        path.push(*act);
        match next.step(*act) {
            Ok(outcome) => coverage.add(outcome),
            Err(message) => panic!("{message}\n sequence: {path:?}"),
        }
        executed += 1;
        if depth > 1 {
            executed += explore(&next, alphabet, depth - 1, path, coverage);
        }
        path.pop();
    }
    executed
}

mod tests {
    use super::*;

    const MAX: u64 = u64::MAX;

    fn honest() -> Act {
        Act::Callback(Target::Cur, Variant::Honest)
    }

    fn unshield(asset: ShieldAsset, amount: u64) -> Act {
        Act::Unshield(asset, UAmt::Exact(amount))
    }

    /// A script that reaches every kind of step once: it activates the
    /// Position, credits, expires, duplicates, refunds under a pause, hits a
    /// credit overflow, an oversized and an exact unshield, rotates, and sends
    /// an old-attempt callback.
    fn script() -> Vec<Act> {
        vec![
            Act::InitPosition,
            honest(),
            Act::Shield(SOL, 100),
            honest(),
            Act::Shield(USDC, 40),
            Act::Advance(2_000),
            honest(),
            honest(),
            Act::PauseShield,
            Act::RefundShield(Sel::Last),
            Act::UnpauseShield,
            Act::Shield(SOL, MAX),
            honest(),
            unshield(SOL, 1_000),
            honest(),
            unshield(SOL, 100),
            honest(),
            Act::Rotate(2),
            honest(),
            Act::Callback(Target::First, Variant::Honest),
            Act::RefundShield(Sel::Last),
            Act::Shield(SOL, 7),
            Act::Callback(Target::Cur, Variant::Honest),
        ]
    }

    fn run_ok(acts: &[Act]) -> (Harness, Vec<Outcome>) {
        run(Fault::None, acts).unwrap_or_else(|message| panic!("{message}"))
    }

    // ---- Scripted scenarios ----------------------------------------------------

    #[test]
    fn balance_script_agrees_and_ends_with_the_expected_balances() {
        let (harness, outcomes) = run_ok(&script());
        assert_eq!(
            outcomes,
            vec![
                Outcome::Done,
                Outcome::Applied,
                Outcome::Done,
                Outcome::Applied,
                Outcome::Done,
                Outcome::Done,
                Outcome::Expired,
                Outcome::Ignored,
                Outcome::Done,
                Outcome::Refund(40),
                Outcome::Done,
                Outcome::Done,
                Outcome::Zero,
                Outcome::Done,
                Outcome::Zero,
                Outcome::Done,
                Outcome::Approved(100),
                Outcome::Done,
                Outcome::Applied,
                Outcome::Ignored,
                Outcome::Refund(MAX),
                Outcome::Done,
                Outcome::Applied,
            ]
        );
        let snap = harness.real.snap();
        assert_eq!(snap.balances, Some([7, 0]));
        assert_eq!(snap.epoch, 1);
        assert_eq!(snap.key, 2);
        // Vault: the shield of 7 (credited) only; 100 unshielded, MAX and 40
        // refunded.
        assert_eq!(snap.vault, [7, 0]);
        assert_eq!(snap.out, [100, 0]);
        assert_eq!(snap.wallet[0] + snap.vault[0], WALLET_START);
    }

    #[test]
    fn balance_oversized_unshield_returns_zero_and_exact_leaves_zero() {
        let (harness, outcomes) = run_ok(&[
            Act::InitPosition,
            honest(),
            Act::Shield(USDC, 50),
            honest(),
            Act::Unshield(USDC, UAmt::BalancePlusOne),
            honest(),
            Act::Unshield(USDC, UAmt::Huge),
            honest(),
            Act::Unshield(USDC, UAmt::Balance),
            honest(),
            Act::Unshield(USDC, UAmt::Balance),
        ]);
        assert_eq!(
            &outcomes[4..],
            &[
                Outcome::Done,
                Outcome::Zero,
                Outcome::Done,
                Outcome::Zero,
                Outcome::Done,
                Outcome::Approved(50),
                // A zero amount is rejected before it can queue.
                Outcome::Rejected(Why::Invalid),
            ]
        );
        assert_eq!(harness.real.snap().balances, Some([0, 0]));
        assert_eq!(harness.real.snap().vault, [0, 0]);
    }

    #[test]
    fn balance_credit_overflow_stays_deposited_and_is_refundable() {
        let (harness, outcomes) = run_ok(&[
            Act::InitPosition,
            honest(),
            Act::Shield(SOL, MAX),
            honest(),
            Act::Shield(SOL, 1),
            honest(),
            Act::RefundShield(Sel::Last),
            Act::RefundShield(Sel::Last),
        ]);
        assert_eq!(outcomes[5], Outcome::Zero);
        assert_eq!(outcomes[6], Outcome::Refund(1));
        assert_eq!(outcomes[7], Outcome::Rejected(Why::Invalid));
        let snap = harness.real.snap();
        assert_eq!(snap.balances, Some([MAX, 0]));
        assert_eq!(snap.vault, [u128::from(MAX), 0]);
    }

    #[test]
    fn balance_failed_and_late_callbacks_change_nothing() {
        // Stale, expired, unauthenticated, wrong binding, duplicate, late
        // after abandonment, and old-attempt callbacks.
        let (harness, outcomes) = run_ok(&[
            Act::InitPosition,
            honest(),
            Act::Shield(SOL, 10),
            Act::Callback(Target::Cur, Variant::Unauth),
            Act::Callback(Target::Cur, Variant::WrongKey),
            Act::Callback(Target::Cur, Variant::WrongNonce),
            Act::Callback(Target::Cur, Variant::WrongOffset),
            Act::ExternalBump,
            honest(),
            Act::RetryShield(Sel::Last),
            Act::Advance(2_000),
            Act::Abandon,
            honest(),
            Act::Callback(Target::First, Variant::Honest),
            Act::RetryShield(Sel::Last),
            honest(),
        ]);
        assert_eq!(
            &outcomes[3..],
            &[
                Outcome::Rejected(Why::Unauth),
                Outcome::Rejected(Why::Binding),
                Outcome::Rejected(Why::Binding),
                Outcome::Rejected(Why::Binding),
                Outcome::Done,
                Outcome::Stale,
                Outcome::Done,
                Outcome::Done,
                Outcome::Abandoned,
                Outcome::Ignored,
                Outcome::Ignored,
                Outcome::Done,
                Outcome::Applied,
            ]
        );
        assert_eq!(harness.real.snap().balances, Some([10, 0]));
    }

    #[test]
    fn balance_rotation_applies_atomically_and_keeps_balances() {
        let (harness, outcomes) = run_ok(&[
            Act::InitPosition,
            honest(),
            Act::Shield(SOL, 33),
            honest(),
            Act::Rotate(2),
            Act::Callback(Target::Cur, Variant::WrongKey),
            Act::Advance(2_000),
            honest(),
            Act::Rotate(2),
            honest(),
            Act::Rotate(2),
            Act::Rotate(1),
            honest(),
        ]);
        assert_eq!(outcomes[7], Outcome::Expired);
        assert_eq!(outcomes[9], Outcome::Applied);
        assert_eq!(outcomes[10], Outcome::Rejected(Why::Invalid));
        assert_eq!(outcomes[11], Outcome::Done);
        assert_eq!(outcomes[12], Outcome::Applied);
        let snap = harness.real.snap();
        assert_eq!((snap.epoch, snap.key, snap.balances), (2, 1, Some([33, 0])));
    }

    #[test]
    fn balance_pause_blocks_only_new_shield_and_unshield() {
        let (_, outcomes) = run_ok(&[
            Act::InitPosition,
            honest(),
            Act::Shield(SOL, 5),
            honest(),
            Act::Shield(USDC, 9),
            Act::Advance(2_000),
            honest(),
            Act::GuardianPause(pause_flags::SHIELD | pause_flags::UNSHIELD, 300),
            Act::Shield(SOL, 1),
            Act::Unshield(SOL, UAmt::Balance),
            Act::RetryShield(Sel::Last),
            Act::Rotate(3),
            honest(),
            Act::RefundShield(Sel::Last),
            Act::Advance(300),
            Act::Shield(SOL, 1),
        ]);
        assert_eq!(outcomes[8], Outcome::Rejected(Why::Paused));
        assert_eq!(outcomes[9], Outcome::Rejected(Why::Paused));
        assert_eq!(outcomes[10], Outcome::Done);
        assert_eq!(outcomes[11], Outcome::Rejected(Why::Invalid));
        assert_eq!(outcomes[13], Outcome::Rejected(Why::Invalid));
        // The guardian pause has expired: Shield queues again.
        assert_eq!(outcomes[15], Outcome::Done);
    }

    #[test]
    fn balance_shield_into_an_occupied_account_is_rejected() {
        let (_, outcomes) = run_ok(&[
            Act::InitPosition,
            honest(),
            Act::Shield(SOL, 5),
            honest(),
            Act::ShieldOverwrite(Sel::First),
            Act::ShieldOverwrite(Sel::Last),
        ]);
        assert_eq!(outcomes[4], Outcome::Rejected(Why::Invalid));
        assert_eq!(outcomes[5], Outcome::Rejected(Why::Invalid));
    }

    #[test]
    fn balance_init_failure_is_retryable() {
        let (harness, outcomes) = run_ok(&[
            Act::InitPosition,
            Act::Advance(2_000),
            honest(),
            Act::RetryInit,
            Act::Advance(2_000),
            Act::Abandon,
            Act::RetryInit,
            honest(),
        ]);
        assert_eq!(outcomes[2], Outcome::Expired);
        assert_eq!(outcomes[5], Outcome::Abandoned);
        assert_eq!(outcomes[7], Outcome::Applied);
        assert_eq!(harness.real.snap().status, PositionStatus::Active);
        assert_eq!(harness.real.snap().balances, Some([0, 0]));
    }

    // ---- Pause matrix --------------------------------------------------------------

    /// For every durable and guardian pause state (including the never
    /// pausable bits set), every guardian expiry side, and every action class:
    /// only new Shield and Unshield queueing is ever blocked.
    #[test]
    fn balance_pause_matrix_blocks_only_new_shield_and_unshield() {
        use crate::market::{ensure_action_allowed, Action};
        let rescue = [
            Action::Repay,
            Action::AddCollateral,
            Action::Callback,
            Action::AbandonIntent,
            Action::PendingShieldRetry,
            Action::PendingShieldRefund,
            Action::ViewingKeyRotation,
            Action::PositionInit,
        ];
        for flags in 0..=pause_flags::ALL {
            for guardian_flags in [
                0,
                pause_flags::ALL,
                pause_flags::SHIELD,
                pause_flags::UNSHIELD,
            ] {
                for (expiry, slot) in [(0u64, 50u64), (100, 50), (100, 100), (100, 5_000)] {
                    let pauses = PauseState {
                        flags,
                        guardian_flags,
                        guardian_expiry_slot: expiry,
                    };
                    let guardian_live = guardian_flags != 0 && slot < expiry;
                    let effective = (flags | if guardian_live { guardian_flags } else { 0 })
                        & pause_flags::PAUSABLE;
                    for action in rescue {
                        assert!(
                            ensure_action_allowed(&pauses, action, slot).is_ok(),
                            "PAUSE AND RESCUE: {action:?} blocked by {pauses:?} at {slot}"
                        );
                    }
                    for (action, bit) in [
                        (Action::Shield, pause_flags::SHIELD),
                        (Action::Unshield, pause_flags::UNSHIELD),
                    ] {
                        assert_eq!(
                            ensure_action_allowed(&pauses, action, slot).is_err(),
                            effective & bit != 0,
                            "{action:?} under {pauses:?} at {slot}"
                        );
                    }
                }
            }
        }
    }

    // ---- Shared golden vectors ---------------------------------------------------------

    const VECTORS: &str =
        include_str!("../../../../packages/reference-model/vectors/balance.vectors");

    fn parse_asset(word: &str) -> ShieldAsset {
        match word {
            "sol" => SOL,
            "usdc" => USDC,
            other => panic!("unknown asset {other}"),
        }
    }

    fn parse_sel(word: &str) -> Sel {
        match word {
            "first" => Sel::First,
            "last" => Sel::Last,
            other => panic!("unknown selector {other}"),
        }
    }

    /// Maps one vector step onto actions. See `balance.vectors` for the
    /// grammar; the TypeScript reference model reads the same file.
    fn vector_acts(line: &str) -> Vec<Act> {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["init"] => vec![Act::InitPosition],
            ["retry_init"] => vec![Act::RetryInit],
            ["shield", asset, amount] => {
                vec![Act::Shield(parse_asset(asset), amount.parse().unwrap())]
            }
            ["credit"] => vec![honest()],
            ["expire"] => vec![Act::Advance(2_000), honest()],
            ["stale"] => vec![Act::ExternalBump, honest()],
            ["unauth"] => vec![Act::Callback(Target::Cur, Variant::Unauth)],
            ["old"] => vec![Act::Callback(Target::First, Variant::Honest)],
            ["abandon"] => vec![Act::Advance(2_000), Act::Abandon],
            ["retry_shield", sel] => vec![Act::RetryShield(parse_sel(sel))],
            ["refund", sel] => vec![Act::RefundShield(parse_sel(sel))],
            ["unshield", asset, amount] => {
                vec![unshield(parse_asset(asset), amount.parse().unwrap())]
            }
            ["rotate", k] => vec![Act::Rotate(k.parse().unwrap())],
            ["pause_shield"] => vec![Act::PauseShield],
            ["unpause_shield"] => vec![Act::UnpauseShield],
            ["pause_unshield"] => vec![Act::PauseUnshield],
            ["unpause_unshield"] => vec![Act::UnpauseUnshield],
            ["advance", n] => vec![Act::Advance(n.parse().unwrap())],
            other => panic!("unknown vector step {other:?}"),
        }
    }

    #[test]
    fn balance_golden_vectors_hold_for_production_and_the_reference_model() {
        let mut harness: Option<Harness> = None;
        let mut name = String::new();
        let mut scenarios = 0;
        let finish = |harness: &Option<Harness>, name: &str| {
            if harness.is_some() {
                eprintln!("vector scenario {name}: ok");
            }
        };
        for raw in VECTORS.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("scenario ") {
                finish(&harness, &name);
                name = rest.to_string();
                harness = Some(Harness::new(Fault::None));
                scenarios += 1;
                continue;
            }
            let h = harness.as_mut().expect("a step before any scenario");
            if let Some(rest) = line.strip_prefix("expect ") {
                let words: Vec<&str> = rest.split_whitespace().collect();
                let snap = h.real.snap();
                let number = |w: &str| w.parse::<u128>().unwrap();
                match words.as_slice() {
                    ["free", sol, usdc] => assert_eq!(
                        snap.balances,
                        Some([number(sol) as u64, number(usdc) as u64]),
                        "{name}: {line}"
                    ),
                    ["vault", sol, usdc] => {
                        assert_eq!(snap.vault, [number(sol), number(usdc)], "{name}: {line}")
                    }
                    ["out", sol, usdc] => {
                        assert_eq!(snap.out, [number(sol), number(usdc)], "{name}: {line}")
                    }
                    ["epoch", n] => assert_eq!(snap.epoch as u128, number(n), "{name}: {line}"),
                    ["key", n] => assert_eq!(snap.key as u128, number(n), "{name}: {line}"),
                    ["shield", i, status] => {
                        let want = match *status {
                            "deposited" => PendingShieldStatus::Deposited,
                            "credited" => PendingShieldStatus::Credited,
                            "refunded" => PendingShieldStatus::Refunded,
                            other => panic!("unknown status {other}"),
                        };
                        assert_eq!(
                            snap.shields[i.parse::<usize>().unwrap()].2,
                            want,
                            "{name}: {line}"
                        );
                    }
                    other => panic!("unknown expectation {other:?}"),
                }
                continue;
            }
            let (must_reject, step) = match line.strip_prefix('!') {
                Some(step) => (true, step.trim()),
                None => (false, line),
            };
            let mut last = Outcome::Skipped;
            for act in vector_acts(step) {
                last = h
                    .step(act)
                    .unwrap_or_else(|message| panic!("{name}: {step}: {message}"));
            }
            assert_eq!(
                matches!(last, Outcome::Rejected(_)),
                must_reject,
                "{name}: {step} -> {last:?}"
            );
        }
        finish(&harness, &name);
        assert!(scenarios >= 8, "expected at least 8 vector scenarios");
    }

    // ---- Negative controls -----------------------------------------------------------

    fn fault_message(fault: Fault) -> String {
        match run(fault, &script()) {
            Ok(_) => panic!("{fault:?} was not detected"),
            Err(message) => message,
        }
    }

    #[test]
    fn balance_injected_faults_are_caught_by_the_named_invariant() {
        let cases = [
            (Fault::RefundKeepsVault, "VAULT CONSERVATION"),
            (Fault::RefundFlipsToCredited, "PENDING SHIELD EXCLUSIVITY"),
            (Fault::ZeroWrites, "FAILURE AND LATE CALLBACK NEUTRALITY"),
            (Fault::IgnoredWrites, "FAILURE AND LATE CALLBACK NEUTRALITY"),
            (Fault::LateCallbackMovesTokens, "VAULT CONSERVATION"),
            (Fault::OversizedDrains, "OVERSIZED UNSHIELD RETURNS ZERO"),
            (Fault::RefundBlockedByPause, "PAUSE AND RESCUE"),
            (Fault::RotationKeepsEpoch, "ROTATION"),
            (Fault::RotationSwapsKeyOnly, "ROTATION"),
            (Fault::NonceReuse, "NONCE UNIQUENESS"),
        ];
        for (fault, name) in cases {
            let message = fault_message(fault);
            assert!(
                message.contains(&format!("{name}:")),
                "{fault:?}: expected {name}, got: {message}"
            );
        }
        // The same script without a fault passes.
        run_ok(&script());
    }

    #[test]
    fn balance_a_reference_model_defect_is_an_agreement_failure() {
        let mut harness = Harness::new(Fault::None);
        harness.step(Act::InitPosition).unwrap();
        harness.model.generation += 1;
        let message = harness.step(honest()).unwrap_err();
        assert!(message.contains("AGREEMENT"), "{message}");
    }

    // ---- Local decryption of the oracle cipher ---------------------------------------------

    #[test]
    fn balance_oracle_cipher_rejects_a_wrong_key_or_nonce() {
        let sealed = seal(&[1; 32], 7, [1, 2, 3, 4, 5]);
        assert_eq!(unseal(&[1; 32], 7, &sealed), Some([1, 2, 3, 4, 5]));
        assert_eq!(unseal(&[2; 32], 7, &sealed), None);
        assert_eq!(unseal(&[1; 32], 8, &sealed), None);
    }

    // ---- Model-based coverage ---------------------------------------------------------------

    fn lifecycle_alphabet() -> Vec<Act> {
        vec![
            Act::InitPosition,
            honest(),
            Act::Shield(SOL, 100),
            Act::Shield(SOL, MAX),
            Act::Unshield(SOL, UAmt::Balance),
            Act::Unshield(SOL, UAmt::BalancePlusOne),
            Act::Rotate(2),
            Act::RefundShield(Sel::Last),
            Act::RetryShield(Sel::Last),
            Act::Advance(2_000),
        ]
    }

    fn adversarial_alphabet() -> Vec<Act> {
        vec![
            Act::InitPosition,
            honest(),
            Act::Shield(USDC, 50),
            Act::Callback(Target::Cur, Variant::Unauth),
            Act::Abandon,
            Act::Advance(2_000),
            Act::GuardianPause(pause_flags::SHIELD, 100),
            Act::RefundShield(Sel::Last),
            Act::ExternalBump,
            Act::RetryShield(Sel::Last),
        ]
    }

    /// A funded Position: `Active` with 100 SOL credited, the Lock free.
    fn funded() -> Harness {
        let (harness, _) = run_ok(&[Act::InitPosition, honest(), Act::Shield(SOL, 100), honest()]);
        harness
    }

    fn funded_lifecycle_alphabet() -> Vec<Act> {
        vec![
            Act::Shield(SOL, 50),
            Act::Shield(SOL, MAX),
            Act::Unshield(SOL, UAmt::Balance),
            Act::Unshield(SOL, UAmt::BalancePlusOne),
            Act::Unshield(SOL, UAmt::Within),
            Act::Rotate(2),
            honest(),
            Act::Advance(2_000),
            Act::RefundShield(Sel::Last),
            Act::RetryShield(Sel::Last),
        ]
    }

    fn funded_adversarial_alphabet() -> Vec<Act> {
        vec![
            Act::Shield(USDC, 20),
            honest(),
            Act::Callback(Target::Cur, Variant::Unauth),
            Act::Callback(Target::First, Variant::Honest),
            Act::Abandon,
            Act::Advance(2_000),
            Act::ExternalBump,
            Act::GuardianPause(pause_flags::SHIELD | pause_flags::UNSHIELD, 100),
            Act::Unshield(SOL, UAmt::Within),
            Act::RefundShield(Sel::Last),
        ]
    }

    fn exhaustive(start: &Harness, alphabet: &[Act]) -> (u64, Coverage) {
        let mut coverage = Coverage::default();
        let executed = explore(start, alphabet, 5, &mut Vec::new(), &mut coverage);
        // 10 + 10^2 + ... + 10^5 actions.
        assert_eq!(executed, 111_110);
        eprintln!("exhaustive: {executed} actions {:?}", coverage.counts);
        (executed, coverage)
    }

    fn require(coverage: &Coverage, names: &[&str]) {
        for name in names {
            assert!(
                coverage.get(name) > 0,
                "no {name} outcome in the exhaustive run"
            );
        }
    }

    #[test]
    fn prop_balance_exhaustive_lifecycle_sequences_up_to_length_5() {
        let (_, coverage) = exhaustive(&Harness::new(Fault::None), &lifecycle_alphabet());
        require(&coverage, &["Applied", "Zero", "Expired", "Ignored"]);
    }

    #[test]
    fn prop_balance_exhaustive_adversarial_sequences_up_to_length_5() {
        let (_, coverage) = exhaustive(&Harness::new(Fault::None), &adversarial_alphabet());
        require(
            &coverage,
            &[
                "Applied",
                "Abandoned",
                "Stale",
                "Ignored",
                "Rejected(Paused)",
                "Rejected(Unauth)",
                "Rejected(Cooldown)",
            ],
        );
    }

    #[test]
    fn prop_balance_exhaustive_funded_lifecycle_sequences_up_to_length_5() {
        let (_, coverage) = exhaustive(&funded(), &funded_lifecycle_alphabet());
        require(
            &coverage,
            &["Applied", "Zero", "Approved", "Refund", "Expired"],
        );
    }

    #[test]
    fn prop_balance_exhaustive_funded_adversarial_sequences_up_to_length_5() {
        let (_, coverage) = exhaustive(&funded(), &funded_adversarial_alphabet());
        require(
            &coverage,
            &[
                "Applied",
                "Abandoned",
                "Stale",
                "Ignored",
                "Refund",
                "Rejected(Paused)",
                "Rejected(Unauth)",
            ],
        );
    }
    #[test]
    fn prop_balance_random_sequences_full_alphabet() {
        const SEQUENCES: u64 = 20_000;
        let mut rng = Lcg(0x5EED_BA1A_0C3E);
        let mut coverage = Coverage::default();
        let mut actions = 0u64;
        for _ in 0..SEQUENCES {
            let length = 1 + rng.below(40);
            let mut harness = Harness::new(Fault::None);
            let mut trace = Vec::new();
            for _ in 0..length {
                let act = random_act(&mut rng, &harness.model);
                trace.push(act);
                match harness.step(act) {
                    Ok(outcome) => coverage.add(outcome),
                    Err(message) => panic!("{message}\n sequence: {trace:?}"),
                }
                actions += 1;
            }
        }
        eprintln!(
            "random: {SEQUENCES} sequences, {actions} actions {:?}",
            coverage.counts
        );
        for name in [
            "Done",
            "Applied",
            "Zero",
            "Stale",
            "Expired",
            "Ignored",
            "Abandoned",
            "Refund",
            "Approved",
            "Rejected(Paused)",
            "Rejected(Cooldown)",
            "Rejected(Unauth)",
            "Rejected(Binding)",
            "Rejected(Invalid)",
        ] {
            assert!(
                coverage.get(name) >= 100,
                "the random run produced only {} {name} outcomes",
                coverage.get(name)
            );
        }
    }
}
