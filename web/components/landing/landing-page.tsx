"use client";

import React from "react";
import { Hero } from "./hero.tsx";
import { ProductShowcase } from "./product-showcase.tsx";
import { Industries } from "./industries.tsx";
import { ConfidentialLedger } from "./confidential-ledger.tsx";
import { ArchitecturePillars } from "./architecture-pillars.tsx";
import { EcosystemPartners } from "./ecosystem-partners.tsx";
import { CodeTerminal } from "./code-terminal.tsx";
import { FinalCta } from "./final-cta.tsx";
import { Footer } from "./footer.tsx";

export function LandingPage() {
  return (
    <div className="min-h-screen bg-[#fbfbfb] text-[#111111] font-sans antialiased selection:bg-[#FFE052] selection:text-black">
      {/* 1. Hero Section */}
      <Hero />

      {/* 2. Product Showcase (Shield, Earn, Swap) */}
      <ProductShowcase />

      {/* 4. Built for Industries (Bento Grid) */}
      <Industries />

      {/* 5. Feature Split with Confidential Ledger Table Preview */}
      <ConfidentialLedger />

      {/* 6. Dual Architectural Pillars (Light & Dark Contrast) */}
      <ArchitecturePillars />

      {/* 7. Ecosystem Partners & Infrastructure Spotlight */}
      <EcosystemPartners />

      {/* 8. Developer Code Terminal */}
      <CodeTerminal />

      {/* 9. Final Conversion Banner */}
      <FinalCta />

      {/* 10. Multi-Column Footer + Giant WALNUT Watermark */}
      <Footer />
    </div>
  );
}
