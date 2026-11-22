"use client";

import React, { useState } from "react";
import { ArrowUpRight } from "./icons.tsx";

export function FinalCta() {
  const [contactModalOpen, setContactModalOpen] = useState(false);

  return (
    <>
      <section id="cta" className="py-10 sm:py-16 bg-[#fbfbfb]">
        {/* Standard 75px side margins */}
        <div className="w-full px-4 sm:px-8 lg:px-[75px]">
          {/* Enclosed Linen Stone Card matching Zama CTA reference */}
          <div className="rounded-[18px] sm:rounded-[22px] bg-[#F3EFEC] p-8 sm:p-12 lg:p-14 min-h-[280px] sm:min-h-[320px] lg:min-h-[350px] flex flex-col justify-between border border-black/[0.03] shadow-xs">
            <div>
              <h2
                className="text-2xl sm:text-3xl lg:text-[38px] xl:text-[42px] tracking-tight text-black max-w-2xl select-none"
                style={{
                  fontFamily: "var(--font-telegraf), ui-sans-serif, system-ui, sans-serif",
                  fontWeight: 200,
                  lineHeight: 1.15,
                }}
              >
                From billions to trillions with <br />
                confidential onchain finance. Talk to <br />
                the Walnut team and start building.
              </h2>
            </div>

            <div className="pt-8 sm:pt-12">
              <button
                type="button"
                onClick={() => setContactModalOpen(true)}
                className="inline-block px-6 py-2.5 rounded-[6px] bg-black text-white text-xs font-normal hover:bg-neutral-800 transition cursor-pointer shadow-xs active:scale-[0.99]"
              >
                Contact us
              </button>
            </div>
          </div>
        </div>
      </section>

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
