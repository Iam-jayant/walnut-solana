"use client";

import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { explorerTx } from "../lib/config.ts";

export type ToastType =
  | "success"
  | "error"
  | "warn"
  | "info"
  | "pending"
  | "cancelled";

export interface ToastItem {
  id: string;
  type: ToastType;
  title: string;
  message?: string | undefined;
  signature?: string | undefined;
  duration?: number | undefined;
  timestamp: number;
}

interface ToastContextValue {
  toasts: ToastItem[];
  showToast: (toast: Omit<ToastItem, "id" | "timestamp"> & { id?: string | undefined }) => string;
  updateToast: (id: string, toast: Partial<ToastItem>) => void;
  dismissToast: (id: string) => void;
  clearToasts: () => void;
}

const ToastContext = createContext<ToastContextValue | null>(null);

export function useToast(): ToastContextValue {
  const ctx = useContext(ToastContext);
  if (!ctx) {
    throw new Error("useToast must be used within ToastProvider");
  }
  return ctx;
}

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const idCounter = useRef(0);
  const timersRef = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());

  const dismissToast = useCallback((id: string) => {
    const timer = timersRef.current.get(id);
    if (timer) {
      clearTimeout(timer);
      timersRef.current.delete(id);
    }
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const scheduleDismiss = useCallback(
    (id: string, duration = 6000) => {
      const existing = timersRef.current.get(id);
      if (existing) clearTimeout(existing);
      if (duration > 0) {
        const timer = setTimeout(() => {
          dismissToast(id);
        }, duration);
        timersRef.current.set(id, timer);
      }
    },
    [dismissToast],
  );

  const showToast = useCallback(
    (toast: Omit<ToastItem, "id" | "timestamp"> & { id?: string | undefined }): string => {
      const id = toast.id ?? `toast-${++idCounter.current}-${Date.now()}`;
      const item: ToastItem = {
        ...toast,
        id,
        timestamp: Date.now(),
      };

      setToasts((prev) => {
        // If an item with the same id exists, replace it, otherwise append to top
        const idx = prev.findIndex((t) => t.id === id);
        if (idx !== -1) {
          const next = [...prev];
          next[idx] = item;
          return next;
        }
        return [item, ...prev].slice(0, 6); // Max 6 toasts
      });

      if (toast.type !== "pending") {
        scheduleDismiss(id, toast.duration ?? 6000);
      }
      return id;
    },
    [scheduleDismiss],
  );

  const updateToast = useCallback(
    (id: string, updates: Partial<ToastItem>) => {
      setToasts((prev) =>
        prev.map((t) => {
          if (t.id === id) {
            const next = { ...t, ...updates };
            if (updates.type && updates.type !== "pending") {
              scheduleDismiss(id, updates.duration ?? 6500);
            }
            return next;
          }
          return t;
        }),
      );
    },
    [scheduleDismiss],
  );

  const clearToasts = useCallback(() => {
    timersRef.current.forEach((t) => clearTimeout(t));
    timersRef.current.clear();
    setToasts([]);
  }, []);

  useEffect(() => {
    return () => {
      timersRef.current.forEach((t) => clearTimeout(t));
      timersRef.current.clear();
    };
  }, []);

  return (
    <ToastContext.Provider
      value={{ toasts, showToast, updateToast, dismissToast, clearToasts }}
    >
      {children}
      <ToastContainer toasts={toasts} onDismiss={dismissToast} />
    </ToastContext.Provider>
  );
}

function ToastContainer({
  toasts,
  onDismiss,
}: {
  toasts: ToastItem[];
  onDismiss: (id: string) => void;
}) {
  if (toasts.length === 0) return null;

  return (
    <div
      aria-live="polite"
      className="fixed bottom-5 right-5 z-50 flex flex-col gap-2.5 max-w-sm w-full px-4 sm:px-0 pointer-events-none select-none font-app-sans"
      style={{
        fontFamily:
          '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif',
      }}
    >
      {toasts.map((toast) => (
        <ToastCard key={toast.id} toast={toast} onDismiss={() => onDismiss(toast.id)} />
      ))}
    </div>
  );
}

function ToastCard({
  toast,
  onDismiss,
}: {
  toast: ToastItem;
  onDismiss: () => void;
}) {
  const { type, title, message, signature } = toast;

  let borderColor = "border-black/[0.08]";
  let iconBg = "bg-gray-100 text-gray-700";
  let icon = null;

  if (type === "success") {
    borderColor = "border-emerald-300/80 bg-white shadow-[0_8px_30px_rgba(16,185,129,0.12)]";
    iconBg = "bg-emerald-100 text-emerald-700";
    icon = (
      <svg className="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
        <polyline points="20 6 9 17 4 12" />
      </svg>
    );
  } else if (type === "error") {
    borderColor = "border-red-300/80 bg-white shadow-[0_8px_30px_rgba(239,68,68,0.12)]";
    iconBg = "bg-red-100 text-red-700";
    icon = (
      <svg className="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="12" cy="12" r="10" />
        <line x1="12" y1="8" x2="12" y2="12" />
        <line x1="12" y1="16" x2="12.01" y2="16" />
      </svg>
    );
  } else if (type === "cancelled" || type === "warn") {
    borderColor = "border-[#D8D4CB] bg-white shadow-[0_8px_30px_rgba(17,17,17,0.04)]";
    iconBg = "bg-[#EAE7E0] text-[#2B2926]";
    icon = (
      <svg className="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="12" cy="12" r="10" />
        <line x1="15" y1="9" x2="9" y2="15" />
        <line x1="9" y1="9" x2="15" y2="15" />
      </svg>
    );
  } else if (type === "pending") {
    borderColor = "border-black/[0.1] bg-white shadow-[0_8px_30px_rgba(0,0,0,0.08)]";
    iconBg = "bg-[#111111] text-white";
    icon = (
      <svg className="w-4 h-4 animate-spin" viewBox="0 0 24 24" fill="none">
        <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="3" />
        <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v4a4 4 0 00-4 4H4z" />
      </svg>
    );
  }

  return (
    <div
      className={`pointer-events-auto rounded-xl border p-4 transition-all duration-200 ${borderColor}`}
    >
      <div className="flex items-start gap-3">
        <div className={`h-7 w-7 rounded-lg flex items-center justify-center shrink-0 ${iconBg}`}>
          {icon}
        </div>
        <div className="flex-1 min-w-0 pr-1">
          <h4 className="text-[13px] font-semibold text-gray-900 leading-tight">
            {title}
          </h4>
          {message && (
            <p className="text-[12px] text-gray-500 mt-1 leading-snug break-words">
              {message}
            </p>
          )}
          {signature && (
            <a
              href={explorerTx(signature)}
              target="_blank"
              rel="noreferrer"
              className="inline-flex items-center gap-1.5 mt-2 text-[11px] font-semibold text-black underline underline-offset-2 hover:opacity-75 transition"
            >
              <span>View on Solana Explorer</span>
              <svg className="w-3 h-3 text-gray-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
                <polyline points="15 3 21 3 21 9" />
                <line x1="10" y1="14" x2="21" y2="3" />
              </svg>
            </a>
          )}
        </div>
        <button
          type="button"
          onClick={onDismiss}
          className="text-gray-400 hover:text-gray-700 transition p-1 -mr-1 -mt-1 rounded-md cursor-pointer"
          aria-label="Dismiss notification"
        >
          <svg className="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>
    </div>
  );
}
