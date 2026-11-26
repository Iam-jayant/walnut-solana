"use client";

import React, { useState, useRef, useEffect, type ReactNode } from "react";
import Link from "next/link";
import {
  ArrowUpRight,
  ChevronDown,
  ShieldIcon,
  UnshieldIcon,
  EarnIcon,
  BorrowIcon,
  TokensIcon,
  VaultIcon,
  LayersIcon,
  SdkIcon,
} from "./icons.tsx";
import { PartnerLogo } from "./partner-logo.tsx";
import { WalnutLockup } from "../brand-logo.tsx";

const APP_HREF = "/app";

const GITHUB_HREF = "https://github.com/Iam-jayant/walnut-solana";

interface NavLeaf {
  label: string;
  href: string;
  desc?: string;
  icon?: ReactNode;
  logo?: string;
  external?: boolean;
}

type NavEntry =
  | { kind: "link"; label: string; href: string }
  | {
      kind: "menu";
      id: string;
      label: string;
      width: string;
      columns: ReadonlyArray<{ title: string; items: ReadonlyArray<NavLeaf> }>;
    };

const NAV: ReadonlyArray<NavEntry> = [
  {
    kind: "menu",
    id: "products",
    label: "Products",
    width: "w-[760px]",
    columns: [
      {
        title: "Confidential tokens",
        items: [
          { label: "Shield", desc: "Wrap SOL and USDC into encrypted tokens", href: APP_HREF, icon: <ShieldIcon /> },
          { label: "Unshield", desc: "Withdraw back to your public wallet", href: APP_HREF, icon: <UnshieldIcon /> },
        ],
      },
      {
        title: "Lending",
        items: [
          { label: "Collateral", desc: "Add or withdraw private collateral", href: APP_HREF, icon: <LayersIcon /> },
          { label: "Borrow & Repay", desc: "Private debt, checked by MPC", href: APP_HREF, icon: <BorrowIcon /> },
          { label: "Supply & Earn", desc: "Earn yield on supplied USDC", href: APP_HREF, icon: <EarnIcon /> },
        ],
      },
      {
        title: "Monitor",
        items: [
          { label: "Liquidations", desc: "Health checks and liquidations", href: APP_HREF, icon: <VaultIcon /> },
          { label: "Activity", desc: "Live status of your operations", href: APP_HREF, icon: <TokensIcon /> },
        ],
      },
    ],
  },
  { kind: "link", label: "How it works", href: "#solana-spotlight" },
  { kind: "link", label: "Use cases", href: "#industries" },
  {
    kind: "menu",
    id: "developers",
    label: "Developers",
    width: "w-[520px]",
    columns: [
      {
        title: "Build",
        items: [
          { label: "TypeScript SDK", desc: "Client code and examples", href: "#developers", icon: <SdkIcon /> },
          { label: "GitHub", desc: "Source code and issues", href: GITHUB_HREF, logo: "/logos/github.svg", external: true },
        ],
      },
      {
        title: "Powered by",
        items: [
          { label: "Arcium", desc: "Multi-party computation", href: "https://arcium.com", logo: "/logos/arcium.png", external: true },
          { label: "Pyth Network", desc: "SOL/USD price oracle", href: "https://pyth.network", logo: "/logos/pyth.png", external: true },
        ],
      },
    ],
  },
  {
    kind: "menu",
    id: "community",
    label: "Community",
    width: "w-[300px]",
    columns: [
      {
        title: "Follow Walnut",
        items: [
          { label: "X / Twitter", desc: "News and updates", href: "https://x.com/walnut_solana", logo: "/logos/x.svg", external: true },
          { label: "Telegram", desc: "Chat with the team", href: "https://t.me/walnut_solana", logo: "/logos/telegram.svg", external: true },
          { label: "GitHub", desc: "Star and contribute", href: GITHUB_HREF, logo: "/logos/github.svg", external: true },
        ],
      },
    ],
  },
];

function NavLeafLink({ item, onNavigate }: { item: NavLeaf; onNavigate: () => void }) {
  const inner = (
    <>
      {item.logo ? (
        <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[8px] border border-black/[0.08] bg-white">
          <PartnerLogo src={item.logo} name={item.label} size={18} className="rounded-[4px]" />
        </span>
      ) : (
        <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[8px] bg-[#D8D4CB] shadow-xs transition-transform group-hover:scale-105">
          {item.icon}
        </span>
      )}
      <span className="min-w-0">
        <span className="flex items-center gap-1 text-[14px] font-medium text-black">
          {item.label}
          {item.external ? <span className="text-[12px] text-black/50">↗</span> : null}
        </span>
        {item.desc ? (
          <span className="mt-0.5 block text-[12px] leading-snug text-black/50">{item.desc}</span>
        ) : null}
      </span>
    </>
  );
  const cls = "group flex items-start gap-3 rounded-xl p-2 transition-colors hover:bg-black/[0.04]";

  if (item.external) {
    return (
      <a href={item.href} target="_blank" rel="noreferrer" onClick={onNavigate} className={cls}>
        {inner}
      </a>
    );
  }
  if (item.href.startsWith("#")) {
    return (
      <a href={item.href} onClick={onNavigate} className={cls}>
        {inner}
      </a>
    );
  }
  return (
    <Link href={item.href} onClick={onNavigate} className={cls}>
      {inner}
    </Link>
  );
}

export function Navbar() {
  const [dropdownOpen, setDropdownOpen] = useState<string | null>(null);
  const [contactModalOpen, setContactModalOpen] = useState(false);
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false);
  const navRef = useRef<HTMLElement>(null);

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setDropdownOpen(null);
        setMobileMenuOpen(false);
      }
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);

  const toggleDropdown = (name: string) => {
    setDropdownOpen(dropdownOpen === name ? null : name);
  };

  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (navRef.current && !navRef.current.contains(event.target as Node)) {
        setDropdownOpen(null);
      }
    }
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  return (
    <>
      <header
        ref={navRef}
        className="relative z-40 w-full bg-transparent transition-all"
      >
        <div className="flex h-[56px] w-full items-center justify-between px-0">
          {/* Left: Text Logo (WALNUT) matching ZAMA typography */}
          <div className="flex items-center">
            <Link
              href="/"
              className="hover:opacity-85 transition select-none"
              aria-label="Walnut home"
            >
              <WalnutLockup size={28} />
            </Link>
          </div>

          {/* Center: Navigation (desktop) */}
          <nav className="hidden lg:flex items-center gap-7 text-[13.5px] font-normal text-black" aria-label="Main">
            {NAV.map((entry) =>
              entry.kind === "link" ? (
                <a
                  key={entry.label}
                  href={entry.href}
                  className="py-2 text-black transition hover:text-black/70"
                >
                  {entry.label}
                </a>
              ) : (
                <div key={entry.id}>
                  <button
                    type="button"
                    onClick={() => toggleDropdown(entry.id)}
                    aria-expanded={dropdownOpen === entry.id}
                    aria-haspopup="true"
                    className="flex cursor-pointer items-center gap-1 py-2 font-normal text-black transition hover:text-black/70"
                  >
                    {entry.label}
                    <ChevronDown
                      className={`h-3.5 w-3.5 text-black/70 transition-transform duration-200 ${
                        dropdownOpen === entry.id ? "rotate-180" : ""
                      }`}
                    />
                  </button>

                  {dropdownOpen === entry.id && (
                    <div
                      className={`absolute left-1/2 top-full z-50 mt-3 max-w-[calc(100vw-2rem)] -translate-x-1/2 rounded-2xl border border-black/[0.08] bg-white p-5 shadow-[0_25px_60px_-15px_rgba(0,0,0,0.15)] animate-in fade-in slide-in-from-top-2 duration-150 ${entry.width}`}
                    >
                      <div
                        className="grid gap-x-6 gap-y-4"
                        style={{ gridTemplateColumns: `repeat(${entry.columns.length}, minmax(0, 1fr))` }}
                      >
                        {entry.columns.map((col) => (
                          <div key={col.title}>
                            <div className="mb-2 px-2 text-[11px] font-bold uppercase tracking-wider text-black/45">
                              {col.title}
                            </div>
                            <div className="flex flex-col gap-0.5">
                              {col.items.map((item) => (
                                <NavLeafLink
                                  key={item.label}
                                  item={item}
                                  onNavigate={() => setDropdownOpen(null)}
                                />
                              ))}
                            </div>
                          </div>
                        ))}
                      </div>
                    </div>
                  )}
                </div>
              ),
            )}
          </nav>

          {/* Right Action Buttons matching Zama: Outline Contact us + ink Launch App */}
          <div className="hidden lg:flex items-center gap-2.5">
            <button
              type="button"
              onClick={() => setContactModalOpen(true)}
              className="inline-flex items-center justify-center rounded-[6px] border border-black/15 bg-transparent px-4 py-2 text-[13px] font-medium text-black transition hover:bg-black/[0.05] cursor-pointer"
            >
              Contact us
            </button>
            <Link
              href={APP_HREF}
              className="inline-flex items-center justify-center gap-1 rounded-[6px] bg-[#111111] hover:bg-[#2B2B2B] px-4 py-2 text-[13px] font-medium text-white transition cursor-pointer shadow-2xs"
            >
              <span>Launch App</span>
              <span className="text-[12px] font-normal">↗</span>
            </Link>
          </div>

          {/* Mobile Menu Hamburger */}
          <div className="flex lg:hidden items-center gap-2">
            <Link
              href={APP_HREF}
              className="inline-flex items-center justify-center rounded-[6px] bg-[#111111] px-3.5 py-1.5 text-[13px] font-semibold text-white"
            >
              App ↗
            </Link>
            <button
              type="button"
              onClick={() => setMobileMenuOpen(!mobileMenuOpen)}
              className="p-2 text-black hover:opacity-75 transition"
              aria-label="Toggle menu"
            >
              {mobileMenuOpen ? (
                <svg className="w-6 h-6" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M18 6L6 18M6 6l12 12" /></svg>
              ) : (
                <svg className="w-6 h-6" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M4 6h16M4 12h16M4 18h16" /></svg>
              )}
            </button>
          </div>
        </div>

        {/* Mobile Dropdown Drawer */}
        {mobileMenuOpen && (
          <div className="border-b border-black/[0.08] bg-white px-6 py-5 lg:hidden space-y-4 animate-in slide-in-from-top-2">
            <div className="flex max-h-[60vh] flex-col gap-4 overflow-y-auto text-sm text-black">
              {NAV.map((entry) =>
                entry.kind === "link" ? (
                  <a
                    key={entry.label}
                    href={entry.href}
                    onClick={() => setMobileMenuOpen(false)}
                    className="py-1 font-semibold"
                  >
                    {entry.label}
                  </a>
                ) : (
                  <div key={entry.id}>
                    <div className="mb-1.5 text-[11px] font-bold uppercase tracking-wider text-black/45">
                      {entry.label}
                    </div>
                    <div className="flex flex-col">
                      {entry.columns.flatMap((col) => col.items).map((item) => (
                        <NavLeafLink
                          key={`${entry.id}-${item.label}`}
                          item={item}
                          onNavigate={() => setMobileMenuOpen(false)}
                        />
                      ))}
                    </div>
                  </div>
                ),
              )}
            </div>
            <div className="pt-3 border-t border-black/[0.08] flex gap-3">
              <button
                type="button"
                onClick={() => { setMobileMenuOpen(false); setContactModalOpen(true); }}
                className="w-1/2 rounded-[6px] border border-black/15 py-2.5 text-center text-xs font-semibold text-black"
              >
                Contact us
              </button>
              <Link
                href={APP_HREF}
                className="w-1/2 rounded-[6px] bg-[#111111] py-2.5 text-center text-xs font-semibold text-white flex items-center justify-center gap-1"
              >
                Launch App ↗
              </Link>
            </div>
          </div>
        )}
      </header>

      {/* Contact Us Modal */}
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
