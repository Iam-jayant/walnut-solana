// Two-owner circuit outputs return a tuple of `Enc<Shared, _>` values; this is
// the documented Arcium sealing shape, so allow the type-complexity lint crate-wide
// rather than hiding it behind type aliases that the Arcis macro may not parse.
#![allow(clippy::type_complexity)]
// Fixed-point and checkpoint helpers are called by the circuits of Tasks 7.4 to 7.6
// and by the equivalence tests; until then they are unused in the library build.
#![allow(dead_code)]

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

    // ---- Task 7.1: fixed-point helpers ----------------------------------------
    //
    // Plain `u128` arithmetic, the same algorithm as
    // `programs/walnut/src/math/fixed.rs` and
    // `packages/reference-model/src/fixed.ts`. Arcis has no 256-bit type and no
    // `Result`, so every helper assumes its caller range-checked the operands
    // against the committed domain (`shares <= 10^16`, `WAD <= index <= 10^21`,
    // pool values <= 2^96), which keeps `a * b` inside `u128`. The `*_checked`
    // helpers perform that check and return an `ok` flag instead of failing;
    // on `!ok` they compute with harmless clamped inputs (never an overflow or
    // an underflow) and the caller must treat the whole step as refused.
    //
    // Equivalence with the program and the TypeScript mirror is established by
    // the `tests` module at the bottom of this file, which replays the shared
    // golden vectors (`packages/reference-model/vectors/*.vectors`) against
    // these exact functions.

    /// `10^18`, the WAD scale.
    pub fn wad() -> u128 {
        1_000_000_000_000_000_000
    }

    /// `floor(a * b / d)`. `d` must be nonzero and `a * b` must fit `u128`.
    pub fn mul_div_floor(a: u128, b: u128, d: u128) -> u128 {
        (a * b) / d
    }

    /// `ceil(a * b / d)`. `d` must be nonzero and `a * b` must fit `u128`.
    pub fn mul_div_ceil(a: u128, b: u128, d: u128) -> u128 {
        let p = a * b;
        let q = p / d;
        if q * d == p {
            q
        } else {
            q + 1
        }
    }

    /// `floor(a * b / WAD)`.
    pub fn mul_wad_floor(a: u128, b: u128) -> u128 {
        (a * b) / 1_000_000_000_000_000_000
    }

    /// `ceil(a * b / WAD)`.
    pub fn mul_wad_ceil(a: u128, b: u128) -> u128 {
        let p = a * b;
        let q = p / 1_000_000_000_000_000_000;
        if q * 1_000_000_000_000_000_000 == p {
            q
        } else {
            q + 1
        }
    }

    /// `floor(a * WAD / b)`; a zero `b` yields zero.
    pub fn div_wad_floor(a: u128, b: u128) -> u128 {
        let safe = if b == 0 { 1 } else { b };
        let q = (a * 1_000_000_000_000_000_000) / safe;
        if b == 0 {
            0
        } else {
            q
        }
    }

    /// `ceil(a * WAD / b)`; a zero `b` yields zero.
    pub fn div_wad_ceil(a: u128, b: u128) -> u128 {
        let safe = if b == 0 { 1 } else { b };
        let p = a * 1_000_000_000_000_000_000;
        let q = p / safe;
        let up = if q * safe == p { q } else { q + 1 };
        if b == 0 {
            0
        } else {
            up
        }
    }

    /// Whether `index` is a valid Borrow Index: `WAD <= index <= 1000 * WAD`.
    pub fn index_in_range(index: u128) -> bool {
        index >= 1_000_000_000_000_000_000 && index <= 1_000_000_000_000_000_000_000
    }

    /// Debt units `ceil(shares * index / WAD)` with the domain check. Returns
    /// `(debt, ok)`; on `!ok` the debt is zero.
    pub fn debt_from_shares_checked(shares: u128, index: u128) -> (u128, bool) {
        let ok = shares <= 10_000_000_000_000_000 && index_in_range(index);
        let s = if ok { shares } else { 0 };
        let i = if ok { index } else { 1_000_000_000_000_000_000 };
        (mul_wad_ceil(s, i), ok)
    }

    // ---- Task 7.2: utilization band -------------------------------------------

    /// The utilization band (0..=19, 5% each) of `debt / (cash + debt)` with
    /// `debt = ceil(debt_shares * index / WAD)`, computed without dividing by a
    /// secret: band = number of edges `5k%` (`k = 1..=19`) that
    /// `debt * 10_000 >= total * k * 500` reaches. A zero total is band 0.
    ///
    /// Out-of-domain inputs (debt shares above 10^16, index outside
    /// `[WAD, 1000 * WAD]`, cash above 2^96) return the highest band (19):
    /// the conservative choice, since the accrual rate uses the band's upper
    /// edge.
    pub fn utilization_band_index(cash: u128, debt_shares: u128, index: u128) -> u8 {
        let (debt, debt_ok) = debt_from_shares_checked(debt_shares, index);
        let cash_ok = cash <= 79_228_162_514_264_337_593_543_950_336;
        let c = if cash_ok { cash } else { 0 };
        let total = c + debt;
        let scaled = debt * 10_000;
        let mut band: u8 = 0;
        for k in 1..20usize {
            let edge = total * (k as u128 * 500);
            if scaled >= edge {
                band += 1;
            }
        }
        let band = if total == 0 { 0 } else { band };
        if debt_ok && cash_ok {
            band
        } else {
            19
        }
    }

    /// Reads the Pool's encrypted Cash and Debt Shares and reveals only the
    /// utilization band index at the public accounting Borrow Index.
    ///
    /// Read-only: it neither checkpoints nor writes the Pool, so there is no
    /// encrypted output and no nonce to derive. `pool` is the account-backed
    /// `Enc<Mxe, PoolSecret>`; `accounting_index` is the public captured
    /// index. The callback applies the band only if the Pool generation, the
    /// accounting index, and the rate epoch still match the Intent.
    #[instruction]
    pub fn update_utilization_band(pool: Enc<Mxe, PoolSecret>, accounting_index: u128) -> u8 {
        let p = pool.to_arcis();
        utilization_band_index(p.cash_usdc, p.total_debt_shares, accounting_index).reveal()
    }

    // ---- Task 7.3: Pool interest checkpoint -----------------------------------

    /// Scalar form of the Pool interest checkpoint (see
    /// `programs/walnut/src/math/checkpoint.rs` for the definition and the
    /// rounding): returns `(reserves', supplier_equity, new_debt, ok)`.
    ///
    /// ```text
    /// old_debt    = ceil(debt_shares * accounted_index  / WAD)
    /// new_debt    = ceil(debt_shares * accounting_index / WAD)
    /// reserve_add = floor((new_debt - old_debt) * reserve_factor_bps / 10_000)
    /// reserves'   = reserves + reserve_add
    /// equity      = cash + new_debt - reserves'
    /// ```
    ///
    /// `ok` is false (and reserves are returned unchanged, equity zero) when
    /// an index is out of range, the accounting index is below the accounted
    /// one, debt shares exceed 10^16, cash or reserves exceed 2^96, the
    /// reserve factor exceeds 10,000 bps, or reserves exceed `cash + new_debt`.
    pub fn checkpoint_pool_values(
        cash: u128,
        debt_shares: u128,
        reserves: u128,
        accounted_index: u128,
        accounting_index: u128,
        reserve_factor_bps: u128,
    ) -> (u128, u128, u128, bool) {
        let max_value: u128 = 79_228_162_514_264_337_593_543_950_336;
        let inputs_ok = index_in_range(accounted_index)
            && index_in_range(accounting_index)
            && accounting_index >= accounted_index
            && debt_shares <= 10_000_000_000_000_000
            && cash <= max_value
            && reserves <= max_value
            && reserve_factor_bps <= 10_000;
        let s = if inputs_ok { debt_shares } else { 0 };
        let a = if inputs_ok {
            accounted_index
        } else {
            1_000_000_000_000_000_000
        };
        let b = if inputs_ok {
            accounting_index
        } else {
            1_000_000_000_000_000_000
        };
        let rf = if inputs_ok { reserve_factor_bps } else { 0 };
        let c = if inputs_ok { cash } else { 0 };
        let r = if inputs_ok { reserves } else { 0 };
        let old_debt = mul_wad_ceil(s, a);
        let new_debt = mul_wad_ceil(s, b);
        let interest = new_debt - old_debt;
        let reserve_add = (interest * rf) / 10_000;
        let r2 = r + reserve_add;
        let assets = c + new_debt;
        let solvent = assets >= r2;
        let sub = if solvent { r2 } else { 0 };
        let equity = assets - sub;
        let ok = inputs_ok && solvent;
        let out_reserves = if ok { r2 } else { reserves };
        let out_equity = if ok { equity } else { 0 };
        (out_reserves, out_equity, new_debt, ok)
    }

    /// The Pool interest checkpoint over the decrypted payload. This is the
    /// FIRST step of every Pool-mutating circuit (supply, withdraw, borrow,
    /// repay, reserve withdrawal, partial liquidation, Absorb): call it
    /// before any share or Cash mutation and use the returned payload,
    /// `supplier_equity` (the NAV before the operation), and `ok`. When `ok`
    /// is false the payload is returned unchanged and the caller must refuse
    /// the operation (move no value).
    ///
    /// Returns `(pool', supplier_equity, ok)`. Only `reserves_usdc` changes.
    pub fn checkpoint_pool(
        pool: PoolSecret,
        accounted_index: u128,
        accounting_index: u128,
        reserve_factor_bps: u128,
    ) -> (PoolSecret, u128, bool) {
        let (reserves, equity, _new_debt, ok) = checkpoint_pool_values(
            pool.cash_usdc,
            pool.total_debt_shares,
            pool.reserves_usdc,
            accounted_index,
            accounting_index,
            reserve_factor_bps,
        );
        let next = PoolSecret {
            cash_usdc: pool.cash_usdc,
            total_supply_shares: pool.total_supply_shares,
            total_debt_shares: pool.total_debt_shares,
            reserves_usdc: reserves,
        };
        (next, equity, ok)
    }

    // ---- Task 7.5: reserve withdrawal ------------------------------------------

    /// Reserve withdrawal over the decrypted payload (plaintext twin:
    /// `programs/walnut/src/math/checkpoint.rs::reserve_withdrawal`).
    ///
    /// FIRST the interest checkpoint (`checkpoint_pool`), THEN all-or-zero
    /// approval: the full `requested` amount is approved only if it is
    /// nonzero, the checkpoint succeeded, and it is no greater than Cash and
    /// Reserves after the checkpoint. On approval Cash and Reserves both
    /// decrease by exactly `requested`, so supplier equity is unchanged. On
    /// refusal the INPUT payload is returned unchanged (no checkpoint is
    /// committed on zero) and the approved amount is zero. Returns
    /// `(payload to store, approved)`. The subtrahend is zero when refused, so
    /// nothing underflows.
    pub fn reserve_withdrawal_values(
        pool: PoolSecret,
        accounted_index: u128,
        accounting_index: u128,
        reserve_factor_bps: u128,
        requested: u64,
    ) -> (PoolSecret, u64) {
        let (cp, _equity, ok) =
            checkpoint_pool(pool, accounted_index, accounting_index, reserve_factor_bps);
        let amount = requested as u128;
        let approve = ok && amount > 0 && amount <= cp.cash_usdc && amount <= cp.reserves_usdc;
        let debit = if approve { amount } else { 0 };
        let cash = if approve {
            cp.cash_usdc - debit
        } else {
            pool.cash_usdc
        };
        let reserves = if approve {
            cp.reserves_usdc - debit
        } else {
            pool.reserves_usdc
        };
        let next = PoolSecret {
            cash_usdc: cash,
            total_supply_shares: pool.total_supply_shares,
            total_debt_shares: pool.total_debt_shares,
            reserves_usdc: reserves,
        };
        (next, debit as u64)
    }

    /// Reserve withdrawal (Requirement 7.15): the Pool payload (`Enc<Mxe>`),
    /// the public accounted and accounting Borrow Indexes, the public reserve
    /// factor, and the public requested amount. Returns the payload to store,
    /// the approved amount (the request or zero), and the request echo (public
    /// anyway; the callback needs it to rebuild the Intent's input commitment
    /// when the approval is zero). The output nonce is chosen by the MXE.
    #[instruction]
    pub fn reserve_withdrawal(
        pool: Enc<Mxe, PoolSecret>,
        accounted_index: u128,
        accounting_index: u128,
        reserve_factor_bps: u128,
        requested: u64,
    ) -> (Enc<Mxe, PoolSecret>, u64, u64) {
        let p = pool.to_arcis();
        let (next, approved) = reserve_withdrawal_values(
            p,
            accounted_index,
            accounting_index,
            reserve_factor_bps,
            requested,
        );
        (
            Mxe::get().from_arcis(next),
            approved.reveal(),
            requested.reveal(),
        )
    }
    // ---- Task 8: supply and withdraw supply ------------------------------------
    /// Supply or withdraw-supply over the decrypted Pool and Position
    /// (plaintext twin: `programs/walnut/src/math/supply.rs::supply_flow`).
    ///
    /// FIRST the interest checkpoint, THEN all-or-zero:
    /// supply (`requested` = USDC units) mints `requested * 1_000` shares into
    /// a canonically empty Pool (Cash, Debt Shares, Reserves all zero and
    /// `claims_clear`), else `floor(requested * total_shares / equity)`, and
    /// requires `equity + requested <= supply_cap`. Withdraw (`requested` =
    /// shares) pays `floor(requested * equity / total_shares)` and needs the
    /// owner's shares and full Cash coverage. Everything stays inside the
    /// `u64`/`2^63` domain so no product wraps; every operand of a dead branch
    /// is clamped to zero. A refusal returns BOTH inputs unchanged.
    /// Returns `(pool, position, applied)`.
    pub fn supply_flow_values(
        pool: PoolSecret,
        position: PositionSecret,
        accounted_index: u128,
        accounting_index: u128,
        reserve_factor_bps: u128,
        is_withdraw: bool,
        requested: u128,
        supply_cap: u64,
        claims_clear: bool,
    ) -> (PoolSecret, PositionSecret, bool) {
        let max_value: u128 = 18_446_744_073_709_551_615;
        let max_shares: u128 = 9_223_372_036_854_775_808;
        let scale: u128 = 1_000;
        let (cp, equity, ck_ok) =
            checkpoint_pool(pool, accounted_index, accounting_index, reserve_factor_bps);
        let domain = ck_ok
            && equity <= max_value
            && cp.cash_usdc <= max_value
            && cp.total_supply_shares <= max_shares
            && requested > 0
            && requested <= max_value;
        let r = if domain { requested } else { 0 };
        let eq = if domain { equity } else { 0 };
        let ts = if domain { cp.total_supply_shares } else { 0 };
        let cash = if domain { cp.cash_usdc } else { 0 };
        let free = position.free_usdc as u128;
        let empty = ts == 0;
        let canonical = cp.cash_usdc == 0
            && cp.total_debt_shares == 0
            && cp.reserves_usdc == 0
            && claims_clear;
        // Supply.
        let safe_eq = if eq == 0 { 1 } else { eq };
        let minted = if empty { r * scale } else { (r * ts) / safe_eq };
        let supply_ok = domain
            && !is_withdraw
            && r <= free
            && eq + r <= supply_cap as u128
            && minted > 0
            && ts + minted <= max_shares
            && ((empty && canonical) || (!empty && eq > 0));
        // Withdraw.
        let safe_ts = if empty { 1 } else { ts };
        let claim = (r * eq) / safe_ts;
        let withdraw_ok = domain
            && is_withdraw
            && !empty
            && r <= position.supply_shares_usdc
            && r <= ts
            && claim > 0
            && claim <= cash
            && free + claim <= max_value;
        let applied = supply_ok || withdraw_ok;
        let add_cash = if supply_ok { r } else { 0 };
        let sub_cash = if withdraw_ok { claim } else { 0 };
        let add_shares = if supply_ok { minted } else { 0 };
        let sub_shares = if withdraw_ok { r } else { 0 };
        let pool_out = PoolSecret {
            cash_usdc: if applied {
                cp.cash_usdc + add_cash - sub_cash
            } else {
                pool.cash_usdc
            },
            total_supply_shares: if applied {
                cp.total_supply_shares + add_shares - sub_shares
            } else {
                pool.total_supply_shares
            },
            total_debt_shares: pool.total_debt_shares,
            reserves_usdc: if applied {
                cp.reserves_usdc
            } else {
                pool.reserves_usdc
            },
        };
        let new_free = free - add_cash + sub_cash;
        let position_out = PositionSecret {
            free_sol: position.free_sol,
            free_usdc: new_free as u64,
            collateral_sol: position.collateral_sol,
            supply_shares_usdc: position.supply_shares_usdc + add_shares - sub_shares,
            debt_shares_usdc: position.debt_shares_usdc,
        };
        (pool_out, position_out, applied)
    }
    /// Supply or withdraw supply: Pool (`Enc<Mxe>`), Position (`Enc<Shared>`),
    /// the bare `Shared` Position output owner, the public indexes and reserve
    /// factor, the public `is_withdraw` flag and request, the public supply
    /// cap, and the public recovery-claims-clear flag. Returns the Pool
    /// payload, the Position payload, the applied flag, and the request echo
    /// (the callback needs it to rebuild the Intent's input commitment when
    /// the request resolves Zero).
    #[instruction]
    pub fn supply_flow(
        pool: Enc<Mxe, PoolSecret>,
        position: Enc<Shared, PositionSecret>,
        owner: Shared,
        accounted_index: u128,
        accounting_index: u128,
        reserve_factor_bps: u128,
        is_withdraw: bool,
        requested: u128,
        supply_cap: u64,
        claims_clear: bool,
    ) -> (Enc<Mxe, PoolSecret>, Enc<Shared, PositionSecret>, bool, u128) {
        let p = pool.to_arcis();
        let s = position.to_arcis();
        let (np, ns, applied) = supply_flow_values(
            p,
            s,
            accounted_index,
            accounting_index,
            reserve_factor_bps,
            is_withdraw,
            requested,
            supply_cap,
            claims_clear,
        );
        (
            Mxe::get().from_arcis(np),
            owner.from_arcis(ns),
            applied.reveal(),
            requested.reveal(),
        )
    }
}

#[cfg(test)]
mod tests {
    //! Replays the shared golden vectors on the circuit helpers, so the
    //! Arcis-side arithmetic provably equals the program-side and the
    //! TypeScript definitions (Tasks 7.1 to 7.3).
    use super::circuits::*;

    const FIXED: &str = include_str!("../../packages/reference-model/vectors/fixed.vectors");
    const RATE: &str = include_str!("../../packages/reference-model/vectors/rate.vectors");
    const CHECKPOINT: &str =
        include_str!("../../packages/reference-model/vectors/checkpoint.vectors");

    const WAD: u128 = 1_000_000_000_000_000_000;
    const MAX_SHARES: u128 = 10_000_000_000_000_000;
    const MAX_INDEX: u128 = 1_000 * WAD;

    fn lines(text: &str) -> impl Iterator<Item = Vec<&str>> {
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| l.split_whitespace().collect())
    }

    fn n(word: &str) -> u128 {
        word.parse().unwrap()
    }

    /// The circuit helpers have no error channel: they are only defined where
    /// the vector expects a number and the product fits `u128`.
    #[test]
    fn fixed_point_matches_the_golden_vectors() {
        let mut checked = 0;
        for w in lines(FIXED) {
            let Ok(expected) = w[w.len() - 1].parse::<u128>() else {
                continue; // an error vector: the circuit callers range-check first
            };
            let got = match w[0] {
                "mul_div_floor" => mul_div_floor(n(w[1]), n(w[2]), n(w[3])),
                "mul_div_ceil" => mul_div_ceil(n(w[1]), n(w[2]), n(w[3])),
                "mul_wad_floor" => mul_wad_floor(n(w[1]), n(w[2])),
                "mul_wad_ceil" => mul_wad_ceil(n(w[1]), n(w[2])),
                "div_wad_floor" => div_wad_floor(n(w[1]), n(w[2])),
                "div_wad_ceil" => div_wad_ceil(n(w[1]), n(w[2])),
                "debt_from_shares" => {
                    let (debt, ok) = debt_from_shares_checked(n(w[1]), n(w[2]));
                    assert!(ok, "vector: {w:?}");
                    debt
                }
                // Wide and share-conversion variants are program-side only.
                _ => continue,
            };
            assert_eq!(got, expected, "vector: {w:?}");
            checked += 1;
        }
        assert!(checked >= 60, "only {checked} fixed-point vectors ran");
    }

    #[test]
    fn domain_violations_are_flagged_not_computed() {
        assert!(!debt_from_shares_checked(MAX_SHARES + 1, WAD).1);
        assert!(!debt_from_shares_checked(5, WAD - 1).1);
        assert!(!debt_from_shares_checked(5, MAX_INDEX + 1).1);
        assert_eq!(
            debt_from_shares_checked(MAX_SHARES, MAX_INDEX),
            (MAX_SHARES * 1_000, true)
        );
        assert_eq!(div_wad_ceil(5, 0), 0);
        assert_eq!(div_wad_floor(5, 0), 0);
    }

    #[test]
    fn utilization_bands_match_the_golden_vectors() {
        let mut checked = 0;
        for w in lines(RATE) {
            match w[0] {
                "band" => {
                    // `band cash debt` feeds debt directly: use index WAD so
                    // that ceil(debt * WAD / WAD) == debt.
                    if n(w[2]) <= MAX_SHARES && n(w[1]) <= (1u128 << 96) {
                        let got = utilization_band_index(n(w[1]), n(w[2]), WAD);
                        assert_eq!(got as u128, n(w[3]), "vector: {w:?}");
                        checked += 1;
                    }
                }
                "band_pool" => {
                    let got = utilization_band_index(n(w[1]), n(w[2]), n(w[3]));
                    match w[4].parse::<u128>() {
                        Ok(expected) => assert_eq!(got as u128, expected, "vector: {w:?}"),
                        // Out of domain: the circuit returns the top band.
                        Err(_) => assert_eq!(got, 19, "vector: {w:?}"),
                    }
                    checked += 1;
                }
                _ => {}
            }
        }
        assert!(checked >= 12, "only {checked} band vectors ran");
    }

    #[test]
    fn checkpoint_matches_the_golden_vectors() {
        let mut checked = 0;
        for line in CHECKPOINT.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (input, expected) = line.split_once("=>").unwrap();
            let w: Vec<&str> = input.split_whitespace().collect();
            let (reserves, equity, _debt, ok) =
                checkpoint_pool_values(n(w[1]), n(w[3]), n(w[4]), n(w[5]), n(w[6]), n(w[7]));
            let expected = expected.trim();
            if expected == "err" {
                assert!(!ok, "vector: {line}");
                assert_eq!(
                    (reserves, equity),
                    (n(w[4]), 0),
                    "refusal leaves state: {line}"
                );
            } else {
                let e: Vec<u128> = expected.split_whitespace().map(n).collect();
                assert!(ok, "vector: {line}");
                assert_eq!([reserves, equity], [e[0], e[1]], "vector: {line}");
            }
            checked += 1;
        }
        assert!(checked >= 30, "only {checked} checkpoint vectors ran");
    }
}
