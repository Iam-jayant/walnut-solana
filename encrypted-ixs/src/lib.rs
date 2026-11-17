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
/// Task 4.1 adds the first production circuit:
///
/// * `init_pool_state` seals the all-zero `PoolSecret` to the MXE so the USDC
///   Pool State starts with a valid `Enc<Mxe>` payload.
///
/// Task 5.1 adds `init_position_state`, which seals the all-zero
/// `PositionSecret` to a production Position owner's Viewing Key.
///
/// Task 5.3 adds `shield_credit`, which adds a public shielded amount to the
/// stored Position's free SOL or free USDC balance. Task 5.4 adds `unshield`,
/// which debits a public requested amount all-or-nothing. Both seal their
/// output to a bare `Shared` input that carries a program-derived unique
/// nonce, so no attempt ever reuses an output (key, nonce).
///
/// Task 5.5 adds `rotate_position_key`, which re-seals the stored Position
/// payload to a new Viewing public key with a program-derived output nonce.
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

    // ---- Task 4.1: Pool State initialization ------------------------------

    /// The encrypted USDC Pool payload (design: `PoolSecret`).
    ///
    /// Field order is the canonical ciphertext order stored in
    /// `PoolState.encrypted.ciphertexts`.
    #[derive(Copy, Clone)]
    pub struct PoolSecret {
        cash_usdc: u128,
        total_supply_shares: u128,
        total_debt_shares: u128,
        reserves_usdc: u128,
    }

    /// Produces the all-zero Pool payload sealed to the MXE (`Enc<Mxe>`).
    ///
    /// No client can encrypt to the MXE, so `initialize_market` leaves the Pool
    /// `Initializing` and this computation creates the first payload. It takes
    /// no input and reveals nothing.
    #[instruction]
    pub fn init_pool_state() -> Enc<Mxe, PoolSecret> {
        let zero = PoolSecret {
            cash_usdc: 0,
            total_supply_shares: 0,
            total_debt_shares: 0,
            reserves_usdc: 0,
        };
        Mxe::get().from_arcis(zero)
    }

    // ---- Task 5.1: production Position initialization ----------------------

    /// Produces the all-zero `PositionSecret` sealed to the owner's Viewing
    /// Key (`Enc<Shared>`), the first encrypted state of a production Position.
    ///
    /// `owner` is a bare `Shared` parameter: the queue instruction passes the
    /// registered `viewing_pubkey` and a fresh client nonce, and no ciphertext.
    /// The circuit takes no client-chosen value, so the stored state is zero
    /// by construction. The output nonce is the input nonce + 1 (ADR-0003); the
    /// callback relies on that to bind the input nonce to the Intent.
    #[instruction]
    pub fn init_position_state(owner: Shared) -> Enc<Shared, PositionSecret> {
        let zero = PositionSecret {
            free_sol: 0,
            free_usdc: 0,
            collateral_sol: 0,
            supply_shares_usdc: 0,
            debt_shares_usdc: 0,
        };
        owner.from_arcis(zero)
    }

    // ---- Task 5.3: shield credit -------------------------------------------

    /// Credits a public shield `amount` to `free_usdc` (`is_usdc`) or
    /// `free_sol` of the stored Position state and re-seals it to the owner.
    ///
    /// `state` is the account-backed payload the queue instruction reads from
    /// the Position. `owner` is a bare `Shared` (the Viewing public key plus a
    /// fresh output-nonce input, no ciphertext): the result is sealed to
    /// `owner`, so the output nonce is `owner`'s nonce + 1. The queue
    /// instruction derives that nonce from the Position and the unique
    /// Operation Intent sequence, so no two attempts, applied or not, ever
    /// seal under the same (key, nonce). The output nonce is independent of
    /// the stored state's nonce (ADR-0003, nonce safety).
    ///
    /// `is_usdc` and `amount` are public: the SPL transfer into the vault
    /// already revealed them. The sum is formed in `u128`; if it does not fit
    /// in `u64` the state is returned unchanged and the revealed flag is
    /// `false`, so the callback resolves `Zero` and the Pending Shield stays
    /// refundable. Nothing wraps. The flag reveals only that the balance would
    /// overflow, which cannot happen while every balance is backed by a `u64`
    /// vault. No other field changes.
    #[instruction]
    pub fn shield_credit(
        state: Enc<Shared, PositionSecret>,
        owner: Shared,
        is_usdc: bool,
        amount: u64,
    ) -> (Enc<Shared, PositionSecret>, bool) {
        let mut s = state.to_arcis();
        let current = if is_usdc { s.free_usdc } else { s.free_sol };
        let sum = current as u128 + amount as u128;
        let fits = sum <= 18_446_744_073_709_551_615u128;
        let credited = if fits { sum as u64 } else { current };
        if is_usdc {
            s.free_usdc = credited;
        } else {
            s.free_sol = credited;
        }
        (owner.from_arcis(s), fits.reveal())
    }

    // ---- Task 5.4: unshield -------------------------------------------------

    /// Debits a public requested `amount` from the free USDC (`is_usdc`) or
    /// SOL balance of the stored Position state, all or nothing, and re-seals
    /// the result to `owner` (see `shield_credit` for the output-nonce rule).
    ///
    /// If the free balance of the asset is at least `amount`, the amount is
    /// subtracted and the revealed approved amount equals the request.
    /// Otherwise the state is returned unchanged and the approved amount is
    /// zero (Requirement 5.7): never a partial approval, never an underflow
    /// (the subtrahend is zero when the request is refused). The request is
    /// revealed too: it is public anyway (the outgoing SPL transfer shows
    /// it), and the callback needs it to rebuild the Intent's input
    /// commitment even when the approved amount is zero. No other field
    /// changes.
    #[instruction]
    pub fn unshield(
        state: Enc<Shared, PositionSecret>,
        owner: Shared,
        is_usdc: bool,
        amount: u64,
    ) -> (Enc<Shared, PositionSecret>, u64, u64) {
        let mut s = state.to_arcis();
        let current = if is_usdc { s.free_usdc } else { s.free_sol };
        let approved = current >= amount;
        let debit = if approved { amount } else { 0 };
        let remaining = current - debit;
        if is_usdc {
            s.free_usdc = remaining;
        } else {
            s.free_sol = remaining;
        }
        (owner.from_arcis(s), debit.reveal(), amount.reveal())
    }

    // ---- Task 5.5: production Viewing Key rotation --------------------------

    /// Re-seals the stored Position payload from its current Viewing Key to a
    /// new one, revealing nothing (Requirement 5.11).
    ///
    /// `state` is the account-backed payload still sealed to the OLD key
    /// (stored public key, stored nonce, five ciphertexts). `new_owner` is a
    /// bare `Shared` input: the NEW Viewing public key and a fresh
    /// output-nonce input, no ciphertext. The five values are copied
    /// unchanged and sealed to `new_owner`, so the output nonce is
    /// `new_owner`'s nonce + 1. The queue instruction derives that nonce from
    /// the Position and the unique Operation Intent sequence (see
    /// `shield_credit`), so no rotation attempt, applied or not, ever seals
    /// under a repeated (key, nonce).
    ///
    /// Unlike the Task 2.3 spike `rotate_viewing_key`, no throwaway
    /// ciphertext carries the new key and the client picks no nonce. No
    /// value is revealed: the callback learns only the sealed output, and the
    /// new key and epoch are already public in the Intent.
    #[instruction]
    pub fn rotate_position_key(
        state: Enc<Shared, PositionSecret>,
        new_owner: Shared,
    ) -> Enc<Shared, PositionSecret> {
        let s = state.to_arcis();
        new_owner.from_arcis(s)
    }
}
