use arcis::*;

/// Arcis circuits for Walnut-Solana.
///
/// Scaffold only: `add_together` is the standard Arcium hello-world circuit
/// used to verify the toolchain. Walnut circuits are added after the
/// Arcium feasibility gate (Task 2).
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
}
