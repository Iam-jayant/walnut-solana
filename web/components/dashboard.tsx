"use client";

import { useWallet } from "@solana/wallet-adapter-react";
import {
  bandEdgesBps,
  borrowRateBps,
  RATE_SOURCE_BAND,
} from "@walnut/sdk";
import type { ReactNode } from "react";
import { SOL_DECIMALS, USDC_DECIMALS } from "../lib/config.ts";
import { formatUnits } from "../lib/format.ts";
import {
  formatBps,
  indexesAt,
  marketStats,
  positionValues,
} from "../lib/lend.ts";
import {
  borrowMetrics,
  healthOf,
  type Health,
} from "../lib/metrics.ts";

const HEALTH_STYLE: Record<
  Health,
  { label: string; bar: string; tone: "good" | "warn" | "danger" | "neutral" }
> = {
  none: { label: "No debt", bar: "bg-emerald-500", tone: "neutral" },
  healthy: { label: "Healthy", bar: "bg-emerald-500", tone: "good" },
  moderate: { label: "Moderate", bar: "bg-amber-400", tone: "warn" },
  high: { label: "High liquidation risk", bar: "bg-red-500", tone: "danger" },
};
import { ActionPanel } from "./action-panel.tsx";
import { ActivityPanel } from "./activity-panel.tsx";
import type { AppTab } from "./app-sidebar.tsx";
import { LiquidationsPanel } from "./liquidations-panel.tsx";
import { PrivacyPanel } from "./privacy-panel.tsx";
import {
  Badge,
  Button,
  LockIcon,
  Notice,
  Spinner,
} from "./ui.tsx";
import { useNowSeconds } from "./use-now.ts";
import { useWalnut } from "./walnut-provider.tsx";

function Stat({
  label,
  value,
  unit,
  hint,
  badge,
  icon,
}: {
  label: string;
  value: ReactNode;
  unit?: string;
  hint?: string;
  badge?: ReactNode;
  icon?: ReactNode;
}) {
  return (
    <div className="rounded-xl bg-[#F4F5F7] border border-black/[0.03] p-4.5 hover:border-black/10 transition">
      <div className="flex items-center justify-between gap-2 mb-2">
        <span className="text-[12px] font-medium text-gray-500">{label}</span>
        {icon || badge}
      </div>
      <p className="text-[20px] md:text-[22px] font-semibold text-black tracking-tight flex items-baseline gap-1.5">
        <span>{value}</span>
        {unit && <span className="text-xs font-normal text-gray-400">{unit}</span>}
      </p>
      {hint ? (
        <p className="mt-1 text-[11px] text-gray-400 font-normal leading-tight">
          {hint}
        </p>
      ) : null}
    </div>
  );
}

function Hidden() {
  return (
    <span
      className="inline-block tracking-[2.5px] text-gray-300 select-none font-bold"
      title="Unlock with your Viewing Key to view"
    >
      ••••••
    </span>
  );
}

export function SetupCard() {
  const {
    accountState,
    createAccount,
    busy,
    protocol,
    error,
  } = useWalnut();

  if (accountState === "loading") {
    return (
      <div className="rounded-[24px] bg-white border border-black/[0.04] p-6 shadow-xs mb-6 flex items-center gap-3 text-sm text-gray-500 font-app-sans">
        <Spinner /> Loading your account...
      </div>
    );
  }
  if (accountState === "none") {
    return (
      <div className="rounded-[24px] bg-gradient-to-r from-amber-50 to-yellow-50 border border-amber-200/60 p-6 shadow-sm mb-6">
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
          <div>
            <h2 className="text-lg font-bold text-amber-950 flex items-center gap-2">
              <span className="flex h-6 w-6 items-center justify-center rounded-full bg-amber-200 text-amber-900 text-xs font-bold">
                !
              </span>
              Initialize your confidential account
            </h2>
            <p className="mt-1 text-sm text-amber-900/80 max-w-xl">
              Your account holds your encrypted balances, collateral and debt. You will sign a message to create your Viewing Key, then confirm one transaction. Setup takes a few seconds while the MPC network initializes your encrypted state.
            </p>
          </div>
          <button
            type="button"
            onClick={() => void createAccount()}
            disabled={busy || protocol === null}
            className="inline-flex items-center justify-center gap-2 rounded-xl bg-[#FFD208] hover:bg-[#F5C800] px-5 py-2.5 font-medium text-sm text-black shadow-xs transition active:scale-[0.99] disabled:opacity-50 shrink-0 cursor-pointer"
          >
            {busy ? <Spinner /> : null}
            {busy ? "Initializing..." : "Create Account"}
          </button>
        </div>
        {error !== null && (
          <p className="mt-3 text-xs text-red-700 bg-red-50 p-2.5 rounded-lg border border-red-200">
            {error}
          </p>
        )}
      </div>
    );
  }
  return null;
}

export function Notices() {
  const { stuck, walletBalances, abandonStuck, unwrapPending, busy } =
    useWalnut();
  return (
    <>
      {stuck !== null ? (
        <div className="rounded-2xl bg-amber-50 border border-amber-200 p-4 mb-4">
          <p className="font-bold text-amber-900 text-sm">
            Previous MPC computation was not settled on-chain.
          </p>
          <div className="mt-2 flex items-center gap-2">
            <button
              type="button"
              onClick={abandonStuck}
              className="text-xs font-semibold text-amber-900 underline hover:no-underline"
            >
              Clear state
            </button>
          </div>
        </div>
      ) : null}

      {walletBalances !== null && walletBalances.wsol > 0n ? (
        <div className="rounded-2xl bg-blue-50 border border-blue-200 p-4 mb-4 flex items-center justify-between">
          <p className="text-xs text-blue-900 font-medium">
            {formatUnits(walletBalances.wsol, SOL_DECIMALS)} wrapped SOL (wSOL) is waiting in your wallet.
          </p>
          <button
            type="button"
            onClick={() => void unwrapPending()}
            disabled={busy}
            className="text-xs font-bold text-blue-900 bg-white px-3 py-1 rounded-lg border border-blue-200 hover:bg-blue-100 transition cursor-pointer"
          >
            {busy ? "Unwrapping..." : "Unwrap to SOL"}
          </button>
        </div>
      ) : null}
    </>
  );
}

/**
 * User Wallet Summary: Detailed view of the 7 core private positions,
 * position health, borrow utilization, and public balances.
 */
export function Overview({ onSelectTab }: { onSelectTab?: ((tab: AppTab) => void) | undefined }) {
  const {
    balances,
    price,
    protocol,
    lending,
    walletBalances,
    refresh,
    refreshing,
    unlocked,
    unlock,
  } = useWalnut();
  const now = useNowSeconds();
  const idx = indexesAt(lending, protocol, now);
  const values = balances === null ? null : positionValues(balances, idx);
  const m =
    balances !== null && protocol !== null && values !== null
      ? borrowMetrics(balances, price.price, protocol.ltvBps, values.debt)
    : null;
  const health =
    m === null ? null : HEALTH_STYLE[healthOf(m.usedPercent, m.debt)];
  const usdc = (v: bigint) => formatUnits(v, USDC_DECIMALS);
  const sol = (v: bigint) => formatUnits(v, SOL_DECIMALS);

  return (
    <div className="space-y-6">
      {/* Unlock Viewing Key Banner if balances are encrypted */}
      {!unlocked && (
        <div className="rounded-2xl bg-white border border-black/[0.06] shadow-2xs p-5 md:p-6 flex flex-col sm:flex-row sm:items-center justify-between gap-4">
          <div className="flex items-center gap-3.5">
            <div className="h-10 w-10 rounded-xl bg-amber-50 text-amber-700 flex items-center justify-center shrink-0 border border-amber-200/50">
              <LockIcon />
            </div>
            <div>
              <h3 className="text-sm font-semibold text-gray-900">
                Private positions are encrypted
              </h3>
              <p className="text-xs text-gray-500 mt-0.5">
                Sign a zero-gas message to decrypt and inspect your onchain confidential positions.
              </p>
            </div>
          </div>
          <button
            type="button"
            onClick={() => void unlock()}
            className="inline-flex items-center justify-center gap-2 rounded-xl bg-[#FFD208] hover:bg-[#F5C800] px-5 py-2.5 font-medium text-xs text-black shadow-xs transition active:scale-[0.99] shrink-0 cursor-pointer"
          >
            <LockIcon /> Unlock private balances
          </button>
        </div>
      )}

      {/* Main Private Positions Card */}
      <div className="rounded-2xl bg-white border border-black/[0.06] shadow-[0_2px_16px_rgba(0,0,0,0.03)] p-6 md:p-8">
        <div className="flex flex-wrap items-center justify-between gap-3 pb-5 border-b border-black/[0.04]">
          <div>
            <h2 className="text-[20px] font-semibold text-black tracking-tight flex items-center gap-2.5">
              <span>Private Positions</span>
              <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full bg-emerald-50 text-emerald-700 text-[11px] font-medium border border-emerald-200">
                <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
                Encrypted on-chain
              </span>
            </h2>
            <p className="text-[13px] text-gray-500 mt-0.5">
              Your confidential balances, collateral, and debt verified privately by Arcium MPC
            </p>
          </div>
          <button
            type="button"
            onClick={() => void refresh()}
            disabled={refreshing}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-[#F5F5F7] hover:bg-[#EAEAEA] text-xs font-medium text-gray-700 transition cursor-pointer"
            title="Refresh onchain state"
          >
            {refreshing ? <Spinner /> : <span className="text-xs">↻</span>}
            <span>Refresh</span>
          </button>
        </div>

        {/* The 7 Core Private Position Tiles */}
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4 mt-6">
          {/* 1. Free SOL */}
          <Stat
            label="Free SOL"
            unit="SOL"
            value={balances ? sol(balances.freeSol) : <Hidden />}
            hint="Available to withdraw or deposit as collateral"
            badge={
              <span className="text-[10px] font-medium px-2 py-0.5 rounded-full bg-emerald-50 text-emerald-700">
                Shielded
              </span>
            }
          />

          {/* 2. Free USDC */}
          <Stat
            label="Free USDC"
            unit="USDC"
            value={balances ? usdc(balances.freeUsdc) : <Hidden />}
            hint="Available to withdraw or supply to pool"
            badge={
              <span className="text-[10px] font-medium px-2 py-0.5 rounded-full bg-emerald-50 text-emerald-700">
                Shielded
              </span>
            }
          />

          {/* 3. Collateral (SOL) */}
          <Stat
            label="Collateral"
            unit="SOL"
            value={balances ? sol(balances.collateralSol) : <Hidden />}
            hint="Backs your private borrowing limit"
            badge={
              <span className="text-[10px] font-medium px-2 py-0.5 rounded-full bg-amber-50 text-amber-700">
                Locked
              </span>
            }
          />

          {/* 4. Supplied (USDC) */}
          <Stat
            label="Supplied"
            unit="USDC"
            value={values ? usdc(values.supply) : <Hidden />}
            hint="Earning interest, updated live"
            badge={
              <span className="text-[10px] font-medium px-2 py-0.5 rounded-full bg-blue-50 text-blue-700">
                Earning APY
              </span>
            }
          />

          {/* 5. Debt (USDC) */}
          <Stat
            label="Debt"
            unit="USDC"
            value={values ? usdc(values.debt) : <Hidden />}
            hint="Includes accrued interest, updated live"
            badge={
              <span className="text-[10px] font-medium px-2 py-0.5 rounded-full bg-red-50 text-red-600">
                Borrowed
              </span>
            }
          />

          {/* 6. Borrow Limit */}
          <Stat
            label="Borrow Limit"
            unit="USDC"
            value={m ? usdc(m.limit) : <Hidden />}
            hint="Calculated via collateral & Pyth price"
            badge={
              <span className="text-[10px] font-medium px-2 py-0.5 rounded-full bg-gray-100 text-gray-700">
                Max LTV
              </span>
            }
          />

          {/* 7. Available to Borrow */}
          <div className="rounded-xl bg-[#F4F5F7] border border-black/[0.03] p-4.5 sm:col-span-2 hover:border-black/10 transition">
            <div className="flex items-center justify-between gap-2 mb-2">
              <span className="text-[12px] font-medium text-gray-500">
                Available to Borrow
              </span>
              {onSelectTab && (
                <button
                  type="button"
                  onClick={() => onSelectTab("borrow")}
                  className="text-[11px] font-semibold text-black underline underline-offset-2 hover:opacity-75 transition cursor-pointer"
                >
                  Borrow now →
                </button>
              )}
            </div>
            <p className="text-[20px] md:text-[22px] font-bold text-black tracking-tight flex items-baseline gap-1.5">
              <span>{m ? usdc(m.available) : <Hidden />}</span>
              <span className="text-xs font-normal text-gray-400">USDC</span>
            </p>
            <p className="mt-1 text-[11px] text-gray-400 font-normal leading-tight">
              Instant private borrowing against encrypted collateral
            </p>
          </div>
        </div>

        {/* Position Health & Utilization Bar */}
        <div className="mt-6 rounded-xl bg-[#F4F5F7] border border-black/[0.03] p-4.5">
          <div className="flex items-center justify-between mb-2.5">
            <span className="text-[13px] font-medium text-gray-700">
              Position Health & Borrow Utilization
            </span>
            {health !== null ? (
              <span
                className={`px-2.5 py-0.5 rounded-md text-[11px] font-semibold ${
                  health.tone === "good"
                    ? "bg-emerald-100 text-emerald-800"
                    : health.tone === "warn"
                    ? "bg-amber-100 text-amber-800"
                    : health.tone === "danger"
                    ? "bg-red-100 text-red-800"
                    : "bg-gray-100 text-gray-700"
                }`}
              >
                {health.label}
              </span>
            ) : null}
          </div>

          <div className="h-2 w-full bg-gray-200 rounded-full overflow-hidden">
            <div
              className={`h-full rounded-full transition-all duration-300 ${
                health?.bar ?? "bg-emerald-500"
              }`}
              style={{
                width: `${m === null ? 0 : Math.max(m.usedPercent, m.debt > 0n ? 2 : 0)}%`,
              }}
            />
          </div>

          <div className="flex flex-wrap items-center justify-between text-xs text-gray-400 mt-2.5 gap-2">
            <span>
              {m === null
                ? "Unlock viewing key to see your position health"
                : `${m.usedPercent.toFixed(1)}% of borrow limit used`}
            </span>
            <span>
              SOL price (Pyth): ${usdc(price.price)}
              {protocol !== null ? ` · Max LTV ${Number(protocol.ltvBps) / 100}%` : ""}
            </span>
          </div>
        </div>
      </div>

      {/* Public Balances & Quick Action Shortcuts */}
      <div className="grid gap-6 md:grid-cols-2">
        {/* Public Wallet Balances */}
        <div className="rounded-2xl bg-white border border-black/[0.06] shadow-[0_2px_16px_rgba(0,0,0,0.03)] p-6">
          <div className="flex items-center justify-between mb-4 pb-3 border-b border-black/[0.04]">
            <h3 className="text-base font-semibold text-gray-900">Public Wallet Balances</h3>
            <span className="text-xs text-gray-400">On-chain L1</span>
          </div>

          <dl className="space-y-3">
            <div className="flex items-center justify-between">
              <dt className="text-xs text-gray-500">SOL Balance</dt>
              <dd className="text-base font-semibold text-black">
                {walletBalances ? sol(walletBalances.sol) : "-"} <span className="text-xs text-gray-400 font-normal">SOL</span>
              </dd>
            </div>
            <div className="flex items-center justify-between">
              <dt className="text-xs text-gray-500">USDC Balance</dt>
              <dd className="text-base font-semibold text-black">
                {walletBalances ? usdc(walletBalances.usdc) : "-"} <span className="text-xs text-gray-400 font-normal">USDC</span>
              </dd>
            </div>
          </dl>

          <div className="mt-5 pt-4 border-t border-black/[0.04] flex items-center justify-end">
            {onSelectTab && (
              <button
                type="button"
                onClick={() => onSelectTab("deposit")}
                className="inline-flex items-center gap-1.5 px-3.5 py-1.5 rounded-lg bg-[#FFD208] hover:bg-[#F5C800] text-xs font-semibold text-black transition cursor-pointer"
              >
                Shield assets →
              </button>
            )}
          </div>
        </div>

        {/* Quick Tools Navigator */}
        <div className="rounded-2xl bg-white border border-black/[0.06] shadow-[0_2px_16px_rgba(0,0,0,0.03)] p-6 flex flex-col justify-between">
          <div>
            <div className="flex items-center justify-between mb-4 pb-3 border-b border-black/[0.04]">
              <h3 className="text-base font-semibold text-gray-900">Confidential Operations</h3>
              <span className="text-xs text-gray-400">Arcium MPC</span>
            </div>
            <p className="text-xs text-gray-500 leading-relaxed mb-4">
              All tools are accessible directly from the sidebar. You can shield assets, borrow privately, or earn yield with complete onchain privacy.
            </p>
          </div>

          <div className="grid grid-cols-2 gap-2">
            <button
              type="button"
              onClick={() => onSelectTab?.("deposit")}
              className="flex items-center justify-center gap-2 py-2.5 px-3 rounded-lg bg-[#F4F5F7] border border-black/[0.03] hover:bg-[#ECEEF1] text-xs font-medium text-gray-800 transition cursor-pointer"
            >
              Deposit (Shield)
            </button>
            <button
              type="button"
              onClick={() => onSelectTab?.("withdraw")}
              className="flex items-center justify-center gap-2 py-2.5 px-3 rounded-lg bg-[#F4F5F7] border border-black/[0.03] hover:bg-[#ECEEF1] text-xs font-medium text-gray-800 transition cursor-pointer"
            >
              Withdraw (Unshield)
            </button>
            <button
              type="button"
              onClick={() => onSelectTab?.("borrow")}
              className="flex items-center justify-center gap-2 py-2.5 px-3 rounded-lg bg-[#F4F5F7] border border-black/[0.03] hover:bg-[#ECEEF1] text-xs font-medium text-gray-800 transition cursor-pointer"
            >
              Borrow & Repay
            </button>
            <button
              type="button"
              onClick={() => onSelectTab?.("supply")}
              className="flex items-center justify-center gap-2 py-2.5 px-3 rounded-lg bg-[#F4F5F7] border border-black/[0.03] hover:bg-[#ECEEF1] text-xs font-medium text-gray-800 transition cursor-pointer"
            >
              Supply & Earn
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

export function MarketCard() {
  const { protocol, lending, lendingChecked, refreshBand, busy } = useWalnut();
  const now = useNowSeconds();
  const usdc = (v: bigint) => formatUnits(v, USDC_DECIMALS);

  if (lending === null || protocol === null) {
    return (
      <div className="rounded-[24px] md:rounded-[28px] bg-white border border-black/[0.04] shadow-[0_4px_30px_rgba(0,0,0,0.02)] p-6 md:p-8 font-app-sans">
        <h2 className="text-[20px] font-semibold text-black tracking-tight mb-4">
          Lending Market
        </h2>
        {lendingChecked && protocol !== null ? (
          <div className="rounded-[18px] bg-amber-50/70 border border-amber-200/60 p-5 text-amber-900">
            <p className="font-semibold text-sm">Lending market is being initialized</p>
            <p className="mt-1 text-xs text-amber-800/80">
              Supplying and borrowing open as soon as the market is live. Deposits and withdrawals work now.
            </p>
          </div>
        ) : (
          <p className="flex items-center gap-2 text-xs text-gray-500 py-4">
            <Spinner /> Loading lending market data...
          </p>
        )}
      </div>
    );
  }

  if (lending.pool.rateSource === RATE_SOURCE_BAND) {
    const [lower, upper] = bandEdgesBps(lending.band);
    const observed = lending.bandObservationSlot > 0;
    const age = Math.max(0, lending.slot - lending.bandObservationSlot);
    const isStale = age > 9000;
    const isNearing = age > 6000;
    const supplyApy =
      (borrowRateBps(protocol.rates, lower) *
        lower *
        (10_000n - protocol.reserveFactorBps)) /
      (10_000n * 10_000n);

    return (
      <div className="rounded-2xl bg-white border border-black/[0.06] shadow-[0_2px_16px_rgba(0,0,0,0.03)] p-6 md:p-8 font-app-sans">
        <div className="flex flex-wrap items-center justify-between gap-3 pb-5 border-b border-black/[0.04] mb-6">
          <div>
            <div className="flex items-center gap-2.5">
              <h2 className="text-[20px] font-semibold text-black tracking-tight">
                Lending Market
              </h2>
              <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full bg-emerald-50 text-[11px] font-semibold text-emerald-700 border border-emerald-200/60">
                <span className="h-1.5 w-1.5 rounded-full bg-emerald-500 animate-pulse" />
                Live Pool
              </span>
            </div>
            <p className="text-xs text-gray-500 mt-0.5">
              Confidential pool parameters, interest curves, and MPC-verified utilization
            </p>
          </div>
        </div>

        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          <Stat
            label="Utilization"
            unit=""
            value={`${Number(lower) / 100}–${Number(upper) / 100}%`}
            hint="Revealed as a 5% band, not an exact figure."
          />
          <Stat
            label="Borrow APR"
            unit=""
            value={formatBps(borrowRateBps(protocol.rates, upper))}
            hint="At the upper edge of the band."
          />
          <Stat
            label="Supply APY"
            unit=""
            value={formatBps(supplyApy)}
            hint="At the lower edge of the band."
          />
          <Stat
            label="Total Supplied"
            unit=""
            value={
              <span className="inline-flex items-center gap-1.5 text-[20px] font-bold text-black">
                <span className="text-amber-500 text-sm">🔒</span> Encrypted
              </span>
            }
            hint="Confidential pool liquidity"
          />
          <Stat
            label="Total Borrowed"
            unit=""
            value={
              <span className="inline-flex items-center gap-1.5 text-[20px] font-bold text-black">
                <span className="text-amber-500 text-sm">🔒</span> Encrypted
              </span>
            }
            hint="Confidential active loans"
          />
          <div className="rounded-xl bg-[#F4F5F7] border border-black/[0.03] p-4.5 hover:border-black/10 transition flex flex-col justify-between">
            <div>
              <div className="flex items-center justify-between gap-2 mb-2">
                <span className="text-[12px] font-medium text-gray-500">Band Age</span>
                {isStale ? (
                  <span className="inline-flex items-center gap-1 text-[11px] font-medium text-red-700 bg-red-50 border border-red-200/50 px-2 py-0.5 rounded-full">
                    <span className="h-1.5 w-1.5 rounded-full bg-red-500" />
                    Paused (stale)
                  </span>
                ) : isNearing ? (
                  <span className="inline-flex items-center gap-1 text-[11px] font-medium text-amber-700 bg-amber-50 border border-amber-200/50 px-2 py-0.5 rounded-full">
                    <span className="h-1.5 w-1.5 rounded-full bg-amber-500" />
                    Expiring soon
                  </span>
                ) : (
                  <span className="inline-flex items-center gap-1 text-[11px] font-medium text-emerald-700 bg-emerald-50 border border-emerald-200/50 px-2 py-0.5 rounded-full">
                    <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
                    Active curve
                  </span>
                )}
              </div>
              <p className="text-[20px] md:text-[22px] font-bold text-black tracking-tight flex items-baseline gap-1.5">
                <span>{observed ? age.toLocaleString("en-US") : "Not observed"}</span>
                {observed && <span className="text-xs font-normal text-gray-400">slots</span>}
              </p>
              <p className="mt-1 text-[11px] text-gray-400 font-normal leading-tight">
                {isStale
                  ? "Rate curve paused due to slot age"
                  : isNearing
                  ? "Approaching observation window limit"
                  : "Active onchain interest rate curve"}
              </p>
            </div>
            {observed && (
              <button
                type="button"
                onClick={() => void refreshBand()}
                disabled={busy}
                className="mt-3.5 w-full inline-flex items-center justify-center gap-2 rounded-lg bg-black hover:bg-gray-800 text-white px-3.5 py-2 text-xs font-semibold shadow-xs transition active:scale-[0.99] disabled:opacity-50 cursor-pointer"
              >
                {busy ? <Spinner /> : null}
                <span>Refresh rate curve</span>
              </button>
            )}
          </div>
        </div>

        <div className="mt-6 pt-4 border-t border-black/[0.04] flex items-center gap-2 text-xs text-gray-400">
          <svg className="w-3.5 h-3.5 text-gray-400 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <rect x="3" y="11" width="18" height="11" rx="2" ry="2" />
            <path d="M7 11V7a5 5 0 0 1 10 0v4" />
          </svg>
          <span>
            Pool totals are encrypted and computed by the Arcium MPC network. Only the utilization band and the interest indexes are public.
          </span>
        </div>
      </div>
    );
  }

  const idx = indexesAt(lending, protocol, now);
  const s = marketStats(lending, protocol, idx);

  return (
    <div className="rounded-2xl bg-white border border-black/[0.06] shadow-[0_2px_16px_rgba(0,0,0,0.03)] p-6 md:p-8 font-app-sans">
      <div className="flex flex-wrap items-center justify-between gap-3 pb-5 border-b border-black/[0.04] mb-6">
        <div>
          <div className="flex items-center gap-2.5">
            <h2 className="text-[20px] font-semibold text-black tracking-tight">
              Lending Market
            </h2>
            <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full bg-emerald-50 text-[11px] font-semibold text-emerald-700 border border-emerald-200/60">
              <span className="h-1.5 w-1.5 rounded-full bg-emerald-500 animate-pulse" />
              Live Pool
            </span>
          </div>
          <p className="text-xs text-gray-500 mt-0.5">
            Confidential pool parameters, interest curves, and MPC-verified utilization
          </p>
        </div>
      </div>

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        <Stat
          label="Utilization"
          unit=""
          value={formatBps(s.utilizationBps)}
          hint="Portion of supplied capital currently borrowed."
        />
        <Stat
          label="Borrow APR"
          unit=""
          value={formatBps(s.borrowAprBps)}
          hint="Interest accrued by outstanding debt."
        />
        <Stat
          label="Supply APY"
          unit=""
          value={formatBps(s.supplyApyBps)}
          hint="Compounded yield earned by pool shares."
        />
        <Stat
          label="Total Supplied"
          unit=""
          value={
            <span className="inline-flex items-center gap-1.5 text-[20px] font-bold text-black">
              <span className="text-amber-500 text-sm">🔒</span> Encrypted
            </span>
          }
          hint="Confidential pool liquidity"
        />
        <Stat
          label="Total Borrowed"
          unit=""
          value={
            <span className="inline-flex items-center gap-1.5 text-[20px] font-bold text-black">
              <span className="text-amber-500 text-sm">🔒</span> Encrypted
            </span>
          }
          hint="Confidential active loans"
        />
        <Stat
          label="LTV Limit"
          unit=""
          value={`${Number(protocol.ltvBps) / 100}%`}
          hint="Maximum debt allowed per unit of collateral."
        />
      </div>

      <div className="mt-6 pt-4 border-t border-black/[0.04] flex items-center gap-2 text-xs text-gray-400">
        <svg className="w-3.5 h-3.5 text-gray-400 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
          <rect x="3" y="11" width="18" height="11" rx="2" ry="2" />
          <path d="M7 11V7a5 5 0 0 1 10 0v4" />
        </svg>
        <span>
          Pool totals are encrypted and computed by the Arcium MPC network. Only the utilization rate and the interest indexes are public.
        </span>
      </div>
    </div>
  );
}

export function Dashboard({
  activeTab,
  onSelectTab,
}: {
  activeTab: AppTab;
  onSelectTab?: (tab: AppTab) => void;
}) {
  const { connected } = useWallet();

  return (
    <div className="mx-auto max-w-6xl font-app-sans">
      <div className="space-y-6">
        <SetupCard />
        <Notices />

        {connected ? (
          <>
            {/* Dashboard: Comprehensive User Wallet Summary with all 7 private positions */}
            {activeTab === "dashboard" && (
              <>
                <Overview onSelectTab={onSelectTab} />
                <MarketCard />
              </>
            )}

            {/* Deposit / Shield */}
            {(activeTab === "deposit" || activeTab === "shield") && (
              <div className="max-w-[480px] mx-auto w-full py-4">
                <ActionPanel
                  initialTab="deposit"
                  allowedTabs={["deposit", "withdraw"]}
                  title="Shield"
                />
              </div>
            )}

            {/* Withdraw / Unshield */}
            {(activeTab === "withdraw" || activeTab === "unshield") && (
              <div className="max-w-[480px] mx-auto w-full py-4">
                <ActionPanel
                  initialTab="withdraw"
                  allowedTabs={["deposit", "withdraw"]}
                  title="Shield"
                />
              </div>
            )}

            {/* Collateral */}
            {activeTab === "collateral" && (
              <div className="max-w-[480px] mx-auto w-full py-4">
                <ActionPanel
                  allowedTabs={["collateral"]}
                  title="Collateral"
                />
              </div>
            )}

            {/* Borrow & Repay */}
            {activeTab === "borrow" && (
              <div className="max-w-[480px] mx-auto w-full py-4">
                <ActionPanel
                  initialTab="borrow"
                  allowedTabs={["borrow", "repay"]}
                  title="Borrow & Repay"
                />
              </div>
            )}

            {/* Supply & Earn */}
            {(activeTab === "supply" || activeTab === "earn") && (
              <div className="max-w-[480px] mx-auto w-full py-4">
                <ActionPanel
                  initialTab="supply"
                  allowedTabs={["supply", "unsupply"]}
                  title="Earn"
                />
              </div>
            )}

            {/* Activity */}
            {activeTab === "activity" && (
              <div className="w-full">
                <ActivityPanel />
              </div>
            )}

            {/* Liquidations */}
            {activeTab === "liquidations" && (
              <div className="w-full">
                <LiquidationsPanel />
              </div>
            )}

            {/* Privacy */}
            {activeTab === "privacy" && (
              <div className="w-full">
                <PrivacyPanel />
              </div>
            )}
          </>
        ) : (
          <ActivityPanel />
        )}
      </div>
    </div>
  );
}
