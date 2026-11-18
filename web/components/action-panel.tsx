"use client";

import { applyPositionOp, PositionOp } from "@walnut/sdk";
import { useState, type ReactNode } from "react";
import { SOL_DECIMALS, SOL_FEE_RESERVE, USDC_DECIMALS } from "../lib/config.ts";
import { formatUnits, parseUnits, toInputString } from "../lib/format.ts";
import { borrowMetrics, maxRemovableCollateral } from "../lib/metrics.ts";
import type { Asset } from "../lib/ops.ts";
import { Button, Card, CardTitle, Spinner } from "./ui.tsx";
import { useWalnut } from "./walnut-provider.tsx";

type Tab = "deposit" | "withdraw" | "collateral" | "borrow" | "repay";

const TABS: ReadonlyArray<[Tab, string]> = [
  ["deposit", "Deposit"],
  ["withdraw", "Withdraw"],
  ["collateral", "Collateral"],
  ["borrow", "Borrow"],
  ["repay", "Repay"],
];

function Segmented<T extends string>({
  value,
  options,
  onChange,
}: {
  value: T;
  options: ReadonlyArray<[T, string]>;
  onChange: (value: T) => void;
}) {
  return (
    <div className="inline-flex rounded-pill bg-walnut-mist p-1">
      {options.map(([key, label]) => (
        <button
          key={key}
          type="button"
          onClick={() => onChange(key)}
          className={`rounded-pill px-4 py-1.5 text-small font-[800] transition ${
            value === key
              ? "bg-walnut-black text-walnut-white"
              : "text-walnut-ink/70 hover:text-walnut-black"
          }`}
        >
          {label}
        </button>
      ))}
    </div>
  );
}

interface AmountFormProps {
  symbol: string;
  decimals: number;
  max: bigint | null;
  maxLabel: string;
  cta: string;
  note?: string;
  warning: (amount: bigint) => string | null;
  needsUnlock: boolean;
  onSubmit: (amount: bigint) => Promise<void>;
}

function AmountForm(props: AmountFormProps) {
  const { busy, unlocked, stuck } = useWalnut();
  const [text, setText] = useState("");
  const amount = parseUnits(text, props.decimals);
  const valid = amount !== null && amount > 0n;
  const warning = valid ? props.warning(amount) : null;
  let blocked: string | null = null;
  if (props.needsUnlock && !unlocked)
    blocked = "Unlock private balances to continue.";
  else if (stuck !== null)
    blocked = "A previous operation is still processing.";

  return (
    <form
      className="space-y-4"
      onSubmit={(event) => {
        event.preventDefault();
        if (!valid || blocked !== null || busy) return;
        void props.onSubmit(amount).then(() => setText(""));
      }}
    >
      <div>
        <div className="flex items-center rounded-card bg-walnut-mist px-4 py-3 ring-1 ring-transparent focus-within:ring-walnut-black">
          <input
            inputMode="decimal"
            autoComplete="off"
            placeholder="0.00"
            value={text}
            onChange={(e) => setText(e.target.value)}
            className="min-w-0 flex-1 bg-transparent text-h3 font-[800] outline-none placeholder:text-walnut-ink/30"
            aria-label={`Amount in ${props.symbol}`}
          />
          <span className="mr-3 text-small font-[800] text-walnut-ink/70">
            {props.symbol}
          </span>
          <button
            type="button"
            disabled={props.max === null}
            onClick={() =>
              props.max !== null &&
              setText(toInputString(props.max, props.decimals))
            }
            className="rounded-pill bg-walnut-black px-3 py-1 text-caption font-[800] text-walnut-yellow disabled:opacity-30"
          >
            Max
          </button>
        </div>
        <p className="mt-2 text-caption text-walnut-ink/60">
          {props.maxLabel}:{" "}
          {props.max === null
            ? "locked"
            : `${formatUnits(props.max, props.decimals)} ${props.symbol}`}
        </p>
      </div>
      {text !== "" && !valid ? (
        <p className="text-small text-red-700">Enter a valid amount.</p>
      ) : null}
      {warning !== null ? (
        <p className="rounded-2xl bg-amber-50 px-4 py-2.5 text-small text-amber-900 ring-1 ring-amber-200">
          {warning} You can still submit it.
        </p>
      ) : null}
      {props.note !== undefined ? (
        <p className="text-caption text-walnut-ink/60">{props.note}</p>
      ) : null}
      {blocked !== null ? (
        <p className="text-small text-walnut-ink/70">{blocked}</p>
      ) : null}
      <Button
        type="submit"
        className="w-full py-3"
        disabled={!valid || blocked !== null || busy}
      >
        {busy ? <Spinner /> : null} {props.cta}
      </Button>
    </form>
  );
}

const ASSETS: ReadonlyArray<[Asset, string]> = [
  ["sol", "SOL"],
  ["usdc", "USDC"],
];

export function ActionPanel() {
  const w = useWalnut();
  const [tab, setTab] = useState<Tab>("deposit");
  const [asset, setAsset] = useState<Asset>("sol");
  const [collateralMode, setCollateralMode] = useState<"add" | "remove">("add");
  const { balances, walletBalances, price, protocol } = w;
  const ltv = protocol?.ltvBps ?? 5_000n;

  const check = (
    op: PositionOp,
    amount: bigint,
    message: string,
  ): string | null => {
    if (balances === null) return null;
    return applyPositionOp(balances, op, amount, price.price, ltv).applied
      ? null
      : message;
  };

  let body: ReactNode;
  if (tab === "deposit") {
    const isSol = asset === "sol";
    const walletMax =
      walletBalances === null
        ? null
        : isSol
          ? walletBalances.sol > SOL_FEE_RESERVE
            ? walletBalances.sol - SOL_FEE_RESERVE
            : 0n
          : walletBalances.usdc;
    body = (
      <>
        <Segmented value={asset} options={ASSETS} onChange={setAsset} />
        <div className="mt-5">
          <AmountForm
            key={`dep-${asset}`}
            symbol={isSol ? "SOL" : "USDC"}
            decimals={isSol ? SOL_DECIMALS : USDC_DECIMALS}
            max={walletMax}
            maxLabel="Available in wallet"
            cta={isSol ? "Deposit SOL" : "Deposit USDC"}
            note={
              isSol
                ? "SOL is wrapped automatically in the same transaction. The deposit amount becomes private once the MPC network credits it."
                : "The deposit amount is public in the transfer; your resulting balance stays private."
            }
            needsUnlock={false}
            warning={(a) =>
              walletMax !== null && a > walletMax
                ? "This is more than your wallet can spend."
                : null
            }
            onSubmit={(a) => w.deposit(asset, a)}
          />
        </div>
      </>
    );
  } else if (tab === "withdraw") {
    const isSol = asset === "sol";
    const free =
      balances === null ? null : isSol ? balances.freeSol : balances.freeUsdc;
    body = (
      <>
        <Segmented value={asset} options={ASSETS} onChange={setAsset} />
        <div className="mt-5">
          <AmountForm
            key={`wd-${asset}`}
            symbol={isSol ? "SOL" : "USDC"}
            decimals={isSol ? SOL_DECIMALS : USDC_DECIMALS}
            max={free}
            maxLabel="Free private balance"
            cta={isSol ? "Withdraw SOL" : "Withdraw USDC"}
            note={
              isSol
                ? "Withdrawing SOL takes two wallet confirmations: the private withdrawal, then unwrapping to native SOL."
                : "A USDC token account is created in your wallet first if you do not have one."
            }
            needsUnlock
            warning={(a) =>
              free !== null && a > free
                ? "This is more than your free private balance, so the withdrawal will be refused."
                : null
            }
            onSubmit={(a) => w.withdraw(asset, a)}
          />
        </div>
      </>
    );
  } else if (tab === "collateral") {
    const add = collateralMode === "add";
    const max =
      balances === null
        ? null
        : add
          ? balances.freeSol
          : maxRemovableCollateral(balances, price.price, ltv);
    body = (
      <>
        <Segmented
          value={collateralMode}
          options={[
            ["add", "Add"],
            ["remove", "Remove"],
          ]}
          onChange={setCollateralMode}
        />
        <div className="mt-5">
          <AmountForm
            key={`col-${collateralMode}`}
            symbol="SOL"
            decimals={SOL_DECIMALS}
            max={max}
            maxLabel={add ? "Free SOL" : "Removable collateral"}
            cta={add ? "Add collateral" : "Remove collateral"}
            needsUnlock
            warning={(a) =>
              add
                ? check(
                    PositionOp.AddCollateral,
                    a,
                    "This is more than your free SOL.",
                  )
                : check(
                    PositionOp.WithdrawCollateral,
                    a,
                    "This is more than you can remove while keeping your debt within the borrow limit.",
                  )
            }
            onSubmit={(a) =>
              w.positionOp(
                add ? PositionOp.AddCollateral : PositionOp.WithdrawCollateral,
                add ? "Add collateral" : "Remove collateral",
                a,
              )
            }
          />
        </div>
      </>
    );
  } else if (tab === "borrow") {
    const m =
      balances === null ? null : borrowMetrics(balances, price.price, ltv);
    body = (
      <AmountForm
        symbol="USDC"
        decimals={USDC_DECIMALS}
        max={m === null ? null : m.available}
        maxLabel="Available to borrow"
        cta="Borrow USDC"
        note="Borrowed USDC lands in your private free balance. Withdraw it to your wallet from the Withdraw tab."
        needsUnlock
        warning={(a) =>
          check(PositionOp.Borrow, a, "This would exceed your borrow limit.")
        }
        onSubmit={(a) => w.positionOp(PositionOp.Borrow, "Borrow USDC", a)}
      />
    );
  } else {
    const max =
      balances === null
        ? null
        : balances.freeUsdc < balances.debtSharesUsdc
          ? balances.freeUsdc
          : balances.debtSharesUsdc;
    body = (
      <AmountForm
        symbol="USDC"
        decimals={USDC_DECIMALS}
        max={max}
        maxLabel="Repayable now"
        cta="Repay USDC"
        note="Repayment is paid from your private free USDC. Deposit USDC first if you need more."
        needsUnlock
        warning={(a) =>
          check(
            PositionOp.Repay,
            a,
            "This is more than your free USDC or your outstanding debt.",
          )
        }
        onSubmit={(a) => w.positionOp(PositionOp.Repay, "Repay USDC", a)}
      />
    );
  }

  return (
    <Card>
      <CardTitle>Manage</CardTitle>
      <div className="mb-6 flex flex-wrap gap-1 border-b border-black/5 pb-3">
        {TABS.map(([key, label]) => (
          <button
            key={key}
            type="button"
            onClick={() => setTab(key)}
            className={`rounded-pill px-4 py-2 text-small font-[800] transition ${
              tab === key
                ? "bg-walnut-yellow text-walnut-black"
                : "text-walnut-ink/70 hover:bg-walnut-mist"
            }`}
          >
            {label}
          </button>
        ))}
      </div>
      {body}
    </Card>
  );
}
