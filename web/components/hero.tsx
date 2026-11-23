"use client";

import { useWallet } from "@solana/wallet-adapter-react";
import { useWalletModal } from "@solana/wallet-adapter-react-ui";
import { FAUCET_URL } from "../lib/config.ts";
import { Button, LockIcon } from "./ui.tsx";

const POINTS = [
  [
    "Encrypted balances",
    "Free balances, collateral and debt are stored encrypted on-chain.",
  ],
  [
    "Arcium MPC",
    "Operations are computed by a multi-party computation network, never in the clear.",
  ],
  [
    "Only you can decrypt",
    "Your Viewing Key is derived from a wallet signature and never leaves your browser.",
  ],
] as const;

export function Hero() {
  const { connected, connecting } = useWallet();
  const { setVisible } = useWalletModal();

  return (
    <section className="mx-auto max-w-(--container-page) px-6 pt-8 md:px-10 md:pt-12">
      <div className="relative overflow-hidden rounded-[2rem] bg-walnut-black px-6 py-12 text-walnut-white md:px-14 md:py-20">
        <div
          aria-hidden
          className="pointer-events-none absolute -right-24 -top-24 h-96 w-96 rounded-full bg-walnut-yellow/20 blur-3xl"
        />
        <p className="inline-flex items-center gap-2 rounded-pill bg-white/10 px-3 py-1.5 text-caption font-[800] uppercase text-walnut-yellow">
          <LockIcon /> Private lending on Solana
        </p>
        <h1
          className={`mt-6 max-w-4xl font-[200] ${connected ? "text-h1" : "text-display"}`}
        >
          Borrow against your SOL.
          <span className="font-[800] text-walnut-yellow">
            {" "}
            Keep it private.
          </span>
        </h1>
        <p className="mt-6 max-w-2xl text-lead text-white/75">
          Walnut is a lending protocol powered by Arcium multi-party
          computation. Your balances, collateral and debt are encrypted, and
          only you can decrypt them with your Viewing Key.
        </p>
        {!connected ? (
          <div className="mt-8 flex flex-wrap items-center gap-4">
            <Button
              onClick={() => setVisible(true)}
              disabled={connecting}
              className="px-7 py-3.5 text-body"
            >
              {connecting ? "Connecting..." : "Connect Wallet"}
            </Button>
          </div>
        ) : null}
        {!connected ? (
          <ul className="mt-12 grid gap-4 md:grid-cols-3">
            {POINTS.map(([title, text]) => (
              <li
                key={title}
                className="rounded-card bg-white/5 p-5 ring-1 ring-white/10"
              >
                <p className="text-small font-[800] text-walnut-yellow">
                  {title}
                </p>
                <p className="mt-2 text-small text-white/70">{text}</p>
              </li>
            ))}
          </ul>
        ) : null}
      </div>
    </section>
  );
}
