"use client";

import {
  applyLendOp,
  borrowMint,
  clampPrice,
  CollateralOp,
  fullRepayAmountPrivate,
  LendOp,
  PoolOp,
  repayBurn,
  supplySharesFor,
  withdrawSupplyBurn,
} from "@walnut/sdk";
import { useState, type ReactNode } from "react";
import { SOL_DECIMALS, SOL_FEE_RESERVE, USDC_DECIMALS } from "../lib/config.ts";
import { formatUnits, parseUnits, toInputString } from "../lib/format.ts";
import { indexesAt, positionValues } from "../lib/lend.ts";
import { borrowMetrics, maxRemovableCollateral } from "../lib/metrics.ts";
import type { Asset } from "../lib/ops.ts";
import {
  Badge,
  LockIcon,
  Notice,
  Spinner,
} from "./ui.tsx";
import { useNowSeconds } from "./use-now.ts";
import { useWalnut } from "./walnut-provider.tsx";
import { TokenLogo } from "./token-logo.tsx";

export type Tab =
  | "deposit"
  | "withdraw"
  | "collateral"
  | "borrow"
  | "repay"
  | "supply"
  | "unsupply";

const TABS: ReadonlyArray<[Tab, string]> = [
  ["deposit", "Shield"],
  ["withdraw", "Unshield"],
  ["collateral", "Collateral"],
  ["borrow", "Borrow"],
  ["repay", "Repay"],
  ["supply", "Supply"],
  ["unsupply", "Withdraw Supply"],
];

/** Real token logos (USDC, SOL) with the confidential shield badge for c-tokens. */
export function TokenIcon({ symbol }: { symbol: string }) {
  return <TokenLogo symbol={symbol} size={24} />;
}

interface AmountFormProps {
  inputLabel: string;
  outputLabel: string;
  inputSymbol: string;
  outputSymbol: string;
  symbol?: string;
  decimals: number;
  max: bigint | null;
  maxLabel: string;
  cta: string;
  note?: string;
  warning: (amount: bigint) => string | null;
  needsUnlock: boolean;
  mode?: "private" | "public";
  strict?: boolean;
  onToggleAsset?: () => void;
  outputMultiplier?: number;
  onSubmit: (amount: bigint) => Promise<boolean | void>;
}

const PUBLIC_AMOUNT_NOTE =
  "Amounts are visible on chain for this action until private mode is enabled.";

function AmountForm(props: AmountFormProps) {
  const { busy, unlocked, stuck } = useWalnut();
  const [text, setText] = useState("");
  const amount = parseUnits(text, props.decimals);
  const valid = amount !== null && amount > 0n;
  const warning = valid ? props.warning(amount) : null;
  let blocked: string | null = null;
  if (props.needsUnlock && !unlocked)
    blocked = "Unlock private balances to continue.";
  else if (props.strict === true && warning !== null) blocked = warning;
  else if (stuck !== null)
    blocked = "A previous operation is still processing.";

  const outputValue =
    valid && props.outputMultiplier !== undefined
      ? (Number(text) * props.outputMultiplier).toLocaleString("en-US", {
          maximumFractionDigits: 4,
        })
      : valid
      ? text
      : "0";

  return (
    <form
      className="space-y-2.5"
      onSubmit={(event) => {
        event.preventDefault();
        if (!valid || blocked !== null || busy) return;
        void props.onSubmit(amount).then((ok) => {
          if (ok !== false) setText("");
        });
      }}
    >
      {/* 1. Top Input Box ("You shield" / "You deposit" / "You borrow" etc.) */}
      <div className="rounded-xl bg-[#F4F5F7] p-4.5 border border-black/[0.02] focus-within:bg-[#EEF0F3] transition">
        <div className="flex items-center justify-between mb-1.5">
          <span className="text-xs font-medium text-gray-500">{props.inputLabel}</span>
        </div>
        <div className="flex items-center justify-between gap-3">
          <input
            inputMode="decimal"
            autoComplete="off"
            placeholder="0"
            value={text}
            onChange={(e) => setText(e.target.value)}
            className="min-w-0 flex-1 bg-transparent text-3xl md:text-4xl font-normal text-gray-900 outline-none placeholder:text-gray-400"
            aria-label={`Amount in ${props.inputSymbol}`}
          />
          {props.onToggleAsset ? (
            <button
              type="button"
              onClick={props.onToggleAsset}
              className="bg-white border border-black/[0.06] rounded-xl px-3 py-1.5 flex items-center gap-2 shadow-2xs hover:bg-gray-50 transition cursor-pointer shrink-0"
              title="Click to switch token"
            >
              <TokenIcon symbol={props.inputSymbol} />
              <span className="text-sm font-semibold text-gray-900">{props.inputSymbol}</span>
              <svg
                className="w-3.5 h-3.5 text-gray-400"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.5"
              >
                <polyline points="6 9 12 15 18 9" />
              </svg>
            </button>
          ) : (
            <div className="bg-white border border-black/[0.06] rounded-xl px-3 py-1.5 flex items-center gap-2 shadow-2xs shrink-0">
              <TokenIcon symbol={props.inputSymbol} />
              <span className="text-sm font-semibold text-gray-900">{props.inputSymbol}</span>
            </div>
          )}
        </div>
        <div className="flex items-center justify-between mt-3 pt-2.5 border-t border-black/[0.04]">
          <span className="text-xs text-gray-500">
            {props.maxLabel}:{" "}
            <span className="font-medium text-gray-800">
              {props.max === null
                ? "locked"
                : `${formatUnits(props.max, props.decimals)} ${props.inputSymbol}`}
            </span>
          </span>
          <div className="flex items-center gap-1.5">
            <button
              type="button"
              disabled={props.max === null || props.max === 0n}
              onClick={() =>
                props.max !== null &&
                setText(toInputString(props.max / 2n, props.decimals))
              }
              className="rounded-md bg-black/[0.04] hover:bg-black/[0.08] text-gray-700 text-[11px] font-medium px-2 py-0.5 transition cursor-pointer disabled:opacity-30"
            >
              50%
            </button>
            <button
              type="button"
              disabled={props.max === null || props.max === 0n}
              onClick={() =>
                props.max !== null &&
                setText(toInputString(props.max, props.decimals))
              }
              className="rounded-md bg-black/[0.04] hover:bg-black/[0.08] text-gray-700 text-[11px] font-medium px-2 py-0.5 transition cursor-pointer disabled:opacity-30"
            >
              Max
            </button>
          </div>
        </div>
      </div>

      {/* 2. Output Box ("You receive" / Outcome) */}
      <div className="rounded-xl bg-[#F4F5F7] p-4.5 border border-black/[0.02]">
        <div className="flex items-center justify-between mb-1.5">
          <span className="text-xs font-medium text-gray-500">{props.outputLabel}</span>
          <span className="inline-flex items-center gap-1 text-[11px] font-medium text-emerald-700 bg-emerald-50 px-2 py-0.5 rounded-md">
            <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
            Encrypted
          </span>
        </div>
        <div className="flex items-center justify-between gap-3">
          <div className="text-3xl md:text-4xl font-normal text-gray-900 truncate">
            {outputValue}
          </div>
          <div className="bg-white border border-black/[0.06] rounded-xl px-3 py-1.5 flex items-center gap-2 shadow-2xs shrink-0">
            <TokenIcon symbol={props.outputSymbol} />
            <span className="text-sm font-semibold text-gray-900">{props.outputSymbol}</span>
          </div>
        </div>
        <div className="flex items-center justify-between mt-3 pt-2.5 border-t border-black/[0.04]">
          <span className="text-[11px] text-gray-400">
            1:1 confidential conversion · Verified onchain by Arcium MPC
          </span>
        </div>
      </div>

      {/* Warnings & Notes */}
      {text !== "" && !valid ? (
        <p className="text-xs text-red-600 px-1">Enter a valid amount.</p>
      ) : null}
      {warning !== null ? (
        <p className="rounded-xl bg-[#F1EFEA] px-3.5 py-2 text-xs text-[#2B2926] border border-[#D8D4CB]">
          {warning}
          {props.strict === true ? "" : " You can still submit it."}
        </p>
      ) : null}
      {props.mode === "private" ? (
        <p className="px-1">
          <Badge tone="dark">
            <LockIcon /> Amount encrypted
          </Badge>
        </p>
      ) : null}
      {props.mode === "public" ? (
        <p className="text-xs text-gray-400 px-1">{PUBLIC_AMOUNT_NOTE}</p>
      ) : null}
      {props.note !== undefined ? (
        <p className="text-xs text-gray-400 leading-relaxed px-1">{props.note}</p>
      ) : null}
      {blocked !== null ? (
        <p className="text-xs text-gray-500 px-1">{blocked}</p>
      ) : null}

      {/* 3. Action CTA Button */}
      <button
        type="submit"
        disabled={!valid || blocked !== null || busy}
        className={`w-full py-3.5 rounded-xl font-semibold text-sm transition active:scale-[0.99] mt-3 flex items-center justify-center gap-2 ${
          !valid || blocked !== null || busy
            ? "bg-[#EEF0F2] text-[#9EA3A8] cursor-not-allowed"
            : "bg-[#111111] hover:bg-[#2B2B2B] text-white shadow-xs cursor-pointer"
        }`}
      >
        {busy ? <Spinner /> : null}
        {blocked !== null
          ? blocked
          : !valid
          ? "Enter an amount"
          : props.cta}
      </button>
    </form>
  );
}

const LENDING_TABS: ReadonlySet<Tab> = new Set<Tab>([
  "collateral",
  "borrow",
  "repay",
  "supply",
  "unsupply",
]);

const min = (a: bigint, b: bigint): bigint => (a < b ? a : b);

export function ActionPanel({
  activeTab,
  onTabChange,
  allowedTabs,
  initialTab,
  title = "Manage",
}: {
  activeTab?: Tab;
  onTabChange?: (tab: Tab) => void;
  allowedTabs?: ReadonlyArray<Tab>;
  initialTab?: Tab;
  title?: string;
} = {}) {
  const w = useWalnut();
  const now = useNowSeconds();
  const [internalTab, setInternalTab] = useState<Tab>(
    initialTab ?? (allowedTabs && allowedTabs[0] ? allowedTabs[0] : "deposit"),
  );
  const tab = activeTab !== undefined ? activeTab : internalTab;
  const setTab = onTabChange !== undefined ? onTabChange : setInternalTab;
  const [asset, setAsset] = useState<Asset>("sol");
  const [collateralMode, setCollateralMode] = useState<"add" | "remove">("add");

  const toggleAsset = () => setAsset(asset === "sol" ? "usdc" : "sol");

  const {
    balances,
    walletBalances,
    price,
    protocol,
    lending,
    lendingChecked,
    privateMode,
  } = w;
  const mode = privateMode ? "private" : "public";
  const ltv = protocol?.ltvBps ?? 5_000n;
  const idx = indexesAt(lending, protocol, now);
  const values = balances === null ? null : positionValues(balances, idx);
  const debt = values?.debt ?? 0n;
  const liquidity = lending?.liquidity ?? 0n;

  /** Mirror of the circuit rules; null when the amount would be applied. */
  const check = (
    op: LendOp,
    amount: bigint,
    message: string,
  ): string | null => {
    if (balances === null || protocol === null) return null;
    try {
      let shares = 0n;
      if (op === LendOp.Borrow) shares = borrowMint(amount, idx.borrow);
      else if (op === LendOp.Repay) shares = repayBurn(amount, idx.borrow);
      else if (op === LendOp.Supply)
        shares = supplySharesFor(amount, idx.supply);
      else if (op === LendOp.WithdrawSupply) {
        shares = withdrawSupplyBurn(amount, idx.supply);
      }
      return applyLendOp(balances, op, amount, shares, {
        borrowIndex1e9: idx.borrow,
        price: clampPrice(price.price),
        ltvBps: ltv,
        liqThresholdBps: protocol.liquidationThresholdBps,
        positionDebtCap: protocol.positionDebtCap,
      }).applied
        ? null
        : message;
    } catch {
      return null;
    }
  };

  let body: ReactNode;
  if (LENDING_TABS.has(tab) && lending === null) {
    body = !lendingChecked ? (
      <p className="flex items-center gap-2 text-xs text-gray-500 py-6">
        <Spinner /> Loading lending market...
      </p>
    ) : (
      <Notice tone="info">
        <p className="font-semibold text-sm">Lending market is being initialized</p>
        <p className="mt-1 text-xs">
          Supplying, borrowing and collateral management open as soon as the
          market is live. Deposits and withdrawals work now.
        </p>
      </Notice>
    );
  } else if (tab === "deposit") {
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
      <AmountForm
        key={`dep-${asset}`}
        inputLabel="You shield"
        outputLabel="You receive"
        inputSymbol={isSol ? "SOL" : "USDC"}
        outputSymbol={isSol ? "cSOL" : "cUSDC"}
        symbol={isSol ? "SOL" : "USDC"}
        decimals={isSol ? SOL_DECIMALS : USDC_DECIMALS}
        max={walletMax}
        maxLabel="Available in wallet"
        cta={isSol ? "Shield SOL" : "Shield USDC"}
        onToggleAsset={toggleAsset}
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
    );
  } else if (tab === "withdraw") {
    const isSol = asset === "sol";
    const free =
      balances === null ? null : isSol ? balances.freeSol : balances.freeUsdc;
    body = (
      <AmountForm
        key={`wd-${asset}`}
        inputLabel="You unshield"
        outputLabel="You receive"
        inputSymbol={isSol ? "cSOL" : "cUSDC"}
        outputSymbol={isSol ? "SOL" : "USDC"}
        symbol={isSol ? "SOL" : "USDC"}
        decimals={isSol ? SOL_DECIMALS : USDC_DECIMALS}
        max={free}
        maxLabel="Free private balance"
        cta={isSol ? "Unshield SOL" : "Unshield USDC"}
        onToggleAsset={toggleAsset}
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
    );
  } else if (tab === "collateral") {
    const add = collateralMode === "add";
    const max =
      balances === null
        ? null
        : add
          ? balances.freeSol
          : maxRemovableCollateral(balances, price.price, ltv, debt);
    body = (
      <>
        {/* Sub-switcher for Collateral: Add vs Remove */}
        <div className="grid grid-cols-2 p-1 bg-[#F1F3F5] rounded-xl mb-4">
          <button
            type="button"
            onClick={() => setCollateralMode("add")}
            className={`py-2 text-xs md:text-sm font-semibold rounded-lg transition cursor-pointer text-center ${
              add
                ? "bg-white text-black shadow-xs"
                : "text-gray-500 hover:text-black"
            }`}
          >
            Add Collateral
          </button>
          <button
            type="button"
            onClick={() => setCollateralMode("remove")}
            className={`py-2 text-xs md:text-sm font-semibold rounded-lg transition cursor-pointer text-center ${
              !add
                ? "bg-white text-black shadow-xs"
                : "text-gray-500 hover:text-black"
            }`}
          >
            Remove Collateral
          </button>
        </div>

        <AmountForm
          key={`col-${collateralMode}`}
          inputLabel={add ? "Collateral to deposit" : "Collateral to remove"}
          outputLabel={add ? "Borrowing power added" : "Free SOL returned"}
          inputSymbol={add ? "SOL" : "cSOL"}
          outputSymbol={add ? "cSOL" : "SOL"}
          symbol="SOL"
          decimals={SOL_DECIMALS}
          max={max}
          maxLabel={add ? "Free SOL" : "Removable collateral"}
          cta={add ? "Deposit Collateral" : "Remove Collateral"}
          mode={mode}
          strict={privateMode}
          needsUnlock
          warning={(a) =>
            add
              ? check(
                  LendOp.AddCollateral,
                  a,
                  "This is more than your free SOL.",
                )
              : check(
                  LendOp.WithdrawCollateral,
                  a,
                  "This is more than you can remove while keeping your debt within the borrow limit.",
                )
          }
          onSubmit={(a) =>
            privateMode
              ? w.privateCollateral(
                  add ? CollateralOp.Add : CollateralOp.Withdraw,
                  add ? "Add collateral" : "Remove collateral",
                  a,
                )
              : w.lendOp(
                  add ? LendOp.AddCollateral : LendOp.WithdrawCollateral,
                  add ? "Add collateral" : "Remove collateral",
                  a,
                )
          }
        />
      </>
    );
  } else if (tab === "borrow") {
    const m =
      balances === null
        ? null
        : borrowMetrics(balances, price.price, ltv, debt);
    const capRoom =
      protocol !== null && protocol.positionDebtCap > debt
        ? protocol.positionDebtCap - debt
        : 0n;
    body = (
      <AmountForm
        inputLabel="You borrow"
        outputLabel="You receive"
        inputSymbol="USDC"
        outputSymbol="cUSDC"
        symbol="USDC"
        decimals={USDC_DECIMALS}
        max={
          m === null
            ? null
            : min(
                privateMode ? m.available : min(m.available, liquidity),
                capRoom,
              )
        }
        maxLabel="Available to borrow"
        cta="Borrow USDC"
        mode={mode}
        strict={privateMode}
        note="Borrowed USDC lands in your private free balance, and interest accrues on it continuously. Withdraw it to your wallet from the Withdraw tab."
        needsUnlock
        warning={(a) =>
          !privateMode && a > liquidity
            ? "The pool does not have this much free USDC right now."
            : check(LendOp.Borrow, a, "This would exceed your borrow limit.")
        }
        onSubmit={(a) =>
          privateMode
            ? w.privatePoolOp(PoolOp.Borrow, "Borrow USDC", a)
            : w.lendOp(LendOp.Borrow, "Borrow USDC", a)
        }
      />
    );
  } else if (tab === "repay") {
    const full =
      balances === null
        ? null
        : privateMode
          ? fullRepayAmountPrivate(balances.debtSharesUsdc, idx.borrow)
          : debt;
    const max =
      balances === null || full === null ? null : min(balances.freeUsdc, full);
    body = (
      <AmountForm
        inputLabel="You repay"
        outputLabel="Debt reduction"
        inputSymbol="cUSDC"
        outputSymbol="USDC"
        symbol="USDC"
        decimals={USDC_DECIMALS}
        max={max}
        maxLabel="Repayable now"
        cta="Repay USDC"
        mode={mode}
        strict={privateMode}
        note="Repayment is paid from your private free USDC and includes the interest accrued so far. Deposit USDC first if you need more."
        needsUnlock
        warning={(a) =>
          check(
            LendOp.Repay,
            a,
            "This is more than your free USDC or your outstanding debt.",
          )
        }
        onSubmit={(a) =>
          privateMode
            ? w.privatePoolOp(
                PoolOp.Repay,
                "Repay USDC",
                a,
                balances?.debtSharesUsdc,
              )
            : w.lendOp(LendOp.Repay, "Repay USDC", a)
        }
      />
    );
  } else if (tab === "supply") {
    body = (
      <AmountForm
        inputLabel="You supply"
        outputLabel="Pool shares received"
        inputSymbol="cUSDC"
        outputSymbol="cUSDC Shares"
        symbol="USDC"
        decimals={USDC_DECIMALS}
        max={balances === null ? null : balances.freeUsdc}
        maxLabel="Free private USDC"
        cta="Supply USDC"
        mode={mode}
        strict={privateMode}
        note="Supplied USDC earns interest from borrowers. Your supplied balance stays private. Deposit USDC first if you need more."
        needsUnlock
        warning={(a) =>
          check(LendOp.Supply, a, "This is more than your free private USDC.")
        }
        onSubmit={(a) =>
          privateMode
            ? w.privatePoolOp(PoolOp.Supply, "Supply USDC", a)
            : w.lendOp(LendOp.Supply, "Supply USDC", a)
        }
      />
    );
  } else {
    body = (
      <AmountForm
        inputLabel="You withdraw"
        outputLabel="You receive"
        inputSymbol="cUSDC Shares"
        outputSymbol="cUSDC"
        symbol="USDC"
        decimals={USDC_DECIMALS}
        max={
          values === null
            ? null
            : privateMode
              ? values.supply
              : min(values.supply, liquidity)
        }
        maxLabel="Withdrawable now"
        cta="Withdraw supply"
        mode={mode}
        strict={privateMode}
        note="Your supply plus the interest it earned returns to your private free USDC. Withdraw it to your wallet from the Withdraw tab."
        needsUnlock
        warning={(a) =>
          !privateMode && a > liquidity
            ? "The pool does not have this much free USDC right now. Borrowers need to repay first."
            : check(
                LendOp.WithdrawSupply,
                a,
                "This is more than you have supplied.",
              )
        }
        onSubmit={(a) =>
          privateMode
            ? w.privatePoolOp(
                PoolOp.WithdrawSupply,
                "Withdraw supply",
                a,
                balances?.supplySharesUsdc,
              )
            : w.lendOp(LendOp.WithdrawSupply, "Withdraw supply", a)
        }
      />
    );
  }

  const tabsToRender = allowedTabs
    ? TABS.filter(([key]) => allowedTabs.includes(key))
    : TABS;

  return (
    <div className="rounded-2xl bg-white border border-black/[0.06] shadow-[0_2px_16px_rgba(0,0,0,0.03)] p-6 md:p-7 max-w-[480px] w-full mx-auto font-app-sans text-left">
      <div className="mb-4 flex items-center justify-between gap-3">
        <h2 className="text-base font-semibold text-gray-900 tracking-tight">{title}</h2>
      </div>

      {tabsToRender.length > 1 && (
        <div className="grid grid-cols-2 p-1 bg-[#F1F3F5] rounded-xl mb-4">
          {tabsToRender.map(([key, label]) => (
            <button
              key={key}
              type="button"
              onClick={() => setTab(key)}
              className={`py-2 text-xs md:text-sm font-semibold rounded-lg transition cursor-pointer text-center ${
                tab === key
                  ? "bg-white text-black shadow-xs"
                  : "text-gray-500 hover:text-black"
              }`}
            >
              {label}
            </button>
          ))}
        </div>
      )}

      {body}
    </div>
  );
}
