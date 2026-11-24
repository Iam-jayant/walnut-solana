"use client";

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
        <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" fill="#FFD208" />
        <path
          d="M8.5 12l2.5 2.5 4.5-4.5"
          stroke="#000000"
          strokeWidth="2.5"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    </span>
  );
}

/** Walnut's own mark (cWALNUT), the shell emblem on brand yellow. */
export function WalnutMark({ size = 40 }: { size?: number }) {
  return (
    <span
      className="flex items-center justify-center rounded-full bg-[#FFD208] shadow-[0_2px_8px_rgba(255,210,8,0.3)]"
      style={{ width: size, height: size }}
    >
      <svg style={{ width: size * 0.55, height: size * 0.55 }} viewBox="0 0 24 24" fill="none">
        <path
          d="M12 2.2C11.2 2.2 9.7 3.4 8.8 5C6.9 7.8 5.8 10.7 5.8 13.5C5.8 17.6 8 20.8 11.1 21.6C11.4 21.7 11.7 21.8 12 21.8C12.3 21.8 12.6 21.7 12.9 21.6C16 20.8 18.2 17.6 18.2 13.5C18.2 10.7 17.1 7.8 15.2 5C14.3 3.4 12.8 2.2 12 2.2Z"
          fill="#0A0A0A"
        />
        <path d="M12 4.2V19.8C9.7 18.8 7.8 16.4 7.8 13.5C7.8 11 8.6 8.6 10.1 6.3C10.6 5.5 11.3 4.8 12 4.2Z" fill="#FFD208" />
        <path d="M12 4.2C12.7 4.8 13.4 5.5 13.9 6.3C15.4 8.6 16.2 11 16.2 13.5C16.2 16.4 14.3 18.8 12 19.8V4.2Z" fill="#FFC400" />
        <path d="M12 3.2V20.8" stroke="#0A0A0A" strokeWidth="1.5" strokeLinecap="round" />
        <path d="M9.9 9.8C9.2 10.8 8.8 12 8.8 13.5C8.8 14.9 9.3 16.2 10.2 17.1" stroke="#0A0A0A" strokeWidth="1.3" strokeLinecap="round" />
        <path d="M14.1 9.8C14.8 10.8 15.2 12 15.2 13.5C15.2 14.9 14.7 16.2 13.8 17.1" stroke="#0A0A0A" strokeWidth="1.3" strokeLinecap="round" />
      </svg>
    </span>
  );
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
