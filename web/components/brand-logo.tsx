import React, { useId } from "react";

/**
 * The Walnut mark: a coin split along a walnut seam.
 * Two tones, transparent gap (works on any background).
 * See /brand in the repo root for usage rules.
 */
export type WalnutLogoTone = "ink" | "bone";

const TONES: Record<WalnutLogoTone, { left: string; right: string }> = {
  ink: { left: "#111111", right: "#8C8880" },
  bone: { left: "#F4F2EE", right: "#8C8880" },
};

const SEAM = "M100 192 C70 130 130 70 100 8";

export function WalnutLogo({
  size = 32,
  tone = "ink",
  className,
}: {
  size?: number;
  tone?: WalnutLogoTone;
  className?: string;
}) {
  const id = useId().replace(/:/g, "");
  const c = TONES[tone];
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 200 200"
      className={className}
      role="img"
      aria-label="Walnut"
      style={{ flexShrink: 0 }}
    >
      <defs>
        <mask id={`seam-${id}`}>
          <rect width="200" height="200" fill="#fff" />
          <path d={SEAM} stroke="#000" strokeWidth="9" fill="none" strokeLinecap="round" />
        </mask>
      </defs>
      <g mask={`url(#seam-${id})`}>
        <path d={`M100 8 A92 92 0 0 0 100 192 C70 130 130 70 100 8 Z`} fill={c.left} />
        <path d={`M100 8 A92 92 0 0 1 100 192 C70 130 130 70 100 8 Z`} fill={c.right} />
      </g>
    </svg>
  );
}

/** Mark + wordmark, for headers and footers. */
export function WalnutLockup({
  size = 28,
  tone = "ink",
  className = "",
}: {
  size?: number;
  tone?: WalnutLogoTone;
  className?: string;
}) {
  return (
    <span className={`inline-flex items-center gap-2.5 ${className}`}>
      <WalnutLogo size={size} tone={tone} />
      <span
        className={`font-extrabold leading-none tracking-tight ${tone === "bone" ? "text-[#F4F2EE]" : "text-[#111111]"}`}
        style={{ fontSize: size * 0.8 }}
      >
        Walnut
      </span>
    </span>
  );
}
