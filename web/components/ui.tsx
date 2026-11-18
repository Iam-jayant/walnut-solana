"use client";

import type { ButtonHTMLAttributes, ReactNode } from "react";

export function Card({
  children,
  className = "",
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <section
      className={`rounded-card bg-walnut-white p-6 shadow-[0_1px_0_rgba(0,0,0,0.04)] ring-1 ring-black/5 ${className}`}
    >
      {children}
    </section>
  );
}

export function CardTitle({
  children,
  right,
}: {
  children: ReactNode;
  right?: ReactNode;
}) {
  return (
    <div className="mb-4 flex items-center justify-between gap-3">
      <h2 className="text-h3 font-[800] tracking-tight">{children}</h2>
      {right}
    </div>
  );
}

type Variant = "primary" | "dark" | "ghost";

const VARIANTS: Record<Variant, string> = {
  primary:
    "bg-walnut-yellow text-walnut-black hover:brightness-95 disabled:bg-walnut-mist disabled:text-walnut-ink/40",
  dark: "bg-walnut-black text-walnut-white hover:bg-walnut-ink disabled:bg-walnut-mist disabled:text-walnut-ink/40",
  ghost:
    "bg-transparent text-walnut-black ring-1 ring-black/15 hover:bg-walnut-mist disabled:text-walnut-ink/40",
};

export function Button({
  variant = "primary",
  className = "",
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant }) {
  return (
    <button
      {...props}
      className={`inline-flex items-center justify-center gap-2 rounded-pill px-5 py-2.5 text-small font-[800] transition duration-150 disabled:cursor-not-allowed ${VARIANTS[variant]} ${className}`}
    />
  );
}

export function Badge({
  children,
  tone = "neutral",
}: {
  children: ReactNode;
  tone?: "neutral" | "dark" | "good" | "warn" | "bad";
}) {
  const tones = {
    neutral: "bg-walnut-mist text-walnut-ink",
    dark: "bg-walnut-black text-walnut-yellow",
    good: "bg-emerald-100 text-emerald-800",
    warn: "bg-amber-100 text-amber-900",
    bad: "bg-red-100 text-red-800",
  } as const;
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-pill px-2.5 py-1 text-caption font-[800] ${tones[tone]}`}
    >
      {children}
    </span>
  );
}

export function LockIcon({
  className = "h-3.5 w-3.5",
}: {
  className?: string;
}) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.2"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden
    >
      <rect x="4" y="11" width="16" height="10" rx="2" />
      <path d="M8 11V7a4 4 0 0 1 8 0v4" />
    </svg>
  );
}

export function Spinner() {
  return (
    <span
      className="inline-block h-3.5 w-3.5 animate-spin rounded-full border-2 border-current border-t-transparent"
      aria-hidden
    />
  );
}

export function Notice({
  tone = "warn",
  children,
}: {
  tone?: "warn" | "bad" | "info";
  children: ReactNode;
}) {
  const tones = {
    warn: "bg-amber-50 text-amber-900 ring-amber-200",
    bad: "bg-red-50 text-red-800 ring-red-200",
    info: "bg-walnut-mist text-walnut-ink ring-black/5",
  } as const;
  return (
    <div
      className={`rounded-2xl px-4 py-3 text-small ring-1 ${tones[tone]}`}
      role={tone === "bad" ? "alert" : "status"}
    >
      {children}
    </div>
  );
}
