import type { Metadata, Viewport } from "next";
import localFont from "next/font/local";
import "./globals.css";

/*
 * Telegraf is loaded locally so no font request leaves the origin
 * while a Viewing Key is unlocked (Requirement 14, strict CSP). The three
 * weights map to the design scale: 200 UltraLight, 400 Regular, 800 UltraBold.
 */
const telegraf = localFont({
  src: [
    {
      path: "./fonts/Telegraf-UltraLight.otf",
      weight: "100",
      style: "normal",
    },
    {
      path: "./fonts/Telegraf-UltraLight.otf",
      weight: "200",
      style: "normal",
    },
    {
      path: "./fonts/Telegraf-UltraLight.otf",
      weight: "300",
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
  title: "Walnut - Private lending on Solana",
  description:
    "Walnut is private lending on Solana powered by Arcium MPC. Balances, collateral and debt are encrypted, and only you can decrypt them with your Viewing Key.",
  openGraph: {
    title: "Walnut - Private lending on Solana",
    description: "Private lending on Solana, powered by Arcium MPC.",
    images: [{ url: "/og-image.png", width: 1200, height: 630 }],
  },
  twitter: {
    card: "summary_large_image",
    title: "Walnut - Private lending on Solana",
    description: "Private lending on Solana, powered by Arcium MPC.",
    images: ["/og-image.png"],
  },
};

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
  viewportFit: "cover",
};

export default function RootLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en" className={telegraf.variable} suppressHydrationWarning>
      <body
        className="bg-[#F5F3EF] font-sans text-walnut-black antialiased"
        suppressHydrationWarning
      >
        {children}
      </body>
    </html>
  );
}
