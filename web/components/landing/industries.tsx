"use client";

import React from "react";

// Minimalist Line Icons matching Walnut & Zama aesthetic
function VaultIcon() {
  return (
    <svg
      className="w-6 h-6 text-black/60"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.3"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <circle cx="12" cy="12" r="4" />
      <path d="M12 8v2M12 14v2M8 12h2M14 12h2" />
    </svg>
  );
}

function ShieldLockIcon() {
  return (
    <svg
      className="w-6 h-6 text-black/60"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.3"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
      <circle cx="12" cy="11" r="2.5" />
      <path d="M12 13.5v2.5" />
    </svg>
  );
}

function CandlestickIcon() {
  return (
    <svg
      className="w-6 h-6 text-black/60"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.3"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M3 3v17a1 1 0 001 1h17" />
      <line x1="8" y1="6" x2="8" y2="17" />
      <rect x="6.5" y="8" width="3" height="6" rx="0.5" fill="none" />
      <line x1="15" y1="5" x2="15" y2="15" />
      <rect x="13.5" y="7" width="3" height="5" rx="0.5" fill="none" />
    </svg>
  );
}

function ViewingKeyIcon() {
  return (
    <svg
      className="w-6 h-6 text-black/60"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.3"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <circle cx="8" cy="15" r="4" />
      <path d="M10.8 12.2L19 4M15 8l3 3M17 6l2 2" />
    </svg>
  );
}

export function Industries() {
  return (
    <section id="industries" className="py-16 sm:py-24 bg-[#F5F3EF]">
      {/* Standard 75px side margins */}
      <div className="w-full px-4 sm:px-8 lg:px-[75px]">
        {/* Section Header: Left-aligned, two-line light grotesque headline */}
        <div className="max-w-3xl mb-12 sm:mb-14">
          <h2
            className="text-3xl sm:text-4xl lg:text-[46px] xl:text-[50px] tracking-tight text-black mb-3 select-none"
            style={{
              fontFamily: "var(--font-telegraf), ui-sans-serif, system-ui, sans-serif",
              fontWeight: 200,
              lineHeight: 1.08,
            }}
          >
            Built for onchain finance where <br />
            confidentiality isn&apos;t optional
          </h2>
          <p className="max-w-2xl text-[13.5px] sm:text-[14.5px] font-normal leading-relaxed text-black/75">
            From encrypted collateral to selective compliance — Walnut delivers native privacy to Solana L1, preventing toxic MEV and liquidation hunting.
          </p>
        </div>

        {/* 4 Warm Linen Cards (#F2EFEC) */}
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-5 sm:gap-6">
          {/* Card 1: Confidential Lending & Borrowing */}
          <div className="rounded-[16px] sm:rounded-[18px] bg-[#F2EFEC] p-6 sm:p-7 lg:p-8 min-h-[300px] sm:min-h-[320px] flex flex-col justify-between transition-all duration-200 hover:shadow-xs">
            <div>
              <VaultIcon />
            </div>
            <div>
              <h3 className="text-lg sm:text-[20px] font-light text-black leading-snug mb-2.5 tracking-tight">
                Confidential Lending <br />
                &amp; Borrowing
              </h3>
              <p className="text-[12.5px] sm:text-[13px] leading-relaxed text-black/60 font-normal">
                Supply assets to earn private yield or post SOL collateral to borrow without broadcasting loan sizes, leverage, or debt positions.
              </p>
            </div>
          </div>

          {/* Card 2: Encrypted Solvency & Health Factors */}
          <div className="rounded-[16px] sm:rounded-[18px] bg-[#F2EFEC] p-6 sm:p-7 lg:p-8 min-h-[300px] sm:min-h-[320px] flex flex-col justify-between transition-all duration-200 hover:shadow-xs">
            <div>
              <ShieldLockIcon />
            </div>
            <div>
              <h3 className="text-lg sm:text-[20px] font-light text-black leading-snug mb-2.5 tracking-tight">
                Encrypted Solvency <br />
                &amp; Health Factors
              </h3>
              <p className="text-[12.5px] sm:text-[13px] leading-relaxed text-black/60 font-normal">
                Arcium MPC circuits verify account solvency and health ratios in multi-party computation, eliminating predatory liquidation hunting.
              </p>
            </div>
          </div>

          {/* Card 3: Shield Vaults & Private Yield */}
          <div className="rounded-[16px] sm:rounded-[18px] bg-[#F2EFEC] p-6 sm:p-7 lg:p-8 min-h-[300px] sm:min-h-[320px] flex flex-col justify-between transition-all duration-200 hover:shadow-xs">
            <div>
              <CandlestickIcon />
            </div>
            <div>
              <h3 className="text-lg sm:text-[20px] font-light text-black leading-snug mb-2.5 tracking-tight">
                Shield Vaults <br />
                &amp; Private Yield
              </h3>
              <p className="text-[12.5px] sm:text-[13px] leading-relaxed text-black/60 font-normal">
                Deposit USDC to mint encrypted cUSDC 1:1 and compound onchain yield through curated vaults with balance confidentiality.
              </p>
            </div>
          </div>

          {/* Card 4: Selective Compliance & Viewing Keys */}
          <div className="rounded-[16px] sm:rounded-[18px] bg-[#F2EFEC] p-6 sm:p-7 lg:p-8 min-h-[300px] sm:min-h-[320px] flex flex-col justify-between transition-all duration-200 hover:shadow-xs">
            <div>
              <ViewingKeyIcon />
            </div>
            <div>
              <h3 className="text-lg sm:text-[20px] font-light text-black leading-snug mb-2.5 tracking-tight">
                Selective Compliance <br />
                &amp; Viewing Keys
              </h3>
              <p className="text-[12.5px] sm:text-[13px] leading-relaxed text-black/60 font-normal">
                Generate cryptographic Viewing Keys to selectively disclose positions and audit history to verifiers without making data public.
              </p>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
