// Fixed-point primitives (Task 7.1): the bigint mirror of
// programs/walnut/src/math/fixed.rs and the Arcis helpers in
// encrypted-ixs/src/lib.rs. All three replay vectors/fixed.vectors.
//
// Every function throws FixedPointError instead of saturating or wrapping.

export const WAD = 10n ** 18n;
const U64_MAX = (1n << 64n) - 1n;
export const U128_MAX = (1n << 128n) - 1n;
/** Largest accepted Borrow Index: 1,000 x WAD. */
export const MAX_BORROW_INDEX = 1_000n * WAD;
/** Largest accepted share count. */
export const MAX_SHARES = 10n ** 16n;
export const BPS_DENOMINATOR = 10_000n;

export type FixedPointErrorCode =
  | "overflow"
  | "div0"
  | "range"
  | "underflow"
  | "invalid"
  | "regress"
  | "elapsed";

export class FixedPointError extends Error {
  readonly code: FixedPointErrorCode;
  constructor(code: FixedPointErrorCode) {
    super(`fixed-point error: ${code}`);
    this.name = "FixedPointError";
    this.code = code;
  }
}

function fit(value: bigint): bigint {
  if (value < 0n || value > U128_MAX) throw new FixedPointError("overflow");
  return value;
}

/** floor(a x b / d); the product must fit u128 (the program's domain). */
export function mulDivFloor(a: bigint, b: bigint, d: bigint): bigint {
  if (d === 0n) throw new FixedPointError("div0");
  return fit(fit(a * b) / d);
}

/** ceil(a x b / d); the product must fit u128. */
export function mulDivCeil(a: bigint, b: bigint, d: bigint): bigint {
  if (d === 0n) throw new FixedPointError("div0");
  const p = fit(a * b);
  return fit((p + d - 1n) / d);
}

export const mulWadFloor = (a: bigint, b: bigint): bigint =>
  mulDivFloor(a, b, WAD);
export const mulWadCeil = (a: bigint, b: bigint): bigint =>
  mulDivCeil(a, b, WAD);
export const divWadFloor = (a: bigint, b: bigint): bigint =>
  mulDivFloor(a, WAD, b);
export const divWadCeil = (a: bigint, b: bigint): bigint =>
  mulDivCeil(a, WAD, b);

/** floor(a x b / WAD) where the product may exceed u128; only the result must fit. */
export function mulWadFloorWide(a: bigint, b: bigint): bigint {
  return fit((a * b) / WAD);
}

/** ceil(a x b / WAD) where the product may exceed u128. */
export function mulWadCeilWide(a: bigint, b: bigint): bigint {
  return fit((a * b + WAD - 1n) / WAD);
}

export function checkIndex(index: bigint): bigint {
  if (index < WAD || index > MAX_BORROW_INDEX)
    throw new FixedPointError("range");
  return index;
}

export function checkShares(shares: bigint): bigint {
  if (shares < 0n || shares > MAX_SHARES) throw new FixedPointError("range");
  return shares;
}

export function toU64(value: bigint): bigint {
  if (value < 0n || value > U64_MAX) throw new FixedPointError("overflow");
  return value;
}

/** Debt units owed by `shares` at `index`: ceil(shares x index / WAD). */
export function debtFromShares(shares: bigint, index: bigint): bigint {
  checkShares(shares);
  checkIndex(index);
  return mulWadCeil(shares, index);
}

/** Debt shares issued for `amount` borrowed at `index`: ceil(amount x WAD / index). */
export function sharesFromAmountCeil(amount: bigint, index: bigint): bigint {
  checkIndex(index);
  return divWadCeil(amount, index);
}

/** Debt shares for `amount` at `index`, rounded down. */
export function sharesFromAmountFloor(amount: bigint, index: bigint): bigint {
  checkIndex(index);
  return divWadFloor(amount, index);
}
