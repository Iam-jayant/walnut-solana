"use client";

import React from "react";
import type { PublicKey } from "@solana/web3.js";
import { shortAddress } from "../lib/format.ts";
import { WalletAvatar } from "./wallet-avatar.tsx";
import { WalnutLogo, WalnutLockup } from "./brand-logo.tsx";

export type AppTab =
  | "dashboard"
  | "deposit"
  | "withdraw"
  | "collateral"
  | "borrow"
  | "supply"
  | "activity"
  | "liquidations"
  // Backward compatibility aliases
  | "shield"
  | "unshield"
  | "earn"
  | "privacy";

interface AppSidebarProps {
  activeTab: AppTab;
  onSelectTab: (tab: AppTab) => void;
  connected: boolean;
  publicKey: PublicKey | null;
  connecting: boolean;
  onConnect: () => void;
  onDisconnect: () => void;
  unlocked: boolean;
  onLock: () => void;
  isOpen?: boolean;
  onClose?: () => void;
  isCollapsed?: boolean;
  onToggleCollapse?: () => void;
}

export function AppSidebar({
  activeTab,
  onSelectTab,
  connected,
  publicKey,
  connecting,
  onConnect,
  onDisconnect,
  isOpen = false,
  onClose,
  isCollapsed = false,
  onToggleCollapse,
}: AppSidebarProps) {
  // Normalize alias tabs to primary ids for active state comparison
  const normalizedActive =
    activeTab === "shield"
      ? "deposit"
      : activeTab === "unshield"
      ? "withdraw"
      : activeTab === "earn"
      ? "supply"
      : activeTab;

  const navItems: Array<{
    id: AppTab;
    label: string;
    icon: (active: boolean) => React.ReactNode;
  }> = [
    {
      id: "dashboard",
      label: "Dashboard",
      icon: (active) => (
        <svg
          className={`h-4 w-4 shrink-0 ${active ? "text-white" : "text-[#555555]"}`}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.75"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <rect x="3" y="3" width="7" height="7" rx="1" />
          <rect x="14" y="3" width="7" height="7" rx="1" />
          <rect x="14" y="14" width="7" height="7" rx="1" />
          <rect x="3" y="14" width="7" height="7" rx="1" />
        </svg>
      ),
    },
    {
      id: "deposit",
      label: "Deposit (Shield)",
      icon: (active) => (
        <svg
          className={`h-4 w-4 shrink-0 ${active ? "text-white" : "text-[#555555]"}`}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.75"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
        </svg>
      ),
    },
    {
      id: "withdraw",
      label: "Withdraw (Unshield)",
      icon: (active) => (
        <svg
          className={`h-4 w-4 shrink-0 ${active ? "text-white" : "text-[#555555]"}`}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.75"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
          <line x1="4.5" y1="4.5" x2="19.5" y2="19.5" />
        </svg>
      ),
    },
    {
      id: "collateral",
      label: "Collateral",
      icon: (active) => (
        <svg
          className={`h-4 w-4 shrink-0 ${active ? "text-white" : "text-[#555555]"}`}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.75"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <polygon points="12 2 2 7 12 12 22 7 12 2" />
          <polyline points="2 17 12 22 22 17" />
          <polyline points="2 12 12 17 22 12" />
        </svg>
      ),
    },
    {
      id: "borrow",
      label: "Borrow & Repay",
      icon: (active) => (
        <svg
          className={`h-4 w-4 shrink-0 ${active ? "text-white" : "text-[#555555]"}`}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.75"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <line x1="22" y1="2" x2="11" y2="13" />
          <polygon points="22 2 15 22 11 13 2 9 22 2" />
        </svg>
      ),
    },
    {
      id: "supply",
      label: "Supply & Earn",
      icon: (active) => (
        <svg
          className={`h-4 w-4 shrink-0 ${active ? "text-white" : "text-[#555555]"}`}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.75"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <circle cx="8" cy="8" r="6" />
          <path d="M18 12a6 6 0 1 1-8.5-5.5" />
          <path d="M12 18a6 6 0 0 0 6-6" />
        </svg>
      ),
    },
    {
      id: "activity",
      label: "Activity",
      icon: (active) => (
        <svg
          className={`h-4 w-4 shrink-0 ${active ? "text-white" : "text-[#555555]"}`}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.75"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <circle cx="12" cy="12" r="10" />
          <polyline points="12 6 12 12 16 14" />
        </svg>
      ),
    },
    {
      id: "liquidations",
      label: "Liquidations",
      icon: (active) => (
        <svg
          className={`h-4 w-4 shrink-0 ${active ? "text-white" : "text-[#555555]"}`}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.75"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="m16 16 3-8 3 8c-.87.65-1.92 1-3 1s-2.13-.35-3-1Z" />
          <path d="m2 16 3-8 3 8c-.87.65-1.92 1-3 1s-2.13-.35-3-1Z" />
          <path d="M7 21h10" />
          <path d="M12 3v18" />
          <path d="M3 7h2c2 0 5-1 7-2 2 1 5 2 7 2h2" />
        </svg>
      ),
    },
  ];

  return (
    <aside
      className={`fixed lg:sticky top-0 left-0 z-50 h-dvh bg-[#F4F2EE] lg:bg-transparent border-r border-black/[0.06] flex flex-col justify-between py-5 px-3 select-none font-app-sans transition-all duration-200 ease-in-out ${
        isOpen
          ? "translate-x-0 shadow-2xl lg:shadow-none"
          : "-translate-x-full lg:translate-x-0"
      } ${isCollapsed ? "w-[68px]" : "w-[220px]"}`}
      style={{
        fontFamily:
          '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif',
      }}
    >
      <div>
        {/* Brand Header & Toggle Buttons */}
        <div className="px-2 pt-1 mb-5 flex items-center justify-between">
          <a
            href="/"
            className="group flex items-center gap-2.5 transition"
            title="Return to Walnut Home"
          >
            {isCollapsed ? (
              <WalnutLogo size={32} />
            ) : (
              <span className="group-hover:opacity-80 transition">
                <WalnutLockup size={30} />
              </span>
            )}
          </a>

          {/* Controls: Desktop Collapse Toggle & Mobile Close */}
          <div className="flex items-center gap-1">
            {onToggleCollapse && (
              <button
                type="button"
                onClick={onToggleCollapse}
                className="hidden lg:flex p-1.5 rounded-lg text-gray-400 hover:text-black hover:bg-black/5 transition cursor-pointer"
                title={isCollapsed ? "Expand sidebar" : "Collapse sidebar"}
              >
                <svg
                  className="w-4 h-4"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  {isCollapsed ? (
                    <path d="M13 17l5-5-5-5M6 17l5-5-5-5" />
                  ) : (
                    <path d="M11 17l-5-5 5-5M18 17l-5-5 5-5" />
                  )}
                </svg>
              </button>
            )}

            {onClose && (
              <button
                type="button"
                onClick={onClose}
                className="lg:hidden p-1.5 rounded-lg text-gray-500 hover:text-black hover:bg-black/5 transition cursor-pointer"
                title="Close navigation"
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
                  <line x1="18" y1="6" x2="6" y2="18" />
                  <line x1="6" y1="6" x2="18" y2="18" />
                </svg>
              </button>
            )}
          </div>
        </div>

        {/* Primary Navigation */}
        <nav className="space-y-0.5">
          {navItems.map((item) => {
            const isActive = normalizedActive === item.id;
            return (
              <button
                key={item.id}
                type="button"
                onClick={() => {
                  onSelectTab(item.id);
                  onClose?.();
                }}
                title={isCollapsed ? item.label : undefined}
                className={`w-full flex items-center gap-3 px-3 py-2 rounded-lg text-[13px] transition cursor-pointer text-left ${
                  isCollapsed ? "justify-center px-0" : ""
                } ${
                  isActive
                    ? "bg-[#111111] text-white font-medium shadow-[0_1px_2px_rgba(0,0,0,0.12)]"
                    : "text-[#4A4A4A] font-normal hover:text-black hover:bg-black/[0.03]"
                }`}
              >
                {item.icon(isActive)}
                {!isCollapsed && <span>{item.label}</span>}
              </button>
            );
          })}
        </nav>
      </div>

      {/* Bottom Section */}
      <div className="space-y-3 pt-3">
        {/* Secondary Links (Hidden when collapsed) */}
        {!isCollapsed && (
          <div className="space-y-0.5 px-0.5">
            <a
              href="https://github.com/Iam-jayant/walnut-solana"
              target="_blank"
              rel="noreferrer"
              className="flex items-center justify-between px-2.5 py-1.5 rounded-lg text-[12px] font-normal text-[#555555] hover:text-black hover:bg-black/[0.03] transition"
            >
              <span className="flex items-center gap-2">
                <svg
                  className="w-3.5 h-3.5 text-[#666666]"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.75"
                >
                  <circle cx="12" cy="12" r="10" />
                  <path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3" />
                  <line x1="12" y1="17" x2="12.01" y2="17" />
                </svg>
                Get help
              </span>
              <span className="text-[10px] text-gray-400">↗</span>
            </a>
          </div>
        )}

        {/* Connected Account / Wallet Button */}
        <div className="pt-2 border-t border-black/[0.04]">
          {connected && publicKey !== null ? (
            <div
              className={`flex items-center justify-between rounded-xl bg-black/[0.03] border border-black/[0.04] ${
                isCollapsed ? "p-1.5 justify-center" : "px-2.5 py-2"
              }`}
            >
              <div
                className="flex items-center gap-2 min-w-0"
                title={isCollapsed ? publicKey.toBase58() : undefined}
              >
                {/* Profile picture generated from the wallet address */}
                <WalletAvatar address={publicKey.toBase58()} size={32} />
                {!isCollapsed && (
                  <div className="min-w-0">
                    <p className="text-[12px] font-semibold text-black truncate leading-tight">
                      {shortAddress(publicKey.toBase58())}
                    </p>
                    <p className="text-[10px] text-emerald-600 font-medium leading-tight">
                      Connected
                    </p>
                  </div>
                )}
              </div>
              {!isCollapsed && (
                <button
                  type="button"
                  onClick={onDisconnect}
                  className="text-gray-400 hover:text-red-600 transition p-1 rounded-md hover:bg-black/5 cursor-pointer"
                  title="Disconnect wallet"
                >
                  <svg
                    className="h-3.5 w-3.5"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                  >
                    <path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4" />
                    <polyline points="16 17 21 12 16 7" />
                    <line x1="21" y1="12" x2="9" y2="12" />
                  </svg>
                </button>
              )}
            </div>
          ) : (
            <button
              type="button"
              onClick={onConnect}
              disabled={connecting}
              title={isCollapsed ? "Connect wallet" : undefined}
              className={`w-full flex items-center justify-center gap-2 rounded-lg bg-[#111111] hover:bg-[#2B2B2B] py-2.5 px-3 text-[13px] font-medium text-white transition cursor-pointer ${
                isCollapsed ? "px-0" : ""
              }`}
            >
              <svg
                className="w-3.5 h-3.5 text-white shrink-0"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.75"
              >
                <rect x="2" y="4" width="20" height="16" rx="2" />
                <path d="M7 15h.01" />
                <rect x="16" y="10" width="6" height="4" rx="1" />
              </svg>
              {!isCollapsed && (
                <span>{connecting ? "Connecting..." : "Connect wallet"}</span>
              )}
            </button>
          )}
        </div>
      </div>
    </aside>
  );
}
