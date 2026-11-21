"use client";

import React, { useState, useRef, useEffect } from "react";
import Link from "next/link";
import {
  ArrowUpRight,
  ChevronDown,
  ShieldIcon,
  UnshieldIcon,
  SendIcon,
  EarnIcon,
  SwapIcon,
  TokensIcon,
  VaultIcon,
  LayersIcon,
  SdkIcon,
} from "./icons.tsx";

const APP_HREF = "/app";
const AUDIT_HREF = "/app/audit";

export function Navbar() {
  const [dropdownOpen, setDropdownOpen] = useState<string | null>(null);
  const [contactModalOpen, setContactModalOpen] = useState(false);
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false);
  const navRef = useRef<HTMLElement>(null);

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
              className="text-[21px] font-black tracking-tight text-black font-sans uppercase hover:opacity-85 transition select-none"
              aria-label="Walnut home"
            >
              WALNUT
            </Link>
          </div>

          {/* Center: Navigation Links matching Zama header styling */}
          <nav className="hidden md:flex items-center gap-7 text-[13.5px] font-normal text-black">
            {/* Products Dropdown */}
            <div className="relative">
              <button
                type="button"
                onClick={() => toggleDropdown("products")}
                className="flex items-center gap-1 text-black hover:text-black/70 transition py-2 cursor-pointer font-normal"
              >
                Products
                <ChevronDown
                  className={`w-3.5 h-3.5 text-black/70 transition-transform duration-200 ${
                    dropdownOpen === "products" ? "rotate-180" : ""
                  }`}
                />
              </button>

              {dropdownOpen === "products" && (
                <div className="absolute top-full left-1/2 -translate-x-1/2 mt-3 w-[780px] rounded-2xl bg-white p-7 shadow-[0_25px_60px_-15px_rgba(0,0,0,0.15)] border border-black/[0.08] grid grid-cols-3 gap-0 z-50 animate-in fade-in slide-in-from-top-2 duration-150">
                  {/* Column 1: Walnut App */}
                  <div className="border-r border-black/[0.08] pr-6">
                    <div className="text-[13px] font-bold uppercase tracking-wider text-black/50 mb-3.5">
                      Walnut App
                    </div>
                    <div className="flex flex-col gap-1.5">
                      <Link
                        href={APP_HREF}
                        onClick={() => setDropdownOpen(null)}
                        className="group flex items-center gap-3 py-1.5 px-2 -mx-2 rounded-xl hover:bg-black/[0.03] transition-colors"
                      >
                        <div className="flex h-7 w-7 items-center justify-center rounded-[6px] bg-[#FED022] text-black shrink-0 shadow-xs transition-transform group-hover:scale-105">
                          <ShieldIcon />
                        </div>
                        <span className="text-[14px] font-normal text-black group-hover:opacity-75 transition-colors flex items-center gap-1">
                          Shield <span className="text-[12px] text-black/60">↗</span>
                        </span>
                      </Link>

                      <Link
                        href={APP_HREF}
                        onClick={() => setDropdownOpen(null)}
                        className="group flex items-center gap-3 py-1.5 px-2 -mx-2 rounded-xl hover:bg-black/[0.03] transition-colors"
                      >
                        <div className="flex h-7 w-7 items-center justify-center rounded-[6px] bg-[#FED022] text-black shrink-0 shadow-xs transition-transform group-hover:scale-105">
                          <UnshieldIcon />
                        </div>
                        <span className="text-[14px] font-normal text-black group-hover:opacity-75 transition-colors flex items-center gap-1">
                          Unshield <span className="text-[12px] text-black/60">↗</span>
                        </span>
                      </Link>

                      <Link
                        href={APP_HREF}
                        onClick={() => setDropdownOpen(null)}
                        className="group flex items-center gap-3 py-1.5 px-2 -mx-2 rounded-xl hover:bg-black/[0.03] transition-colors"
                      >
                        <div className="flex h-7 w-7 items-center justify-center rounded-[6px] bg-[#FED022] text-black shrink-0 shadow-xs transition-transform group-hover:scale-105">
                          <SendIcon />
                        </div>
                        <span className="text-[14px] font-normal text-black group-hover:opacity-75 transition-colors flex items-center gap-1">
                          Send <span className="text-[12px] text-black/60">↗</span>
                        </span>
                      </Link>

                      <Link
                        href={APP_HREF}
                        onClick={() => setDropdownOpen(null)}
                        className="group flex items-center gap-3 py-1.5 px-2 -mx-2 rounded-xl hover:bg-black/[0.03] transition-colors"
                      >
                        <div className="flex h-7 w-7 items-center justify-center rounded-[6px] bg-[#FED022] text-black shrink-0 shadow-xs transition-transform group-hover:scale-105">
                          <EarnIcon />
                        </div>
                        <span className="text-[14px] font-normal text-black group-hover:opacity-75 transition-colors flex items-center gap-1">
                          Earn <span className="text-[12px] text-black/60">↗</span>
                        </span>
                      </Link>

                      <Link
                        href={APP_HREF}
                        onClick={() => setDropdownOpen(null)}
                        className="group flex items-center gap-3 py-1.5 px-2 -mx-2 rounded-xl hover:bg-black/[0.03] transition-colors"
                      >
                        <div className="flex h-7 w-7 items-center justify-center rounded-[6px] bg-[#FED022] text-black shrink-0 shadow-xs transition-transform group-hover:scale-105">
                          <SwapIcon />
                        </div>
                        <span className="text-[14px] font-normal text-black group-hover:opacity-75 transition-colors flex items-center gap-1">
                          Swap <span className="text-[12px] text-black/60">↗</span>
                        </span>
                      </Link>
                    </div>
                  </div>

                  {/* Column 2: Onchain Finance */}
                  <div className="border-r border-black/[0.08] px-6">
                    <div className="text-[13px] font-bold uppercase tracking-wider text-black/50 mb-3.5">
                      Onchain Finance
                    </div>
                    <div className="flex flex-col gap-1.5">
                      <a
                        href="#products"
                        onClick={() => setDropdownOpen(null)}
                        className="group flex items-center gap-3 py-1.5 px-2 -mx-2 rounded-xl hover:bg-black/[0.03] transition-colors"
                      >
                        <div className="flex h-7 w-7 items-center justify-center rounded-[6px] bg-[#FED022] text-black shrink-0 shadow-xs transition-transform group-hover:scale-105">
                          <TokensIcon />
                        </div>
                        <span className="text-[14px] font-normal text-black group-hover:opacity-75 transition-colors flex items-center gap-1">
                          Confidential Tokens <span className="text-[12px] text-black/60">↗</span>
                        </span>
                      </a>

                      <a
                        href="#products"
                        onClick={() => setDropdownOpen(null)}
                        className="group flex items-center gap-3 py-1.5 px-2 -mx-2 rounded-xl hover:bg-black/[0.03] transition-colors"
                      >
                        <div className="flex h-7 w-7 items-center justify-center rounded-[6px] bg-[#FED022] text-black shrink-0 shadow-xs transition-transform group-hover:scale-105">
                          <VaultIcon />
                        </div>
                        <span className="text-[14px] font-normal text-black group-hover:opacity-75 transition-colors flex items-center gap-1">
                          Confidential Vaults <span className="text-[12px] text-black/60">↗</span>
                        </span>
                      </a>

                      <a
                        href="#products"
                        onClick={() => setDropdownOpen(null)}
                        className="group flex items-center gap-3 py-1.5 px-2 -mx-2 rounded-xl hover:bg-black/[0.03] transition-colors"
                      >
                        <div className="flex h-7 w-7 items-center justify-center rounded-[6px] bg-[#FED022] text-black shrink-0 shadow-xs transition-transform group-hover:scale-105">
                          <SwapIcon />
                        </div>
                        <span className="text-[14px] font-normal text-black group-hover:opacity-75 transition-colors flex items-center gap-1">
                          Confidential Swap <span className="text-[12px] text-black/60">↗</span>
                        </span>
                      </a>
                    </div>
                  </div>

                  {/* Column 3: Infra */}
                  <div className="pl-6">
                    <div className="text-[13px] font-bold uppercase tracking-wider text-black/50 mb-3.5">
                      Infra
                    </div>
                    <div className="flex flex-col gap-1.5">
                      <a
                        href="#solana-spotlight"
                        onClick={() => setDropdownOpen(null)}
                        className="group flex items-center gap-3 py-1.5 px-2 -mx-2 rounded-xl hover:bg-black/[0.03] transition-colors"
                      >
                        <div className="flex h-7 w-7 items-center justify-center rounded-[6px] bg-[#FED022] text-black shrink-0 shadow-xs transition-transform group-hover:scale-105">
                          <LayersIcon />
                        </div>
                        <span className="text-[14px] font-normal text-black group-hover:opacity-75 transition-colors flex items-center gap-1">
                          Confidential SPL <span className="text-[12px] text-black/60">↗</span>
                        </span>
                      </a>

                      <a
                        href="#developers"
                        onClick={() => setDropdownOpen(null)}
                        className="group flex items-center gap-3 py-1.5 px-2 -mx-2 rounded-xl hover:bg-black/[0.03] transition-colors"
                      >
                        <div className="flex h-7 w-7 items-center justify-center rounded-[6px] bg-[#FED022] text-black shrink-0 shadow-xs transition-transform group-hover:scale-105">
                          <SdkIcon />
                        </div>
                        <span className="text-[14px] font-normal text-black group-hover:opacity-75 transition-colors flex items-center gap-1">
                          SDK <span className="text-[12px] text-black/60">↗</span>
                        </span>
                      </a>
                    </div>
                  </div>
                </div>
              )}
            </div>

            {/* Audit Link in Center */}
            <Link
              href={AUDIT_HREF}
              className="text-black hover:text-black/70 transition py-2 font-normal"
            >
              Audit
            </Link>
          </nav>

          {/* Right Action Buttons matching Zama: Jet black Contact us + #FED022 Launch App */}
          <div className="hidden sm:flex items-center gap-2.5">
            <button
              type="button"
              onClick={() => setContactModalOpen(true)}
              className="inline-flex items-center justify-center rounded-[6px] bg-black px-4 py-2 text-[13px] font-normal text-white transition hover:bg-neutral-800 cursor-pointer shadow-2xs"
            >
              Contact us
            </button>
            <Link
              href={APP_HREF}
              className="inline-flex items-center justify-center gap-1 rounded-[6px] bg-[#FED022] hover:bg-[#fad01b] px-4 py-2 text-[13px] font-normal text-black transition cursor-pointer shadow-2xs"
            >
              <span>Launch App</span>
              <span className="text-[12px] font-normal">↗</span>
            </Link>
          </div>

          {/* Mobile Menu Hamburger */}
          <div className="flex md:hidden items-center gap-2">
            <Link
              href={APP_HREF}
              className="inline-flex items-center justify-center rounded-[6px] bg-[#FFE052] px-3.5 py-1.5 text-[13px] font-semibold text-black"
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
          <div className="border-b border-black/[0.08] bg-white px-6 py-5 md:hidden space-y-4 animate-in slide-in-from-top-2">
            <div className="flex flex-col gap-3 text-sm font-semibold text-black">
              <a href="#products" onClick={() => setMobileMenuOpen(false)} className="py-1">Products</a>
              <Link href={AUDIT_HREF} onClick={() => setMobileMenuOpen(false)} className="py-1">Audit ↗</Link>
            </div>
            <div className="pt-3 border-t border-black/[0.08] flex gap-3">
              <button
                type="button"
                onClick={() => { setMobileMenuOpen(false); setContactModalOpen(true); }}
                className="w-1/2 rounded-[6px] bg-black py-2.5 text-center text-xs font-semibold text-white"
              >
                Contact us
              </button>
              <Link
                href={APP_HREF}
                className="w-1/2 rounded-[6px] bg-[#FFE052] py-2.5 text-center text-xs font-semibold text-black flex items-center justify-center gap-1"
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
