"use client";

import "../lib/polyfill.ts";
import "@solana/wallet-adapter-react-ui/styles.css";
import { WalletProvider } from "@solana/wallet-adapter-react";
import { WalletModalProvider } from "@solana/wallet-adapter-react-ui";
import { NETWORK_LABEL } from "../lib/config.ts";
import { Dashboard } from "./dashboard.tsx";
import { Header } from "./header.tsx";
import { Hero } from "./hero.tsx";
import { WalnutProvider } from "./walnut-provider.tsx";

export default function App() {
  // Wallet Standard wallets (Phantom, Solflare, Backpack, ...) are detected
  // automatically, so no adapter list is needed.
  return (
    <WalletProvider wallets={[]} autoConnect>
      <WalletModalProvider>
        <WalnutProvider>
          <Header />
          <main>
            <Hero />
            <Dashboard />
          </main>
          <footer className="mx-auto max-w-(--container-page) px-6 py-10 text-caption text-walnut-ink/60 md:px-10">
            Walnut · Private lending on Solana, powered by Arcium MPC ·{" "}
            {NETWORK_LABEL}
          </footer>
        </WalnutProvider>
      </WalletModalProvider>
    </WalletProvider>
  );
}
