"use client";

import React from "react";
import Link from "next/link";

const APP_HREF = "/app";

// Solana Official 3-Bar Logo
function SolanaLogo({ className = "w-6 h-6" }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 397 311" fill="currentColor">
      <path d="M64.6 237.9c2.4-2.4 5.7-3.8 9.2-3.8h317.4c5.8 0 8.7 7 4.6 11.1l-62.7 62.7c-2.4 2.4-5.7 3.8-9.2 3.8H6.5c-5.8 0-8.7-7-4.6-11.1l62.7-62.7zM64.6 3.8C67.1 1.4 70.4 0 73.8 0h317.4c5.8 0 8.7 7 4.6 11.1l-62.7 62.7c-2.4 2.4-5.7 3.8-9.2 3.8H6.5c-5.8 0-8.7-7-4.6-11.1L64.6 3.8zM332.4 120.9c-2.4-2.4-5.7-3.8-9.2-3.8H5.8c-5.8 0-8.7 7-4.6 11.1l62.7 62.7c2.4 2.4 5.7 3.8 9.2 3.8h317.4c5.8 0 8.7-7 4.6-11.1l-62.7-62.7z" />
    </svg>
  );
}

export function ArchitecturePillars() {
  return (
    <section id="solana-spotlight" className="py-10 sm:py-16 bg-[#F5F3EF]">
      {/* Standard 75px side margins */}
      <div className="w-full px-4 sm:px-8 lg:px-[75px]">
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-6 sm:gap-7 items-stretch">
          {/* Left Card: Luminous Warm Yellow Atmosphere with WALNUT | Solana Brand Lockup */}
          <div
            className="group relative overflow-hidden rounded-[18px] sm:rounded-[22px] min-h-[340px] sm:min-h-[380px] lg:min-h-[410px] flex items-center justify-center p-8 sm:p-12 border border-black/[0.04] shadow-xs transition-all duration-300 hover:shadow-md"
            style={{
              backgroundColor: "#F8F6F7",
              backgroundImage: `
                radial-gradient(ellipse 75% 75% at 0% 65%, #D8D4CB 0%, rgba(216, 212, 203, 0.45) 45%, transparent 75%),
                radial-gradient(circle 500px at 15% 95%, #D8D4CB 0%, transparent 60%),
                linear-gradient(135deg, rgba(255, 255, 255, 0.95) 0%, rgba(248, 246, 247, 0.7) 100%)
              `,
            }}
          >
            {/* Ambient diffuse sheen */}
            <div
              className="pointer-events-none absolute inset-0 opacity-60"
              style={{
                background: "radial-gradient(ellipse 60% 60% at 75% 30%, rgba(255, 255, 255, 0.9) 0%, transparent 70%)",
              }}
            />

            {/* Centered Brand Lockup */}
            <div className="relative z-10 flex items-center gap-5 sm:gap-6 select-none transition-transform duration-300 group-hover:scale-[1.02]">
              <span className="text-2xl sm:text-3xl font-black tracking-wider text-black font-sans">
                WALNUT
              </span>
              <div className="h-8 w-px bg-black/25" />
              <div className="flex items-center gap-3">
                <SolanaLogo className="w-6 h-5 text-black" />
                <span className="text-xl sm:text-2xl font-medium text-black tracking-tight font-sans">
                  Solana
                </span>
              </div>
            </div>
          </div>

          {/* Right Card: Warm Linen Stone (#F3EFEC) with Solana Lending Narrative & Action */}
          <div className="group rounded-[18px] sm:rounded-[22px] bg-[#F3EFEC] p-8 sm:p-10 lg:p-12 min-h-[340px] sm:min-h-[380px] lg:min-h-[410px] flex flex-col justify-between border border-black/[0.03] shadow-xs transition-all duration-300 hover:shadow-md">
            <div>
              <h3
                className="text-2xl sm:text-3xl lg:text-[34px] tracking-tight text-black mb-4 select-none"
                style={{
                  fontFamily: "var(--font-telegraf), ui-sans-serif, system-ui, sans-serif",
                  fontWeight: 200,
                  lineHeight: 1.15,
                }}
              >
                Solana – Confidentiality Meets <br />
                DeFi Lending
              </h3>
              <p className="text-xs sm:text-[13px] leading-relaxed text-black/75 max-w-lg mt-4">
                Walnut brings native confidential positions and private liquidity to Solana. Supply USDC to earn yield, post SOL collateral, and borrow with balances, health factors, and liquidation thresholds encrypted onchain via Arcium MPC.
              </p>
            </div>

            <div className="pt-8">
              <Link
                href={APP_HREF}
                className="inline-block px-6 py-2.5 rounded-[8px] bg-black text-white text-xs font-medium hover:bg-neutral-800 transition cursor-pointer shadow-xs active:scale-[0.99]"
              >
                Learn more
              </Link>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
