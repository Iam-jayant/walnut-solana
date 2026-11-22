"use client";

import React from "react";

export function HttpsMoment() {
  return (
    <section id="learn-more-section" className="py-24 md:py-36 bg-[#fbfbfb] border-b border-black/[0.06]">
      <div className="mx-auto max-w-5xl px-6 text-center md:px-10">
        <div className="inline-block text-xs font-bold uppercase tracking-widest text-black/70 mb-4 bg-[#FFE052]/40 px-3 py-1 rounded-full border border-[#FFE052]/60">
          The Privacy Paradigm
        </div>
        <h2 className="text-3xl sm:text-5xl lg:text-6xl font-black tracking-tight text-black leading-tight mb-8">
          Walnut is the HTTPS moment for public blockchains
        </h2>
        <p className="mx-auto max-w-3xl text-lg sm:text-xl font-normal leading-relaxed text-black/70 mb-6">
          Just as HTTPS unlocked banking, commerce, and enterprise applications on the internet, the Walnut Protocol enables institutions to operate on public blockchains through encrypted, verifiable, and programmable onchain computation with Arcium MPC.
        </p>

        {/* 3D Flowing Shield Component matching Zama Section 2 */}
        <div className="relative mx-auto my-14 flex max-w-4xl items-center justify-between px-4 py-8">
          {/* Left Plaintext Tokens (Entering) */}
          <div className="flex items-center gap-3 sm:gap-6 opacity-75">
            <div className="flex h-12 w-12 sm:h-14 sm:w-14 items-center justify-center rounded-full bg-white shadow-md border border-black/[0.08] text-xs sm:text-sm font-bold text-black">
              SOL
            </div>
            <div className="flex h-12 w-12 sm:h-14 sm:w-14 items-center justify-center rounded-full bg-white shadow-md border border-black/[0.08] text-xs sm:text-sm font-bold text-black">
              USDC
            </div>
            <div className="hidden sm:flex h-14 w-14 items-center justify-center rounded-full bg-white shadow-md border border-black/[0.08] text-sm font-bold text-black">
              USD
            </div>
            {/* Animated flow dots */}
            <div className="hidden md:flex items-center gap-1.5 text-[#FFE052]">
              <span className="h-1.5 w-1.5 rounded-full bg-[#FFE052] animate-ping" />
              <span className="h-1.5 w-1.5 rounded-full bg-[#FFE052]" />
              <span className="h-1.5 w-1.5 rounded-full bg-[#FFE052]" />
            </div>
          </div>

          {/* Central 3D Frosted Glass Shield */}
          <div className="relative mx-4 flex h-48 w-44 sm:h-56 sm:w-52 items-center justify-center">
            {/* Ambient radial glow behind shield */}
            <div
              className="absolute inset-0 rounded-full blur-2xl opacity-40 pointer-events-none"
              style={{ background: "radial-gradient(circle, #FFE052 0%, transparent 70%)" }}
            />
            {/* 3D Shield Vector Geometry */}
            <svg
              viewBox="0 0 200 240"
              className="relative z-10 h-full w-full drop-shadow-[0_20px_35px_rgba(255,224,82,0.30)]"
              fill="none"
              xmlns="http://www.w3.org/2000/svg"
            >
              {/* Glass shield outer boundary */}
              <path
                d="M100 15L180 45V115C180 170 145 210 100 225C55 210 20 170 20 115V45L100 15Z"
                fill="url(#shield-grad)"
                stroke="rgba(255, 255, 255, 0.9)"
                strokeWidth="3"
              />
              {/* Inner facet lines for crystal 3D look */}
              <path
                d="M100 15V225M100 15L180 45M100 15L20 45"
                stroke="rgba(255, 255, 255, 0.5)"
                strokeWidth="1.5"
              />
              {/* Inner shield core */}
              <path
                d="M100 40L160 65V120C160 162 135 192 100 205C65 192 40 162 40 120V65L100 40Z"
                fill="rgba(255, 224, 82, 0.18)"
                stroke="rgba(255, 224, 82, 0.45)"
                strokeWidth="1.5"
              />
              {/* Central Walnut Brand Lock Symbol */}
              <circle cx="100" cy="115" r="24" fill="#FFE052" />
              <path
                d="M93 113V107C93 103.134 96.134 100 100 100C103.866 100 107 103.134 107 107V113M90 113H110V127H90V113Z"
                stroke="black"
                strokeWidth="2.5"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
              <defs>
                <linearGradient id="shield-grad" x1="20" y1="15" x2="180" y2="225" gradientUnits="userSpaceOnUse">
                  <stop stopColor="rgba(255, 255, 255, 0.90)" />
                  <stop offset="0.5" stopColor="rgba(255, 248, 220, 0.70)" />
                  <stop offset="1" stopColor="rgba(255, 224, 82, 0.35)" />
                </linearGradient>
              </defs>
            </svg>
          </div>

          {/* Right Encrypted Cipher Tokens (Output) */}
          <div className="flex items-center gap-3 sm:gap-6">
            {/* Animated flow dots */}
            <div className="hidden md:flex items-center gap-1.5 text-[#FFE052]">
              <span className="h-1.5 w-1.5 rounded-full bg-[#FFE052]" />
              <span className="h-1.5 w-1.5 rounded-full bg-[#FFE052]" />
              <span className="h-1.5 w-1.5 rounded-full bg-[#FFE052] animate-ping" />
            </div>
            <div className="relative flex h-12 w-12 sm:h-14 sm:w-14 items-center justify-center rounded-full bg-[#FFE052] text-black shadow-lg text-xs sm:text-sm font-bold border border-black/10">
              <span>0x7F</span>
              <span className="absolute -top-1 -right-1 flex h-4 w-4 items-center justify-center rounded-full bg-black text-[9px] text-white">🔒</span>
            </div>
            <div className="relative flex h-12 w-12 sm:h-14 sm:w-14 items-center justify-center rounded-full bg-[#FFE052] text-black shadow-lg text-xs sm:text-sm font-bold border border-black/10">
              <span>0x3A</span>
              <span className="absolute -top-1 -right-1 flex h-4 w-4 items-center justify-center rounded-full bg-black text-[9px] text-white">🔒</span>
            </div>
            <div className="relative hidden sm:flex h-14 w-14 items-center justify-center rounded-full bg-[#FFE052] text-black shadow-lg text-sm font-bold border border-black/10">
              <span>0xE2</span>
              <span className="absolute -top-1 -right-1 flex h-4 w-4 items-center justify-center rounded-full bg-black text-[9px] text-white">🔒</span>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
