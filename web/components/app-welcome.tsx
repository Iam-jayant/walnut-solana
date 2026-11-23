"use client";

import React from "react";

interface AppWelcomeProps {
  onConnect: () => void;
  connecting: boolean;
}

const APP_FONT_STYLE = {
  fontFamily:
    "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif",
};

/**
 * Authentic Walnut Protocol Token Icon (cWALNUT)
 * Detailed geometric Walnut shell emblem with golden confidential shield badge.
 */
function WalnutTokenIcon() {
  return (
    <div className="relative shrink-0">
      <div className="flex h-10 w-10 items-center justify-center rounded-full bg-[#FFD208] shadow-[0_2px_8px_rgba(255,210,8,0.3)]">
        <svg
          className="h-5.5 w-5.5 text-black"
          viewBox="0 0 24 24"
          fill="none"
        >
          {/* Walnut Shell Outer Silhouette */}
          <path
            d="M12 2.2C11.2 2.2 9.7 3.4 8.8 5C6.9 7.8 5.8 10.7 5.8 13.5C5.8 17.6 8 20.8 11.1 21.6C11.4 21.7 11.7 21.8 12 21.8C12.3 21.8 12.6 21.7 12.9 21.6C16 20.8 18.2 17.6 18.2 13.5C18.2 10.7 17.1 7.8 15.2 5C14.3 3.4 12.8 2.2 12 2.2Z"
            fill="#0A0A0A"
          />
          {/* Inner Golden Lobes */}
          <path
            d="M12 4.2V19.8C9.7 18.8 7.8 16.4 7.8 13.5C7.8 11 8.6 8.6 10.1 6.3C10.6 5.5 11.3 4.8 12 4.2Z"
            fill="#FFD208"
          />
          <path
            d="M12 4.2C12.7 4.8 13.4 5.5 13.9 6.3C15.4 8.6 16.2 11 16.2 13.5C16.2 16.4 14.3 18.8 12 19.8V4.2Z"
            fill="#FFC400"
          />
          {/* Center Dividing Ridge */}
          <path
            d="M12 3.2V20.8"
            stroke="#0A0A0A"
            strokeWidth="1.5"
            strokeLinecap="round"
          />
          {/* Sculpted Kernel Folds */}
          <path
            d="M9.9 9.8C9.2 10.8 8.8 12 8.8 13.5C8.8 14.9 9.3 16.2 10.2 17.1"
            stroke="#0A0A0A"
            strokeWidth="1.3"
            strokeLinecap="round"
          />
          <path
            d="M14.1 9.8C14.8 10.8 15.2 12 15.2 13.5C15.2 14.9 14.7 16.2 13.8 17.1"
            stroke="#0A0A0A"
            strokeWidth="1.3"
            strokeLinecap="round"
          />
        </svg>
      </div>
      {/* Signature Golden Shield Badge with checkmark */}
      <div className="absolute -bottom-0.5 -right-0.5 flex h-4.5 w-4.5 items-center justify-center rounded-full bg-white shadow-2xs">
        <svg className="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none">
          <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" fill="#FFD208" />
          <path
            d="M8.5 12l2.5 2.5 4.5-4.5"
            stroke="#000000"
            strokeWidth="2.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </div>
    </div>
  );
}

/**
 * Official Solana Token Icon (cSOL)
 * Solana Foundation vector logo with linear gradient and golden confidential shield badge.
 */
function SolanaTokenIcon() {
  return (
    <div className="relative shrink-0">
      <div className="flex h-10 w-10 items-center justify-center rounded-full bg-[#101217] shadow-[0_2px_8px_rgba(0,0,0,0.25)] border border-white/10">
        <svg
          className="h-4 w-5"
          viewBox="0 0 397 311"
          fill="none"
        >
          <linearGradient id="solGrad" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stopColor="#00FFA3" />
            <stop offset="100%" stopColor="#DC1FFF" />
          </linearGradient>
          <path
            fill="url(#solGrad)"
            d="M64.6 237.9c2.4-2.4 5.7-3.8 9.2-3.8h317.4c5.8 0 8.7 7 4.6 11.1l-62.7 62.7c-2.4 2.4-5.7 3.8-9.2 3.8H6.5c-5.8 0-8.7-7-4.6-11.1l62.7-62.7zM64.6 3.8C67.1 1.4 70.4 0 73.8 0h317.4c5.8 0 8.7 7 4.6 11.1l-62.7 62.7c-2.4 2.4-5.7 3.8-9.2 3.8H6.5c-5.8 0-8.7-7-4.6-11.1L64.6 3.8zM333.1 120.1c-2.4-2.4-5.7-3.8-9.2-3.8H6.5c-5.8 0-8.7 7-4.6 11.1l62.7 62.7c2.4 2.4 5.7 3.8 9.2 3.8h317.4c5.8 0 8.7-7 4.6-11.1l-62.7-62.7z"
          />
        </svg>
      </div>
      {/* Signature Golden Shield Badge with checkmark */}
      <div className="absolute -bottom-0.5 -right-0.5 flex h-4.5 w-4.5 items-center justify-center rounded-full bg-white shadow-2xs">
        <svg className="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none">
          <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" fill="#FFD208" />
          <path
            d="M8.5 12l2.5 2.5 4.5-4.5"
            stroke="#000000"
            strokeWidth="2.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </div>
    </div>
  );
}

/**
 * Official Centre USD Coin Token Icon (cUSDC)
 * Official USDC double-arc vector emblem with golden confidential shield badge.
 */
function UsdcTokenIcon() {
  return (
    <div className="relative shrink-0">
      <div className="flex h-10 w-10 items-center justify-center rounded-full bg-[#2775CA] shadow-[0_2px_8px_rgba(39,117,202,0.3)]">
        <svg
          className="h-5.5 w-5.5"
          viewBox="0 0 2000 2000"
          fill="none"
        >
          {/* Inner '$' glyph */}
          <path
            fill="#FFFFFF"
            d="M1250 1130c0-110-60-150-180-170-80-20-100-40-100-80 0-40 30-70 80-70 60 0 90 20 110 60l80-50c-30-60-80-90-150-100V600h-80v120c-90 10-150 70-150 150 0 100 60 140 180 170 80 20 100 50 100 80 0 50-40 80-90 80-70 0-110-30-130-80l-80 50c30 70 90 110 180 120v120h80v-120c90-10 150-70 150-160z"
          />
          {/* Outer Curved Parentheses Brackets */}
          <path
            fill="#FFFFFF"
            d="M760 1410c-230-110-350-360-310-610 30-220 200-400 420-450l20 90c-180 40-310 180-340 360-30 200 70 390 250 480l-40 130zm480 0l-40-130c180-90 280-280 250-480-30-180-160-320-340-360l20-90c220 50 390 230 420 450 40 250-80 500-310 610z"
          />
        </svg>
      </div>
      {/* Signature Golden Shield Badge with checkmark */}
      <div className="absolute -bottom-0.5 -right-0.5 flex h-4.5 w-4.5 items-center justify-center rounded-full bg-white shadow-2xs">
        <svg className="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none">
          <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" fill="#FFD208" />
          <path
            d="M8.5 12l2.5 2.5 4.5-4.5"
            stroke="#000000"
            strokeWidth="2.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </div>
    </div>
  );
}

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
              <WalnutTokenIcon />
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
              <SolanaTokenIcon />
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
              <UsdcTokenIcon />
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
