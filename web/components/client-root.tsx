"use client";

import dynamic from "next/dynamic";

// The wallet and cryptography code only runs in the browser.
const App = dynamic(() => import("./app.tsx"), {
  ssr: false,
  loading: () => (
    <div className="mx-auto max-w-(--container-page) px-6 py-10 md:px-10">
      <div className="flex items-center gap-2.5">
        <span className="flex h-8 w-8 items-center justify-center rounded-lg bg-walnut-yellow text-base font-[800]">
          W
        </span>
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
