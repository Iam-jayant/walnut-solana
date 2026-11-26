"use client";

import React from "react";
import { Navbar } from "./navbar.tsx";
import { WalnutLogo } from "../brand-logo.tsx";

function ChevronDownSmall() {
  return (
    <svg className="w-3 h-3 ml-0.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M6 9l6 6 6-6" />
    </svg>
  );
}

function LockGlyph() {
  return (
    <svg className="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <rect x="5" y="11" width="14" height="9" rx="2" />
      <path d="M8 11V8a4 4 0 0 1 8 0v3" />
    </svg>
  );
}

function CheckGlyph() {
  return (
    <svg className="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M5 12.5l4.5 4.5L19 7.5" />
    </svg>
  );
}

/** A redacted value: shows that a number exists but cannot be read. */
function Redacted({ w }: { w: string }) {
  return (
    <span
      className={`inline-block h-3 rounded-full ${w}`}
      style={{
        background:
          "repeating-linear-gradient(90deg, rgba(255,255,255,0.28) 0 6px, rgba(255,255,255,0.10) 6px 10px)",
      }}
      aria-label="Encrypted value"
    />
  );
}

/** Illustration of a private position. No real numbers. */
function PositionCard() {
  return (
    <div className="relative w-[360px] rounded-[20px] border border-white/15 bg-white/[0.06] p-5 backdrop-blur-xl shadow-[0_30px_80px_-20px_rgba(0,0,0,0.7)]">
      <div className="flex items-center justify-between">
        <div className="text-[13px] font-medium text-white/80">Your position</div>
        <span className="inline-flex items-center gap-1.5 rounded-full border border-white/15 bg-white/[0.08] px-2.5 py-1 text-[11px] font-medium text-white/80">
          <LockGlyph /> Encrypted
        </span>
      </div>

      <dl className="mt-5 space-y-4">
        {[
          ["Collateral", "w-28"],
          ["Debt", "w-20"],
          ["Liquidation price", "w-24"],
        ].map(([k, w]) => (
          <div key={k} className="flex items-center justify-between">
            <dt className="text-[13px] text-white/55">{k}</dt>
            <dd>
              <Redacted w={w ?? "w-20"} />
            </dd>
          </div>
        ))}
      </dl>

      <div className="mt-5 flex items-center justify-between rounded-[14px] border border-white/10 bg-black/30 px-4 py-3">
        <div>
          <div className="text-[11px] text-white/50">Health check</div>
          <div className="text-[15px] font-medium text-white">Healthy</div>
        </div>
        <span className="flex h-8 w-8 items-center justify-center rounded-full bg-[#F4F2EE] text-[#111111]">
          <CheckGlyph />
        </span>
      </div>

      <div className="mt-4 text-[11px] leading-relaxed text-white/45">
        Computed in Arcium MPC. Only the result leaves the circuit, never your numbers.
      </div>
    </div>
  );
}

export function Hero() {
  return (
    <section className="bg-[#F5F3EF] pt-1 sm:pt-2 pb-6 sm:pb-10">
      <div className="w-full px-4 sm:px-8 lg:px-[70px]">
        <Navbar />
      </div>

      <div className="w-full px-4 sm:px-8 lg:px-[70px] mt-3 sm:mt-3.5">
        <div className="relative isolate overflow-hidden rounded-[22px] sm:rounded-[30px] bg-[#0D0D0D] min-h-[660px] sm:min-h-[800px] lg:min-h-[860px] flex flex-col justify-between ring-1 ring-black/20 shadow-[0_40px_90px_-40px_rgba(0,0,0,0.55)]">
          {/* Layer 1: fine dot grid, fading out toward the edges */}
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-0 z-0"
            style={{
              backgroundImage: "radial-gradient(rgba(255,255,255,0.13) 1px, transparent 1px)",
              backgroundSize: "26px 26px",
              WebkitMaskImage: "radial-gradient(ellipse 80% 70% at 62% 45%, #000 20%, transparent 78%)",
              maskImage: "radial-gradient(ellipse 80% 70% at 62% 45%, #000 20%, transparent 78%)",
            }}
          />
          {/* Layer 2: soft light from the top right */}
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-0 z-0"
            style={{
              background:
                "radial-gradient(ellipse 55% 55% at 82% 8%, rgba(244,242,238,0.16) 0%, transparent 70%), radial-gradient(ellipse 45% 45% at 0% 100%, rgba(140,136,128,0.18) 0%, transparent 70%)",
            }}
          />
          {/* Layer 3: concentric rings and a ghost coin, right side */}
          <svg
            aria-hidden="true"
            className="pointer-events-none absolute -right-40 top-1/2 z-0 hidden h-[900px] w-[900px] -translate-y-1/2 lg:block"
            viewBox="0 0 900 900"
            fill="none"
          >
            {[420, 330, 240].map((r, i) => (
              <circle key={r} cx="450" cy="450" r={r} stroke="rgba(255,255,255,0.08)" strokeWidth="1" strokeDasharray={i === 1 ? "2 8" : undefined} />
            ))}
          </svg>
          <div className="pointer-events-none absolute -right-24 top-1/2 z-0 hidden -translate-y-1/2 opacity-[0.10] lg:block">
            <WalnutLogo size={560} tone="bone" />
          </div>

          {/* Content */}
          <div className="relative z-10 grid grid-cols-1 lg:grid-cols-[minmax(0,1fr)_380px] gap-10 items-center px-5 sm:px-12 lg:px-16 pt-[110px] sm:pt-[170px] lg:pt-[190px] pb-6">
            <div className="max-w-3xl">
              <div className="mb-7 inline-flex items-center gap-2 rounded-full border border-white/15 bg-white/[0.06] px-3.5 py-1.5 text-[12px] font-medium text-white/80 backdrop-blur-sm">
                <WalnutLogo size={16} tone="bone" />
                Private lending on Solana
              </div>

              <h1
                className="text-[38px] min-[400px]:text-[46px] sm:text-6xl lg:text-[80px] xl:text-[96px] tracking-tight text-white mb-7 select-none"
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

              <p className="max-w-[560px] text-base sm:text-lg lg:text-xl font-normal leading-[1.55] text-white/65">
                The Walnut Protocol brings confidential lending, shielded balances, and private yield natively to Solana, powered by Arcium multi-party computation.
              </p>

              <div className="mt-8 flex flex-wrap items-center gap-3">
                <a
                  href="/app"
                  className="inline-flex items-center gap-2 rounded-[10px] bg-[#F4F2EE] px-6 py-3 text-sm font-semibold text-[#111111] shadow-sm transition hover:bg-white active:scale-[0.99]"
                >
                  Launch App
                  <span aria-hidden>→</span>
                </a>
                <a
                  href="https://github.com/Iam-jayant/walnut-solana"
                  target="_blank"
                  rel="noreferrer"
                  className="inline-flex items-center gap-2 rounded-[10px] border border-white/20 bg-white/[0.04] px-6 py-3 text-sm font-medium text-white backdrop-blur-sm transition hover:bg-white/[0.1] active:scale-[0.99]"
                >
                  View on GitHub
                  <span aria-hidden>↗</span>
                </a>
              </div>
            </div>

            <div className="hidden lg:flex justify-end">
              <PositionCard />
            </div>
          </div>

          {/* Bottom row */}
          <div className="relative z-10 flex flex-col sm:flex-row sm:items-end justify-between gap-4 px-5 sm:px-12 lg:px-16 pb-6 sm:pb-10">
            <div className="flex flex-wrap items-center gap-3.5 sm:gap-4">
              <div className="rounded-[12px] border border-white/12 bg-white/[0.05] backdrop-blur-md px-5 py-3 sm:px-6 sm:py-3.5 min-w-[150px] sm:min-w-[190px]">
                <div className="text-[11px] text-white/50 mb-0.5">Network</div>
                <div className="flex items-center gap-2 text-lg sm:text-xl font-medium text-white tracking-tight">
                  <span className="h-2 w-2 rounded-full bg-emerald-400 animate-pulse" />
                  Solana Devnet
                </div>
              </div>
              <div className="rounded-[12px] border border-white/12 bg-white/[0.05] backdrop-blur-md px-5 py-3 sm:px-6 sm:py-3.5 min-w-[150px] sm:min-w-[190px]">
                <div className="text-[11px] text-white/50 mb-0.5">Privacy engine</div>
                <div className="text-lg sm:text-xl font-medium text-white tracking-tight">Arcium MPC</div>
              </div>
            </div>

            <div>
              <a
                href="#products"
                className="inline-flex items-center gap-1.5 rounded-[8px] border border-white/15 bg-white/[0.05] hover:bg-white/[0.12] backdrop-blur-sm px-3.5 py-1.5 text-[11px] font-normal text-white/80 transition cursor-pointer select-none"
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
