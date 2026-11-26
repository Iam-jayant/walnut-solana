"use client";

import React, { useState } from "react";
import Link from "next/link";
import { ArrowUpRight, ShieldIcon, BorrowIcon, EarnIcon, ChartIcon } from "./icons.tsx";
import { PartnerLogo } from "./partner-logo.tsx";
import { WalnutLogo } from "../brand-logo.tsx";
import { PROGRAM_ID } from "../../lib/config.ts";
import { LINKS } from "./links.ts";

const APP_HREF = "/app";

const pill =
  "inline-flex items-center gap-2 rounded-full border border-black/[0.08] bg-white px-3.5 py-2 text-[13px] font-medium text-black/75 transition hover:border-black/25 hover:text-black";
const pillSoon =
  "inline-flex items-center gap-2 rounded-full border border-dashed border-black/[0.12] bg-transparent px-3.5 py-2 text-[13px] font-medium text-black/40 cursor-default";

function Glyph({ d }: { d: string }) {
  return (
    <svg viewBox="0 0 24 24" className="h-4 w-4" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d={d} />
    </svg>
  );
}

type PillItem = { label: string; href: string; logo?: string; glyph?: string; internal?: boolean };

const PILLS: PillItem[] = [
  { label: "Launch app", href: APP_HREF, glyph: "M5 12h14M13 6l6 6-6 6", internal: true },
  { label: "Litepaper", href: LINKS.litepaper, glyph: "M6 3h9l4 4v14H6zM14 3v5h5M9 13h6M9 17h6" },
  { label: "GitHub", href: LINKS.github, logo: "/logos/github.svg" },
  { label: "X", href: LINKS.x, logo: "/logos/x.svg" },
  { label: "Telegram", href: LINKS.telegram, logo: "/logos/telegram.svg" },
  { label: "Discord", href: LINKS.discord, logo: "/logos/discord.svg" },
  { label: "Pitch deck", href: LINKS.pitchDeck, glyph: "M3 4h18v12H3zM12 16v4M8 20h8" },
  { label: "Demo video", href: LINKS.demoVideo, glyph: "M3 6h12v12H3zM15 10l6-3v10l-6-3" },
];

function Pill({ item }: { item: PillItem }) {
  const icon = item.logo ? (
    <PartnerLogo src={item.logo} name={item.label} size={16} className="rounded-[4px]" />
  ) : (
    <Glyph d={item.glyph ?? ""} />
  );
  if (!item.href) {
    return (
      <span className={pillSoon} aria-disabled="true" title="Coming soon">
        {icon}
        {item.label}
        <span className="text-[10px] uppercase tracking-wider">Soon</span>
      </span>
    );
  }
  if (item.internal) {
    return (
      <Link href={item.href} className={pill}>
        {icon}
        {item.label}
      </Link>
    );
  }
  return (
    <a href={item.href} target="_blank" rel="noreferrer" className={pill}>
      {icon}
      {item.label}
    </a>
  );
}

const PRODUCTS = [
  { title: "Shield", sub: "Wrap SOL and USDC privately", icon: <ShieldIcon /> },
  { title: "Borrow", sub: "Private collateral and debt", icon: <BorrowIcon /> },
  { title: "Supply & Earn", sub: "Yield on hidden balances", icon: <EarnIcon /> },
  { title: "Liquidations", sub: "Confidential health checks", icon: <ChartIcon /> },
];

const WORKS_WITH = [
  { name: "Solana", logo: "/logos/sol.png", href: "https://solana.com" },
  { name: "Arcium", logo: "/logos/arcium.png", href: "https://arcium.com" },
  { name: "Pyth Network", logo: "/logos/pyth.png", href: "https://pyth.network" },
  { name: "Phantom", logo: "/logos/phantom.svg", href: "https://phantom.com" },
  { name: "Solflare", logo: "/logos/solflare.svg", href: "https://solflare.com" },
];

function CopyAddress({ value }: { value: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="flex items-center gap-3 rounded-xl border border-black/[0.08] bg-white px-4 py-3">
      <code className="min-w-0 flex-1 truncate font-mono text-[12px] sm:text-[13px] text-black/80">{value}</code>
      <button
        type="button"
        onClick={async () => {
          try {
            await navigator.clipboard.writeText(value);
            setCopied(true);
            setTimeout(() => setCopied(false), 1500);
          } catch {
            /* clipboard unavailable: ignore */
          }
        }}
        className="shrink-0 rounded-lg border border-black/[0.08] px-2.5 py-1 text-[11px] font-semibold text-black/70 hover:border-black/25 hover:text-black transition cursor-pointer"
      >
        {copied ? "Copied" : "Copy"}
      </button>
    </div>
  );
}

export function Footer() {
  const [contactModalOpen, setContactModalOpen] = useState(false);

  return (
    <>
      <footer className="border-t border-black/[0.08] bg-[#F5F3EF] pt-14 pb-0 text-xs text-black/70 overflow-hidden">
        <div className="mx-auto max-w-7xl px-6 md:px-10">
          {/* Brand + link pills */}
          <div className="flex flex-col gap-8 lg:flex-row lg:items-start lg:justify-between">
            <div className="lg:max-w-[260px]">
              <div className="flex items-center gap-2.5">
                <WalnutLogo size={32} />
                <span className="text-lg font-bold text-black">Walnut</span>
              </div>
              <p className="mt-4 text-[15px] leading-relaxed text-black/60">
                Lend and borrow on Solana without showing your balance sheet.
              </p>
            </div>
            <div className="flex flex-wrap gap-2.5 lg:max-w-[820px]">
              {PILLS.map((item) => (
                <Pill key={item.label} item={item} />
              ))}
              <button type="button" onClick={() => setContactModalOpen(true)} className={`${pill} cursor-pointer`}>
                <Glyph d="M4 6h16v12H4zM4 7l8 6 8-6" />
                Contact
              </button>
            </div>
          </div>

          {/* Products */}
          <div className="mt-12">
            <div className="text-[13px] text-black/50 mb-4">Products</div>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4">
              {PRODUCTS.map((p) => (
                <Link
                  key={p.title}
                  href={APP_HREF}
                  className="flex items-center gap-3 rounded-xl p-3 transition hover:bg-black/[0.04]"
                >
                  <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-black/[0.05] text-black/80">
                    {p.icon}
                  </span>
                  <span>
                    <span className="block text-[15px] font-semibold text-black">{p.title}</span>
                    <span className="block text-[13px] text-black/55">{p.sub}</span>
                  </span>
                </Link>
              ))}
            </div>
          </div>

          {/* Works with */}
          <div className="mt-10">
            <div className="text-[13px] text-black/50 mb-4">Works with</div>
            <div className="flex flex-wrap gap-2.5">
              {WORKS_WITH.map((w) => (
                <a key={w.name} href={w.href} target="_blank" rel="noreferrer" className={pill}>
                  <PartnerLogo src={w.logo} name={w.name} size={20} className="rounded-md" />
                  {w.name}
                </a>
              ))}
            </div>
          </div>

          {/* Program ID */}
          <div className="mt-10">
            <div className="text-[13px] text-black/50 mb-3">Program ID</div>
            <CopyAddress value={PROGRAM_ID.toBase58()} />
          </div>

          {/* Bottom line */}
          <div className="mt-8 mb-8 flex flex-col gap-2 text-[12px] text-black/45 sm:flex-row sm:items-center sm:justify-between select-none">
            <span>© 2026 Walnut Protocol · Confidential onchain finance</span>
            <span>Experimental software. Not financial advice.</span>
          </div>
        </div>

        {/* Full-Bleed Edge-to-Edge Stretched Brand Watermark matching Zama */}
        <div className="relative w-full overflow-hidden select-none pointer-events-none -mb-1 sm:-mb-2">
          <svg
            viewBox="0 0 1600 240"
            className="w-full h-auto select-none pointer-events-none block"
            preserveAspectRatio="none"
            style={{ minHeight: "150px", maxHeight: "380px" }}
          >
            <defs>
              <linearGradient id="walnutFadeGradient" x1="0%" y1="0%" x2="0%" y2="100%">
                <stop offset="0%" stopColor="#111111" stopOpacity="0.22" />
                <stop offset="35%" stopColor="#111111" stopOpacity="0.14" />
                <stop offset="70%" stopColor="#111111" stopOpacity="0.04" />
                <stop offset="100%" stopColor="#111111" stopOpacity="0.00" />
              </linearGradient>
            </defs>
            <text
              x="50%"
              y="218"
              textAnchor="middle"
              fill="url(#walnutFadeGradient)"
              textLength="1580"
              lengthAdjust="spacingAndGlyphs"
              style={{
                fontFamily: "var(--font-telegraf), ui-sans-serif, system-ui, sans-serif",
                fontWeight: 900,
                fontSize: "250px",
              }}
            >
              WALNUT
            </text>
          </svg>

          {/* Bottom fade mask to seamlessly merge into the page bottom */}
          <div
            className="pointer-events-none absolute inset-x-0 bottom-0 h-16 sm:h-24"
            style={{
              background: "linear-gradient(to bottom, transparent 0%, #F5F3EF 100%)",
            }}
          />
        </div>
      </footer>

      {/* Contact Modal */}
      {contactModalOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm p-4 animate-in fade-in">
          <div className="w-full max-w-md rounded-2xl bg-white p-7 shadow-2xl border border-black/[0.08]">
            <div className="flex items-center justify-between mb-4">
              <h3 className="text-xl font-bold text-black">Contact Walnut</h3>
              <button
                type="button"
                onClick={() => setContactModalOpen(false)}
                className="rounded-lg p-1 text-black/50 hover:bg-black/5 hover:text-black transition cursor-pointer"
              >
                <svg className="w-5 h-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M18 6L6 18M6 6l12 12" /></svg>
              </button>
            </div>
            <p className="text-sm text-black/70 mb-5">
              Building confidential DeFi on Solana or looking to integrate private lending? Get in touch with our protocol engineering team.
            </p>
            <div className="space-y-3">
              <a
                href={`mailto:${LINKS.email}`}
                className="flex items-center justify-between p-3.5 rounded-xl border border-black/[0.08] hover:border-[#D8D4CB] hover:bg-[#D8D4CB]/10 transition group"
              >
                <div>
                  <div className="text-xs font-semibold uppercase tracking-wider text-black/50">Email Us</div>
                  <div className="text-sm font-semibold text-black group-hover:text-black">contact@walnut.so</div>
                </div>
                <ArrowUpRight className="w-4 h-4 text-black/40 group-hover:text-black" />
              </a>
              <a
                href="https://t.me/walnut_solana"
                target="_blank"
                rel="noopener noreferrer"
                className="flex items-center justify-between p-3.5 rounded-xl border border-black/[0.08] hover:border-[#D8D4CB] hover:bg-[#D8D4CB]/10 transition group"
              >
                <div>
                  <div className="text-xs font-semibold uppercase tracking-wider text-black/50">Telegram Community</div>
                  <div className="text-sm font-semibold text-black group-hover:text-black">@walnut_solana</div>
                </div>
                <ArrowUpRight className="w-4 h-4 text-black/40 group-hover:text-black" />
              </a>
            </div>
            <div className="mt-6">
              <button
                type="button"
                onClick={() => setContactModalOpen(false)}
                className="w-full rounded-xl bg-black py-2.5 text-sm font-semibold text-white hover:bg-neutral-800 transition cursor-pointer"
              >
                Close
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
