use std::collections::HashSet;

use super::*;
use crate::market::pause_flags;
use crate::state::nonce::{derive_output_nonce_input, output_nonce};
use crate::state::pending_shield::{NewPendingShield, PendingShield};
use crate::state::shield::{
    abandon_shield, queue_shield, settle_shield_credit, ShieldCallback, ShieldRequest,
    ShieldSettlement,
};

const OWNER: Pubkey = Pubkey::new_from_array([0x11; 32]);
const STRANGER: Pubkey = Pubkey::new_from_array([0x12; 32]);
const MARKET_KEY: Pubkey = Pubkey::new_from_array([0x22; 32]);
const POSITION_KEY: Pubkey = Pubkey::new_from_array([0x23; 32]);
const DEST: Pubkey = Pubkey::new_from_array([0x24; 32]);
const VIEWING: [u8; 32] = [0x33; 32];

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
        position_count: 1,
        debt_position_count: 0,
        unsettled_recovery_claim_count: 0,
        position_count_cap: 10,
        debt_position_count_cap: 0,
        initialized_slot: 1,
        debt_position_reservations: 0,
        reserved: [0; 120],
    }
}

fn active_position() -> Position {
    let mut position = Position::blank(OWNER, MARKET_KEY, 0xFC);
    position.status = PositionStatus::Active;
    position.viewing_pubkey = VIEWING;
    position.coordination.generation = 1;
    position.encrypted_state = EncryptedPositionPayload {
        nonce: 1_000,
        ciphertexts: [[0xA0; 32]; 5],
    };
    position
}

fn intent_key(sequence: u64) -> Pubkey {
    let mut bytes = [0x60; 32];
    bytes[..8].copy_from_slice(&sequence.to_le_bytes());
    Pubkey::new_from_array(bytes)
}

fn dest(m: &Market, asset: ShieldAsset) -> TokenAccountView {
    TokenAccountView {
        key: DEST,
        mint: asset_mint(m, asset),
        owner: OWNER,
    }
}

fn c<T>(result: Result<T, ErrorCode>) -> Result<T, u32> {
    result.map_err(u32::from)
}

fn code(error: ErrorCode) -> u32 {
    u32::from(error)
}

struct World {
    market: Market,
    pauses: PauseState,
    position: Position,
    counter: OperationCounter,
}

fn world() -> World {
    World {
        market: market(),
        pauses: PauseState::default(),
        position: active_position(),
        counter: OperationCounter::new(OWNER, 0xFA),
    }
}

impl World {
    fn request(&self, asset: ShieldAsset, amount: u64, slot: Slot) -> UnshieldRequest {
        let sequence = self.counter.next_sequence;
        UnshieldRequest {
            intent_key: intent_key(sequence),
            sequence,
            computation_offset: 9_000 + sequence,
            queued_slot: slot,
            asset,
            amount,
            destination: DEST,
        }
    }

    fn queue_with(
        &mut self,
        request: &UnshieldRequest,
        signer: &Pubkey,
        destination: &TokenAccountView,
    ) -> Result<OperationIntent, ErrorCode> {
        queue_unshield(
            &self.market,
            MARKET_KEY,
            &self.pauses,
            &mut self.position,
            POSITION_KEY,
            &mut self.counter,
            signer,
            destination,
            request,
        )
    }

    fn queue(
        &mut self,
        asset: ShieldAsset,
        amount: u64,
        slot: Slot,
    ) -> Result<OperationIntent, ErrorCode> {
        let request = self.request(asset, amount, slot);
        let destination = dest(&self.market, asset);
        self.queue_with(&request, &OWNER, &destination)
    }

    /// The honest callback of `intent` with the given revealed amounts.
    fn report(
        &self,
        intent: &OperationIntent,
        asset: ShieldAsset,
        approved: u64,
        requested: u64,
        slot: Slot,
    ) -> UnshieldReport {
        UnshieldReport {
            output_authenticated: true,
            computation_offset: intent.computation_offset,
            intent_key: intent.key,
            encryption_key: VIEWING,
            output_nonce: output_nonce(&POSITION_KEY, intent.sequence),
            ciphertexts: [[intent.sequence as u8 ^ 0x5A; 32]; 5],
            approved_amount: approved,
            requested_amount: requested,
            destination: dest(&self.market, asset),
            mint: asset_mint(&self.market, asset),
            vault: asset_vault(&self.market, asset),
            vault_authority: self.market.vault_authority,
            token_program: anchor_spl::token::ID,
            current_slot: slot,
        }
    }

    fn settle(
        &mut self,
        intent: &mut OperationIntent,
        report: &UnshieldReport,
    ) -> Result<UnshieldSettlement, ErrorCode> {
        settle_unshield(
            &self.market,
            MARKET_KEY,
            &mut self.position,
            POSITION_KEY,
            intent,
            report,
        )
    }
}

const SOL: ShieldAsset = ShieldAsset::Sol;
const USDC: ShieldAsset = ShieldAsset::Usdc;

#[test]
fn destination_rule_accepts_only_the_owners_account_of_the_asset_mint() {
    let m = market();
    for asset in [SOL, USDC] {
        let (mint, vault) = (asset_mint(&m, asset), asset_vault(&m, asset));
        assert_eq!(
            check_unshield_accounts(&m, asset, &OWNER, &mint, &vault, &dest(&m, asset)),
            Ok(())
        );
    }
    let (mint, vault) = (m.usdc_mint, m.usdc_vault);
    let good = dest(&m, USDC);
    let cases = [
        (m.wsol_mint, vault, good, UnshieldError::MintMismatch),
        (mint, m.wsol_vault, good, UnshieldError::VaultMismatch),
        (
            mint,
            vault,
            dest(&m, SOL),
            UnshieldError::DestinationInvalid,
        ),
        (
            mint,
            vault,
            TokenAccountView {
                owner: STRANGER,
                ..good
            },
            UnshieldError::DestinationInvalid,
        ),
        (
            mint,
            vault,
            TokenAccountView { key: vault, ..good },
            UnshieldError::DestinationInvalid,
        ),
    ];
    for (mint, vault, destination, expected) in cases {
        assert_eq!(
            check_unshield_accounts(&m, USDC, &OWNER, &mint, &vault, &destination),
            Err(expected)
        );
    }
}

#[test]
fn queue_binds_the_intent_and_locks_the_position() {
    let mut w = world();
    let intent = w.queue(USDC, 250, 10).unwrap();
    assert_eq!(intent.kind, OperationKind::Unshield);
    assert_eq!(intent.status, IntentStatus::Queued);
    assert_eq!(intent.decision_expiry_slot, 10 + UNSHIELD_DECISION_SLOTS);
    assert_eq!(
        intent.lock_expiry_slot,
        10 + UNSHIELD_DECISION_SLOTS + UNSHIELD_LOCK_GRACE_SLOTS
    );
    assert!(intent.lock_owner().holds(&w.position.coordination.lock));
    assert_eq!(w.counter.next_sequence, 1);

    let m = market();
    let state = w.position.encrypted_state;
    let nonce_in = derive_output_nonce_input(&POSITION_KEY, 0);
    let inputs = |state: &EncryptedPositionPayload, asset, amount, nonce| {
        input_commitment(
            OperationKind::Unshield,
            &unshield_inputs(VIEWING, 0, state, asset, amount, nonce),
        )
    };
    assert_eq!(
        intent.inputs_commitment,
        inputs(&state, USDC, 250, nonce_in)
    );
    let mut other_state = state;
    other_state.ciphertexts[4][0] ^= 1;
    let mut other_nonce = state;
    other_nonce.nonce += 1;
    for changed in [
        inputs(&state, SOL, 250, nonce_in),
        inputs(&state, USDC, 251, nonce_in),
        inputs(&state, USDC, 250, nonce_in + 1),
        inputs(&other_state, USDC, 250, nonce_in),
        inputs(&other_nonce, USDC, 250, nonce_in),
    ] {
        assert_ne!(intent.inputs_commitment, changed);
    }

    // The account commitment binds the destination, mint, vault, vault
    // authority, and token program.
    let keys = unshield_keys(&m, USDC, DEST);
    let accounts = |keys: &UnshieldKeys| {
        account_commitment(&unshield_bindings(
            OWNER,
            MARKET_KEY,
            m.asset_state,
            POSITION_KEY,
            Some(1),
            keys,
        ))
    };
    assert_eq!(intent.accounts_commitment, accounts(&keys));
    for changed in [
        UnshieldKeys {
            destination: STRANGER,
            ..keys
        },
        UnshieldKeys {
            mint: m.wsol_mint,
            ..keys
        },
        UnshieldKeys {
            vault: m.wsol_vault,
            ..keys
        },
        UnshieldKeys {
            vault_authority: STRANGER,
            ..keys
        },
        UnshieldKeys {
            token_program: STRANGER,
            ..keys
        },
    ] {
        assert_ne!(intent.accounts_commitment, accounts(&changed));
    }
}

#[test]
fn queue_fails_atomically() {
    let m = market();
    let good = dest(&m, SOL);
    let mut cases: Vec<(World, u64, Pubkey, TokenAccountView, ErrorCode)> = Vec::new();
    let mut paused = world();
    paused.pauses.flags = pause_flags::UNSHIELD;
    cases.push((paused, 5, OWNER, good, ErrorCode::ActionPaused));
    let mut inactive = world();
    inactive.market.status = MarketStatus::Uninitialized;
    cases.push((inactive, 5, OWNER, good, ErrorCode::MarketNotActive));
    cases.push((world(), 0, OWNER, good, ErrorCode::UnshieldZeroAmount));
    cases.push((world(), 5, STRANGER, good, ErrorCode::UnshieldNotOwner));
    let mut initializing = world();
    initializing.position.status = PositionStatus::Initializing;
    cases.push((
        initializing,
        5,
        OWNER,
        good,
        ErrorCode::UnshieldPositionNotActive,
    ));
    for bad in [
        dest(&m, USDC),
        TokenAccountView {
            owner: STRANGER,
            ..good
        },
        TokenAccountView {
            key: m.wsol_vault,
            ..good
        },
    ] {
        cases.push((
            world(),
            5,
            OWNER,
            bad,
            ErrorCode::UnshieldDestinationInvalid,
        ));
    }
    for (mut w, amount, signer, destination, expected) in cases {
        let (counter, position) = (w.counter, w.position.clone());
        let request = UnshieldRequest {
            destination: destination.key,
            ..w.request(SOL, amount, 5)
        };
        let result = w.queue_with(&request, &signer, &destination);
        assert_eq!(c(result.map(|_| ())), Err(code(expected)));
        assert_eq!(w.counter, counter);
        assert_eq!(w.position.coordination, position.coordination);
    }

    // A destination other than the one named in the request is rejected.
    let mut w = world();
    let request = w.request(SOL, 5, 5);
    let other = TokenAccountView {
        key: STRANGER,
        ..good
    };
    assert_eq!(
        c(w.queue_with(&request, &OWNER, &other).map(|_| ())),
        Err(code(ErrorCode::UnshieldDestinationInvalid))
    );

    // A Locked Position rejects a second unshield and stays unchanged.
    let mut w = world();
    w.queue(SOL, 5, 1).unwrap();
    let counter = w.counter;
    assert_eq!(
        c(w.queue(USDC, 5, 2).map(|_| ())),
        Err(code(ErrorCode::LockHeld))
    );
    assert_eq!(w.counter, counter);
}

#[test]
fn approved_callback_stores_the_state_and_reports_the_exact_transfer() {
    let mut w = world();
    let mut intent = w.queue(USDC, 70, 10).unwrap();
    let report = w.report(&intent, USDC, 70, 70, 11);
    let settlement = w.settle(&mut intent, &report).unwrap();
    assert_eq!(
        settlement,
        UnshieldSettlement::Approved {
            asset: USDC,
            amount: 70,
            destination: DEST
        }
    );
    assert_eq!(intent.status, IntentStatus::Applied);
    assert_eq!(w.position.coordination.generation, 2);
    assert!(!w.position.coordination.lock.is_active());
    assert_eq!(
        w.position.encrypted_state.nonce,
        output_nonce(&POSITION_KEY, 0)
    );
    assert_eq!(w.position.encrypted_state.ciphertexts, report.ciphertexts);

    // A duplicate callback is a no-op.
    let snapshot = (intent, w.position.clone());
    assert_eq!(
        c(w.settle(&mut intent, &report)),
        Ok(UnshieldSettlement::Ignored)
    );
    assert_eq!(snapshot.0, intent);
    assert_eq!(snapshot.1.encrypted_state, w.position.encrypted_state);
}

#[test]
fn zero_approval_stores_nothing_and_moves_nothing() {
    let mut w = world();
    let before = w.position.encrypted_state;
    let mut intent = w.queue(SOL, 70, 10).unwrap();
    let report = w.report(&intent, SOL, 0, 70, 11);
    assert_eq!(
        c(w.settle(&mut intent, &report)),
        Ok(UnshieldSettlement::Zero { asset: SOL })
    );
    assert_eq!(intent.status, IntentStatus::Zero);
    assert_eq!(w.position.encrypted_state, before);
    assert_eq!(w.position.coordination.generation, 1);
    assert!(!w.position.coordination.lock.is_active());
}

#[test]
fn partial_or_mismatched_approvals_are_rejected_atomically() {
    for (approved, requested) in [(5, 7), (8, 7)] {
        let mut w = world();
        let mut intent = w.queue(SOL, 7, 10).unwrap();
        let before = (intent, w.position.clone());
        let report = w.report(&intent, SOL, approved, requested, 11);
        assert_eq!(
            c(w.settle(&mut intent, &report)),
            Err(code(ErrorCode::UnshieldApprovalInvalid))
        );
        assert_eq!(before.0, intent);
        assert_eq!(before.1.encrypted_state, w.position.encrypted_state);
        assert!(w.position.coordination.lock.is_active());
    }
    // An echoed request other than the committed one fails the commitment.
    let mut w = world();
    let mut intent = w.queue(SOL, 7, 10).unwrap();
    let report = w.report(&intent, SOL, 0, 8, 11);
    assert_eq!(
        c(w.settle(&mut intent, &report)),
        Err(code(ErrorCode::CallbackInputsCommitmentMismatch))
    );
}

#[test]
fn substituted_callback_accounts_are_rejected() {
    let m = market();
    let base = |w: &World, intent: &OperationIntent| w.report(intent, USDC, 9, 9, 11);
    type Mutation = Box<dyn Fn(&mut UnshieldReport)>;
    let mutations: Vec<(&str, Mutation, ErrorCode)> = vec![
        (
            "destination key",
            Box::new(|r| r.destination.key = STRANGER),
            ErrorCode::CallbackAccountsCommitmentMismatch,
        ),
        (
            "vault",
            Box::new(move |r| r.vault = m.wsol_vault),
            ErrorCode::CallbackAccountsCommitmentMismatch,
        ),
        (
            "vault authority",
            Box::new(|r| r.vault_authority = STRANGER),
            ErrorCode::CallbackAccountsCommitmentMismatch,
        ),
        (
            "token program",
            Box::new(|r| r.token_program = STRANGER),
            ErrorCode::CallbackAccountsCommitmentMismatch,
        ),
        (
            "mint",
            Box::new(move |r| r.mint = m.wsol_mint),
            ErrorCode::CallbackAccountsCommitmentMismatch,
        ),
        (
            "unknown mint",
            Box::new(|r| r.mint = STRANGER),
            ErrorCode::UnshieldMintMismatch,
        ),
        (
            "destination token owner changed",
            Box::new(|r| r.destination.owner = STRANGER),
            ErrorCode::UnshieldDestinationInvalid,
        ),
        (
            "destination mint changed",
            Box::new(move |r| r.destination.mint = m.wsol_mint),
            ErrorCode::UnshieldDestinationInvalid,
        ),
        (
            "wrong computation",
            Box::new(|r| r.computation_offset += 1),
            ErrorCode::CallbackOffsetMismatch,
        ),
        (
            "wrong intent",
            Box::new(|r| r.intent_key = STRANGER),
            ErrorCode::CallbackIntentMismatch,
        ),
        (
            "wrong encryption key",
            Box::new(|r| r.encryption_key = [0x44; 32]),
            ErrorCode::CallbackInputsCommitmentMismatch,
        ),
        (
            "legacy stored-nonce + 1 output nonce",
            Box::new(|r| r.output_nonce = 1_001),
            ErrorCode::CallbackInputsCommitmentMismatch,
        ),
        (
            "another attempt's output nonce",
            Box::new(|r| r.output_nonce = output_nonce(&POSITION_KEY, 1)),
            ErrorCode::CallbackInputsCommitmentMismatch,
        ),
    ];
    for (name, mutate, expected) in mutations {
        let mut w = world();
        let mut intent = w.queue(USDC, 9, 10).unwrap();
        let before = (intent, w.position.clone());
        let mut report = base(&w, &intent);
        mutate(&mut report);
        assert_eq!(
            c(w.settle(&mut intent, &report)),
            Err(code(expected)),
            "{name}"
        );
        assert_eq!(before.0, intent, "{name}");
        assert_eq!(
            before.1.encrypted_state, w.position.encrypted_state,
            "{name}"
        );
        assert!(w.position.coordination.lock.is_active(), "{name}");
    }
}

#[test]
fn unauthenticated_and_late_callbacks_move_nothing() {
    let mut w = world();
    let mut intent = w.queue(SOL, 7, 10).unwrap();
    let report = UnshieldReport {
        output_authenticated: false,
        ..w.report(&intent, SOL, 7, 7, 11)
    };
    assert_eq!(
        c(w.settle(&mut intent, &report)),
        Err(code(ErrorCode::CallbackOutputNotAuthenticated))
    );
    assert_eq!(intent.status, IntentStatus::Queued);
    assert!(w.position.coordination.lock.is_active());

    // Delivered after Decision Expiry: Expired, no transfer, state unchanged.
    let late = intent.decision_expiry_slot + 1;
    let report = w.report(&intent, SOL, 7, 7, late);
    assert_eq!(
        c(w.settle(&mut intent, &report)),
        Ok(UnshieldSettlement::Failed(IntentStatus::Expired))
    );
    assert_eq!(w.position.encrypted_state.nonce, 1_000);
    assert!(!w.position.coordination.lock.is_active());
}

#[test]
fn abandon_requires_lock_expiry_and_anyone_may_call() {
    let mut w = world();
    let mut intent = w.queue(SOL, 7, 10).unwrap();
    assert!(abandon_unshield(&mut w.position, POSITION_KEY, &mut intent, 11).is_err());
    assert!(w.position.coordination.lock.is_active());
    let expired = intent.lock_expiry_slot + 1;
    abandon_unshield(&mut w.position, POSITION_KEY, &mut intent, expired).unwrap();
    assert_eq!(intent.status, IntentStatus::Abandoned);
    assert!(!w.position.coordination.lock.is_active());
    assert_eq!(w.position.encrypted_state.nonce, 1_000);
    // A terminal Intent cannot be abandoned twice, and a foreign Intent is
    // rejected.
    assert_eq!(
        c(abandon_unshield(&mut w.position, POSITION_KEY, &mut intent, 9_999).map(|_| ())),
        Err(code(ErrorCode::IntentNotQueued))
    );
    let mut other = w.queue(SOL, 1, 20).unwrap();
    assert_eq!(
        c(abandon_unshield(&mut w.position, STRANGER, &mut other, 99_999).map(|_| ())),
        Err(code(ErrorCode::UnshieldIntentMismatch))
    );
}

#[test]
fn retries_after_a_non_applied_attempt_use_a_new_output_nonce() {
    let mut w = world();
    let stored = w.position.encrypted_state.nonce;
    let mut seen = HashSet::from([(VIEWING, stored)]);
    for round in 0..6u64 {
        let mut intent = w.queue(SOL, 7, 10 + round).unwrap();
        let report = w.report(&intent, SOL, 0, 7, 11 + round);
        assert!(seen.insert((report.encryption_key, report.output_nonce)));
        assert_ne!(report.output_nonce, stored + 1);
        w.settle(&mut intent, &report).unwrap();
        assert_eq!(w.position.encrypted_state.nonce, stored);
    }
}

// ---- Property: value conservation across mixed sequences ---------------------

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
}

enum Inflight {
    Shield {
        shield: PendingShield,
        intent: OperationIntent,
    },
    Unshield {
        intent: OperationIntent,
        asset: ShieldAsset,
        amount: u64,
    },
}

/// Random shield / unshield / zero / expired / unauthenticated / abandoned /
/// replayed-callback sequences over both assets through the real rules.
/// After every step: each vault equals the modelled free balance plus the
/// still-deposited shields; every transfer equals an `Applied` approved
/// request that did not exceed the free balance; failed, zero, stale, and
/// abandoned attempts move nothing; stray callbacks of old attempts are
/// no-ops; and no output (key, nonce) repeats across attempts.
///
/// **Validates: Requirements 5.7, 5.8, 4.8, 2.3**
#[test]
fn prop_unshield_conserves_the_vault_and_never_reuses_an_output_nonce() {
    let mut rng = Lcg(0x5EED_1234);
    for _ in 0..300 {
        let mut w = world();
        let mut slot: Slot = 10;
        let mut vault = [0u128; 2];
        let mut free = [0u128; 2];
        let mut pending = [0u128; 2];
        let mut inflight: Option<Inflight> = None;
        let mut old: Vec<(OperationIntent, ShieldAsset, u64)> = Vec::new();
        let mut outputs = HashSet::from([(VIEWING, 1_000u128)]);
        let mut shield_sequence = 0u64;
        for _ in 0..50 {
            let action = rng.below(8);
            match (action, inflight.take()) {
                (0..=2, None) => {
                    let asset = if rng.below(2) == 0 { SOL } else { USDC };
                    let amount = rng.below(900);
                    if rng.below(2) == 0 {
                        // Shield (the transfer into the vault comes first).
                        let sequence = w.counter.next_sequence;
                        let mut shield = PendingShield::default();
                        let new = NewPendingShield {
                            owner: OWNER,
                            position: POSITION_KEY,
                            source_token_account: key(0x77),
                            asset,
                            amount,
                            sequence: w.position.next_shield_seq,
                            bump: 0xFB,
                        };
                        shield_sequence = new.sequence;
                        let request = ShieldRequest {
                            intent_key: intent_key(sequence),
                            sequence,
                            computation_offset: 9_000 + sequence,
                            queued_slot: slot,
                        };
                        let result = queue_shield(
                            &w.market,
                            MARKET_KEY,
                            &w.pauses,
                            &mut w.position,
                            POSITION_KEY,
                            &mut w.counter,
                            &mut shield,
                            key(0x50 + shield_sequence as u8),
                            &new,
                            &request,
                        );
                        assert_eq!(result.is_ok(), amount > 0);
                        if let Ok(intent) = result {
                            vault[asset_tag(asset) as usize] += u128::from(amount);
                            pending[asset_tag(asset) as usize] += u128::from(amount);
                            inflight = Some(Inflight::Shield { shield, intent });
                        }
                    } else {
                        let result = w.queue(asset, amount, slot);
                        assert_eq!(result.is_ok(), amount > 0);
                        if let Ok(intent) = result {
                            inflight = Some(Inflight::Unshield {
                                intent,
                                asset,
                                amount,
                            });
                        }
                    }
                }
                (0..=2, Some(op)) => {
                    // The Lock is held: any new queue is rejected.
                    assert_eq!(
                        c(w.queue(SOL, 1, slot).map(|_| ())),
                        Err(code(ErrorCode::LockHeld))
                    );
                    inflight = Some(op);
                }
                (
                    3 | 4,
                    Some(Inflight::Shield {
                        mut shield,
                        mut intent,
                    }),
                ) => {
                    slot += 1;
                    let credited = action == 3;
                    let callback = ShieldCallback {
                        output_authenticated: true,
                        computation_offset: intent.computation_offset,
                        intent_key: intent.key,
                        encryption_key: VIEWING,
                        output_nonce: output_nonce(&POSITION_KEY, intent.sequence),
                        ciphertexts: [[intent.sequence as u8 ^ 0x5A; 32]; 5],
                        credited,
                        current_slot: slot,
                    };
                    assert!(outputs.insert((VIEWING, callback.output_nonce)));
                    let result = settle_shield_credit(
                        &w.market,
                        MARKET_KEY,
                        &mut w.position,
                        POSITION_KEY,
                        &mut shield,
                        key(0x50 + shield_sequence as u8),
                        &mut intent,
                        &callback,
                    )
                    .unwrap();
                    if let ShieldSettlement::Credited { asset, amount } = result {
                        let i = asset_tag(asset) as usize;
                        free[i] += u128::from(amount);
                        pending[i] -= u128::from(amount);
                    }
                }
                (
                    3 | 4,
                    Some(Inflight::Unshield {
                        mut intent,
                        asset,
                        amount,
                    }),
                ) => {
                    slot += 1;
                    let i = asset_tag(asset) as usize;
                    // The circuit: all or nothing against the free balance.
                    let approved = if u128::from(amount) <= free[i] {
                        amount
                    } else {
                        0
                    };
                    let report = w.report(&intent, asset, approved, amount, slot);
                    assert!(outputs.insert((VIEWING, report.output_nonce)));
                    let result = w.settle(&mut intent, &report).unwrap();
                    match result {
                        // Delivered after Decision Expiry: nothing moves.
                        UnshieldSettlement::Failed(status) => {
                            assert_eq!(status, IntentStatus::Expired);
                            assert!(slot > intent.decision_expiry_slot);
                        }
                        UnshieldSettlement::Approved {
                            asset: a,
                            amount: paid,
                            destination,
                        } => {
                            assert_eq!((a, paid, destination), (asset, amount, DEST));
                            assert!(approved > 0 && u128::from(paid) <= free[i]);
                            free[i] -= u128::from(paid);
                            vault[i] -= u128::from(paid);
                        }
                        other => {
                            assert_eq!(approved, 0);
                            assert_eq!(other, UnshieldSettlement::Zero { asset });
                        }
                    }
                    old.push((intent, asset, amount));
                }
                (5, Some(op)) => {
                    // Late delivery: Expired, or unauthenticated: reverts.
                    slot = slot.max(match &op {
                        Inflight::Shield { intent, .. } | Inflight::Unshield { intent, .. } => {
                            intent.decision_expiry_slot + 1
                        }
                    });
                    match op {
                        Inflight::Unshield {
                            mut intent,
                            asset,
                            amount,
                        } => {
                            let i = asset_tag(asset) as usize;
                            let approved = if u128::from(amount) <= free[i] {
                                amount
                            } else {
                                0
                            };
                            let mut report = w.report(&intent, asset, approved, amount, slot);
                            if rng.below(2) == 0 {
                                report.output_authenticated = false;
                                assert!(w.settle(&mut intent, &report).is_err());
                                assert_eq!(intent.status, IntentStatus::Queued);
                                inflight = Some(Inflight::Unshield {
                                    intent,
                                    asset,
                                    amount,
                                });
                            } else {
                                assert!(outputs.insert((VIEWING, report.output_nonce)));
                                assert_eq!(
                                    c(w.settle(&mut intent, &report)),
                                    Ok(UnshieldSettlement::Failed(IntentStatus::Expired))
                                );
                                old.push((intent, asset, amount));
                            }
                        }
                        Inflight::Shield { shield, intent } => {
                            // Resolved by the next abandonment instead.
                            inflight = Some(Inflight::Shield { shield, intent });
                        }
                    }
                }
                (6, Some(op)) => {
                    // Abandon after Lock expiry (any signer).
                    match op {
                        Inflight::Shield { shield, mut intent } => {
                            slot = slot.max(intent.lock_expiry_slot + 1);
                            abandon_shield(
                                &mut w.position,
                                POSITION_KEY,
                                &shield,
                                &mut intent,
                                slot,
                            )
                            .unwrap();
                        }
                        Inflight::Unshield {
                            mut intent,
                            asset,
                            amount,
                        } => {
                            slot = slot.max(intent.lock_expiry_slot + 1);
                            abandon_unshield(&mut w.position, POSITION_KEY, &mut intent, slot)
                                .unwrap();
                            old.push((intent, asset, amount));
                        }
                    }
                }
                (7, op) => {
                    // A stray callback of an already-terminal unshield is a
                    // no-op: nothing is stored and nothing moves.
                    if let Some((mut intent, asset, amount)) = old.last().copied() {
                        let before = (intent, w.position.encrypted_state);
                        let report = w.report(&intent, asset, amount, amount, slot);
                        assert_eq!(
                            c(w.settle(&mut intent, &report)),
                            Ok(UnshieldSettlement::Ignored)
                        );
                        assert_eq!(before, (intent, w.position.encrypted_state));
                    }
                    inflight = op;
                }
                (_, op) => inflight = op,
            }
            assert_eq!(
                vault,
                [free[0] + pending[0], free[1] + pending[1]],
                "vault conservation"
            );
            match &inflight {
                Some(Inflight::Shield { intent, .. } | Inflight::Unshield { intent, .. }) => {
                    assert!(intent.lock_owner().holds(&w.position.coordination.lock));
                }
                None => assert!(!w.position.coordination.lock.is_active()),
            }
        }
    }
}
