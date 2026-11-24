"use client";

import React, { useMemo } from "react";

/**
 * Profile picture for a connected wallet.
 *
 * Generated locally from the wallet address, so every wallet gets its own
 * stable picture without any external image request (keeps the CSP strict).
 */

function fnv1a(input: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < input.length; i++) {
    h ^= input.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

function mulberry32(seed: number): () => number {
  let a = seed;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Builds a symmetric 5x5 identicon as an SVG data URI. */
export function avatarDataUri(seed: string): string {
  const rand = mulberry32(fnv1a(seed));
  const hue = Math.floor(rand() * 360);
  const hue2 = (hue + 40 + Math.floor(rand() * 80)) % 360;
  const bg = `hsl(${hue} 70% 93%)`;
  const fg = `hsl(${hue} 62% 40%)`;
  const accent = `hsl(${hue2} 75% 55%)`;

  let cells = "";
  for (let y = 0; y < 5; y++) {
    for (let x = 0; x < 3; x++) {
      const v = rand();
      if (v < 0.5) continue;
      const color = v > 0.85 ? accent : fg;
      const mirror = 4 - x;
      cells += `<rect x="${x * 4 + 2}" y="${y * 4 + 2}" width="4" height="4" rx="1" fill="${color}"/>`;
      if (mirror !== x) {
        cells += `<rect x="${mirror * 4 + 2}" y="${y * 4 + 2}" width="4" height="4" rx="1" fill="${color}"/>`;
      }
    }
  }
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect width="24" height="24" fill="${bg}"/>${cells}</svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}

export function WalletAvatar({
  address,
  size = 32,
  className = "",
}: {
  address: string;
  size?: number;
  className?: string;
}) {
  const src = useMemo(() => avatarDataUri(address), [address]);
  return (
    // eslint-disable-next-line @next/next/no-img-element
    <img
      src={src}
      alt="Wallet profile picture"
      width={size}
      height={size}
      draggable={false}
      className={`shrink-0 rounded-full object-cover ring-1 ring-black/10 ${className}`}
      style={{ width: size, height: size }}
    />
  );
}
