"use client";

import React, { useState } from "react";
import Link from "next/link";

const APP_HREF = "/app";
const AUDIT_HREF = "/app/audit";

// --- SVG Icons (Pixel-accurate to Zama.org) ---
function ArrowRight({ className = "w-4 h-4" }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
      <path d="M5 12H19M19 12L12 5M19 12L12 19" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function ArrowUpRight({ className = "w-3.5 h-3.5" }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
      <path d="M7 17L17 7M17 7H7M17 7V17" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function ChevronDown({ className = "w-4 h-4" }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
      <path d="M7.5 10.5L12 15L16.5 10.5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function ShieldIcon() {
  return (
    <svg width="40" height="40" viewBox="0 0 40 40" fill="none" xmlns="http://www.w3.org/2000/svg">
      <rect width="40" height="40" rx="8" fill="#FFD209" fillOpacity="0.2" />
      <path d="M20 10L12 14V21C12 26.5 15.4 31.6 20 33C24.6 31.6 28 26.5 28 21V14L20 10Z" stroke="#000000" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M17 20L19.5 22.5L24 17.5" stroke="#000000" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function VaultIcon() {
  return (
    <svg width="40" height="40" viewBox="0 0 40 40" fill="none" xmlns="http://www.w3.org/2000/svg">
      <rect width="40" height="40" rx="8" fill="#FFD209" fillOpacity="0.2" />
      <rect x="11" y="13" width="18" height="15" rx="2" stroke="#000000" strokeWidth="2" />
      <circle cx="20" cy="20.5" r="3.5" stroke="#000000" strokeWidth="2" />
      <path d="M20 18V19.5" stroke="#000000" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );
}

function SwapIcon() {
  return (
    <svg width="40" height="40" viewBox="0 0 40 40" fill="none" xmlns="http://www.w3.org/2000/svg">
      <rect width="40" height="40" rx="8" fill="#FFD209" fillOpacity="0.2" />
      <path d="M12 16H27M27 16L23 12M27 16L23 20" stroke="#000000" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M28 24H13M13 24L17 20M13 24L17 28" stroke="#000000" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function LandmarkIcon() {
  return (
    <svg className="w-8 h-8 text-black" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.75">
      <path d="M3 21h18M3 10h18M5 10v11M9 10v11M15 10v11M19 10v11M12 3L2 10h20L12 3z" />
    </svg>
  );
}

function CreditCardIcon() {
  return (
    <svg className="w-8 h-8 text-black" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.75">
      <rect x="2" y="5" width="20" height="14" rx="2" />
      <path d="M2 10h20M7 15h.01M11 15h2" />
    </svg>
  );
}

function ChartIcon() {
  return (
    <svg className="w-8 h-8 text-black" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.75">
      <path d="M3 3v18h18" />
      <path d="M7 13l4-4 4 4 6-6" />
      <circle cx="7" cy="13" r="1" fill="currentColor" />
      <circle cx="11" cy="9" r="1" fill="currentColor" />
      <circle cx="15" cy="13" r="1" fill="currentColor" />
      <circle cx="21" cy="7" r="1" fill="currentColor" />
    </svg>
  );
}

function CoinsIcon() {
  return (
    <svg className="w-8 h-8 text-black" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.75">
      <circle cx="9" cy="9" r="6" />
      <path d="M14.5 9a6 6 0 0 1 4.5 5.8 6 6 0 0 1-5.8 4.2H9" />
    </svg>
  );
}

// --- Logo matching Zama geometric branding ---
function ZamaLogo() {
  return (
    <div className="flex items-center gap-3">
      <div className="relative flex h-8 w-8 items-center justify-center rounded-lg bg-[#ffd208] shadow-sm">
        <span className="text-lg font-black tracking-tighter text-black">W</span>
        <div className="absolute -top-1 -right-1 h-2 w-2 rounded-full bg-black animate-pulse" />
      </div>
      <span className="text-xl font-bold tracking-tight text-black">WALNUT</span>
    </div>
  );
}

export function LandingPage() {
  const [activeTab, setActiveTab] = useState<"ts" | "sol">("ts");
  const [dropdownOpen, setDropdownOpen] = useState<string | null>(null);

  const toggleDropdown = (name: string) => {
    setDropdownOpen(dropdownOpen === name ? null : name);
  };

  return (
    <div className="min-h-screen bg-[#f1f1ef] text-[#111111] font-sans antialiased selection:bg-[#ffd208] selection:text-black">
      {/* ------------------------------------------------------------- */}
      {/* GLOBAL NAVIGATION HEADER                                      */}
      {/* ------------------------------------------------------------- */}
      <header className="sticky top-0 z-50 border-b border-black/[0.06] bg-[#f1f1ef]/85 backdrop-blur-md transition-all">
        <div className="mx-auto flex max-w-7xl items-center justify-between px-6 py-4 md:px-10">
          <div className="flex items-center gap-10">
            <Link href="/" aria-label="Walnut home">
              <ZamaLogo />
            </Link>

            {/* Desktop Nav Items with Dropdowns */}
            <nav className="hidden md:flex items-center gap-7 text-[14px] font-medium text-black/80">
              {/* Products Dropdown */}
              <div className="relative">
                <button
                  type="button"
                  onClick={() => toggleDropdown("products")}
                  className="flex items-center gap-1.5 hover:text-black transition py-1 cursor-pointer"
                >
                  Products <ChevronDown className="w-3.5 h-3.5 text-black/60" />
                </button>
                {dropdownOpen === "products" && (
                  <div className="absolute top-full left-0 mt-3 w-80 rounded-2xl bg-white p-5 shadow-2xl border border-black/[0.08] grid gap-4 z-50">
                    <div>
                      <div className="text-[11px] font-bold uppercase tracking-wider text-black/40 mb-2">Walnut App</div>
                      <div className="grid gap-2">
                        <Link href={APP_HREF} className="flex items-center justify-between text-sm font-semibold hover:text-[#d4af00] transition">
                          Shield (cUSDC) <ArrowUpRight />
                        </Link>
                        <Link href={APP_HREF} className="flex items-center justify-between text-sm font-semibold hover:text-[#d4af00] transition">
                          Confidential Lend <ArrowUpRight />
                        </Link>
                        <Link href={APP_HREF} className="flex items-center justify-between text-sm font-semibold hover:text-[#d4af00] transition">
                          Confidential Borrow <ArrowUpRight />
                        </Link>
                      </div>
                    </div>
                    <div className="border-t border-black/[0.06] pt-3">
                      <div className="text-[11px] font-bold uppercase tracking-wider text-black/40 mb-2">Onchain Infra</div>
                      <Link href={AUDIT_HREF} className="flex items-center justify-between text-sm font-semibold hover:text-[#d4af00] transition">
                        Zero-Knowledge Audit <ArrowUpRight />
                      </Link>
                    </div>
                  </div>
                )}
              </div>

              {/* Solutions Dropdown */}
              <div className="relative">
                <button
                  type="button"
                  onClick={() => toggleDropdown("solutions")}
                  className="flex items-center gap-1.5 hover:text-black transition py-1 cursor-pointer"
                >
                  Solutions <ChevronDown className="w-3.5 h-3.5 text-black/60" />
                </button>
                {dropdownOpen === "solutions" && (
                  <div className="absolute top-full left-0 mt-3 w-72 rounded-2xl bg-white p-5 shadow-2xl border border-black/[0.08] grid gap-2 z-50">
                    <a href="#industries" className="text-sm font-semibold hover:text-[#d4af00] transition">Confidential RWA</a>
                    <a href="#industries" className="text-sm font-semibold hover:text-[#d4af00] transition">Confidential Payments</a>
                    <a href="#industries" className="text-sm font-semibold hover:text-[#d4af00] transition">Institutional DeFi</a>
                    <a href="#industries" className="text-sm font-semibold hover:text-[#d4af00] transition">Selective Viewing Keys</a>
                  </div>
                )}
              </div>

              <a href="#developers" className="hover:text-black transition">Developers</a>
              <a href="#ecosystem" className="hover:text-black transition">Ecosystem</a>
              <Link href={AUDIT_HREF} className="hover:text-black transition">Audit ↗</Link>
            </nav>
          </div>

          {/* Right Action Buttons */}
          <div className="flex items-center gap-3">
            <Link
              href={AUDIT_HREF}
              className="hidden sm:inline-flex text-[14px] font-medium text-black/80 hover:text-black transition px-3 py-2"
            >
              Verify Audit
            </Link>
            <Link
              href={APP_HREF}
              className="inline-flex items-center justify-center gap-2 rounded-full bg-black px-5 py-2.5 text-[14px] font-semibold text-white transition hover:bg-[#222222] hover:shadow-lg cursor-pointer"
            >
              Launch App
              <ArrowUpRight className="w-4 h-4 text-[#ffd208]" />
            </Link>
          </div>
        </div>
      </header>

      {/* ------------------------------------------------------------- */}
      {/* HERO SECTION (Zama Signature Ambient Gradient & Typography)    */}
      {/* ------------------------------------------------------------- */}
      <section className="relative overflow-hidden pt-20 pb-28 md:pt-32 md:pb-40">
        {/* Zama's exact radial glow fallback gradient */}
        <div
          className="pointer-events-none absolute inset-0 z-0 opacity-90"
          style={{
            background:
              "radial-gradient(60% 70% at 10% 90%, rgba(254, 210, 8, 0.8) 0%, rgba(254, 210, 8, 0) 60%), radial-gradient(60% 70% at 90% 95%, rgba(255, 181, 116, 0.6) 0%, rgba(255, 181, 116, 0) 60%), radial-gradient(70% 70% at 50% 30%, #f1f1ef 0%, transparent 80%), #f1f1ef",
          }}
        />

        <div className="relative z-10 mx-auto max-w-7xl px-6 md:px-10">
          <div className="max-w-4xl">
            {/* Tag / Badge */}
            <div className="inline-flex items-center gap-2 rounded-full bg-black/[0.04] px-4 py-1.5 text-xs font-bold uppercase tracking-wider text-black/80 mb-6 border border-black/[0.06]">
              <span className="h-1.5 w-1.5 rounded-full bg-[#ffd208]" />
              Confidential Onchain Finance on Solana
            </div>

            {/* Main Display Headline */}
            <h1 className="text-5xl font-black tracking-tight text-black sm:text-7xl lg:text-8xl leading-[0.98] mb-8">
              Confidential <br />
              Onchain Finance
            </h1>

            {/* Sub-headline */}
            <p className="max-w-xl text-lg sm:text-xl font-normal leading-relaxed text-black/75 mb-10">
              The Walnut Protocol enables confidential assets to be issued, traded, and lent natively on Solana, with full verifiability and programmable compliance using Arcium MPC & RescueCipher FHE.
            </p>

            {/* Hero CTAs */}
            <div className="flex flex-wrap items-center gap-4 mb-20">
              <Link
                href={APP_HREF}
                className="inline-flex items-center justify-center gap-2 rounded-full bg-black px-7 py-3.5 text-base font-semibold text-white transition hover:bg-[#222222] hover:scale-[1.02] active:scale-[0.98]"
              >
                Launch Protocol
                <ArrowUpRight className="w-4 h-4 text-[#ffd208]" />
              </Link>
              <a
                href="#learn-more-section"
                className="inline-flex items-center gap-2 rounded-full bg-white/70 backdrop-blur px-6 py-3.5 text-sm font-semibold text-black border border-black/10 hover:bg-white transition"
              >
                Scroll to explore
                <span className="text-xs">↓</span>
              </a>
            </div>
          </div>

          {/* Zama Live Telemetry / KPI Strip */}
          <div className="border-t border-black/10 pt-8 grid grid-cols-2 md:grid-cols-4 gap-6">
            <div>
              <div className="text-xs font-semibold uppercase tracking-wider text-black/50 mb-1">Shielded Volume</div>
              <div className="text-2xl sm:text-3xl font-black tracking-tight text-black">$18,452,900</div>
            </div>
            <div>
              <div className="text-xs font-semibold uppercase tracking-wider text-black/50 mb-1">Shielded TVL</div>
              <div className="text-2xl sm:text-3xl font-black tracking-tight text-black">$6,218,400</div>
            </div>
            <div>
              <div className="text-xs font-semibold uppercase tracking-wider text-black/50 mb-1">MPC Cluster Finality</div>
              <div className="text-2xl sm:text-3xl font-black tracking-tight text-black">&lt; 400 ms</div>
            </div>
            <div>
              <div className="text-xs font-semibold uppercase tracking-wider text-black/50 mb-1">Onchain Leakage</div>
              <div className="text-2xl sm:text-3xl font-black tracking-tight text-black">0 Bytes (100% Ciphertext)</div>
            </div>
          </div>
        </div>
      </section>

      {/* ------------------------------------------------------------- */}
      {/* THE "HTTPS MOMENT" NARRATIVE SECTION                           */}
      {/* ------------------------------------------------------------- */}
      <section id="learn-more-section" className="py-24 md:py-36 bg-[#f1f1ef] border-t border-black/[0.06]">
        <div className="mx-auto max-w-5xl px-6 text-center md:px-10">
          <div className="inline-block text-xs font-bold uppercase tracking-widest text-black/40 mb-4">
            The Privacy Paradigm
          </div>
          <h2 className="text-3xl sm:text-5xl lg:text-6xl font-black tracking-tight text-black leading-tight mb-8">
            Walnut is the HTTPS moment for public blockchains
          </h2>
          <p className="mx-auto max-w-3xl text-lg sm:text-xl font-normal leading-relaxed text-black/70">
            Just as HTTPS unlocked banking, commerce, and enterprise applications on the internet, the Walnut Protocol enables institutions to operate on public blockchains through encrypted, verifiable, and programmable onchain computation using Fully Homomorphic Encryption (FHE) and Arcium MPC.
          </p>
        </div>
      </section>

      {/* ------------------------------------------------------------- */}
      {/* "CONFIDENTIAL ONCHAIN FINANCE IS LIVE" (Zama 3 Feature Cards) */}
      {/* ------------------------------------------------------------- */}
      <section className="py-20 md:py-28 bg-[#f1f1ef]">
        <div className="mx-auto max-w-7xl px-6 md:px-10">
          <div className="mb-12">
            <h2 className="text-3xl sm:text-4xl font-black tracking-tight text-black mb-3">
              Confidential onchain finance is live
            </h2>
            <p className="text-base sm:text-lg text-black/60">
              Hold, transfer, earn, and trade confidentially on public blockchains.
            </p>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
            {/* Card 1: Shield */}
            <div className="group relative rounded-2xl bg-white/70 backdrop-blur-md p-8 border border-white shadow-sm transition hover:shadow-xl hover:-translate-y-1">
              <div className="mb-6">
                <ShieldIcon />
              </div>
              <div className="text-[11px] font-bold uppercase tracking-widest text-black/50 mb-2 flex items-center gap-1">
                CONFIDENTIAL COLLATERAL <span className="group-hover:translate-x-1 transition">→</span>
              </div>
              <h3 className="text-2xl font-black text-black mb-3">Shield</h3>
              <p className="text-sm leading-relaxed text-black/70">
                Deposit SOL or USDC into the official confidential vault and receive encrypted tokens 1:1. Your balance and transactions are encrypted from that point on.
              </p>
            </div>

            {/* Card 2: Earn */}
            <div className="group relative rounded-2xl bg-white/70 backdrop-blur-md p-8 border border-white shadow-sm transition hover:shadow-xl hover:-translate-y-1">
              <div className="mb-6">
                <VaultIcon />
              </div>
              <div className="text-[11px] font-bold uppercase tracking-widest text-black/50 mb-2 flex items-center gap-1">
                CONFIDENTIAL LENDING <span className="group-hover:translate-x-1 transition">→</span>
              </div>
              <h3 className="text-2xl font-black text-black mb-3">Earn</h3>
              <p className="text-sm leading-relaxed text-black/70">
                Supply liquidity into the lending pool to earn onchain yield, while keeping deposit amounts, interest accrual, and position size strictly confidential.
              </p>
            </div>

            {/* Card 3: Borrow */}
            <div className="group relative rounded-2xl bg-white/70 backdrop-blur-md p-8 border border-white shadow-sm transition hover:shadow-xl hover:-translate-y-1">
              <div className="mb-6">
                <SwapIcon />
              </div>
              <div className="text-[11px] font-bold uppercase tracking-widest text-black/50 mb-2 flex items-center gap-1">
                CONFIDENTIAL BORROWING <span className="group-hover:translate-x-1 transition">→</span>
              </div>
              <h3 className="text-2xl font-black text-black mb-3">Borrow</h3>
              <p className="text-sm leading-relaxed text-black/70">
                Borrow liquidity against encrypted collateral with zero public liquidation hunting. Health factors and liquidation margins are computed privately inside MPC enclaves.
              </p>
            </div>
          </div>
        </div>
      </section>

      {/* ------------------------------------------------------------- */}
      {/* "BUILT FOR THE INDUSTRIES WHERE CONFIDENTIALITY ISN'T OPTIONAL"*/}
      {/* ------------------------------------------------------------- */}
      <section id="industries" className="py-20 md:py-32 bg-[#f1f1ef] border-t border-black/[0.06]">
        <div className="mx-auto max-w-7xl px-6 md:px-10">
          <div className="max-w-3xl mb-14">
            <h2 className="text-3xl sm:text-5xl font-black tracking-tight text-black mb-4">
              Built for the industries where confidentiality isn&apos;t optional
            </h2>
            <p className="text-lg text-black/65">
              For institutions, confidentiality and compliance aren&apos;t features. They&apos;re operating requirements to move their activities on public blockchains.
            </p>
          </div>

          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-6">
            {/* Bento Card 1 */}
            <div className="rounded-2xl bg-white p-7 shadow-sm border border-black/[0.06] flex flex-col justify-between hover:shadow-md transition">
              <div>
                <div className="mb-5 text-black">
                  <LandmarkIcon />
                </div>
                <h3 className="text-lg font-bold text-black mb-2">Confidential RWA Tokenization</h3>
                <p className="text-sm text-black/65 leading-relaxed">
                  Bring real-world assets onchain without exposing investor identities, deal terms, or cap table structure.
                </p>
              </div>
            </div>

            {/* Bento Card 2 */}
            <div className="rounded-2xl bg-white p-7 shadow-sm border border-black/[0.06] flex flex-col justify-between hover:shadow-md transition">
              <div>
                <div className="mb-5 text-black">
                  <CreditCardIcon />
                </div>
                <h3 className="text-lg font-bold text-black mb-2">Confidential Payments and Payroll</h3>
                <p className="text-sm text-black/65 leading-relaxed">
                  Settle, invoice, and compensate onchain without making transaction amounts visible to the world.
                </p>
              </div>
            </div>

            {/* Bento Card 3 */}
            <div className="rounded-2xl bg-white p-7 shadow-sm border border-black/[0.06] flex flex-col justify-between hover:shadow-md transition">
              <div>
                <div className="mb-5 text-black">
                  <ChartIcon />
                </div>
                <h3 className="text-lg font-bold text-black mb-2">Confidential DeFi and Trading</h3>
                <p className="text-sm text-black/65 leading-relaxed">
                  Trade, stake, and earn yield onchain without leaking positions, strategies, or counterparty data.
                </p>
              </div>
            </div>

            {/* Bento Card 4 */}
            <div className="rounded-2xl bg-white p-7 shadow-sm border border-black/[0.06] flex flex-col justify-between hover:shadow-md transition">
              <div>
                <div className="mb-5 text-black">
                  <CoinsIcon />
                </div>
                <h3 className="text-lg font-bold text-black mb-2">Confidential Token Distribution</h3>
                <p className="text-sm text-black/65 leading-relaxed">
                  Run vesting, airdrops, and allocations onchain without exposing who gets what.
                </p>
              </div>
            </div>
          </div>
        </div>
      </section>

      {/* ------------------------------------------------------------- */}
      {/* ARCHITECTURE PILLARS (Zama Section 6)                          */}
      {/* ------------------------------------------------------------- */}
      <section className="py-20 md:py-28 bg-[#f1f1ef]">
        <div className="mx-auto max-w-7xl px-6 md:px-10">
          <div className="grid grid-cols-1 lg:grid-cols-2 gap-8 items-stretch">
            {/* Light Pillar */}
            <div className="rounded-3xl bg-white p-10 md:p-14 border border-black/[0.06] flex flex-col justify-center shadow-sm">
              <h3 className="text-2xl sm:text-3xl font-black text-black mb-5 leading-tight">
                Confidentiality is the missing piece to unlock institutional onchain finance
              </h3>
              <p className="text-base leading-relaxed text-black/70">
                Until now, every transaction on a public blockchain exposed amounts and balances to anyone. For institutions operating in regulated markets, that exposure has made onchain finance a non-starter. Walnut closes that gap, keeping amounts and balances confidential, while maintaining full verifiability and compliance.
              </p>
            </div>

            {/* Dark Pillar with Amber Glow */}
            <div className="relative overflow-hidden rounded-3xl bg-[#111111] p-10 md:p-14 text-white flex flex-col justify-center shadow-2xl">
              {/* Zama amber background ambient glow */}
              <div
                className="pointer-events-none absolute -bottom-10 -right-10 h-72 w-72 rounded-full opacity-30 blur-3xl"
                style={{ background: "#ffd208" }}
              />
              <h3 className="text-2xl sm:text-3xl font-black text-white mb-5 leading-tight">
                Confidentiality should not require any architectural compromise
              </h3>
              <p className="text-base leading-relaxed text-white/70 mb-6">
                The Walnut Protocol runs natively on Solana L1, using Fully Homomorphic Encryption (FHE) & Arcium MPC enclaves to keep amounts and balances encrypted at all times, including during processing.
              </p>
              <div className="text-sm font-semibold text-[#ffd208]">
                No new chain. No trusted intermediary. Full public verifiability.
              </div>
            </div>
          </div>
        </div>
      </section>

      {/* ------------------------------------------------------------- */}
      {/* SOCIAL PROOF & INSTITUTIONAL PARTNERS (Zama Section 7)        */}
      {/* ------------------------------------------------------------- */}
      <section id="ecosystem" className="py-20 md:py-28 bg-[#f1f1ef] border-t border-black/[0.06]">
        <div className="mx-auto max-w-7xl px-6 md:px-10">
          <div className="mb-12">
            <h2 className="text-3xl sm:text-4xl font-black text-black mb-3">
              Institutions and teams already building with Walnut
            </h2>
            <p className="text-base text-black/60">
              From confidential payroll to institutional trading — real deployments, on Solana, today.
            </p>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
            <div className="rounded-2xl bg-white p-7 border border-black/[0.06] shadow-sm">
              <div className="text-xs font-bold uppercase tracking-wider text-[#d4af00] mb-2">MPC Enclaves</div>
              <h4 className="text-lg font-bold text-black mb-2">Arcium - Confidential MPC Network</h4>
              <p className="text-sm text-black/65 mb-4">
                Hardware MPC enclaves enabling decentralized multiparty computation over RescueCipher ciphertexts on Solana devnet.
              </p>
              <span className="text-xs font-bold text-black hover:underline cursor-pointer">Learn more →</span>
            </div>

            <div className="rounded-2xl bg-white p-7 border border-black/[0.06] shadow-sm">
              <div className="text-xs font-bold uppercase tracking-wider text-[#d4af00] mb-2">Oracles</div>
              <h4 className="text-lg font-bold text-black mb-2">Pyth Network - Push Price Feeds</h4>
              <p className="text-sm text-black/65 mb-4">
                Sub-second SOL/USD price dissemination with strict max price age enforcement to protect confidential loan collateralization.
              </p>
              <span className="text-xs font-bold text-black hover:underline cursor-pointer">Learn more →</span>
            </div>

            <div className="rounded-2xl bg-white p-7 border border-black/[0.06] shadow-sm">
              <div className="text-xs font-bold uppercase tracking-wider text-[#d4af00] mb-2">Auditing & Proofs</div>
              <h4 className="text-lg font-bold text-black mb-2">Zero-Knowledge Viewing Keys</h4>
              <p className="text-sm text-black/65 mb-4">
                Client-side balance decryption and selective disclosures for compliance, tax, and auditing with zero ledger leaks.
              </p>
              <Link href={AUDIT_HREF} className="text-xs font-bold text-black hover:underline">Verify now →</Link>
            </div>
          </div>
        </div>
      </section>

      {/* ------------------------------------------------------------- */}
      {/* CODE INTEGRATION SECTION (Zama Section 9 Code Block)           */}
      {/* ------------------------------------------------------------- */}
      <section id="developers" className="py-20 md:py-32 bg-[#111111] text-white">
        <div className="mx-auto max-w-6xl px-6 md:px-10">
          <div className="text-center mb-12">
            <h2 className="text-3xl sm:text-5xl font-black tracking-tight mb-4">
              Integrate confidentiality in a few lines of code
            </h2>
            <p className="text-base sm:text-lg text-white/60 max-w-xl mx-auto">
              Whether you&apos;re building confidential apps or integrating confidential lending, start with our SDK and docs.
            </p>
          </div>

          {/* Interactive Code Container */}
          <div className="rounded-2xl bg-[#2b2b2b] border border-white/10 shadow-2xl overflow-hidden">
            {/* Code Header Bar */}
            <div className="flex items-center justify-between px-6 py-4 border-b border-white/10 bg-[#222222]">
              <div className="flex items-center gap-3">
                <button
                  type="button"
                  onClick={() => setActiveTab("ts")}
                  className={`text-xs font-bold uppercase tracking-wider px-3 py-1.5 rounded-lg transition ${
                    activeTab === "ts" ? "bg-[#ffd208] text-black" : "text-white/60 hover:text-white"
                  }`}
                >
                  TypeScript SDK
                </button>
                <button
                  type="button"
                  onClick={() => setActiveTab("sol")}
                  className={`text-xs font-bold uppercase tracking-wider px-3 py-1.5 rounded-lg transition ${
                    activeTab === "sol" ? "bg-[#ffd208] text-black" : "text-white/60 hover:text-white"
                  }`}
                >
                  Rust Anchor
                </button>
              </div>
              <a
                href="https://github.com/walnut-solana"
                target="_blank"
                rel="noreferrer"
                className="inline-flex items-center gap-1.5 text-xs font-semibold text-white/80 hover:text-white transition bg-white/10 px-3 py-1.5 rounded-lg"
              >
                Docs <ArrowUpRight className="w-3 h-3 text-[#ffd208]" />
              </a>
            </div>

            {/* Code Body */}
            <div className="p-6 overflow-x-auto font-mono text-[13px] leading-relaxed text-[#f8f8f2]">
              {activeTab === "ts" ? (
                <pre>
                  <code>
{`1  import { WalnutClient, RescueCipher } from "@walnut/sdk";
2  import { Connection, PublicKey } from "@solana/web3.js";
3  
4  // Initialize Walnut on Solana Devnet (Cluster 456)
5  const client = new WalnutClient({
6    connection: new Connection("https://devnet.helius-rpc.com/?api-key=..."),
7    programId: new PublicKey("5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea"),
8    clusterOffset: 456,
9  });
10 
11 // Supply 1,000 USDC confidentially into the lending pool
12 const { txSignature, intentPda } = await client.supplyConfidential({
13   amount: 1_000_000_000n, // Client-encrypted via RescueCipher FHE
14   userKeypair: deployerWallet,
15 });
16 
17 console.log("Confidential Supply Confirmed:", txSignature);`}
                  </code>
                </pre>
              ) : (
                <pre>
                  <code>
{`1  #[program]
2  pub mod walnut {
3      use super::*;
4      
5      // Executes confidential pool operations via Arcium MPC callback
6      pub fn pool_op_callback(
7          ctx: Context<PoolOpCallback>,
8          output: ComputationOutput,
9      ) -> Result<()> {
10         let authenticated = verify_bls_domain_separator(&output)?;
11         require!(authenticated, WalnutError::CallbackNotAuthenticated);
12         
13         // Updates pool reserve and positions entirely in ciphertext
14         apply_confidential_state_transition(&mut ctx.accounts.pool_state, output)?;
15         Ok(())
16     }
17 }`}
                  </code>
                </pre>
              )}
            </div>
          </div>
        </div>
      </section>

      {/* ------------------------------------------------------------- */}
      {/* FINAL CONVERSION BANNER (Zama Section 10)                      */}
      {/* ------------------------------------------------------------- */}
      <section className="py-24 md:py-36 bg-[#f1f1ef] border-t border-black/[0.06] text-center">
        <div className="mx-auto max-w-4xl px-6 md:px-10">
          <h2 className="text-3xl sm:text-5xl lg:text-6xl font-black tracking-tight text-black leading-tight mb-8">
            From billions to trillions with confidential onchain finance.
          </h2>
          <p className="text-lg text-black/70 mb-10 max-w-xl mx-auto">
            Talk to the Walnut team and start building confidential lending and credit protocols today.
          </p>
          <div className="flex flex-wrap items-center justify-center gap-4">
            <Link
              href={APP_HREF}
              className="inline-flex items-center justify-center gap-2 rounded-full bg-black px-8 py-4 text-base font-semibold text-white transition hover:bg-[#222222] hover:scale-[1.02]"
            >
              Launch App
              <ArrowUpRight className="w-4 h-4 text-[#ffd208]" />
            </Link>
            <Link
              href={AUDIT_HREF}
              className="inline-flex items-center justify-center rounded-full bg-white px-8 py-4 text-base font-semibold text-black border border-black/10 hover:bg-black/5 transition"
            >
              Verify Audit Proofs
            </Link>
          </div>
        </div>
      </section>

      {/* ------------------------------------------------------------- */}
      {/* MULTI-COLUMN FOOTER (Zama Section 11)                         */}
      {/* ------------------------------------------------------------- */}
      <footer className="border-t border-black/[0.08] bg-[#f1f1ef] py-16 text-xs text-black/70">
        <div className="mx-auto max-w-7xl px-6 md:px-10">
          <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-6 gap-8 mb-16">
            {/* Col 1 */}
            <div>
              <div className="font-bold text-black uppercase tracking-wider mb-3">Protocol</div>
              <ul className="space-y-2">
                <li><Link href={APP_HREF} className="hover:text-black">Walnut App ↗</Link></li>
                <li><Link href={AUDIT_HREF} className="hover:text-black">Audit & Verifier ↗</Link></li>
                <li><a href="#learn-more-section" className="hover:text-black">Architecture</a></li>
                <li><a href="https://solana.com" target="_blank" rel="noreferrer" className="hover:text-black">Solana Devnet ↗</a></li>
              </ul>
            </div>

            {/* Col 2 */}
            <div>
              <div className="font-bold text-black uppercase tracking-wider mb-3">Solutions</div>
              <ul className="space-y-2">
                <li><a href="#industries" className="hover:text-black">Confidential RWA</a></li>
                <li><a href="#industries" className="hover:text-black">Confidential Payroll</a></li>
                <li><a href="#industries" className="hover:text-black">Confidential Lending</a></li>
                <li><a href="#industries" className="hover:text-black">Viewing Keys</a></li>
              </ul>
            </div>

            {/* Col 3 */}
            <div>
              <div className="font-bold text-black uppercase tracking-wider mb-3">Products</div>
              <ul className="space-y-2">
                <li><Link href={APP_HREF} className="hover:text-black">Shield Vaults</Link></li>
                <li><Link href={APP_HREF} className="hover:text-black">Confidential USDC</Link></li>
                <li><Link href={APP_HREF} className="hover:text-black">Confidential SOL</Link></li>
                <li><Link href={APP_HREF} className="hover:text-black">MPC Health Factor</Link></li>
              </ul>
            </div>

            {/* Col 4 */}
            <div>
              <div className="font-bold text-black uppercase tracking-wider mb-3">Resources</div>
              <ul className="space-y-2">
                <li><a href="https://docs.arcium.com" target="_blank" rel="noreferrer" className="hover:text-black">Arcium MPC ↗</a></li>
                <li><a href="https://pyth.network" target="_blank" rel="noreferrer" className="hover:text-black">Pyth Oracle ↗</a></li>
                <li><a href="https://github.com/walnut-solana" target="_blank" rel="noreferrer" className="hover:text-black">GitHub Repo ↗</a></li>
                <li><Link href={AUDIT_HREF} className="hover:text-black">Security Proofs ↗</Link></li>
              </ul>
            </div>

            {/* Col 5 */}
            <div>
              <div className="font-bold text-black uppercase tracking-wider mb-3">Community</div>
              <ul className="space-y-2">
                <li><a href="https://x.com" target="_blank" rel="noreferrer" className="hover:text-black">X / Twitter ↗</a></li>
                <li><a href="https://discord.com" target="_blank" rel="noreferrer" className="hover:text-black">Discord ↗</a></li>
                <li><a href="https://telegram.org" target="_blank" rel="noreferrer" className="hover:text-black">Telegram ↗</a></li>
                <li><a href="https://github.com" target="_blank" rel="noreferrer" className="hover:text-black">GitHub ↗</a></li>
              </ul>
            </div>

            {/* Col 6 */}
            <div>
              <div className="font-bold text-black uppercase tracking-wider mb-3">Company</div>
              <ul className="space-y-2">
                <li><a href="#" className="hover:text-black">About Walnut</a></li>
                <li><a href="#" className="hover:text-black">Brand Kit</a></li>
                <li><a href="#" className="hover:text-black">Privacy Policy</a></li>
                <li><a href="#" className="hover:text-black">Terms of Service</a></li>
              </ul>
            </div>
          </div>

          <div className="flex flex-col sm:flex-row items-center justify-between border-t border-black/[0.06] pt-8 text-black/50">
            <div className="flex items-center gap-2 mb-4 sm:mb-0">
              <span className="h-2 w-2 rounded-full bg-emerald-500 animate-pulse" />
              <span>Solana Devnet: All 7 Arcium MPC Circuits Operational</span>
            </div>
            <div>
              © {new Date().getFullYear()} Walnut Protocol. Confidential Onchain Finance.
            </div>
          </div>
        </div>
      </footer>
    </div>
  );
}
