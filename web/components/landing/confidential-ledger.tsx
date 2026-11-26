"use client";

import React from "react";

function ChevronRightIcon({ className = "w-3 h-3 text-black" }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M9 18l6-6-6-6" />
    </svg>
  );
}

export function ConfidentialLedger() {
  return (
    <section id="missing-piece" className="py-10 sm:py-16 bg-[#F5F3EF]">
      {/* Outer Section Card: 75px side padding, soft rounded corners */}
      <div className="w-full px-4 sm:px-8 lg:px-[75px]">
        <div className="relative overflow-hidden rounded-[20px] sm:rounded-[24px] border border-black/[0.05] bg-[#F3EFEC] min-h-[520px] sm:min-h-[580px] lg:min-h-[620px] p-8 sm:p-12 lg:p-16 shadow-[0_4px_30px_rgba(0,0,0,0.02)] flex flex-col justify-between">
          {/* Background atmosphere: Left neutral stone #F3EFEC, Right warm sunny yellow bloom */}
          <div
            className="pointer-events-none absolute inset-0 z-0"
            style={{
              background: `
                radial-gradient(ellipse 65% 75% at 80% 50%, #D8D4CB 0%, #EAE7E0 42%, rgba(234, 231, 224, 0.4) 68%, transparent 100%),
                radial-gradient(circle 500px at 90% 25%, rgba(255, 255, 255, 0.85) 0%, transparent 60%),
                linear-gradient(90deg, #F3EFEC 0%, #F3EFEC 48%, rgba(216, 212, 203, 0.35) 68%, #D8D4CB 100%)
              `,
            }}
          />

          {/* Grid Layout: Left Narrative (7 cols), Right Floating Stacked Cards (5 cols) */}
          <div className="relative z-10 grid grid-cols-1 lg:grid-cols-12 gap-8 lg:gap-10 items-center h-full my-auto">
            {/* Left Narrative Column */}
            <div className="lg:col-span-7 flex flex-col justify-between h-full py-3 sm:py-5">
              <div>
                <h2
                  className="text-2xl sm:text-3xl lg:text-[38px] xl:text-[41px] tracking-tight text-black mb-6 select-none"
                  style={{
                    fontFamily: "var(--font-telegraf), ui-sans-serif, system-ui, sans-serif",
                    fontWeight: 200,
                    lineHeight: 1.12,
                  }}
                >
                  Confidentiality is the missing piece <br />
                  to unlock institutional onchain <br />
                  finance
                </h2>
              </div>

              <div>
                <p className="max-w-[430px] text-xs sm:text-[13px] leading-[1.65] text-black/70 mt-12 sm:mt-20 lg:mt-28">
                  Until now, every transaction on Solana exposed balances, collateral ratios, and liquidation thresholds. That exposure invites predatory MEV bots, front-running, and liquidation cascades. Walnut closes that gap, executing confidential state transitions natively on Solana L1 via Arcium MPC while maintaining full mathematical verifiability.
                </p>
              </div>
            </div>

            {/* Right Column: Layered Stack of Confidential Solana Transaction Rows */}
            <div className="lg:col-span-5 flex flex-col items-center justify-center space-y-3 sm:space-y-3.5 max-w-lg mx-auto w-full">
              {/* Card -2: Topmost Faint Row */}
              <div className="w-full rounded-[14px] bg-white/35 backdrop-blur-xs p-3.5 sm:p-4 border border-white/40 shadow-2xs opacity-40 flex items-center justify-between text-xs text-black/60 pointer-events-none select-none">
                <div className="flex items-center gap-3">
                  <div>
                    <span className="text-[10px] text-black/40 block">From</span>
                    <span className="font-mono text-xs">7xKX..94mQ</span>
                  </div>
                  <div className="w-5 h-5 rounded-full bg-[#D8D4CB]/70 flex items-center justify-center">
                    <span className="text-[10px] text-black">›</span>
                  </div>
                  <div>
                    <span className="text-[10px] text-black/40 block">To</span>
                    <span className="font-mono text-xs">9WzD..kP31</span>
                  </div>
                </div>
                <div className="border-l border-black/10 pl-4">
                  <span className="text-[10px] text-black/40 block">Amount</span>
                  <span className="font-mono text-xs text-black/50">••••••••••</span>
                </div>
              </div>

              {/* Card -1: Upper Ghosted Row (Solana Address / Token / Amount) */}
              <div className="w-full rounded-[14px] bg-white/65 backdrop-blur-sm p-4 sm:p-4.5 border border-white/60 shadow-xs opacity-65 flex items-center justify-between text-xs pointer-events-none select-none">
                <div className="space-y-1">
                  <div className="flex items-center gap-6">
                    <span className="text-black/45 w-14">Address</span>
                    <span className="font-mono text-black text-xs font-normal">7xKX..94mQ</span>
                  </div>
                  <div className="flex items-center gap-6">
                    <span className="text-black/45 w-14">Token</span>
                    <span className="text-black font-medium text-xs">cUSDC</span>
                  </div>
                </div>
                <div className="border-l border-black/10 pl-5 text-right">
                  <span className="text-black/45 block text-[11px] mb-0.5">Amount</span>
                  <span className="font-mono text-black/50 text-xs tracking-widest">************</span>
                </div>
              </div>

              {/* Card 0: CENTER HERO ROW (Sharp, highlighted active card) */}
              <div className="w-full rounded-[16px] bg-white p-5 sm:p-6 shadow-[0_16px_40px_rgba(0,0,0,0.07),0_2px_6px_rgba(0,0,0,0.04)] border border-white relative z-10 scale-[1.02] flex items-center justify-between">
                {/* From / To Cluster */}
                <div className="flex items-center gap-3 sm:gap-4">
                  <div>
                    <div className="text-xs text-black/45 font-normal mb-1">
                      From
                    </div>
                    <div className="font-mono text-xs sm:text-sm font-normal text-black tracking-tight">
                      7xKX..94mQ
                    </div>
                  </div>

                  {/* Yellow Circle Action Button with Arrow */}
                  <div className="w-7 h-7 rounded-full bg-[#D8D4CB] flex items-center justify-center text-black shadow-xs shrink-0 mx-1">
                    <ChevronRightIcon className="w-3.5 h-3.5 text-black" />
                  </div>

                  <div>
                    <div className="text-xs text-black/45 font-normal mb-1">
                      To
                    </div>
                    <div className="font-mono text-xs sm:text-sm font-normal text-black tracking-tight">
                      9WzD..kP31
                    </div>
                  </div>
                </div>

                {/* Vertical Divider */}
                <div className="h-9 w-px bg-black/[0.08] mx-3 sm:mx-5 shrink-0" />

                {/* Amount with Masked Stars */}
                <div className="text-right">
                  <div className="text-xs text-black/45 font-normal mb-1">
                    Amount
                  </div>
                  <div className="font-mono text-xs sm:text-sm tracking-widest text-black/60 font-medium">
                    ************
                  </div>
                </div>
              </div>

              {/* Card +1: Lower Ghosted Row (Address / Token / Amount) */}
              <div className="w-full rounded-[14px] bg-white/65 backdrop-blur-sm p-4 sm:p-4.5 border border-white/60 shadow-xs opacity-65 flex items-center justify-between text-xs pointer-events-none select-none">
                <div className="space-y-1">
                  <div className="flex items-center gap-6">
                    <span className="text-black/45 w-14">Address</span>
                    <span className="font-mono text-black text-xs font-normal">9WzD..kP31</span>
                  </div>
                  <div className="flex items-center gap-6">
                    <span className="text-black/45 w-14">Token</span>
                    <span className="text-black font-medium text-xs">cSOL</span>
                  </div>
                </div>
                <div className="border-l border-black/10 pl-5 text-right">
                  <span className="text-black/45 block text-[11px] mb-0.5">Amount</span>
                  <span className="font-mono text-black/50 text-xs tracking-widest">************</span>
                </div>
              </div>

              {/* Card +2: Bottommost Faint Row */}
              <div className="w-full rounded-[14px] bg-white/35 backdrop-blur-xs p-3.5 sm:p-4 border border-white/40 shadow-2xs opacity-40 flex items-center justify-between text-xs text-black/60 pointer-events-none select-none">
                <div className="flex items-center gap-3">
                  <div>
                    <span className="text-[10px] text-black/40 block">From</span>
                    <span className="font-mono text-xs">5Eun..Tcje</span>
                  </div>
                  <div className="w-5 h-5 rounded-full bg-[#D8D4CB]/70 flex items-center justify-center">
                    <span className="text-[10px] text-black">›</span>
                  </div>
                  <div>
                    <span className="text-[10px] text-black/40 block">To</span>
                    <span className="font-mono text-xs">7xKX..94mQ</span>
                  </div>
                </div>
                <div className="border-l border-black/10 pl-4">
                  <span className="text-[10px] text-black/40 block">Amount</span>
                  <span className="font-mono text-xs text-black/50">••••••••••</span>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
