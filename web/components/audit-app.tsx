"use client";

import "../lib/polyfill.ts";
import "@solana/wallet-adapter-react-ui/styles.css";
import { WalletProvider } from "@solana/wallet-adapter-react";
import { WalletModalProvider } from "@solana/wallet-adapter-react-ui";
import { AuditView } from "./audit-view.tsx";
import { Header } from "./header.tsx";
import { WalnutProvider } from "./walnut-provider.tsx";

export default function AuditApp() {
  return (
    <WalletProvider wallets={[]} autoConnect>
      <WalletModalProvider>
        <WalnutProvider>
          <Header />
          <main>
            <AuditView />
          </main>
        </WalnutProvider>
      </WalletModalProvider>
    </WalletProvider>
  );
}
