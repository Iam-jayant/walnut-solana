"use client";

import React, { useState } from "react";
import Link from "next/link";
import { ArrowUpRight } from "./icons.tsx";

const APP_HREF = "/app";
const AUDIT_HREF = "/app/audit";

export function Footer() {
  const [contactModalOpen, setContactModalOpen] = useState(false);

  return (
    <>
      <footer className="border-t border-black/[0.08] bg-[#fbfbfb] pt-16 pb-0 text-xs text-black/70 overflow-hidden">
        <div className="mx-auto max-w-7xl px-6 md:px-10">
          {/* 4 Focused Walnut-Native Columns */}
          <div className="grid grid-cols-2 md:grid-cols-4 gap-8 sm:gap-12 mb-16">
            {/* Col 1: Protocol */}
            <div>
              <div className="font-bold text-black uppercase tracking-wider mb-3">Protocol</div>
              <ul className="space-y-2">
                <li><Link href={APP_HREF} className="hover:text-black transition">Launch App ↗</Link></li>
                <li><Link href={AUDIT_HREF} className="hover:text-black transition">Audit & Verifier ↗</Link></li>
                <li><a href="#solana-spotlight" className="hover:text-black transition">Architecture</a></li>
                <li><a href="https://explorer.solana.com/?cluster=devnet" target="_blank" rel="noreferrer" className="hover:text-black transition">Solana Devnet ↗</a></li>
              </ul>
            </div>

            {/* Col 2: Products */}
            <div>
              <div className="font-bold text-black uppercase tracking-wider mb-3">Products</div>
              <ul className="space-y-2">
                <li><Link href={APP_HREF} className="hover:text-black transition">Confidential Lending</Link></li>
                <li><Link href={APP_HREF} className="hover:text-black transition">Shield & Unshield</Link></li>
                <li><Link href={APP_HREF} className="hover:text-black transition">Private Positions</Link></li>
                <li><Link href={AUDIT_HREF} className="hover:text-black transition">Viewing Keys</Link></li>
              </ul>
            </div>

            {/* Col 3: Resources */}
            <div>
              <div className="font-bold text-black uppercase tracking-wider mb-3">Resources</div>
              <ul className="space-y-2">
                <li><a href="https://arcium.com" target="_blank" rel="noreferrer" className="hover:text-black transition">Arcium MPC ↗</a></li>
                <li><a href="https://pyth.network" target="_blank" rel="noreferrer" className="hover:text-black transition">Pyth Network ↗</a></li>
                <li><a href="#developers" className="hover:text-black transition">TypeScript SDK</a></li>
                <li><a href="https://github.com/walnut-solana" target="_blank" rel="noreferrer" className="hover:text-black transition">GitHub Repository ↗</a></li>
              </ul>
            </div>

            {/* Col 4: Community */}
            <div>
              <div className="font-bold text-black uppercase tracking-wider mb-3">Community</div>
              <ul className="space-y-2">
                <li><a href="https://x.com/walnut_solana" target="_blank" rel="noreferrer" className="hover:text-black transition">X / Twitter ↗</a></li>
                <li><a href="https://t.me/walnut_solana" target="_blank" rel="noreferrer" className="hover:text-black transition">Telegram ↗</a></li>
                <li><a href="https://github.com/walnut-solana" target="_blank" rel="noreferrer" className="hover:text-black transition">GitHub ↗</a></li>
                <li><button type="button" onClick={() => setContactModalOpen(true)} className="hover:text-black transition text-left cursor-pointer">Contact Us ↗</button></li>
              </ul>
            </div>
          </div>

          {/* Devnet Status Bar */}
          <div className="flex items-center justify-center gap-2 mb-6 text-black/50 text-[12px]">
            <span className="h-2 w-2 rounded-full bg-[#FFE052] animate-pulse" />
            <span>Solana Devnet: All 7 Arcium MPC Circuits Operational</span>
          </div>

          {/* Copyright centered matching Zama exact reference */}
          <div className="text-center text-black/45 text-[12px] sm:text-[13px] font-normal mb-8 sm:mb-10 select-none">
            © 2026 Walnut Protocol. Confidential Onchain Finance.
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
              background: "linear-gradient(to bottom, transparent 0%, #fbfbfb 100%)",
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
                href="mailto:contact@walnut.so"
                className="flex items-center justify-between p-3.5 rounded-xl border border-black/[0.08] hover:border-[#FFE052] hover:bg-[#FFE052]/10 transition group"
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
                className="flex items-center justify-between p-3.5 rounded-xl border border-black/[0.08] hover:border-[#FFE052] hover:bg-[#FFE052]/10 transition group"
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
