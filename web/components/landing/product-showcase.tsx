"use client";

import React, { useState } from "react";
import Link from "next/link";
import { TokenLogo, WalnutMark } from "../token-logo.tsx";

const APP_HREF = "/app";

// Real token logos shared with the rest of the app
function UsdcLogo({ className = "w-4 h-4" }: { className?: string }) {
  return <TokenLogo symbol="USDC" size={20} className={className} />;
}

function CUsdcLogo({ className = "w-4 h-4" }: { className?: string }) {
  return <TokenLogo symbol="cUSDC" size={20} className={className} />;
}

function CSolLogo({ className = "w-4 h-4" }: { className?: string }) {
  return <TokenLogo symbol="cSOL" size={20} className={className} />;
}

function WalnutEmblem({ className = "w-6 h-6" }: { className?: string }) {
  return (
    <span className={`inline-flex ${className}`}>
      <WalnutMark size={28} />
    </span>
  );
}

function ShieldBadgeIcon() {
  return (
    <svg className="w-3.5 h-3.5 text-black/80" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
    </svg>
  );
}

function CheckmarkIcon() {
  return (
    <svg className="w-3.5 h-3.5 text-emerald-600" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
      <polyline points="20 6 9 17 4 12" />
    </svg>
  );
}

export function ProductShowcase() {
  const [shieldedState, setShieldedState] = useState(false);
  const [depositedState, setDepositedState] = useState(false);
  const [swapRevealed, setSwapRevealed] = useState(false);

  return (
    <section id="products" className="py-10 sm:py-16 bg-[#fbfbfb]">
      {/* Outer Section Card: 75px side padding, soft rounded corners, warm yellow atmospheric glow on the right */}
      <div className="w-full px-4 sm:px-8 lg:px-[75px]">
        <div className="relative overflow-hidden rounded-[12px] sm:rounded-[14px] border border-black/[0.06] bg-[#fafafa] py-16 sm:py-20 lg:py-24 px-6 sm:px-10 lg:px-14 shadow-[0_4px_30px_rgba(0,0,0,0.02)]">
          {/* Subtle luminous warm yellow glow blooming from the top-right / right edge */}
          <div
            className="pointer-events-none absolute inset-0 z-0"
            style={{
              background: `
                radial-gradient(ellipse 65% 55% at 100% 10%, rgba(255, 224, 82, 0.52) 0%, rgba(255, 224, 82, 0.20) 45%, transparent 75%),
                radial-gradient(circle 650px at 100% 90%, rgba(255, 224, 82, 0.30) 0%, transparent 65%),
                radial-gradient(ellipse 55% 45% at 0% 0%, rgba(235, 240, 248, 0.5) 0%, transparent 60%),
                linear-gradient(180deg, rgba(255, 255, 255, 0.7) 0%, rgba(250, 250, 250, 0.98) 100%)
              `,
            }}
          />

          {/* Header: Centered, light grotesque typography */}
          <div className="relative z-10 text-center max-w-3xl mx-auto mb-12 sm:mb-16">
            <h2
              className="text-3xl sm:text-4xl lg:text-[46px] tracking-tight text-black mb-3 select-none"
              style={{
                fontFamily: "var(--font-telegraf), ui-sans-serif, system-ui, sans-serif",
                fontWeight: 200,
                lineHeight: 1.1,
              }}
            >
              Confidential onchain finance is live
            </h2>
            <p className="text-sm sm:text-base font-normal text-black/60 max-w-xl mx-auto">
              Shield, lend, earn, and swap confidentially on Solana.
            </p>
          </div>

          {/* 3 Interactive Mockup Cards Grid matching Zama Section with Hover Motion */}
          <div className="relative z-10 grid grid-cols-1 lg:grid-cols-3 gap-6 sm:gap-7">
            {/* Card 1: Shield */}
            <div className="group rounded-[10px] bg-white p-6 sm:p-7 border border-black/[0.04] shadow-[0_4px_24px_rgba(0,0,0,0.03)] flex flex-col justify-between transition-all duration-300 ease-out hover:-translate-y-2 hover:shadow-[0_20px_45px_-10px_rgba(0,0,0,0.10)] hover:border-black/[0.08] cursor-pointer">
              <div>
                {/* Embedded UI Mockup Box */}
                <div className="rounded-[8px] bg-[#EDEDEB] p-4 sm:p-5 mb-6 relative flex flex-col justify-between min-h-[305px] border border-black/[0.03]">
                  {/* Top input: You shield */}
                  <div className="rounded-[8px] bg-white p-4 shadow-[0_1px_3px_rgba(0,0,0,0.03)] border border-black/[0.02]">
                    <div className="flex items-center justify-between text-xs text-black/50 font-normal mb-1">
                      <span>You shield</span>
                      <div className="inline-flex items-center gap-1.5 rounded-full bg-gradient-to-b from-[#fcfcfc] to-[#e8e8e8] border border-[#d8d8d8] shadow-[0_1px_2px_rgba(0,0,0,0.06),inset_0_1px_0_rgba(255,255,255,0.9)] px-2.5 py-1 text-xs font-semibold text-black/85">
                        <UsdcLogo className="w-3.5 h-3.5" />
                        <span className="text-[11px]">USDC</span>
                      </div>
                    </div>
                    <div className="text-3xl sm:text-[34px] font-light tracking-tight text-black font-sans">
                      1,000
                    </div>
                  </div>

                  {/* Center connector badge */}
                  <div className="relative my-[-10px] z-10 flex justify-center items-center">
                    <div className="w-6 h-6 rounded-[6px] bg-[#FED022] border border-[#eec218] flex items-center justify-center shadow-xs">
                      <ShieldBadgeIcon />
                    </div>
                  </div>

                  {/* Bottom input: You receive */}
                  <div className="rounded-[8px] bg-white p-4 shadow-[0_1px_3px_rgba(0,0,0,0.03)] border border-black/[0.02]">
                    <div className="flex items-center justify-between text-xs text-black/50 font-normal mb-1">
                      <span>You receive</span>
                      <div className="inline-flex items-center gap-1.5 rounded-full bg-gradient-to-b from-[#fcfcfc] to-[#e8e8e8] border border-[#d8d8d8] shadow-[0_1px_2px_rgba(0,0,0,0.06),inset_0_1px_0_rgba(255,255,255,0.9)] px-2.5 py-1 text-xs font-semibold text-black/85">
                        <CUsdcLogo className="w-3.5 h-3.5" />
                        <span className="text-[11px]">cUSDC</span>
                      </div>
                    </div>
                    <div className="text-3xl sm:text-[34px] font-light tracking-tight text-black font-sans">
                      1,000
                    </div>
                  </div>

                  {/* Glossy Shiny Black Action Button */}
                  <button
                    type="button"
                    onClick={() => setShieldedState(!shieldedState)}
                    className="relative overflow-hidden mt-3 w-full h-[32px] rounded-[6px] text-white text-[11px] font-normal tracking-[0.14em] uppercase flex items-center justify-center transition-all duration-200 cursor-pointer select-none active:scale-[0.99] hover:brightness-110"
                    style={{
                      background: `
                        radial-gradient(ellipse 65% 100% at 50% 45%, rgba(255, 255, 255, 0.22) 0%, rgba(255, 255, 255, 0.05) 55%, transparent 85%),
                        linear-gradient(180deg, rgba(255, 255, 255, 0.08) 0%, rgba(0, 0, 0, 0.25) 100%),
                        linear-gradient(90deg, #050505 0%, #161616 16%, #282828 50%, #161616 84%, #050505 100%)
                      `,
                      boxShadow: "0 2px 6px rgba(0, 0, 0, 0.25), inset 0 1px 0.5px rgba(255, 255, 255, 0.45), inset 0 -1px 0.5px rgba(0, 0, 0, 0.5)",
                    }}
                  >
                    <span className="relative z-10 drop-shadow-[0_1px_1px_rgba(0,0,0,0.8)]">
                      {shieldedState ? "SHIELDED ✓" : "SHIELD"}
                    </span>
                  </button>
                </div>

                {/* Content below Mockup */}
                <Link
                  href={APP_HREF}
                  className="inline-flex items-center gap-1 text-[10.5px] font-medium tracking-[0.08em] uppercase text-[#BFA15C] mb-2 hover:text-[#9e8343] transition"
                >
                  <span>CONFIDENTIAL TOKENS</span>
                  <span className="inline-block transition-transform duration-200 group-hover:translate-x-1">→</span>
                </Link>
                <h3
                  className="text-2xl sm:text-[26px] font-light text-black mb-2 tracking-tight select-none"
                  style={{
                    fontFamily: "var(--font-telegraf), ui-sans-serif, system-ui, sans-serif",
                    fontWeight: 200,
                  }}
                >
                  Shield
                </h3>
                <p className="text-[13px] sm:text-[13.5px] leading-[1.55] text-[#555] font-normal">
                  Deposit USDC into the official cUSDC contract and receive cUSDC 1:1. Your balance is encrypted from that point on.
                </p>
              </div>
            </div>

            {/* Card 2: Earn */}
            <div className="group rounded-[10px] bg-white p-6 sm:p-7 border border-black/[0.04] shadow-[0_4px_24px_rgba(0,0,0,0.03)] flex flex-col justify-between transition-all duration-300 ease-out hover:-translate-y-2 hover:shadow-[0_20px_45px_-10px_rgba(0,0,0,0.10)] hover:border-black/[0.08] cursor-pointer">
              <div>
                {/* Embedded UI Mockup Box */}
                <div className="rounded-[8px] bg-[#EDEDEB] p-4 sm:p-5 mb-6 relative flex flex-col justify-between min-h-[305px] border border-black/[0.03]">
                  {/* Amount card */}
                  <div className="rounded-[8px] bg-white p-4 shadow-[0_1px_3px_rgba(0,0,0,0.03)] border border-black/[0.02]">
                    <div className="flex items-center justify-between text-xs text-black/50 font-normal mb-1">
                      <span>Amount</span>
                      <div className="inline-flex items-center gap-1.5 rounded-full bg-gradient-to-b from-[#fcfcfc] to-[#e8e8e8] border border-[#d8d8d8] shadow-[0_1px_2px_rgba(0,0,0,0.06),inset_0_1px_0_rgba(255,255,255,0.9)] px-2.5 py-1 text-xs font-semibold text-black/85">
                        <CUsdcLogo className="w-3.5 h-3.5" />
                        <span className="text-[11px]">cUSDC</span>
                      </div>
                    </div>
                    <div className="text-3xl sm:text-[34px] font-light tracking-tight text-black font-sans">
                      261,131
                    </div>
                    <div className="text-[11px] text-black/40 font-mono mt-1">
                      Wallet balance: ****** cUSDC
                    </div>
                  </div>

                  {/* Vault badge card */}
                  <div className="rounded-[8px] bg-white p-3 shadow-[0_1px_3px_rgba(0,0,0,0.03)] border border-black/[0.02] flex items-center gap-2.5 mt-2.5">
                    <WalnutEmblem className="w-7 h-7 shrink-0" />
                    <div>
                      <div className="text-xs font-semibold text-black leading-tight">
                        Walnut Confidential Prime USDC
                      </div>
                      <div className="text-[10px] text-black/50">
                        Curated by Walnut Protocol
                      </div>
                    </div>
                  </div>

                  {/* Glossy Shiny Black Action Button */}
                  <button
                    type="button"
                    onClick={() => setDepositedState(!depositedState)}
                    className="relative overflow-hidden mt-3 w-full h-[32px] rounded-[6px] text-white text-[11px] font-normal tracking-[0.14em] uppercase flex items-center justify-center transition-all duration-200 cursor-pointer select-none active:scale-[0.99] hover:brightness-110"
                    style={{
                      background: `
                        radial-gradient(ellipse 65% 100% at 50% 45%, rgba(255, 255, 255, 0.22) 0%, rgba(255, 255, 255, 0.05) 55%, transparent 85%),
                        linear-gradient(180deg, rgba(255, 255, 255, 0.08) 0%, rgba(0, 0, 0, 0.25) 100%),
                        linear-gradient(90deg, #050505 0%, #161616 16%, #282828 50%, #161616 84%, #050505 100%)
                      `,
                      boxShadow: "0 2px 6px rgba(0, 0, 0, 0.25), inset 0 1px 0.5px rgba(255, 255, 255, 0.45), inset 0 -1px 0.5px rgba(0, 0, 0, 0.5)",
                    }}
                  >
                    <span className="relative z-10 drop-shadow-[0_1px_1px_rgba(0,0,0,0.8)]">
                      {depositedState ? "DEPOSITED ✓" : "DEPOSIT"}
                    </span>
                  </button>
                </div>

                {/* Content below Mockup */}
                <Link
                  href={APP_HREF}
                  className="inline-flex items-center gap-1 text-[10.5px] font-medium tracking-[0.08em] uppercase text-[#BFA15C] mb-2 hover:text-[#9e8343] transition"
                >
                  <span>CONFIDENTIAL VAULTS</span>
                  <span className="inline-block transition-transform duration-200 group-hover:translate-x-1">→</span>
                </Link>
                <h3
                  className="text-2xl sm:text-[26px] font-light text-black mb-2 tracking-tight select-none"
                  style={{
                    fontFamily: "var(--font-telegraf), ui-sans-serif, system-ui, sans-serif",
                    fontWeight: 200,
                  }}
                >
                  Earn
                </h3>
                <p className="text-[13px] sm:text-[13.5px] leading-[1.55] text-[#555] font-normal">
                  Earn onchain yield while keeping your position confidential.
                </p>
              </div>
            </div>

            {/* Card 3: Swap */}
            <div className="group rounded-[10px] bg-white p-6 sm:p-7 border border-black/[0.04] shadow-[0_4px_24px_rgba(0,0,0,0.03)] flex flex-col justify-between transition-all duration-300 ease-out hover:-translate-y-2 hover:shadow-[0_20px_45px_-10px_rgba(0,0,0,0.10)] hover:border-black/[0.08] cursor-pointer">
              <div>
                {/* Embedded UI Mockup Box */}
                <div className="rounded-[8px] bg-[#EDEDEB] p-4 sm:p-5 mb-6 relative flex flex-col justify-between min-h-[305px] border border-black/[0.03]">
                  {/* Centered Swap Settled Status Box */}
                  <div className="flex-1 rounded-[8px] bg-white p-5 sm:p-6 shadow-[0_1px_3px_rgba(0,0,0,0.03)] border border-black/[0.02] flex flex-col items-center justify-center text-center">
                    {/* Status check icon */}
                    <div className="w-6 h-6 rounded-full border border-emerald-500/80 bg-emerald-50/50 flex items-center justify-center mb-2.5">
                      <CheckmarkIcon />
                    </div>

                    {/* Status Title */}
                    <div className="text-[15px] sm:text-base font-normal text-black mb-2.5">
                      Swap settled
                    </div>

                    {/* Pair line with bidirectional arrows */}
                    <div className="flex items-center justify-center gap-2 text-xs font-medium text-black/75 mb-2">
                      <div className="inline-flex items-center gap-1">
                        <CUsdcLogo className="w-3.5 h-3.5" />
                        <span className="text-[11.5px]">cUSDC</span>
                      </div>
                      <span className="text-black/40 text-xs">⇄</span>
                      <div className="inline-flex items-center gap-1">
                        <CSolLogo className="w-3.5 h-3.5" />
                        <span className="text-[11.5px]">cSOL</span>
                      </div>
                    </div>

                    {/* Concealed / Revealed amount */}
                    <div className="font-mono text-sm tracking-[0.25em] text-black/50 mt-2">
                      {swapRevealed ? "$14,250.00" : "******"}
                    </div>
                  </div>

                  {/* Glossy Shiny Black Action Button */}
                  <button
                    type="button"
                    onClick={() => setSwapRevealed(!swapRevealed)}
                    className="relative overflow-hidden mt-3 w-full h-[32px] rounded-[6px] text-white text-[11px] font-normal tracking-[0.14em] uppercase flex items-center justify-center transition-all duration-200 cursor-pointer select-none active:scale-[0.99] hover:brightness-110"
                    style={{
                      background: `
                        radial-gradient(ellipse 65% 100% at 50% 45%, rgba(255, 255, 255, 0.22) 0%, rgba(255, 255, 255, 0.05) 55%, transparent 85%),
                        linear-gradient(180deg, rgba(255, 255, 255, 0.08) 0%, rgba(0, 0, 0, 0.25) 100%),
                        linear-gradient(90deg, #050505 0%, #161616 16%, #282828 50%, #161616 84%, #050505 100%)
                      `,
                      boxShadow: "0 2px 6px rgba(0, 0, 0, 0.25), inset 0 1px 0.5px rgba(255, 255, 255, 0.45), inset 0 -1px 0.5px rgba(0, 0, 0, 0.5)",
                    }}
                  >
                    <span className="relative z-10 drop-shadow-[0_1px_1px_rgba(0,0,0,0.8)]">
                      {swapRevealed ? "CONCEAL" : "REVEAL"}
                    </span>
                  </button>
                </div>

                {/* Content below Mockup */}
                <Link
                  href={APP_HREF}
                  className="inline-flex items-center gap-1 text-[10.5px] font-medium tracking-[0.08em] uppercase text-[#BFA15C] mb-2 hover:text-[#9e8343] transition"
                >
                  <span>CONFIDENTIAL SWAPS</span>
                  <span className="inline-block transition-transform duration-200 group-hover:translate-x-1">→</span>
                </Link>
                <h3
                  className="text-2xl sm:text-[26px] font-light text-black mb-2 tracking-tight select-none"
                  style={{
                    fontFamily: "var(--font-telegraf), ui-sans-serif, system-ui, sans-serif",
                    fontWeight: 200,
                  }}
                >
                  Swap
                </h3>
                <p className="text-[13px] sm:text-[13.5px] leading-[1.55] text-[#555] font-normal">
                  Trade confidentially at any size, with your amount, direction, and price limit encrypted.
                </p>
              </div>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}