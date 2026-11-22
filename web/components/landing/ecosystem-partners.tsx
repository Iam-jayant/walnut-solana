"use client";

import React from "react";

// Official & Stylized DevTools / Ecosystem Logos
function SolanaIcon({ className = "w-[19px] h-[15px]" }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 397 311" fill="currentColor">
      <path d="M64.6 237.9c2.4-2.4 5.7-3.8 9.2-3.8h317.4c5.8 0 8.7 7 4.6 11.1l-62.7 62.7c-2.4 2.4-5.7 3.8-9.2 3.8H6.5c-5.8 0-8.7-7-4.6-11.1l62.7-62.7zM64.6 3.8C67.1 1.4 70.4 0 73.8 0h317.4c5.8 0 8.7 7 4.6 11.1l-62.7 62.7c-2.4 2.4-5.7 3.8-9.2 3.8H6.5c-5.8 0-8.7-7-4.6-11.1L64.6 3.8zM332.4 120.9c-2.4-2.4-5.7-3.8-9.2-3.8H5.8c-5.8 0-8.7 7-4.6 11.1l62.7 62.7c2.4 2.4 5.7 3.8 9.2 3.8h317.4c5.8 0 8.7-7 4.6-11.1l-62.7-62.7z" />
    </svg>
  );
}

function ArciumIcon({ className = "w-[19px] h-[19px]" }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <polygon points="12 2 2 7 12 12 22 7 12 2" />
      <polyline points="2 17 12 22 22 17" />
      <polyline points="2 12 12 17 22 12" />
    </svg>
  );
}

function PythIcon({ className = "w-[19px] h-[19px]" }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none">
      <circle cx="12" cy="12" r="10" fill="#5E35B1" />
      <path d="M8 8.5h8M10 8.5v7M14 8.5v7" stroke="#FFFFFF" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );
}

function PhantomIcon({ className = "w-[19px] h-[19px]" }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="#AB9FF2">
      <path d="M12 2C6.477 2 2 6.477 2 12c0 3.313 1.611 6.249 4.093 8.047.534.388 1.25.132 1.45-.487.358-1.11.838-2.227 1.543-3.193.308-.422.842-.61 1.343-.497.666.15 1.353.23 2.053.23 4.142 0 7.518-3.376 7.518-7.518C20 5.485 16.418 2 12 2zM9 11a1.5 1.5 0 110-3 1.5 1.5 0 010 3zm6 0a1.5 1.5 0 110-3 1.5 1.5 0 010 3z" />
    </svg>
  );
}

function SolflareIcon({ className = "w-[19px] h-[19px]" }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none">
      <circle cx="12" cy="12" r="10" fill="#FC8C00" />
      <path d="M12 6.5l1.8 3.5 4 .9-3 3 .8 4.1-3.6-2.2-3.6 2.2.8-4.1-3-3 4-.9 1.8-3.5z" fill="#FFFFFF" />
    </svg>
  );
}

function AnchorIcon({ className = "w-[19px] h-[19px]" }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="5" r="3" />
      <line x1="12" y1="22" x2="12" y2="8" />
      <path d="M5 12H2a10 10 0 0020 0h-3" />
    </svg>
  );
}

function RustIcon({ className = "w-[19px] h-[19px]" }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="#CE422B">
      <path d="M12 2a2 2 0 00-2 2v.2A9.98 9.98 0 006 6.3V6a2 2 0 00-4 0v1.5a10.03 10.03 0 000 9A2 2 0 002 18v.5a2 2 0 004 0v-.3a9.98 9.98 0 004 2.1v.2a2 2 0 004 0v-.2a9.98 9.98 0 004-2.1v.3a2 2 0 004 0V18a2 2 0 000-1.5 10.03 10.03 0 000-9V6a2 2 0 00-4 0v.3a9.98 9.98 0 00-4-2.1V4a2 2 0 00-2-2zm-3 6h6a3 3 0 013 3 3 3 0 01-3 3h-2v3h-2V8h2a1 1 0 001-1 1 1 0 00-1-1H9v6z" />
    </svg>
  );
}

function TypeScriptIcon({ className = "w-[19px] h-[19px]" }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none">
      <rect width="24" height="24" rx="4" fill="#3178C6" />
      <path d="M6 10h6M9 10v9M14 15.5c.8.8 2 1.3 3.3 1.3 1.2 0 2-.5 2-1.3 0-2-4.5-1.5-4.5-4.2 0-1.7 1.4-2.8 3.5-2.8 1.4 0 2.5.4 3.2 1l-1 1.7c-.6-.5-1.4-.8-2.2-.8-1 0-1.6.4-1.6 1 0 1.9 4.5 1.3 4.5 4.3 0 1.8-1.4 2.8-3.7 2.8-1.6 0-3-.5-4-1.4l1.2-1.6z" stroke="#FFFFFF" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function BackpackIcon({ className = "w-[19px] h-[19px]" }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none">
      <rect x="4" y="6" width="16" height="15" rx="3" fill="#E54335" />
      <path d="M9 6V4a3 3 0 016 0v2M8 12h8" stroke="#FFFFFF" strokeWidth="1.8" strokeLinecap="round" />
    </svg>
  );
}

function JupiterIcon({ className = "w-[19px] h-[19px]" }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none">
      <circle cx="12" cy="12" r="9" fill="#17181F" />
      <ellipse cx="12" cy="12" rx="9" ry="3.5" stroke="#FBA43A" strokeWidth="1.6" transform="rotate(-25 12 12)" />
      <circle cx="12" cy="12" r="5" fill="#E9882B" />
    </svg>
  );
}

const DEVTOOLS = [
  { name: "SOLANA", icon: SolanaIcon },
  { name: "ARCIUM", icon: ArciumIcon },
  { name: "PYTH NETWORK", icon: PythIcon },
  { name: "PHANTOM", icon: PhantomIcon },
  { name: "SOLFLARE", icon: SolflareIcon },
  { name: "ANCHOR", icon: AnchorIcon },
  { name: "RUST", icon: RustIcon },
  { name: "TYPESCRIPT", icon: TypeScriptIcon },
  { name: "BACKPACK", icon: BackpackIcon },
  { name: "JUPITER", icon: JupiterIcon },
];

export function EcosystemPartners() {
  return (
    <section id="ecosystem" className="py-8 sm:py-12 bg-[#fbfbfb] overflow-hidden">
      {/* Continuous Horizontal Sliding Marquee - Logos with Text, Completely Borderless & Transparent */}
      <div className="relative w-full overflow-hidden py-3 [mask-image:linear-gradient(to_right,transparent_0%,black_10%,black_90%,transparent_100%)]">
        <div className="animate-devtools-slide flex w-max items-center gap-8 sm:gap-12">
          {[...DEVTOOLS, ...DEVTOOLS, ...DEVTOOLS, ...DEVTOOLS].map((item, idx) => {
            const Icon = item.icon;
            return (
              <div
                key={`${item.name}-${idx}`}
                className="flex items-center gap-2.5 sm:gap-3 shrink-0 select-none opacity-80 hover:opacity-100 hover:scale-[1.03] transition-all cursor-default"
              >
                <Icon className="w-[19px] h-[19px] shrink-0 text-black/85" />
                <span className="text-[13px] sm:text-[14.5px] font-bold tracking-[0.12em] text-[#556987] hover:text-[#111111] uppercase whitespace-nowrap transition-colors">
                  {item.name}
                </span>
              </div>
            );
          })}
        </div>
      </div>
    </section>
  );
}
