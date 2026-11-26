"use client";

import { WalnutLogo } from "./brand-logo.tsx";
import React from "react";

/**
 * Shared token artwork for the whole app.
 *
 * Uses the official USDC and SOL logo files in /public/logos so every page
 * (landing, wallet screen, action panels, liquidations) shows the same real
 * coin logos. Confidential tokens (cSOL, cUSDC, cWALNUT) get the golden
 * shield badge.
 */

const OFFICIAL_LOGOS: Record<string, string> = {
  SOL: "/logos/sol.png",
  USDC: "/logos/usdc.png",
};

/** Splits "cUSDC" into { confidential: true, base: "USDC" }. */
export function parseTokenSymbol(symbol: string): {
  confidential: boolean;
  base: string;
} {
  const first = symbol.trim().split(/\s+/)[0] ?? symbol;
  const confidential = /^c[A-Z]/.test(first);
  const base = (confidential ? first.slice(1) : first).toUpperCase();
  return { confidential, base };
}

export function ConfidentialBadge({ size = 14 }: { size?: number }) {
  return (
    <span
      className="absolute -bottom-0.5 -right-0.5 flex items-center justify-center rounded-full bg-white shadow-[0_1px_3px_rgba(0,0,0,0.25)]"
      style={{ width: size, height: size }}
    >
      <svg
        style={{ width: size * 0.78, height: size * 0.78 }}
        viewBox="0 0 24 24"
        fill="none"
      >
        <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" fill="#111111" />
        <path
          d="M8.5 12l2.5 2.5 4.5-4.5"
          stroke="#FFFFFF"
          strokeWidth="2.5"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    </span>
  );
}

/** Walnut's own mark (cWALNUT): the seam coin. */
export function WalnutMark({ size = 40 }: { size?: number }) {
  return <WalnutLogo size={size} />;
}

export function TokenLogo({
  symbol,
  size = 24,
  className = "",
}: {
  symbol: string;
  size?: number;
  className?: string;
}) {
  const { confidential, base } = parseTokenSymbol(symbol);
  const src = OFFICIAL_LOGOS[base.startsWith("SOL") ? "SOL" : base];
  const badge = Math.max(12, Math.round(size * 0.42));

  return (
    <span
      className={`relative inline-flex shrink-0 items-center justify-center ${className}`}
      style={{ width: size, height: size }}
    >
      {base === "WALNUT" ? (
        <WalnutMark size={size} />
      ) : src ? (
        // eslint-disable-next-line @next/next/no-img-element
        <img
          src={src}
          alt={base}
          width={size}
          height={size}
          draggable={false}
          className="h-full w-full rounded-full object-cover"
        />
      ) : (
        <span
          className="flex h-full w-full items-center justify-center rounded-full bg-black/10 text-[10px] font-bold text-black/60"
          aria-label={base}
        >
          {base.slice(0, 1)}
        </span>
      )}
      {confidential && <ConfidentialBadge size={badge} />}
    </span>
  );
}
