// Two-owner circuit outputs return a tuple of `Enc<Shared, _>` values; this is
// the documented Arcium sealing shape, so allow the type-complexity lint crate-wide
// rather than hiding it behind type aliases that the Arcis macro may not parse.
#![allow(clippy::type_complexity)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::manual_range_contains)]
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

    // ---- Demo: position_op ---------------------------------------------------

    /// Demo position operation on the stored Position state: 0 add collateral,
    /// 1 withdraw collateral, 2 borrow, 3 repay (see the walnut-diffn design).
    /// `debt_shares_usdc` holds plain debt in USDC base units. Public `op`,
    /// `amount`, `price` and `ltv_bps` are multiplied and compared only; there
    /// is no division. A refusal returns the state unchanged, still resealed
    /// to `owner`. Returns `(state, applied, op, amount)`; the echoes let the
    /// callback rebuild the input commitment.
    #[instruction]
    pub fn position_op(
        state: Enc<Shared, PositionSecret>,
        owner: Shared,
        op: u8,
        amount: u64,
        price: u64,
        ltv_bps: u64,
    ) -> (Enc<Shared, PositionSecret>, bool, u8, u64) {
        let mut s = state.to_arcis();
        let a = amount as u128;
        let free_sol = s.free_sol as u128;
        let free_usdc = s.free_usdc as u128;
        let coll = s.collateral_sol as u128;
        let debt = s.debt_shares_usdc;
        let limit = (price as u128) * (ltv_bps as u128);
        let scale = 10_000_000_000_000u128;

        let ok0 = a <= free_sol;
        let fits1 = a <= coll;
        let a1 = if fits1 { a } else { 0 };
        let rest = coll - a1;
        let ok1 = fits1 && debt * scale <= rest * limit;
        let ok2 =
            (debt + a) * scale <= coll * limit && free_usdc + a <= 18_446_744_073_709_551_615u128;
        let ok3 = a <= free_usdc && a <= debt;

        let applied = (op == 0 && ok0) || (op == 1 && ok1) || (op == 2 && ok2) || (op == 3 && ok3);
        let d = if applied { a } else { 0 };
        let sol_to_coll = op == 0;
        let coll_to_sol = op == 1;
        let borrow = op == 2;
        let repay = op == 3;

        let new_free_sol = if sol_to_coll {
            free_sol - d
        } else if coll_to_sol {
            free_sol + d
        } else {
            free_sol
        };
        let new_coll = if sol_to_coll {
            coll + d
        } else if coll_to_sol {
            coll - d
        } else {
            coll
        };
        let new_free_usdc = if borrow {
            free_usdc + d
        } else if repay {
            free_usdc - d
        } else {
            free_usdc
        };
        let new_debt = if borrow {
            debt + d
        } else if repay {
            debt - d
        } else {
            debt
        };
        s.free_sol = new_free_sol as u64;
        s.collateral_sol = new_coll as u64;
        s.free_usdc = new_free_usdc as u64;
        s.debt_shares_usdc = new_debt;
        (
            owner.from_arcis(s),
            applied.reveal(),
            op.reveal(),
            amount.reveal(),
        )
    }

    // ---- walnut-lend: lend_op ------------------------------------------------

    /// Lending position operation (walnut-lend design): 0 add collateral,
    /// 1 withdraw collateral, 2 borrow, 3 repay, 4 supply, 5 withdraw supply,
    /// 6 health check, 7 liquidate. `shares` is the public share amount the
    /// Program computed; for op 7 `amount` is the seize lamports and `shares`
    /// the debt burn. Only multiplication and comparison touch secret values;
    /// dead-branch operands are clamped to zero so nothing underflows.
    /// Returns `(state, applied, op, amount, shares)`; the echoes let the
    /// callback rebuild the input commitment. Op 6 never changes the state and
    /// reveals `applied = unhealthy`.
    #[instruction]
    pub fn lend_op(
        state: Enc<Shared, PositionSecret>,
        owner: Shared,
        op: u64,
        amount: u64,
        shares: u64,
        borrow_index_1e9: u64,
        price: u64,
        ltv_bps: u64,
        liq_threshold_bps: u64,
        position_debt_cap: u64,
    ) -> (Enc<Shared, PositionSecret>, bool, u64, u64, u64) {
        let mut s = state.to_arcis();
        let a = amount as u128;
        let sh = shares as u128;
        let idx = borrow_index_1e9 as u128;
        let px = price as u128;
        let max64 = 18_446_744_073_709_551_615u128;
        let free_sol = s.free_sol as u128;
        let free_usdc = s.free_usdc as u128;
        let coll = s.collateral_sol as u128;
        let sup = s.supply_shares_usdc;
        let debt = s.debt_shares_usdc;

        let debt_value = debt * idx * 10_000u128;
        let unhealthy = debt_value > coll * px * (liq_threshold_bps as u128);

        let ok0 = a <= free_sol;
        let fits1 = a <= coll;
        let a1 = if fits1 { a } else { 0 };
        let ok1 = fits1 && debt_value <= (coll - a1) * px * (ltv_bps as u128);
        let new_debt2 = debt + sh;
        let ok2 = new_debt2 * idx * 10_000u128 <= coll * px * (ltv_bps as u128)
            && new_debt2 * idx <= (position_debt_cap as u128) * 1_000_000_000u128
            && free_usdc + a <= max64;
        let ok3 = sh <= debt && a <= free_usdc;
        let ok4 = a <= free_usdc && sup + sh <= max64;
        let ok5 = sh <= sup && free_usdc + a <= max64;
        let ok7 = unhealthy && sh <= debt && (sh * 2 <= debt || sh == debt) && a <= coll;

        let applied = (op == 0 && ok0)
            || (op == 1 && ok1)
            || (op == 2 && ok2)
            || (op == 3 && ok3)
            || (op == 4 && ok4)
            || (op == 5 && ok5)
            || (op == 6 && unhealthy)
            || (op == 7 && ok7);
        let mutates = applied && op != 6;
        let da = if mutates { a } else { 0 };
        let ds = if mutates { sh } else { 0 };

        let new_free_sol = if op == 0 {
            free_sol - da
        } else if op == 1 {
            free_sol + da
        } else {
            free_sol
        };
        let new_coll = if op == 0 {
            coll + da
        } else if op == 1 || op == 7 {
            coll - da
        } else {
            coll
        };
        let new_free_usdc = if op == 2 || op == 5 {
            free_usdc + da
        } else if op == 3 || op == 4 {
            free_usdc - da
        } else {
            free_usdc
        };
        let new_debt = if op == 2 {
            debt + ds
        } else if op == 3 || op == 7 {
            debt - ds
        } else {
            debt
        };
        let new_sup = if op == 4 {
            sup + ds
        } else if op == 5 {
            sup - ds
        } else {
            sup
        };
        s.free_sol = new_free_sol as u64;
        s.collateral_sol = new_coll as u64;
        s.free_usdc = new_free_usdc as u64;
        s.debt_shares_usdc = new_debt;
        s.supply_shares_usdc = new_sup;
        (
            owner.from_arcis(s),
            applied.reveal(),
            op.reveal(),
            amount.reveal(),
            shares.reveal(),
        )
    }

    // ---- walnut-private: encrypted-amount pool operations ---------------------

    /// Encrypted operation input. The owner's client computes `shares` from the
    /// public index and encrypts both values; the circuit only VERIFIES the
    /// conversion with multiplications and comparisons (no division by a
    /// secret, which is what made `supply_flow` cost 20.7e9).
    #[derive(Copy, Clone)]
    pub struct OpInput {
        amount: u64,
        shares: u64,
    }

    /// Encrypted liquidation input from the liquidator: repay amount, the debt
    /// shares it burns and the collateral lamports it seizes.
    #[derive(Copy, Clone)]
    pub struct LiqInput {
        amount: u64,
        burn_shares: u64,
        seize: u64,
    }

    /// Pool operation with an ENCRYPTED amount. `op`: 0 supply, 1 withdraw
    /// supply, 2 borrow, 3 repay. The Pool (`Enc<Mxe>`) and the owner's
    /// Position change together or not at all. Share conversions use the public
    /// indexes and are verified, never computed: supply `shares =
    /// floor(a*1e9/sidx)`, withdraw and borrow `shares = ceil(a*1e9/idx)`,
    /// repay `shares = floor(a*1e9/bidx)`. Only `applied` is revealed.
    #[instruction]
    pub fn pool_op(
        pool: Enc<Mxe, PoolSecret>,
        state: Enc<Shared, PositionSecret>,
        input: Enc<Shared, OpInput>,
        owner: Shared,
        op: u64,
        supply_index_1e9: u64,
        borrow_index_1e9: u64,
        price: u64,
        ltv_bps: u64,
        position_debt_cap: u64,
        supply_cap: u64,
    ) -> (Enc<Mxe, PoolSecret>, Enc<Shared, PositionSecret>, bool) {
        let mut p = pool.to_arcis();
        let mut s = state.to_arcis();
        let inp = input.to_arcis();
        let a = inp.amount as u128;
        let sh = inp.shares as u128;
        let sidx = supply_index_1e9 as u128;
        let bidx = borrow_index_1e9 as u128;
        let px = price as u128;
        let max64 = 18_446_744_073_709_551_615u128;
        let scale = 1_000_000_000u128;
        let a_scaled = a * scale;
        let shm1 = if sh > 0 { sh - 1 } else { 0 };

        let free_usdc = s.free_usdc as u128;
        let coll = s.collateral_sol as u128;
        let sup = s.supply_shares_usdc;
        let debt = s.debt_shares_usdc;
        let cash = p.cash_usdc;
        let tss = p.total_supply_shares;
        let tds = p.total_debt_shares;

        let supply_conv = sh > 0 && sh * sidx <= a_scaled && a_scaled < (sh + 1) * sidx;
        let withdraw_conv = sh > 0 && a > 0 && sh * sidx >= a_scaled && shm1 * sidx < a_scaled;
        let borrow_conv = sh > 0 && a > 0 && sh * bidx >= a_scaled && shm1 * bidx < a_scaled;
        let repay_conv = sh > 0 && sh * bidx <= a_scaled && a_scaled < (sh + 1) * bidx;

        let new_debt = debt + sh;
        let healthy = new_debt * bidx * 10_000u128 <= coll * px * (ltv_bps as u128)
            && new_debt * bidx <= (position_debt_cap as u128) * scale;

        let ok0 = supply_conv
            && a <= free_usdc
            && sup + sh <= max64
            && (tss + sh) * sidx <= (supply_cap as u128) * scale;
        let ok1 = withdraw_conv && sh <= sup && sh <= tss && a <= cash && free_usdc + a <= max64;
        let ok2 = borrow_conv && healthy && a <= cash && free_usdc + a <= max64;
        let ok3 = repay_conv && sh <= debt && sh <= tds && a <= free_usdc;

        let applied = (op == 0 && ok0) || (op == 1 && ok1) || (op == 2 && ok2) || (op == 3 && ok3);
        let da = if applied { a } else { 0 };
        let ds = if applied { sh } else { 0 };

        let new_free_usdc = if op == 0 || op == 3 {
            free_usdc - da
        } else {
            free_usdc + da
        };
        let new_sup = if op == 0 {
            sup + ds
        } else if op == 1 {
            sup - ds
        } else {
            sup
        };
        let new_debt_s = if op == 2 {
            debt + ds
        } else if op == 3 {
            debt - ds
        } else {
            debt
        };
        let new_cash = if op == 0 || op == 3 {
            cash + da
        } else {
            cash - da
        };
        let new_tss = if op == 0 {
            tss + ds
        } else if op == 1 {
            tss - ds
        } else {
            tss
        };
        let new_tds = if op == 2 {
            tds + ds
        } else if op == 3 {
            tds - ds
        } else {
            tds
        };
        s.free_usdc = new_free_usdc as u64;
        s.supply_shares_usdc = new_sup;
        s.debt_shares_usdc = new_debt_s;
        p.cash_usdc = new_cash;
        p.total_supply_shares = new_tss;
        p.total_debt_shares = new_tds;
        (
            Mxe::get().from_arcis(p),
            owner.from_arcis(s),
            applied.reveal(),
        )
    }

    /// Private liquidation. The liquidator pays `amount` USDC from its own
    /// encrypted free balance and receives `seize` lamports of the target's
    /// collateral into its own encrypted free SOL; the Pool books the repaid
    /// cash and burns the debt shares. The liquidator supplies the conversions
    /// (`burn_shares = floor(a*1e9/bidx)`, `seize =
    /// floor(a*1e9*(10_000+bonus)/(price*10_000))`) and the circuit verifies
    /// them. Only `applied` is revealed.
    #[instruction]
    pub fn liquidate_pool(
        pool: Enc<Mxe, PoolSecret>,
        target: Enc<Shared, PositionSecret>,
        liquidator: Enc<Shared, PositionSecret>,
        input: Enc<Shared, LiqInput>,
        target_owner: Shared,
        liquidator_owner: Shared,
        borrow_index_1e9: u64,
        price: u64,
        liq_threshold_bps: u64,
        bonus_bps: u64,
    ) -> (
        Enc<Mxe, PoolSecret>,
        Enc<Shared, PositionSecret>,
        Enc<Shared, PositionSecret>,
        bool,
    ) {
        let mut p = pool.to_arcis();
        let mut t = target.to_arcis();
        let mut l = liquidator.to_arcis();
        let inp = input.to_arcis();
        let a = inp.amount as u128;
        let sh = inp.burn_shares as u128;
        let z = inp.seize as u128;
        let bidx = borrow_index_1e9 as u128;
        let px = price as u128;
        let scale = 1_000_000_000u128;
        let max64 = 18_446_744_073_709_551_615u128;
        let a_scaled = a * scale;

        let coll = t.collateral_sol as u128;
        let debt = t.debt_shares_usdc;
        let unhealthy = debt * bidx * 10_000u128 > coll * px * (liq_threshold_bps as u128);
        let burn_conv = sh > 0 && sh * bidx <= a_scaled && a_scaled < (sh + 1) * bidx;
        let seize_num = a_scaled * (10_000u128 + bonus_bps as u128);
        let seize_conv =
            z > 0 && z * px * 10_000u128 <= seize_num && seize_num < (z + 1) * px * 10_000u128;
        let applied = unhealthy
            && burn_conv
            && seize_conv
            && sh <= debt
            && (sh * 2 <= debt || sh == debt)
            && sh <= p.total_debt_shares
            && z <= coll
            && a <= l.free_usdc as u128
            && (l.free_sol as u128) + z <= max64;
        let da = if applied { a } else { 0 };
        let ds = if applied { sh } else { 0 };
        let dz = if applied { z } else { 0 };

        t.collateral_sol = (coll - dz) as u64;
        t.debt_shares_usdc = debt - ds;
        l.free_usdc = ((l.free_usdc as u128) - da) as u64;
        l.free_sol = ((l.free_sol as u128) + dz) as u64;
        p.cash_usdc = p.cash_usdc + da;
        p.total_debt_shares = p.total_debt_shares - ds;
        (
            Mxe::get().from_arcis(p),
            target_owner.from_arcis(t),
            liquidator_owner.from_arcis(l),
            applied.reveal(),
        )
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

    // ---- walnut-lend task 1.2: selective disclosure --------------------------
    /// Re-seals a field-masked copy of the Position to an auditor key. Mask
    /// bits 0 to 4 select free_sol, free_usdc, collateral, supply_shares,
    /// debt_shares; unselected fields are zero. The stored Position is not
    /// changed. The mask is public. Selection only, no division, no loops.
    #[instruction]
    pub fn disclose_position(
        state: Enc<Shared, PositionSecret>,
        auditor: Shared,
        mask: u64,
    ) -> Enc<Shared, PositionSecret> {
        let s = state.to_arcis();
        let m = PositionSecret {
            free_sol: if mask & 1 != 0 { s.free_sol } else { 0 },
            free_usdc: if mask & 2 != 0 { s.free_usdc } else { 0 },
            collateral_sol: if mask & 4 != 0 { s.collateral_sol } else { 0 },
            supply_shares_usdc: if mask & 8 != 0 {
                s.supply_shares_usdc
            } else {
                0
            },
            debt_shares_usdc: if mask & 16 != 0 {
                s.debt_shares_usdc
            } else {
                0
            },
        };
        auditor.from_arcis(m)
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
        let canonical =
            cp.cash_usdc == 0 && cp.total_debt_shares == 0 && cp.reserves_usdc == 0 && claims_clear;
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
    ) -> (
        Enc<Mxe, PoolSecret>,
        Enc<Shared, PositionSecret>,
        bool,
        u128,
    ) {
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

    /// Scalar wrapper of [`supply_flow_values`] for the host replay of the
    /// shared golden vectors (the secret structs have private fields).
    /// Arguments: Pool `(cash, supply shares, debt shares, reserves)`,
    /// Position `(free USDC, supply shares)`, then the `supply_flow_values`
    /// inputs. Returns `(cash, supply shares, reserves, free USDC, position
    /// shares, applied)`.
    pub fn supply_flow_scalars(
        pool: (u128, u128, u128, u128),
        position: (u64, u128),
        accounted_index: u128,
        accounting_index: u128,
        reserve_factor_bps: u128,
        is_withdraw: bool,
        requested: u128,
        supply_cap: u64,
        claims_clear: bool,
    ) -> (u128, u128, u128, u64, u128, bool) {
        let p = PoolSecret {
            cash_usdc: pool.0,
            total_supply_shares: pool.1,
            total_debt_shares: pool.2,
            reserves_usdc: pool.3,
        };
        let s = PositionSecret {
            free_sol: 0,
            free_usdc: position.0,
            collateral_sol: 0,
            supply_shares_usdc: position.1,
            debt_shares_usdc: 0,
        };
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
            np.cash_usdc,
            np.total_supply_shares,
            np.reserves_usdc,
            ns.free_usdc,
            ns.supply_shares_usdc,
            applied,
        )
    }

    /// Scalar wrapper of [`recover_empty_pool_values`] for the host replay of
    /// the golden vectors. Returns `(reserves, applied)`.
    pub fn recover_empty_pool_scalars(
        pool: (u128, u128, u128, u128),
        accounted_index: u128,
        accounting_index: u128,
        reserve_factor_bps: u128,
        dust_limit: u128,
        claims_clear: bool,
    ) -> (u128, bool) {
        let p = PoolSecret {
            cash_usdc: pool.0,
            total_supply_shares: pool.1,
            total_debt_shares: pool.2,
            reserves_usdc: pool.3,
        };
        let (np, applied) = recover_empty_pool_values(
            p,
            accounted_index,
            accounting_index,
            reserve_factor_bps,
            dust_limit,
            claims_clear,
        );
        (np.reserves_usdc, applied)
    }

    // ---- Task 8.1: recovery of an empty Pool's dust residue ----------------------

    /// Recovery of an empty Pool's dust residue (plaintext twin:
    /// `programs/walnut/src/math/supply.rs::recover_empty_pool`).
    ///
    /// FIRST the interest checkpoint, THEN all-or-zero: applies only when no
    /// Supply Shares and no Debt Shares exist after the checkpoint, the
    /// public `claims_clear` flag holds, and the residue `equity = cash -
    /// reserves` is nonzero and at most the public `dust_limit`. The one
    /// change is `reserves := cash`. A refusal returns the INPUT payload.
    /// Returns `(pool, applied)`.
    pub fn recover_empty_pool_values(
        pool: PoolSecret,
        accounted_index: u128,
        accounting_index: u128,
        reserve_factor_bps: u128,
        dust_limit: u128,
        claims_clear: bool,
    ) -> (PoolSecret, bool) {
        let (cp, equity, ck_ok) =
            checkpoint_pool(pool, accounted_index, accounting_index, reserve_factor_bps);
        let applied = ck_ok
            && claims_clear
            && cp.total_supply_shares == 0
            && cp.total_debt_shares == 0
            && equity > 0
            && equity <= dust_limit;
        let reserves = if applied {
            cp.cash_usdc
        } else {
            pool.reserves_usdc
        };
        let out = PoolSecret {
            cash_usdc: pool.cash_usdc,
            total_supply_shares: pool.total_supply_shares,
            total_debt_shares: pool.total_debt_shares,
            reserves_usdc: reserves,
        };
        (out, applied)
    }

    /// `recover_empty_pool`: the Pool payload (`Enc<Mxe>`), the public
    /// indexes and reserve factor, the public dust limit, and the public
    /// recovery-claims-clear flag. Returns the Pool payload to store and the
    /// applied flag.
    #[instruction]
    pub fn recover_empty_pool(
        pool: Enc<Mxe, PoolSecret>,
        accounted_index: u128,
        accounting_index: u128,
        reserve_factor_bps: u128,
        dust_limit: u128,
        claims_clear: bool,
    ) -> (Enc<Mxe, PoolSecret>, bool) {
        let p = pool.to_arcis();
        let (next, applied) = recover_empty_pool_values(
            p,
            accounted_index,
            accounting_index,
            reserve_factor_bps,
            dust_limit,
            claims_clear,
        );
        (Mxe::get().from_arcis(next), applied.reveal())
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
    const SUPPLY: &str = include_str!("../../packages/reference-model/vectors/supply.vectors");

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

    /// Replays the supply, withdraw-supply, and recovery vectors on the
    /// circuit bodies (Task 8): the Arcis arithmetic equals the program-side
    /// and the TypeScript definitions on every vector.
    #[test]
    fn supply_flow_and_recovery_match_the_golden_vectors() {
        let (mut supplies, mut recoveries) = (0, 0);
        for line in SUPPLY.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (input, expected) = line.split_once("=>").unwrap();
            let w: Vec<&str> = input.split_whitespace().collect();
            let e: Vec<&str> = expected.split_whitespace().collect();
            match w[0] {
                "supply" => {
                    let pool = (n(w[1]), n(w[2]), n(w[3]), n(w[4]));
                    let position = (n(w[5]) as u64, n(w[6]));
                    let (cash, supply, reserves, free, shares, applied) = supply_flow_scalars(
                        pool,
                        position,
                        n(w[7]),
                        n(w[8]),
                        n(w[9]),
                        w[10] == "1",
                        n(w[11]),
                        n(w[12]) as u64,
                        w[13] == "1",
                    );
                    if e[0] == "zero" {
                        assert!(!applied, "vector: {line}");
                        assert_eq!(
                            (cash, supply, reserves, free as u128, shares),
                            (pool.0, pool.1, pool.3, position.0 as u128, position.1),
                            "a refusal returns the inputs: {line}"
                        );
                    } else {
                        assert!(applied, "vector: {line}");
                        let want: Vec<u128> = e.iter().map(|x| n(x)).collect();
                        assert_eq!(
                            vec![cash, supply, reserves, free as u128, shares],
                            want,
                            "vector: {line}"
                        );
                    }
                    supplies += 1;
                }
                "recover" => {
                    let pool = (n(w[1]), n(w[2]), n(w[3]), n(w[4]));
                    let (reserves, applied) = recover_empty_pool_scalars(
                        pool,
                        n(w[5]),
                        n(w[6]),
                        n(w[7]),
                        n(w[8]),
                        w[9] == "1",
                    );
                    if e[0] == "zero" {
                        assert!(!applied, "vector: {line}");
                        assert_eq!(reserves, pool.3, "vector: {line}");
                    } else {
                        assert!(applied, "vector: {line}");
                        assert_eq!(reserves, n(e[0]), "vector: {line}");
                    }
                    recoveries += 1;
                }
                other => panic!("unknown vector {other}"),
            }
        }
        assert!(supplies >= 30 && recoveries >= 9);
    }
}
