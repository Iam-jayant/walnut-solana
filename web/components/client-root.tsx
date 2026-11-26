"use client";

import dynamic from "next/dynamic";
import { WalnutLogo } from "./brand-logo.tsx";

// The wallet and cryptography code only runs in the browser.
const App = dynamic(() => import("./app.tsx"), {
  ssr: false,
  loading: () => (
    <div className="mx-auto max-w-(--container-page) px-6 py-10 md:px-10">
      <div className="flex items-center gap-2.5">
        <WalnutLogo size={32} />
        <span className="text-lead font-[800] tracking-tight">Walnut</span>
      </div>
      <h1 className="mt-16 max-w-3xl text-h1 font-[200]">
        Private lending on Solana.
      </h1>
    </div>
  ),
});

export function ClientRoot() {
  return <App />;
}
