"use client";

import { explorerTx } from "../lib/config.ts";
import type { Stage } from "../lib/ops.ts";
import { useWalnut } from "./walnut-provider.tsx";

const STAGE_CONFIG: Record<
  Stage,
  {
    label: string;
    badgeClass: string;
    live: boolean;
  }
> = {
  signing: {
    label: "Signing",
    badgeClass: "bg-gray-100 text-gray-700 border border-gray-200/60",
    live: true,
  },
  submitted: {
    label: "Submitted",
    badgeClass: "bg-blue-50 text-blue-700 border border-blue-200/60",
    live: true,
  },
  computing: {
    label: "Computing (MPC)",
    badgeClass: "bg-amber-50 text-amber-800 border border-amber-200/60",
    live: true,
  },
  applied: {
    label: "Applied",
    badgeClass: "bg-emerald-50 text-emerald-700 border border-emerald-200/60",
    live: false,
  },
  rejected: {
    label: "Refused (MPC)",
    badgeClass: "bg-amber-50 text-amber-700 border border-amber-200/60",
    live: false,
  },
  failed: {
    label: "Failed",
    badgeClass: "bg-red-50 text-red-700 border border-red-200/60",
    live: false,
  },
};

function formatTimestamp(ms?: number): string {
  if (!ms) return "Recently";
  const diffSec = Math.floor((Date.now() - ms) / 1000);
  if (diffSec < 60) return "Just now";
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m ago`;
  if (diffSec < 86400) return `${Math.floor(diffSec / 3600)}h ago`;
  return new Date(ms).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function ActivityPanel() {
  const { activity, clearActivity } = useWalnut();

  return (
    <div className="rounded-2xl bg-white border border-black/[0.06] shadow-[0_2px_16px_rgba(0,0,0,0.03)] p-6 md:p-8 font-app-sans text-left">
      <div className="flex flex-wrap items-center justify-between gap-3 pb-5 border-b border-black/[0.04] mb-6">
        <div>
          <h2 className="text-[20px] font-semibold text-black tracking-tight">
            Activity History
          </h2>
          <p className="text-xs text-gray-500 mt-0.5">
            Real-time onchain transactions and Arcium MPC operations
          </p>
        </div>
        {activity.length > 0 && (
          <button
            type="button"
            onClick={clearActivity}
            className="text-xs font-medium text-gray-500 hover:text-red-600 transition px-2.5 py-1 rounded-lg border border-black/[0.06] hover:border-red-200 hover:bg-red-50/50 cursor-pointer"
          >
            Clear History
          </button>
        )}
      </div>

      {activity.length === 0 ? (
        <div className="py-12 flex flex-col items-center justify-center text-center">
          <div className="h-12 w-12 rounded-2xl bg-[#F4F5F7] text-gray-400 flex items-center justify-center mb-3">
            <svg
              className="w-6 h-6"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.75"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <circle cx="12" cy="12" r="10" />
              <polyline points="12 6 12 12 16 14" />
            </svg>
          </div>
          <h3 className="text-sm font-semibold text-gray-800">No activity yet</h3>
          <p className="text-xs text-gray-400 max-w-sm mt-1">
            Operations such as shield deposits, private borrows, and repayments will appear here with live onchain status.
          </p>
        </div>
      ) : (
        <ul className="space-y-3">
          {activity.map((item) => {
            const config = STAGE_CONFIG[item.stage] ?? STAGE_CONFIG.signing;
            return (
              <li
                key={item.id}
                className="rounded-xl bg-[#F4F5F7] border border-black/[0.03] p-4.5 hover:border-black/10 transition"
              >
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="flex items-center gap-2.5">
                    <span className="text-sm font-bold text-gray-900 tracking-tight">
                      {item.label}
                    </span>
                    <span className="text-[11px] text-gray-400 font-normal">
                      · {formatTimestamp(item.startedAt)}
                    </span>
                  </div>

                  <span
                    className={`inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-xs font-semibold ${config.badgeClass}`}
                  >
                    {config.live && (
                      <span className="h-1.5 w-1.5 rounded-full bg-current animate-pulse" />
                    )}
                    <span>{config.label}</span>
                  </span>
                </div>

                {item.detail !== undefined ? (
                  <p className="mt-2 text-xs text-gray-600 leading-relaxed break-words">
                    {item.detail}
                  </p>
                ) : null}

                {item.links.length > 0 ? (
                  <div className="mt-3 pt-2 border-t border-black/[0.04] flex flex-wrap gap-x-4 gap-y-1.5">
                    {item.links.map((link) => (
                      <a
                        key={link.signature}
                        href={explorerTx(link.signature)}
                        target="_blank"
                        rel="noreferrer"
                        className="inline-flex items-center gap-1 text-xs font-semibold text-black underline underline-offset-2 hover:opacity-75 transition"
                      >
                        <span>{link.label}</span>
                        <svg
                          className="w-3 h-3 text-gray-500"
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          strokeWidth="2"
                        >
                          <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
                          <polyline points="15 3 21 3 21 9" />
                          <line x1="10" y1="14" x2="21" y2="3" />
                        </svg>
                      </a>
                    ))}
                  </div>
                ) : null}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
