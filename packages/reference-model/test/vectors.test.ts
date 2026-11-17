// Replays the shared golden vectors (vectors/balance.vectors) on the
// TypeScript reference model. The Rust checkpoint
// (programs/walnut/src/state/balance_checkpoint.rs) replays the same file on
// the real production rules, so both models and the program agree.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  BalanceModel,
  PAUSE_SHIELD,
  PAUSE_UNSHIELD,
  type Asset,
  type Outcome,
  type ShieldSelector,
  type ShieldStatus,
} from "../src/index.ts";

const VECTORS = readFileSync(
  new URL("../vectors/balance.vectors", import.meta.url),
  "utf8",
);

function asset(word: string | undefined): Asset {
  if (word === "sol" || word === "usdc") return word;
  throw new Error(`unknown asset ${String(word)}`);
}

function selector(word: string | undefined): ShieldSelector {
  if (word === "first" || word === "last") return word;
  throw new Error(`unknown selector ${String(word)}`);
}

function runStep(model: BalanceModel, line: string): Outcome {
  const [op, a, b] = line.split(/\s+/);
  switch (op) {
    case "init":
      return model.initPosition();
    case "retry_init":
      return model.retryInit();
    case "shield":
      return model.shield(asset(a), BigInt(b as string));
    case "unshield":
      return model.unshield(asset(a), BigInt(b as string));
    case "rotate":
      return model.rotate(Number(a));
    case "credit":
      return model.callback("cur", "honest");
    case "expire":
      model.advance(2_000n);
      return model.callback("cur", "honest");
    case "stale":
      model.externalBump();
      return model.callback("cur", "honest");
    case "unauth":
      return model.callback("cur", "unauth");
    case "old":
      return model.callback("first", "honest");
    case "abandon":
      model.advance(2_000n);
      return model.abandon();
    case "retry_shield":
      return model.retryShield(selector(a));
    case "refund":
      return model.refund(selector(a));
    case "pause_shield":
      return model.setPause(PAUSE_SHIELD, true);
    case "unpause_shield":
      return model.setPause(PAUSE_SHIELD, false);
    case "pause_unshield":
      return model.setPause(PAUSE_UNSHIELD, true);
    case "unpause_unshield":
      return model.setPause(PAUSE_UNSHIELD, false);
    case "advance":
      return model.advance(BigInt(a as string));
    default:
      throw new Error(`unknown vector step ${line}`);
  }
}

function expectLine(model: BalanceModel, name: string, line: string): void {
  const [, what, x, y] = line.split(/\s+/);
  const snap = model.snapshot();
  const where = `${name}: ${line}`;
  switch (what) {
    case "free":
      assert.deepEqual(
        snap.free,
        [BigInt(x as string), BigInt(y as string)],
        where,
      );
      break;
    case "vault":
      assert.deepEqual(
        snap.vault,
        [BigInt(x as string), BigInt(y as string)],
        where,
      );
      break;
    case "out":
      assert.deepEqual(
        snap.out,
        [BigInt(x as string), BigInt(y as string)],
        where,
      );
      break;
    case "epoch":
      assert.equal(snap.epoch, BigInt(x as string), where);
      break;
    case "key":
      assert.equal(snap.key, Number(x), where);
      break;
    case "shield":
      assert.equal(snap.shields[Number(x)]?.status, y as ShieldStatus, where);
      break;
    default:
      throw new Error(`unknown expectation ${line}`);
  }
}

test("the golden vectors hold for the TypeScript reference model", () => {
  let model: BalanceModel | undefined;
  let name = "";
  let scenarios = 0;
  let steps = 0;
  for (const raw of VECTORS.split("\n")) {
    const line = raw.trim();
    if (line === "" || line.startsWith("#")) continue;
    if (line.startsWith("scenario ")) {
      name = line.slice("scenario ".length);
      model = new BalanceModel();
      scenarios++;
      continue;
    }
    assert.ok(model, "a step before any scenario");
    if (line.startsWith("expect ")) {
      expectLine(model, name, line);
      continue;
    }
    const mustReject = line.startsWith("!");
    const outcome = runStep(model, mustReject ? line.slice(1).trim() : line);
    assert.equal(outcome.type === "rejected", mustReject, `${name}: ${line}`);
    steps++;
  }
  assert.ok(scenarios >= 8, "expected at least 8 vector scenarios");
  assert.ok(steps > 80, "expected the vectors to run steps");
});
