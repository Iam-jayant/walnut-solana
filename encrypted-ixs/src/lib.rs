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
}
