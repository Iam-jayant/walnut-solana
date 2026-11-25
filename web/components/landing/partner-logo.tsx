"use client";

import React, { useState } from "react";

/**
 * Official brand logo from /public/logos. If the file is missing, a neutral
 * monogram is shown so the layout never breaks.
 */
export function PartnerLogo({
  src,
  name,
  size = 24,
  className = "",
}: {
  src: string;
  name: string;
  size?: number;
  className?: string;
}) {
  const [failed, setFailed] = useState(false);

  if (failed) {
    return (
      <span
        className={`inline-flex shrink-0 items-center justify-center rounded-md bg-black text-[11px] font-bold text-white ${className}`}
        style={{ width: size, height: size }}
        aria-label={name}
      >
        {name.slice(0, 1).toUpperCase()}
      </span>
    );
  }

  return (
    // eslint-disable-next-line @next/next/no-img-element
    <img
      src={src}
      alt={name}
      width={size}
      height={size}
      draggable={false}
      onError={() => setFailed(true)}
      className={`shrink-0 object-contain ${className}`}
      style={{ width: size, height: size }}
    />
  );
}
