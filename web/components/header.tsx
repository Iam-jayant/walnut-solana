"use client";

import { useWallet } from "@solana/wallet-adapter-react";
import { useWalletModal } from "@solana/wallet-adapter-react-ui";
import { FAUCET_URL, NETWORK_LABEL } from "../lib/config.ts";
import { shortAddress } from "../lib/format.ts";
import { useWalnut } from "./walnut-provider.tsx";
import { Button, LockIcon } from "./ui.tsx";

export function Logo() {
  return (
    <div className="flex items-center gap-2.5">
      <span className="flex h-8 w-8 items-center justify-center rounded-lg bg-walnut-yellow text-base font-[800] text-walnut-black">
        W
      </span>
      <span className="text-lead font-[800] tracking-tight">Walnut</span>
    </div>
  );
}

export function Header() {
  const { publicKey, connected, disconnect, connecting } = useWallet();
  const { setVisible } = useWalletModal();
  const { unlocked, lock } = useWalnut();

  return (
    <header className="sticky top-0 z-20 border-b border-black/5 bg-walnut-white/90 backdrop-blur">
      <div className="mx-auto flex max-w-(--container-page) flex-wrap items-center justify-between gap-3 px-6 py-3 md:px-10">
        <div className="flex items-center gap-6">
          <Logo />
          <nav className="flex items-center gap-4 text-small font-[800]">
            <a href="/app" className="underline-offset-4 hover:underline">
              App
            </a>
          </nav>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <span className="inline-flex items-center gap-2 rounded-pill bg-walnut-mist px-3 py-1.5 text-caption font-[800]">
            <span className="h-2 w-2 rounded-full bg-emerald-500" />
            {NETWORK_LABEL}
          </span>
          {connected && unlocked ? (
            <Button variant="ghost" onClick={lock} className="py-1.5">
              <LockIcon /> Lock
            </Button>
          ) : null}
          {connected && publicKey !== null ? (
            <Button
              variant="dark"
              onClick={() => void disconnect()}
              className="py-1.5"
              title="Disconnect wallet"
            >
              {shortAddress(publicKey.toBase58())}
            </Button>
          ) : (
            <Button
              onClick={() => setVisible(true)}
              disabled={connecting}
              className="py-1.5"
            >
              {connecting ? "Connecting..." : "Connect Wallet"}
            </Button>
          )}
        </div>
      </div>
    </header>
  );
}
