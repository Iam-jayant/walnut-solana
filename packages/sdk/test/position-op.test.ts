// Mirrors the Rust tests in programs/walnut/src/state/position_op.rs.
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  applyPositionOp,
  PositionOp,
  type PositionBalances,
} from "../src/index.ts";

const PRICE = 150_000_000n;
const LTV = 5_000n;

function st(
  freeSol: bigint,
  freeUsdc: bigint,
  collateralSol: bigint,
  debt: bigint,
): PositionBalances {
  return {
    freeSol,
    freeUsdc,
    collateralSol,
    supplySharesUsdc: 0n,
    debtSharesUsdc: debt,
  };
}
const run = (s: PositionBalances, op: number, a: bigint) =>
  applyPositionOp(s, op, a, PRICE, LTV);

test("add collateral applies and refuses", () => {
  const before = st(5n, 0n, 1n, 0n);
  const ok = run(before, PositionOp.AddCollateral, 5n);
  assert.equal(ok.applied, true);
  assert.deepEqual(ok.state, st(0n, 0n, 6n, 0n));
  const no = run(before, PositionOp.AddCollateral, 6n);
  assert.equal(no.applied, false);
  assert.deepEqual(no.state, before);
});

test("borrow hits the exact LTV boundary", () => {
  const s = st(0n, 0n, 2_000_000_000n, 0n);
  const ok = run(s, PositionOp.Borrow, 150_000_000n);
  assert.equal(ok.applied, true);
  assert.deepEqual(
    ok.state,
    st(0n, 150_000_000n, 2_000_000_000n, 150_000_000n),
  );
  const no = run(s, PositionOp.Borrow, 150_000_001n);
  assert.equal(no.applied, false);
  assert.deepEqual(no.state, s);
});

test("withdraw collateral respects LTV", () => {
  const s = st(0n, 75_000_000n, 2_000_000_000n, 75_000_000n);
  const ok = run(s, PositionOp.WithdrawCollateral, 1_000_000_000n);
  assert.equal(ok.applied, true);
  assert.equal(ok.state.collateralSol, 1_000_000_000n);
  assert.equal(ok.state.freeSol, 1_000_000_000n);
  assert.equal(
    run(s, PositionOp.WithdrawCollateral, 1_000_000_001n).applied,
    false,
  );
  assert.equal(
    run(s, PositionOp.WithdrawCollateral, 2_000_000_001n).applied,
    false,
  );
});

test("repay applies and refuses", () => {
  const s = st(0n, 40n, 1n, 100n);
  const ok = run(s, PositionOp.Repay, 40n);
  assert.equal(ok.applied, true);
  assert.deepEqual(ok.state, st(0n, 0n, 1n, 60n));
  assert.equal(run(s, PositionOp.Repay, 41n).applied, false);
  assert.equal(
    run(st(0n, 200n, 1n, 100n), PositionOp.Repay, 101n).applied,
    false,
  );
});

test("borrow refuses free_usdc overflow and unknown op", () => {
  const max = (1n << 64n) - 1n;
  const s = st(0n, max, max, 0n);
  assert.equal(run(s, PositionOp.Borrow, 1n).applied, false);
  assert.equal(run(s, 4, 1n).applied, false);
});
