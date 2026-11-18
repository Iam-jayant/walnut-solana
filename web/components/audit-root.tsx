"use client";

import dynamic from "next/dynamic";

// The wallet and cryptography code only runs in the browser.
const AuditApp = dynamic(() => import("./audit-app.tsx"), {
  ssr: false,
  loading: () => (
    <div className="mx-auto max-w-(--container-page) px-6 py-10 md:px-10">
      <h1 className="text-h2 font-[200]">Auditor</h1>
    </div>
  ),
});

export function AuditRoot() {
  return <AuditApp />;
}
