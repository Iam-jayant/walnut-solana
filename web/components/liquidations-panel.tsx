"use client";

import { useEffect, useRef, useState } from "react";
import { SOL_DECIMALS, USDC_DECIMALS } from "../lib/config.ts";
import { formatUnits, parseUnits, toInputString } from "../lib/format.ts";
import type { HealthResult } from "../lib/ops.ts";
import { TokenIcon } from "./action-panel.tsx";
import { LockIcon, Spinner } from "./ui.tsx";
import { useWalnut } from "./walnut-provider.tsx";

/** Health check and liquidation of any borrower's private account. */
export function LiquidationsPanel() {
  const w = useWalnut();
  const {
    lending,
    lendingChecked,
    protocol,
    busy,
    stuck,
    privateMode,
    balances,
    unlocked,
    unlock,
  } = w;

  const [borrower, setBorrower] = useState("");
  const [amountText, setAmountText] = useState("");
  const [health, setHealth] = useState<{
    borrower: string;
    result: HealthResult;
  } | null>(null);
  const [checkingHealth, setCheckingHealth] = useState(false);
  const [seize, setSeize] = useState<bigint | null>(null);
  const estimateRef = useRef(w.estimateSeize);
  estimateRef.current = w.estimateSeize;

  const typed = parseUnits(amountText, USDC_DECIMALS);
  useEffect(() => {
    if (!privateMode || typed === null || typed <= 0n) {
      setSeize(null);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(() => {
      void estimateRef.current(typed).then((v) => {
        if (!cancelled) setSeize(v);
      });
    }, 400);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [privateMode, typed]);

  if (lending === null) {
    return (
      <div className="rounded-2xl bg-white border border-black/[0.06] shadow-[0_2px_16px_rgba(0,0,0,0.03)] p-6 md:p-8 font-app-sans text-left">
        <h2 className="text-[20px] font-semibold text-black tracking-tight">Liquidations</h2>
        {!lendingChecked ? (
          <p className="flex items-center gap-2 text-xs text-gray-500 py-6">
            <Spinner /> Loading the lending market...
          </p>
        ) : (
          <div className="mt-4 rounded-xl bg-blue-50 border border-blue-200/60 p-4">
            <p className="font-semibold text-sm text-blue-900">Lending market is being initialized</p>
            <p className="mt-1 text-xs text-blue-700">
              Health checks and liquidations open together with the market.
            </p>
          </div>
        )}
      </div>
    );
  }

  const amount = parseUnits(amountText, USDC_DECIMALS);
  const amountValid = amount !== null && amount > 0n;
  const address = borrower.trim();
  const free =
    privateMode && unlocked && balances !== null ? balances.freeUsdc : null;
  const overFree =
    privateMode && amountValid && (free === null || amount > free);
  const blocked = busy || stuck !== null;
  const shown =
    health !== null && health.borrower === address ? health.result : null;

  const runCheck = async () => {
    if (!address || blocked || checkingHealth) return;
    setCheckingHealth(true);
    try {
      const result = await w.checkHealth(address);
      if (result !== undefined) setHealth({ borrower: address, result });
    } finally {
      setCheckingHealth(false);
    }
  };

  const handlePaste = async () => {
    try {
      if (navigator?.clipboard?.readText) {
        const text = await navigator.clipboard.readText();
        if (text) setBorrower(text.trim());
      }
    } catch {
      // Clipboard access denied or unsupported
    }
  };

  const thresholdPercent = protocol ? `${Number(protocol.liquidationThresholdBps) / 100}%` : "80%";
  const closeFactorPercent = protocol ? `${Number(protocol.closeFactorBps) / 100}%` : "50%";
  const bonusPercent = protocol ? `${Number(protocol.liquidationBonusBps) / 100}%` : "5%";

  return (
    <div className="rounded-2xl bg-white border border-black/[0.06] shadow-[0_2px_16px_rgba(0,0,0,0.03)] p-6 md:p-8 font-app-sans text-left space-y-6">
      {/* 1. Header with Title and Subtitle */}
      <div className="flex flex-wrap items-center justify-between gap-3 pb-5 border-b border-black/[0.04]">
        <div>
          <h2 className="text-[20px] font-semibold text-black tracking-tight flex items-center gap-2.5">
            <span>Liquidations</span>
            <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full bg-emerald-50 text-emerald-700 text-[11px] font-medium border border-emerald-200">
              <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
              Arcium MPC Verified
            </span>
          </h2>
          <p className="text-[13px] text-gray-500 mt-0.5">
            Decentralized liquidation checks and confidential solvency enforcement verified by Arcium MPC
          </p>
        </div>
      </div>

      {/* Unlock Viewing Key Banner if balances are encrypted */}
      {privateMode && !unlocked && (
        <div className="relative overflow-hidden rounded-2xl bg-white/70 backdrop-blur-xl border border-white/80 shadow-[0_8px_30px_rgb(0,0,0,0.04)] p-5 md:p-6 flex flex-col sm:flex-row sm:items-center justify-between gap-4 ring-1 ring-black/[0.04] before:absolute before:inset-0 before:bg-linear-to-b before:from-white/50 before:to-transparent before:pointer-events-none">
          <div className="flex items-center gap-3.5 relative z-10">
            <div className="h-10 w-10 rounded-xl bg-black/[0.06] backdrop-blur-md text-black/70 flex items-center justify-center shrink-0 border border-[#D8D4CB] shadow-2xs">
              <LockIcon />
            </div>
            <div>
              <h3 className="text-sm font-semibold text-gray-900 tracking-tight">
                Private balances are locked
              </h3>
              <p className="text-xs text-gray-500 mt-0.5">
                Unlock your viewing key to liquidate using your confidential free USDC.
              </p>
            </div>
          </div>
          <button
            type="button"
            onClick={() => void unlock()}
            className="relative z-10 inline-flex items-center justify-center gap-2 rounded-xl bg-[#111111] hover:bg-[#2B2B2B] px-5 py-2.5 font-semibold text-xs text-white shadow-[0_2px_12px_rgba(0,0,0,0.18)] transition active:scale-[0.99] shrink-0 cursor-pointer"
          >
            <LockIcon /> Unlock private balances
          </button>
        </div>
      )}

      {/* Pending Liquidation Notice */}
      {w.pendingLiquidation !== null && (
        <div className="rounded-xl bg-[#F1EFEA] border border-[#D8D4CB] p-4">
          <div className="flex items-start gap-3">
            <div className="h-6 w-6 rounded-full bg-[#E2DED5] text-[#2B2926] flex items-center justify-center shrink-0 text-xs font-bold">
              !
            </div>
            <div className="flex-1">
              <p className="font-semibold text-xs text-[#2B2926]">
                A liquidation is currently processing
              </p>
              <p className="mt-0.5 text-xs text-[#3A3732]">
                If the operation does not conclude before the lock timeout, release it to reclaim your repaid USDC.
              </p>
              <button
                type="button"
                disabled={busy}
                onClick={() => void w.abandonLiquidation()}
                className="mt-3 inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-orange-600 hover:bg-orange-700 text-white text-xs font-medium transition cursor-pointer disabled:opacity-40"
              >
                {busy ? <Spinner /> : null} Release liquidation
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Protocol Parameters Micro-cards */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
        <div className="rounded-xl bg-[#F4F5F7] p-3.5 border border-black/[0.03]">
          <span className="text-[11px] font-medium text-gray-500 uppercase tracking-wider block">
            Liquidation Threshold
          </span>
          <span className="text-lg font-semibold text-gray-900 mt-0.5 block">
            {thresholdPercent}
          </span>
          <p className="text-[11px] text-gray-500 mt-1 leading-snug">
            Positions with collateral ratio below this threshold become liquidatable.
          </p>
        </div>

        <div className="rounded-xl bg-[#F4F5F7] p-3.5 border border-black/[0.03]">
          <span className="text-[11px] font-medium text-gray-500 uppercase tracking-wider block">
            Close Factor
          </span>
          <span className="text-lg font-semibold text-gray-900 mt-0.5 block">
            {closeFactorPercent}
          </span>
          <p className="text-[11px] text-gray-500 mt-1 leading-snug">
            Maximum share of outstanding debt that one liquidation may repay.
          </p>
        </div>

        <div className="rounded-xl bg-[#F4F5F7] p-3.5 border border-black/[0.03]">
          <span className="text-[11px] font-medium text-gray-500 uppercase tracking-wider block">
            Liquidation Bonus
          </span>
          <span className="text-lg font-semibold text-emerald-700 mt-0.5 block">
            +{bonusPercent}
          </span>
          <p className="text-[11px] text-gray-500 mt-1 leading-snug">
            Reward bonus paid in SOL collateral over the repaid USDC value.
          </p>
        </div>
      </div>

      {/* Step 1: Health Check Box */}
      <div className="rounded-xl bg-[#F4F5F7] p-5 border border-black/[0.02]">
        <div className="flex items-center justify-between mb-2">
          <span className="text-xs font-semibold text-gray-700 uppercase tracking-wider">
            1. Inspect Borrower Solvency
          </span>
          <button
            type="button"
            onClick={() => void handlePaste()}
            className="text-[11px] font-medium text-gray-600 hover:text-black transition px-2 py-0.5 rounded-md bg-white border border-black/[0.06] cursor-pointer flex items-center gap-1 shadow-2xs"
          >
            <svg className="w-3 h-3 text-gray-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2" />
              <rect x="8" y="2" width="8" height="4" rx="1" ry="1" />
            </svg>
            Paste
          </button>
        </div>

        <p className="text-xs text-gray-500 mb-3 leading-relaxed">
          The Arcium MPC network computes whether the position is below its liquidation threshold without revealing the borrower’s underlying collateral or debt balances.
        </p>

        <div className="flex flex-col sm:flex-row items-stretch sm:items-center gap-2.5">
          <input
            value={borrower}
            onChange={(e) => setBorrower(e.target.value)}
            autoComplete="off"
            spellCheck={false}
            placeholder="Borrower wallet address (e.g. 7xKX...)"
            className="flex-1 bg-white rounded-xl px-3.5 py-2.5 text-xs font-mono text-gray-900 border border-black/[0.06] focus:border-black/30 outline-none transition placeholder:text-gray-400 placeholder:font-sans"
          />

          <button
            type="button"
            disabled={address === "" || blocked || checkingHealth}
            onClick={() => void runCheck()}
            className="rounded-xl bg-black hover:bg-gray-800 text-white text-xs font-semibold px-4 py-2.5 transition active:scale-[0.99] cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed flex items-center justify-center gap-2 shrink-0 shadow-xs"
          >
            {checkingHealth || (busy && health === null) ? <Spinner /> : null}
            Check Health
          </button>
        </div>

        {/* Live Health Status Result */}
        {shown !== null && (
          <div className="mt-3.5 pt-3 border-t border-black/[0.04]">
            {shown.unhealthy === true ? (
              <div className="inline-flex items-center gap-2 px-3 py-1.5 rounded-xl bg-red-50 text-red-700 text-xs font-semibold border border-red-200/80">
                <span className="h-2 w-2 rounded-full bg-red-500 animate-pulse" />
                <span>Below liquidation threshold (Eligible for liquidation)</span>
              </div>
            ) : shown.unhealthy === false ? (
              <div className="inline-flex items-center gap-2 px-3 py-1.5 rounded-xl bg-emerald-50 text-emerald-700 text-xs font-semibold border border-emerald-200/80">
                <span className="h-2 w-2 rounded-full bg-emerald-500" />
                <span>Healthy (Position is safely collateralized)</span>
              </div>
            ) : (
              <div className="inline-flex items-center gap-2 px-3 py-1.5 rounded-xl bg-[#F1EFEA] text-[#3A3732] text-xs font-semibold border border-[#D8D4CB]">
                <span className="h-2 w-2 rounded-full bg-[#F1EFEA]0" />
                <span>Health check inconclusive or account not found</span>
              </div>
            )}
          </div>
        )}
      </div>

      {/* Step 2: Liquidation Execution Box */}
      <div className="rounded-xl bg-[#F4F5F7] p-5 border border-black/[0.02]">
        <div className="flex items-center justify-between mb-2">
          <span className="text-xs font-semibold text-gray-700 uppercase tracking-wider">
            2. Repay Debt & Seize Collateral
          </span>
        </div>

        <div className="flex items-center justify-between gap-3 mt-3">
          <input
            inputMode="decimal"
            autoComplete="off"
            placeholder="0"
            value={amountText}
            onChange={(e) => setAmountText(e.target.value)}
            className="min-w-0 flex-1 bg-transparent text-3xl md:text-4xl font-normal text-gray-900 outline-none placeholder:text-gray-400"
            aria-label="Repay amount in USDC"
          />

          <div className="bg-white border border-black/[0.06] rounded-xl px-3 py-1.5 flex items-center gap-2 shadow-2xs shrink-0">
            <TokenIcon symbol={privateMode ? "cUSDC" : "USDC"} />
            <span className="text-sm font-semibold text-gray-900">
              {privateMode ? "cUSDC" : "USDC"}
            </span>
          </div>
        </div>

        {/* Balance row with 50% & Max quick-fill buttons */}
        <div className="flex items-center justify-between mt-3 pt-2.5 border-t border-black/[0.04]">
          <span className="text-xs text-gray-500">
            Your free {privateMode ? "private" : ""} USDC:{" "}
            <span className="font-medium text-gray-800">
              {free === null
                ? privateMode
                  ? "locked"
                  : "0.00 USDC"
                : `${formatUnits(free, USDC_DECIMALS)} USDC`}
            </span>
          </span>

          <div className="flex items-center gap-1.5">
            <button
              type="button"
              disabled={free === null || free === 0n}
              onClick={() =>
                free !== null &&
                setAmountText(toInputString(free / 2n, USDC_DECIMALS))
              }
              className="rounded-md bg-black/[0.04] hover:bg-black/[0.08] text-gray-700 text-[11px] font-medium px-2 py-0.5 transition cursor-pointer disabled:opacity-30"
            >
              50%
            </button>
            <button
              type="button"
              disabled={free === null || free === 0n}
              onClick={() =>
                free !== null &&
                setAmountText(toInputString(free, USDC_DECIMALS))
              }
              className="rounded-md bg-black/[0.04] hover:bg-black/[0.08] text-gray-700 text-[11px] font-medium px-2 py-0.5 transition cursor-pointer disabled:opacity-30"
            >
              Max
            </button>
          </div>
        </div>

        {/* Estimated Seized SOL Calculation */}
        {seize !== null && amountValid && (
          <div className="mt-3.5 p-3.5 rounded-xl bg-white border border-black/[0.06] flex items-center justify-between gap-3 shadow-2xs">
            <div>
              <span className="text-xs font-semibold text-gray-900 block">
                Estimated Collateral Seized
              </span>
              <span className="text-[11px] text-gray-500 mt-0.5 block">
                Includes +{bonusPercent} liquidation bonus at current oracle price
              </span>
            </div>
            <div className="flex items-center gap-2 shrink-0">
              <span className="text-base font-semibold text-emerald-700">
                {formatUnits(seize, SOL_DECIMALS)}
              </span>
              <TokenIcon symbol="SOL" />
              <span className="text-xs font-semibold text-gray-900">SOL</span>
            </div>
          </div>
        )}

        {/* Over free balance warning */}
        {overFree && (
          <p className="mt-3 rounded-xl bg-red-50 px-3.5 py-2 text-xs text-red-700 border border-red-200/60">
            {free === null
              ? "Unlock private balances to liquidate with free USDC."
              : "This amount is more than your free private USDC balance."}
          </p>
        )}

        {/* Privacy Note */}
        <div className="mt-3 text-xs text-gray-400 leading-relaxed">
          {privateMode ? (
            <p className="flex items-center gap-1.5 text-gray-500">
              <LockIcon />
              <span>
                Your repayment and the seized collateral move inside encrypted confidential positions; no amounts are exposed on the public Solana ledger.
              </span>
            </p>
          ) : (
            <p>
              Your USDC is transferred with the transaction and returned if the liquidation is refused. Amounts are visible onchain until private mode is enabled.
            </p>
          )}
        </div>

        {/* Primary Liquidate Action Button */}
        <button
          type="button"
          disabled={
            address === "" ||
            !amountValid ||
            blocked ||
            (privateMode && (free === null || overFree))
          }
          onClick={() =>
            privateMode
              ? void w
                  .privateLiquidate(address, amount ?? 0n)
                  .then((ok) => (ok ? setAmountText("") : undefined))
              : void w
                  .liquidate(address, amount ?? 0n)
                  .then(() => setAmountText(""))
          }
          className={`w-full py-3.5 rounded-xl font-semibold text-sm transition active:scale-[0.99] mt-4 flex items-center justify-center gap-2 ${
            address === "" ||
            !amountValid ||
            blocked ||
            (privateMode && (free === null || overFree))
              ? "bg-[#EEF0F2] text-[#9EA3A8] cursor-not-allowed"
              : "bg-[#111111] hover:bg-[#2B2B2B] text-white shadow-xs cursor-pointer"
          }`}
        >
          {busy ? <Spinner /> : null}
          {address === ""
            ? "Enter borrower wallet address"
            : !amountValid
            ? "Enter an amount to repay"
            : overFree
            ? "Insufficient free USDC"
            : "Liquidate Position"}
        </button>
      </div>
    </div>
  );
}
