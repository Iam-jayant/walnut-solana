"use client";

import React from "react";
import { formatUnits } from "../lib/format.ts";
import { USDC_DECIMALS } from "../lib/config.ts";
import { useWalnut } from "./walnut-provider.tsx";
import type { AppTab } from "./app-sidebar.tsx";

const TAB_TITLES: Record<string, { title: string; subtitle: string }> = {
  dashboard: {
    title: "Dashboard",
    subtitle: "Overview of your confidential portfolio, debt, and lending positions",
  },
  deposit: {
    title: "Deposit (Shield)",
    subtitle: "Wrap public SOL and USDC into confidential encrypted tokens",
  },
  shield: {
    title: "Deposit (Shield)",
    subtitle: "Wrap public SOL and USDC into confidential encrypted tokens",
  },
  withdraw: {
    title: "Withdraw (Unshield)",
    subtitle: "Withdraw confidential tokens back to your public Solana wallet",
  },
  unshield: {
    title: "Withdraw (Unshield)",
    subtitle: "Withdraw confidential tokens back to your public Solana wallet",
  },
  collateral: {
    title: "Manage Collateral",
    subtitle: "Deposit and withdraw collateral backing your confidential borrowing",
  },
  borrow: {
    title: "Borrow & Repay",
    subtitle: "Confidential debt positions with private MPC solvency verification",
  },
  supply: {
    title: "Supply & Earn",
    subtitle: "Supply USDC to the lending pool to earn real-time APY with private shares",
  },
  earn: {
    title: "Supply & Earn",
    subtitle: "Supply USDC to the lending pool to earn real-time APY with private shares",
  },
  activity: {
    title: "Activity",
    subtitle: "Real-time Arcium MPC computation queue and Solana onchain operations",
  },
  liquidations: {
    title: "Liquidations",
    subtitle: "Decentralized liquidation checks and solvency monitor",
  },
  privacy: {
    title: "Privacy & Viewing Keys",
    subtitle: "Derive, inspect, and export your self-sovereign Viewing Keys",
  },
};

interface AppHeaderBarProps {
  activeTab: AppTab;
  onToggleSidebar?: () => void;
}

export function AppHeaderBar({ activeTab, onToggleSidebar }: AppHeaderBarProps) {
  const { price } = useWalnut();

  const tabInfo = TAB_TITLES[activeTab] ?? TAB_TITLES.dashboard ?? {
    title: "Dashboard",
    subtitle: "Overview of your confidential portfolio, debt, and lending positions",
  };
  const usdc = (v: bigint) => formatUnits(v, USDC_DECIMALS);

  return (
    <header className="px-4 md:px-8 pt-6 pb-2 font-app-sans">
      <div className="flex items-center justify-between gap-4 max-w-6xl mx-auto">
        {/* Mobile Hamburger + Title & Subtitle */}
        <div className="flex items-center gap-3.5">
          {onToggleSidebar && (
            <button
              type="button"
              onClick={onToggleSidebar}
              className="lg:hidden p-2 -ml-1 rounded-xl bg-white border border-black/[0.06] text-gray-700 shadow-2xs hover:bg-gray-50 transition cursor-pointer"
              title="Toggle sidebar navigation"
              aria-label="Toggle navigation"
            >
              <svg
                className="w-5 h-5"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <line x1="3" y1="12" x2="21" y2="12" />
                <line x1="3" y1="6" x2="21" y2="6" />
                <line x1="3" y1="18" x2="21" y2="18" />
              </svg>
            </button>
          )}
          <div>
            <h1 className="text-2xl md:text-3xl font-bold text-black tracking-tight leading-tight">
              {tabInfo.title}
            </h1>
            <p className="text-xs md:text-sm text-gray-500 mt-0.5">
              {tabInfo.subtitle}
            </p>
          </div>
        </div>

        {/* Clean, minimal Pyth Oracle SOL Price indicator */}
        <div className="hidden sm:inline-flex items-center gap-1.5 px-3 py-1.5 rounded-full bg-white border border-black/[0.04] shadow-[0_2px_8px_rgba(0,0,0,0.02)] text-xs font-medium text-gray-600">
          <span className="text-gray-400 font-normal">SOL:</span>
          <span className="font-semibold text-black">${usdc(price.price)}</span>
        </div>
      </div>
    </header>
  );
}
