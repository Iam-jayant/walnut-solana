"use client";

import React from "react";
import { TokenLogo } from "./token-logo.tsx";
import { WalnutLogo } from "./brand-logo.tsx";

interface AppWelcomeProps {
  onConnect: () => void;
  connecting: boolean;
}

const APP_FONT_STYLE = {
  fontFamily:
    "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif",
};

const TOKENS = [
  { symbol: "cWALNUT", name: "Confidential Walnut" },
  { symbol: "cSOL", name: "Confidential Solana" },
  { symbol: "cUSDC", name: "Confidential USD Coin" },
] as const;

const FEATURES = [
  ["Encrypted balances", "Collateral and debt stay encrypted onchain."],
  ["Arcium MPC", "Risk checks run on secret shares, never in the clear."],
  ["Your Viewing Key", "Only you can decrypt your position."],
] as const;

export function AppWelcome({ onConnect, connecting }: AppWelcomeProps) {
  return (
    <div
      className="relative flex-1 w-full min-h-[600px] overflow-hidden rounded-[24px] md:rounded-[32px] bg-[#0D0D0D] text-white px-6 py-12 md:px-12 md:py-14 flex flex-col justify-between"
      style={APP_FONT_STYLE}
    >
      {/* Backdrop: dot grid + soft light + ghost mark */}
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 opacity-[0.35]"
        style={{
          backgroundImage:
            "radial-gradient(rgba(255,255,255,0.14) 1px, transparent 1px)",
          backgroundSize: "28px 28px",
          maskImage:
            "radial-gradient(ellipse 70% 70% at 50% 40%, black 30%, transparent 100%)",
          WebkitMaskImage:
            "radial-gradient(ellipse 70% 70% at 50% 40%, black 30%, transparent 100%)",
        }}
      />
      <div
        aria-hidden
        className="pointer-events-none absolute -top-40 left-1/2 h-[420px] w-[720px] -translate-x-1/2 rounded-full bg-white/[0.07] blur-[110px]"
      />
      <div
        aria-hidden
        className="pointer-events-none absolute -right-24 -bottom-24 hidden lg:block opacity-[0.07]"
      >
        <WalnutLogo size={480} tone="bone" />
      </div>

      <div className="relative z-10 flex flex-1 flex-col items-center justify-center text-center">
        <span className="inline-flex items-center gap-2 rounded-full border border-white/15 bg-white/[0.06] px-3 py-1.5 text-[12px] font-medium text-white/80 backdrop-blur">
          <WalnutLogo size={14} tone="bone" />
          Walnut Protocol · Solana Devnet
        </span>

        <h1
          className="mt-6 max-w-2xl text-[36px] sm:text-[46px] md:text-[56px] font-[500] tracking-[-0.035em] leading-[1.08]"
          style={APP_FONT_STYLE}
        >
          Borrow and earn.
          <br />
          <span className="text-white/55">Keep it confidential.</span>
        </h1>

        <p className="mt-5 max-w-md text-[14px] md:text-[15px] leading-[1.55] text-white/60">
          Shield the assets you already use. Balances and positions are
          encrypted onchain, and only you can read them.
        </p>

        <div className="mt-7 flex justify-center">
          <button
            type="button"
            onClick={onConnect}
            disabled={connecting}
            className="inline-flex items-center gap-2 rounded-[8px] bg-[#F4F2EE] hover:bg-white px-6 py-3 font-medium text-[14px] text-[#111111] shadow-[0_8px_30px_rgba(255,255,255,0.12)] transition active:scale-[0.99] cursor-pointer disabled:opacity-50"
          >
            <svg
              className="w-4 h-4"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.75"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <rect x="2" y="4" width="20" height="16" rx="2" />
              <path d="M7 15h.01" />
              <rect x="16" y="10" width="6" height="4" rx="1" />
            </svg>
            <span>{connecting ? "Connecting..." : "Connect wallet"}</span>
          </button>
        </div>

        {/* Portfolio preview */}
        <div className="mt-10 w-[400px] max-w-full rounded-[20px] border border-white/10 bg-white/[0.05] p-5 text-left shadow-[0_24px_60px_rgba(0,0,0,0.45)] backdrop-blur">
          <div className="flex items-center justify-between pb-1">
            <span className="text-[12px] text-white/55">Portfolio</span>
            <span className="inline-flex items-center gap-1.5 rounded-full bg-white/10 px-2 py-0.5 text-[10px] font-medium text-white/70">
              <span className="h-1.5 w-1.5 rounded-full bg-emerald-400" />
              Encrypted
            </span>
          </div>
          <div className="mt-3 space-y-3.5">
            {TOKENS.map((t) => (
              <div key={t.symbol} className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <TokenLogo symbol={t.symbol} size={40} />
                  <div>
                    <p className="text-[13px] font-semibold leading-tight text-white">
                      {t.symbol}
                    </p>
                    <p className="mt-0.5 text-[11px] leading-tight text-white/45">
                      {t.name}
                    </p>
                  </div>
                </div>
                <span className="h-2.5 w-16 rounded-full bg-white/15" aria-label="Hidden balance" />
              </div>
            ))}
          </div>
        </div>
      </div>

      <ul className="relative z-10 mt-10 grid gap-3 sm:grid-cols-3">
        {FEATURES.map(([title, text]) => (
          <li
            key={title}
            className="rounded-2xl border border-white/10 bg-white/[0.04] px-4 py-3.5"
          >
            <p className="text-[13px] font-semibold text-white">{title}</p>
            <p className="mt-1 text-[12px] leading-snug text-white/55">{text}</p>
          </li>
        ))}
      </ul>
    </div>
  );
}
