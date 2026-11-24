"use client";

import { useState } from "react";
import "../lib/polyfill.ts";
import "@solana/wallet-adapter-react-ui/styles.css";
import { WalletProvider, useWallet } from "@solana/wallet-adapter-react";
import {
  WalletModalProvider,
  useWalletModal,
} from "@solana/wallet-adapter-react-ui";
import { NETWORK_LABEL } from "../lib/config.ts";
import { AppSidebar, type AppTab } from "./app-sidebar.tsx";
import { AppHeaderBar } from "./app-header-bar.tsx";
import { AppWelcome } from "./app-welcome.tsx";
import { Dashboard } from "./dashboard.tsx";
import { WalnutProvider, useWalnut } from "./walnut-provider.tsx";

function AppMain() {
  const [activeTab, setActiveTab] = useState<AppTab>("dashboard");
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [isCollapsed, setIsCollapsed] = useState(false);
  const { connected, publicKey, connecting, disconnect } = useWallet();
  const { setVisible } = useWalletModal();
  const { unlocked, lock } = useWalnut();

  return (
    <div className="min-h-dvh flex bg-[#FAFAFA] bg-[radial-gradient(ellipse_55%_45%_at_0%_0%,_#FCF0B6_0%,_transparent_100%),radial-gradient(ellipse_35%_35%_at_100%_100%,_#E2E9F6_0%,_transparent_100%)] text-gray-900 font-app-sans antialiased relative overflow-x-hidden">
      {/* Mobile Backdrop Overlay when sidebar is open */}
      {sidebarOpen && (
        <div
          className="fixed inset-0 bg-black/30 backdrop-blur-xs z-40 lg:hidden transition-opacity"
          onClick={() => setSidebarOpen(false)}
        />
      )}

      {/* Responsive Openable/Closeable Left Sidebar */}
      <AppSidebar
        activeTab={activeTab}
        onSelectTab={setActiveTab}
        connected={connected}
        publicKey={publicKey}
        connecting={connecting}
        onConnect={() => setVisible(true)}
        onDisconnect={() => void disconnect()}
        unlocked={unlocked}
        onLock={lock}
        isOpen={sidebarOpen}
        onClose={() => setSidebarOpen(false)}
        isCollapsed={isCollapsed}
        onToggleCollapse={() => setIsCollapsed(!isCollapsed)}
      />

      {/* Main Canvas Area */}
      <div className="flex-1 flex flex-col min-w-0 h-dvh overflow-y-auto">
        {connected ? (
          <>
            <AppHeaderBar
              activeTab={activeTab}
              onToggleSidebar={() => setSidebarOpen(!sidebarOpen)}
            />
            <main className="flex-1 p-4 md:p-6 lg:p-7">
              <Dashboard activeTab={activeTab} onSelectTab={setActiveTab} />
            </main>
          </>
        ) : (
          <main className="flex-1 flex flex-col h-full w-full p-4 md:p-6 lg:p-7 relative">
            {/* Mobile Top Menu Button when disconnected */}
            <div className="lg:hidden absolute top-5 left-5 z-20">
              <button
                type="button"
                onClick={() => setSidebarOpen(!sidebarOpen)}
                className="p-2 rounded-lg bg-white/80 backdrop-blur border border-black/[0.06] text-gray-700 shadow-2xs hover:bg-white"
                title="Toggle menu"
                aria-label="Toggle navigation"
              >
                <svg
                  className="w-5 h-5"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <line x1="3" y1="12" x2="21" y2="12" />
                  <line x1="3" y1="6" x2="21" y2="6" />
                  <line x1="3" y1="18" x2="21" y2="18" />
                </svg>
              </button>
            </div>

            <AppWelcome
              onConnect={() => setVisible(true)}
              connecting={connecting}
            />
          </main>
        )}
      </div>
    </div>
  );
}

import { ToastProvider } from "./toast.tsx";

export default function App() {
  // Wallet Standard wallets (Phantom, Solflare, Backpack, ...) are detected
  // automatically, so no adapter list is needed.
  return (
    <WalletProvider wallets={[]} autoConnect>
      <WalletModalProvider>
        <ToastProvider>
          <WalnutProvider>
            <AppMain />
          </WalnutProvider>
        </ToastProvider>
      </WalletModalProvider>
    </WalletProvider>
  );
}
