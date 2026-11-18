// Demo `position_op` (walnut-diffn): bigint mirror of the circuit rules in
// programs/walnut/src/state/position_op.rs. `debtSharesUsdc` holds plain debt.
import type { PositionBalances } from "./viewing-key.ts";

/** Operation tags of the `position_op` circuit. */
export const PositionOp = {
  AddCollateral: 0,
  WithdrawCollateral: 1,
  Borrow: 2,
  Repay: 3,
} as const;
export type PositionOp = (typeof PositionOp)[keyof typeof PositionOp];

const SCALE = 1_000_000_000n * 10_000n;
const U64_MAX = (1n << 64n) - 1n;

/** Applies one op; a refusal returns the input state unchanged. */
export function applyPositionOp(
  state: PositionBalances,
  op: PositionOp | number,
  amount: bigint,
  price: bigint,
  ltvBps: bigint,
): { state: PositionBalances; applied: boolean } {
  const limit = price * ltvBps;
  const debt = state.debtSharesUsdc;
  let applied: boolean;
  switch (op) {
    case PositionOp.AddCollateral:
      applied = amount <= state.freeSol;
      break;
    case PositionOp.WithdrawCollateral:
      applied =
        amount <= state.collateralSol &&
        debt * SCALE <= (state.collateralSol - amount) * limit;
      break;
    case PositionOp.Borrow:
      applied =
        (debt + amount) * SCALE <= state.collateralSol * limit &&
        state.freeUsdc + amount <= U64_MAX;
      break;
    case PositionOp.Repay:
      applied = amount <= state.freeUsdc && amount <= debt;
      break;
    default:
      applied = false;
  }
  if (!applied) return { state, applied: false };
  const s = { ...state };
  switch (op) {
    case PositionOp.AddCollateral:
      s.freeSol -= amount;
      s.collateralSol += amount;
      break;
    case PositionOp.WithdrawCollateral:
      s.collateralSol -= amount;
      s.freeSol += amount;
      break;
    case PositionOp.Borrow:
      s.debtSharesUsdc += amount;
      s.freeUsdc += amount;
      break;
    default:
      s.freeUsdc -= amount;
      s.debtSharesUsdc -= amount;
  }
  return { state: s, applied: true };
}
