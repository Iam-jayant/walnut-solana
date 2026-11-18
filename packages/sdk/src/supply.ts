// Client-side supply math (Task 8). The Pool is `Enc<Mxe>`, so an owner can
// decrypt only their own Position (supply shares); the claim of those shares
// needs the Pool's supplier equity and total supply shares, which a client
// knows only when it can derive them (for example a sole supplier on a Pool
// without borrowers). These are the exact definitions the `supply_flow`
// circuit uses (programs/walnut/src/math/supply.rs), rounded down.

/** Shares minted per USDC base unit by the first supplier of an empty Pool. */
export const INITIAL_SHARE_SCALE = 1_000n;

/** `floor(shares x supplierEquity / totalSupplyShares)`: what a withdrawal pays. */
export function claimForShares(
  shares: bigint,
  supplierEquity: bigint,
  totalSupplyShares: bigint,
): bigint {
  if (totalSupplyShares <= 0n) throw new Error("the Pool has no supply shares");
  if (shares < 0n || supplierEquity < 0n) throw new Error("negative input");
  return (shares * supplierEquity) / totalSupplyShares;
}

/**
 * Shares a supply of `amount` mints: `amount x INITIAL_SHARE_SCALE` into a
 * Pool with no shares, otherwise `floor(amount x totalSupplyShares /
 * supplierEquity)`. Zero means the circuit would move nothing.
 */
export function sharesForSupply(
  amount: bigint,
  totalSupplyShares: bigint,
  supplierEquity: bigint,
): bigint {
  if (amount < 0n) throw new Error("negative amount");
  if (totalSupplyShares === 0n) return amount * INITIAL_SHARE_SCALE;
  if (supplierEquity <= 0n) throw new Error("the Pool has no supplier equity");
  return (amount * totalSupplyShares) / supplierEquity;
}
