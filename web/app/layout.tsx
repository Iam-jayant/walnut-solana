import type { Metadata } from "next";
import localFont from "next/font/local";
import "./globals.css";

/*
 * Telegraf is loaded locally (Task 15.1) so no font request leaves the origin
 * while a Viewing Key is unlocked (Requirement 14, strict CSP). The three
 * weights map to the design scale: 200 UltraLight, 400 Regular, 800 UltraBold.
 */
const telegraf = localFont({
  src: [
    {
      path: "./fonts/Telegraf-UltraLight.otf",
      weight: "200",
      style: "normal",
    },
    { path: "./fonts/Telegraf-Regular.otf", weight: "400", style: "normal" },
    { path: "./fonts/Telegraf-UltraBold.otf", weight: "800", style: "normal" },
  ],
  variable: "--font-telegraf",
  display: "swap",
  fallback: ["ui-sans-serif", "system-ui", "sans-serif"],
});

export const metadata: Metadata = {
  title: "Walnut — Confidential lending on Solana",
  description:
    "Walnut is a private SOL/USDC lending protocol on Solana, with balances and positions kept encrypted through Arcium MPC.",
};

export default function RootLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en" className={telegraf.variable}>
      <body className="bg-walnut-white font-sans text-walnut-black antialiased">
        {children}
      </body>
    </html>
  );
}
