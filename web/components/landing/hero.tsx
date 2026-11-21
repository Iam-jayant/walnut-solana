"use client";

import React from "react";
import { Navbar } from "./navbar.tsx";

function ChevronDownSmall() {
  return (
    <svg className="w-3 h-3 text-black/80 ml-0.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M6 9l6 6 6-6" />
    </svg>
  );
}

export function Hero() {
  return (
    <section className="bg-[#fbfbfb] pt-1 sm:pt-2 pb-6 sm:pb-10">
      {/* 1. Top Navbar: Minimal margin from top */}
      <div className="w-full px-4 sm:px-8 lg:px-[70px]">
        <Navbar />
      </div>

      {/* 2. Hero Section Rectangle: Zama warm luminous yellow atmosphere with desktop widescreen aspect ratio */}
      <div className="w-full px-4 sm:px-8 lg:px-[70px] mt-3 sm:mt-3.5">
        <div className="relative overflow-hidden rounded-[18px] sm:rounded-[22px] border border-black/[0.04] min-h-[780px] sm:min-h-[820px] lg:min-h-[860px] flex flex-col justify-between shadow-[0_16px_50px_-15px_rgba(250,222,138,0.22),inset_0_1px_2px_rgba(255,255,255,0.85)]">
          {/* Luminous, rich warm yellow atmospheric background matching Zama.org exact color sampling */}
          <div
            className="pointer-events-none absolute inset-0 z-0"
            style={{
              backgroundColor: "#FAE7A5",
              backgroundImage: `
                radial-gradient(ellipse 75% 60% at 58% 54%, #FFFEFA 0%, #FDF9EE 35%, rgba(253, 249, 238, 0.7) 55%, transparent 85%),
                linear-gradient(180deg, #F8DB72 0%, rgba(248, 219, 114, 0.8) 15%, rgba(248, 219, 114, 0.25) 35%, transparent 55%),
                radial-gradient(ellipse 55% 55% at 5% 95%, #F7D048 0%, rgba(247, 208, 72, 0.65) 45%, transparent 78%),
                radial-gradient(ellipse 50% 50% at 2% 5%, #F8D455 0%, rgba(248, 212, 85, 0.5) 40%, transparent 75%),
                radial-gradient(ellipse 50% 50% at 98% 8%, #F8D65A 0%, rgba(248, 214, 90, 0.5) 40%, transparent 72%),
                radial-gradient(ellipse 45% 45% at 95% 95%, #FCE7A5 0%, rgba(252, 231, 165, 0.5) 40%, transparent 75%),
                linear-gradient(135deg, rgba(255, 255, 255, 0.25) 0%, transparent 60%)
              `,
            }}
          />

          {/* Content inside Hero rectangle: Left-aligned with comfortable vertical offset matching Zama headroom */}
          <div className="relative z-10 max-w-3xl px-8 sm:px-12 lg:px-16 pt-[250px] sm:pt-[280px] lg:pt-[310px] pb-6">
            {/* Main Headline: Exactly two lines, elegant light grotesque font */}
            <h1
              className="text-5xl sm:text-6xl lg:text-[72px] xl:text-[76px] tracking-tight text-black mb-5 select-none"
              style={{
                fontFamily: "var(--font-telegraf), ui-sans-serif, system-ui, sans-serif",
                fontWeight: 200,
                lineHeight: 1.02,
                WebkitFontSmoothing: "antialiased",
                MozOsxFontSmoothing: "grayscale",
              }}
            >
              Confidential <br />
              Onchain Finance
            </h1>

            {/* Supporting Prose: Narrow 4-line block matching Zama layout */}
            <p className="max-w-[390px] text-xs sm:text-[13px] font-normal leading-[1.55] text-black/80">
              The Walnut Protocol brings confidential lending, shielded balances, and private yield natively to Solana, powered by Arcium multi-party computation.
            </p>
          </div>

          {/* Bottom Row: Frosted Glass Metric Cards on Left, Scroll to Explore Button on Right */}
          <div className="relative z-10 flex flex-col sm:flex-row sm:items-end justify-between gap-4 px-8 sm:px-12 lg:px-16 pb-8 sm:pb-10">
            {/* Left: The Two Frosted Glass Metric Cards */}
            <div className="flex flex-wrap items-center gap-3.5 sm:gap-4">
              {/* Shielded volume Card */}
              <div className="rounded-[10px] sm:rounded-[12px] bg-white/[0.18] backdrop-blur-md border border-white/35 px-5 py-3 sm:px-6 sm:py-3.5 min-w-[170px] sm:min-w-[190px] shadow-2xs">
                <div className="text-[11px] text-black/60 font-normal mb-0.5">
                  Shielded volume
                </div>
                <div className="text-xl sm:text-[22px] font-normal text-black font-sans tracking-tight">
                  $871,393,540
                </div>
              </div>

              {/* Shielded TVL Card */}
              <div className="rounded-[10px] sm:rounded-[12px] bg-white/[0.18] backdrop-blur-md border border-white/35 px-5 py-3 sm:px-6 sm:py-3.5 min-w-[170px] sm:min-w-[190px] shadow-2xs">
                <div className="text-[11px] text-black/60 font-normal mb-0.5">
                  Shielded TVL
                </div>
                <div className="text-xl sm:text-[22px] font-normal text-black font-sans tracking-tight">
                  $87,043,978
                </div>
              </div>
            </div>

            {/* Right: Scroll to explore Button */}
            <div>
              <a
                href="#products"
                className="inline-flex items-center gap-1.5 rounded-[6px] border border-black/[0.12] bg-[#FAF0CA]/50 hover:bg-[#FAF0CA]/80 backdrop-blur-xs px-3.5 py-1.5 text-[11px] font-normal text-black transition cursor-pointer shadow-2xs select-none"
              >
                <span>Scroll to explore</span>
                <ChevronDownSmall />
              </a>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}