// Two-owner circuit outputs return a tuple of `Enc<Shared, _>` values; this is
// the documented Arcium sealing shape, so allow the type-complexity lint crate-wide
// rather than hiding it behind type aliases that the Arcis macro may not parse.
#![allow(clippy::type_complexity)]

use arcis::*;

/// Arcis circuits for Walnut-Solana.
///
/// Scaffold plus the Task 2.2 Arcium feasibility gate.
///
/// * `add_together` is the standard Arcium hello-world circuit used to verify
///   the toolchain (Task 1.5).
/// * `init_position` and `apply_position_delta` prove that the complete
///   proposed Position payload can be persisted as encrypted state in an
///   account, read back into a *later* computation, and updated across many
///   computations (Task 2.2, Requirement 2.2). They carry no financial logic:
///   `apply_position_delta` only adds caller-supplied per-field deltas so the
///   test can drive randomized-order updates and assert the stored state stays
///   consistent. Interest, pricing, and liquidation math arrive only after the
///   feasibility gate passes.
///
/// The Position payload is modelled as `Enc<Shared, PositionSecret>`: the
/// design's preferred "one structure owned by the user's Viewing Key" layout,
/// which lets the client decrypt the stored state locally (the `Enc<Shared>`
/// local-read model). The `Enc<Mxe>` fallback is only adopted by ADR-0003 if
/// this shared layout cannot persist safely (Task 2.3 / Requirement 2.5).
///
/// Task 2.3 adds the nonce and ownership-safety circuits:
///
/// * `liquidation_receipts` proves a single computation can output two
///   independent `Enc<Shared, _>` results, each sealed to a *different* owner's
///   Viewing Key, so each owner decrypts only their own Receipt. This is the
///   "liquidation writes two owners" cross-owner case (Requirement 2.3). Each
///   output carries its own `encryption_key` and `nonce`, so the effective
///   (key, nonce) identity differs per owner even when the plaintext overlaps.
/// * `rotate_viewing_key` re-encrypts the stored Position payload from the old
///   Viewing Key to a new one, so the callback can atomically swap the stored
///   public key and increment the epoch (Requirement 5.11). The new key is
///   supplied as a throwaway `Enc<Shared, u64>` input purely so the circuit
///   can seal to `new_key.owner`; the probe plaintext is discarded.
///
/// Task 2.4 adds one failure-matrix circuit:
///
/// * `oversized_output` returns a 48-field encrypted array so the sealed output
///   cannot fit a single callback transaction, forcing the `OutputTooLarge`
///   failure path (Requirement 2.4). It carries no financial logic.
///
/// Nonce note for ADR-0003: the MXE derives each output nonce as the owning
/// input's nonce + 1 (Arcium encryption docs). Clients therefore MUST pick a
/// fresh random 16-byte nonce per encryption and never pick sequential nonces
/// under one key, so that no effective (key, nonce) pair ever repeats across
/// inputs and outputs. Distinct owners always have distinct effective keys, so
/// their (key, nonce) pairs never collide with each other.
#[encrypted]
mod circuits {
    use arcis::*;

    pub struct InputValues {
        v1: u8,
        v2: u8,
    }

    #[instruction]
    pub fn add_together(input_ctxt: Enc<Shared, InputValues>) -> Enc<Shared, u16> {
        let input = input_ctxt.to_arcis();
        let sum = input.v1 as u16 + input.v2 as u16;
        input_ctxt.owner.from_arcis(sum)
    }

    /// The complete proposed Position payload (design: `PositionSecret`).
    ///
    /// Every encrypted Position field lives in this one structure so a single
    /// Viewing Key / nonce sequence covers the whole payload. Field order here
    /// is the canonical ciphertext order: the queue instruction and the client
    /// must serialize the five scalars in exactly this sequence.
    #[derive(Copy, Clone)]
    pub struct PositionSecret {
        free_sol: u64,
        free_usdc: u64,
        collateral_sol: u64,
        supply_shares_usdc: u128,
        debt_shares_usdc: u128,
    }

    /// A per-field additive update to a `PositionSecret`.
    ///
    /// The feasibility gate only needs to prove that fields can be updated in
    /// arbitrary order across successive computations while the persisted state
    /// stays consistent, so each update is a plain addition. No financial rule
    /// (balance checks, rounding, interest) is applied here.
    #[derive(Copy, Clone)]
    pub struct PositionDelta {
        d_free_sol: u64,
        d_free_usdc: u64,
        d_collateral_sol: u64,
        d_supply_shares_usdc: u128,
        d_debt_shares_usdc: u128,
    }

    /// Re-encrypts a client-supplied initial Position payload to its owner so
    /// the callback can persist it as the account's first encrypted state.
    ///
    /// This is the "store" step: the returned ciphertext (with its own output
    /// nonce) is what the callback writes into the Position account.
    #[instruction]
    pub fn init_position(initial: Enc<Shared, PositionSecret>) -> Enc<Shared, PositionSecret> {
        let state = initial.to_arcis();
        initial.owner.from_arcis(state)
    }

    /// Reads the previously stored Position payload, applies a per-field delta,
    /// and re-encrypts the updated payload to the same owner.
    ///
    /// `state` is the account-backed encrypted input (the ciphertext persisted
    /// by an earlier `init_position` or `apply_position_delta` callback), which
    /// proves persistence *across* computations rather than within one call.
    #[instruction]
    pub fn apply_position_delta(
        state: Enc<Shared, PositionSecret>,
        delta: Enc<Shared, PositionDelta>,
    ) -> Enc<Shared, PositionSecret> {
        let mut s = state.to_arcis();
        let d = delta.to_arcis();
        s.free_sol += d.d_free_sol;
        s.free_usdc += d.d_free_usdc;
        s.collateral_sol += d.d_collateral_sol;
        s.supply_shares_usdc += d.d_supply_shares_usdc;
        s.debt_shares_usdc += d.d_debt_shares_usdc;
        state.owner.from_arcis(s)
    }

    // ---- Task 2.3: nonce and ownership safety -----------------------------

    /// Per-owner input to the two-owner liquidation computation.
    ///
    /// Feasibility scaffolding only: `amount` and `aux` are opaque markers, not
    /// financial quantities. The real partial-liquidation circuit (Task 12)
    /// computes seized collateral and repaid debt under the committed USD
    /// scale; here they only let the client predict both encrypted outputs.
    #[derive(Copy, Clone)]
    pub struct ReceiptInput {
        amount: u64,
        aux: u64,
    }

    /// Proves one computation can emit two `Enc<Shared, _>` outputs, each sealed
    /// to a different owner's Viewing Key (the "liquidation writes two owners"
    /// case, Requirement 2.3).
    ///
    /// Each Receipt is returned as an `Enc<Shared, (u64, u64)>` (a `(seized,
    /// remaining)` pair) rather than a named struct, so the output has no
    /// write-only fields. `transfer` is taken from the borrower input and
    /// credited to the liquidator, so both outputs depend on shared private
    /// data, yet each owner can decrypt only the Receipt sealed to their own
    /// key. The tuple return type makes Arcium generate a two-field output
    /// struct, one `SharedEncryptedStruct<2>` per owner, each with its own
    /// `encryption_key` and `nonce`. No financial rule is applied; the
    /// transforms are plain additions so the client can predict both plaintexts.
    #[instruction]
    pub fn liquidation_receipts(
        borrower: Enc<Shared, ReceiptInput>,
        liquidator: Enc<Shared, ReceiptInput>,
    ) -> (Enc<Shared, (u64, u64)>, Enc<Shared, (u64, u64)>) {
        let b = borrower.to_arcis();
        let l = liquidator.to_arcis();
        let transfer = b.amount;
        let borrower_receipt = (transfer, b.aux);
        let liquidator_receipt = (transfer, l.aux + transfer);
        (
            borrower.owner.from_arcis(borrower_receipt),
            liquidator.owner.from_arcis(liquidator_receipt),
        )
    }

    /// Re-encrypts the stored Position payload from the old Viewing Key to a new
    /// one (Requirement 5.11, key rotation).
    ///
    /// `state` is the account-backed payload still sealed to the old key; the
    /// new key arrives as `new_key`, a throwaway `Enc<Shared, u64>` whose only
    /// purpose is to carry the new key into the circuit so the re-encrypted
    /// state can be sealed to `new_key.owner`. The decrypted probe value is
    /// discarded. The plaintext payload is unchanged, so after the callback
    /// writes the output the Position decrypts under the new key and no longer
    /// under the old one.
    #[instruction]
    pub fn rotate_viewing_key(
        state: Enc<Shared, PositionSecret>,
        new_key: Enc<Shared, u64>,
    ) -> Enc<Shared, PositionSecret> {
        let s = state.to_arcis();
        let _ = new_key.to_arcis();
        new_key.owner.from_arcis(s)
    }

    // ---- Task 2.4: deliberately oversized output --------------------------

    /// Produces an output large enough that the node cannot build the callback
    /// inside Solana's single-transaction limit, so the computation fails with
    /// `OutputTooLarge` (Requirement 2.4). The logic is trivial on purpose: the
    /// point is the output *size*, not the computation. One encrypted scalar is
    /// replicated across 48 fields, so the sealed output carries 48 32-byte
    /// ciphertexts (1,536 bytes) which, with the callback's signatures and
    /// accounts, cannot fit in 1,232 bytes.
    ///
    /// This is feasibility-gate scaffolding only; no production circuit returns
    /// an output this large.
    #[instruction]
    pub fn oversized_output(input: Enc<Shared, u64>) -> Enc<Shared, [u64; 48]> {
        let v = input.to_arcis();
        input.owner.from_arcis([v; 48])
    }
}
