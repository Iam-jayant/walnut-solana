"use client";

import React from "react";
import { PartnerLogo } from "./partner-logo.tsx";

// Official logos live in /public/logos. Only tools Walnut actually uses are listed.
const DEVTOOLS: ReadonlyArray<{ name: string; logo: string; round?: boolean }> = [
  { name: "SOLANA", logo: "/logos/sol.png", round: true },
  { name: "ARCIUM", logo: "/logos/arcium.png" },
  { name: "PYTH NETWORK", logo: "/logos/pyth.png", round: true },
  { name: "PHANTOM", logo: "/logos/phantom.svg" },
  { name: "SOLFLARE", logo: "/logos/solflare.svg" },
  { name: "JUPITER", logo: "/logos/jupiter.png", round: true },
  { name: "RUST", logo: "/logos/rust.svg" },
  { name: "TYPESCRIPT", logo: "/logos/typescript.svg" },
];

export function EcosystemPartners() {
  return (
    <section id="ecosystem" className="py-8 sm:py-12 bg-[#fbfbfb] overflow-hidden">
      <p className="mb-4 text-center text-[11px] font-medium uppercase tracking-[0.18em] text-black/40">
        Built with
      </p>
      {/* Continuous horizontal marquee of official logos */}
      <div className="relative w-full overflow-hidden py-3 [mask-image:linear-gradient(to_right,transparent_0%,black_10%,black_90%,transparent_100%)]">
        <div className="animate-devtools-slide flex w-max items-center gap-10 sm:gap-14">
          {[...DEVTOOLS, ...DEVTOOLS, ...DEVTOOLS, ...DEVTOOLS].map((item, idx) => (
            <div
              key={`${item.name}-${idx}`}
              className="flex items-center gap-2.5 sm:gap-3 shrink-0 select-none opacity-80 hover:opacity-100 hover:scale-[1.03] transition-all cursor-default"
            >
              <PartnerLogo
                src={item.logo}
                name={item.name}
                size={26}
                className={item.round ? "rounded-full" : "rounded-[6px]"}
              />
              <span className="text-[13px] sm:text-[14.5px] font-bold tracking-[0.12em] text-[#556987] hover:text-[#111111] uppercase whitespace-nowrap transition-colors">
                {item.name}
              </span>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
