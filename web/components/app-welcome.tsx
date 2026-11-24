"use client";

import React from "react";
import { TokenLogo } from "./token-logo.tsx";

interface AppWelcomeProps {
  onConnect: () => void;
  connecting: boolean;
}

const APP_FONT_STYLE = {
  fontFamily:
    "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif",
};

export function AppWelcome({ onConnect, connecting }: AppWelcomeProps) {
  return (
    <div
      className="w-full h-full min-h-[580px] flex-1 flex flex-col justify-center items-center rounded-[24px] md:rounded-[32px] bg-white border border-black/[0.04] shadow-[0_4px_32px_rgba(0,0,0,0.03)] px-6 py-14 md:py-18 text-center relative overflow-hidden"
      style={APP_FONT_STYLE}
    >
      {/* Main Headline */}
      <h1
        className="text-[36px] sm:text-[42px] md:text-[48px] font-[450] tracking-[-0.03em] text-[#0A0A0A] leading-[1.12] max-w-xl mx-auto"
        style={APP_FONT_STYLE}
      >
        The easiest way to use
        <br />
        confidential tokens.
      </h1>

      {/* Subtitle */}
      <p
        className="mt-4 text-[14px] md:text-[15px] text-[#555555] max-w-md mx-auto leading-[1.5] font-normal"
        style={APP_FONT_STYLE}
      >
        Shield the assets you already use. Balances and transfers are
        <br className="hidden sm:inline" />
        {" "}encrypted onchain.
      </p>

      {/* Sunny Yellow CTA Button */}
      <div className="mt-6 flex justify-center">
        <button
          type="button"
          onClick={onConnect}
          disabled={connecting}
          className="inline-flex items-center gap-2 rounded-[6px] bg-[#FFD208] hover:bg-[#F5C800] px-5 py-2.5 font-medium text-[13px] text-black shadow-[0_1px_2px_rgba(0,0,0,0.05)] transition active:scale-[0.99] cursor-pointer disabled:opacity-50"
          style={APP_FONT_STYLE}
        >
          <svg
            className="w-4 h-4 text-black"
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

      {/* Floating Portfolio Card (Exact twin of reference screenshot) */}
      <div
        className="mt-12 w-[380px] max-w-full mx-auto rounded-[20px] bg-white border border-black/[0.06] shadow-[0_12px_36px_rgba(0,0,0,0.06)] p-5 text-left"
        style={APP_FONT_STYLE}
      >
        {/* Header Row */}
        <div className="flex items-center justify-between pb-1">
          <span className="text-[12px] font-normal text-[#666666]">Portfolio</span>
          <span className="text-[11px] font-bold text-[#777777] tracking-[2.5px]">
            ••••••
          </span>
        </div>

        {/* Token List: 100% Walnut Native Tokens with Realistic Logos */}
        <div className="mt-3.5 space-y-3.5">
          {/* cWALNUT */}
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <TokenLogo symbol="cWALNUT" size={40} />
              <div>
                <p className="text-[13px] font-semibold text-[#111111] leading-tight">
                  cWALNUT
                </p>
                <p className="text-[11px] text-[#888888] leading-tight mt-0.5">
                  Confidential Walnut
                </p>
              </div>
            </div>
            <span className="text-[11px] font-bold tracking-[2.5px] text-[#777777]">
              ••••••
            </span>
          </div>

          {/* cSOL */}
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <TokenLogo symbol="cSOL" size={40} />
              <div>
                <p className="text-[13px] font-semibold text-[#111111] leading-tight">
                  cSOL
                </p>
                <p className="text-[11px] text-[#888888] leading-tight mt-0.5">
                  Confidential Solana
                </p>
              </div>
            </div>
            <span className="text-[11px] font-bold tracking-[2.5px] text-[#777777]">
              ••••••
            </span>
          </div>

          {/* cUSDC */}
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <TokenLogo symbol="cUSDC" size={40} />
              <div>
                <p className="text-[13px] font-semibold text-[#111111] leading-tight">
                  cUSDC
                </p>
                <p className="text-[11px] text-[#888888] leading-tight mt-0.5">
                  Confidential USD Coin
                </p>
              </div>
            </div>
            <span className="text-[11px] font-bold tracking-[2.5px] text-[#777777]">
              ••••••
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}
