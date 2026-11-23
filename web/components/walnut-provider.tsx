"use client";

// Application state: protocol data, the unlocked Viewing Key session, the
// decrypted Position, wallet balances, and the operation actions with live status.
import "../lib/polyfill.ts";
import {
  deriveViewingKey,
  viewingKeyMessageBytes,
  ViewingKeySession,
  type CollateralOp,
  type LendOp,
  type PoolOp,
  type PositionBalances,
  type WalletEventSource,
} from "@walnut/sdk";
import { useWallet } from "@solana/wallet-adapter-react";
import { PublicKey, type Connection, type Transaction } from "@solana/web3.js";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  ataAddress,
  createChain,
  createConnection,
  detectPrivateMode,
  fetchAccount,
  fetchSolPrice,
  loadLendingView,
  loadProtocol,
  positionAddress,
  tokenBalanceOf,
  variantName,
  type Chain,
  type LendingView,
  type PositionAccount,
  type PriceInfo,
  type Protocol,
} from "../lib/chain.ts";
import {
  FALLBACK_SOL_PRICE,
  NETWORK,
  PROGRAM_ID,
  rpcUrl,
  WSOL_MINT,
} from "../lib/config.ts";
import {
  abandonLiquidation,
  abandonStuck,
  checkHealth,
  deposit,
  disclose,
  estimateSeize,
  findStuckOperation,
  friendlyError,
  initializePosition,
  lendOp,
  liquidate,
  privateCollateral,
  privateLiquidate,
  privatePoolOp,
  refreshPoolBand,
  rotateKey,
  unwrapWsol,
  withdraw,
  type Asset,
  type HealthResult,
  type PendingLiquidation,
  type OpsContext,
  type Progress,
  type Report,
  type Stage,
  type StuckOperation,
} from "../lib/ops.ts";
import walnutIdl from "../lib/idl/walnut.json";
import { useToast } from "./toast.tsx";

export interface ActivityLink {
  label: string;
  signature: string;
}

export interface ActivityItem {
  id: number;
  label: string;
  stage: Stage;
  detail?: string;
  links: ActivityLink[];
  startedAt: number;
}

export interface WalletBalances {
  sol: bigint;
  usdc: bigint;
  wsol: bigint;
}

export type AccountState =
  "loading" | "none" | "initializing" | "active" | "failed";

export interface WalnutState {
  connection: Connection;
  protocol: Protocol | null;
  protocolError: string | null;
  /** The LendingPool; null until it exists on chain. */
  lending: LendingView | null;
  /** The first LendingPool read finished (it may still be absent). */
  lendingChecked: boolean;
  /**
   * The encrypted-amount path is live: its computation definitions exist and
   * the encrypted Pool is Active. Otherwise actions use the public-amount path.
   */
  privateMode: boolean;
  price: PriceInfo;
  accountState: AccountState;
  unlocked: boolean;
  balances: PositionBalances | null;
  walletBalances: WalletBalances | null;
  stuck: StuckOperation | null;
  pendingLiquidation: PendingLiquidation | null;
  activity: ActivityItem[];
  clearActivity(): void;
  busy: boolean;
  error: string | null;
  refreshing: boolean;
  unlock(): Promise<void>;
  lock(): void;
  createAccount(): Promise<void>;
  deposit(asset: Asset, amount: bigint): Promise<void>;
  withdraw(asset: Asset, amount: bigint): Promise<void>;
  lendOp(op: LendOp, label: string, amount: bigint): Promise<void>;
  /** Supply, withdraw supply, borrow or repay with an encrypted amount. True when applied. */
  privatePoolOp(
    op: PoolOp,
    label: string,
    amount: bigint,
    shareCap?: bigint,
  ): Promise<boolean>;
  /** Add or remove collateral with an encrypted amount. True when applied. */
  privateCollateral(
    op: CollateralOp,
    label: string,
    lamports: bigint,
  ): Promise<boolean>;
  /** Liquidates with an encrypted amount from the signer's own free USDC. */
  privateLiquidate(borrower: string, usdcAmount: bigint): Promise<boolean>;
  /** SOL (lamports) a private liquidation of `usdcAmount` would seize; null if unknown. */
  estimateSeize(usdcAmount: bigint): Promise<bigint | null>;
  /** Queues a health check of the borrower's account (a wallet address). */
  checkHealth(borrower: string): Promise<HealthResult | undefined>;
  liquidate(borrower: string, repayAmount: bigint): Promise<void>;
  abandonLiquidation(): Promise<void>;
  /** Rotates the Viewing Key; the session switches to the new key on success. */
  rotateKey(): Promise<void>;
  /** Seals the chosen fields to the auditor's x25519 public key. */
  disclose(auditorPublicKey: Uint8Array, mask: number): Promise<boolean>;
  unwrapPending(): Promise<void>;
  abandonStuck(): Promise<void>;
  refreshBand(): Promise<void>;
  refresh(): Promise<void>;
}

const WalnutContext = createContext<WalnutState | null>(null);

export function useWalnut(): WalnutState {
  const value = useContext(WalnutContext);
  if (value === null) throw new Error("WalnutProvider is missing");
  return value;
}

export function WalnutProvider({ children }: { children: ReactNode }) {
  const wallet = useWallet();
  const { publicKey, connected } = wallet;

  const chain = useMemo<Chain>(
    () =>
      createChain(
        createConnection(rpcUrl()),
        walnutIdl as unknown as Parameters<typeof createChain>[1],
      ),
    [],
  );
  const [protocol, setProtocol] = useState<Protocol | null>(null);
  const [protocolError, setProtocolError] = useState<string | null>(null);
  const [lending, setLending] = useState<LendingView | null>(null);
  const [lendingChecked, setLendingChecked] = useState(false);
  const [privateMode, setPrivateMode] = useState(false);
  const [price, setPrice] = useState<PriceInfo>({
    price: FALLBACK_SOL_PRICE,
    source: "fallback",
  });
  const [accountState, setAccountState] = useState<AccountState>("loading");
  const [unlocked, setUnlocked] = useState(false);
  const [balances, setBalances] = useState<PositionBalances | null>(null);
  const [walletBalances, setWalletBalances] = useState<WalletBalances | null>(
    null,
  );
  const [stuck, setStuck] = useState<StuckOperation | null>(null);
  const [pendingLiquidation, setPendingLiquidation] =
    useState<PendingLiquidation | null>(null);
  const [activity, setActivity] = useState<ActivityItem[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const { showToast, updateToast } = useToast();

  const sessionRef = useRef<ViewingKeySession | null>(null);
  const positionRef = useRef<PositionAccount | null>(null);
  const activityId = useRef(0);
  const busyRef = useRef(false);

  // Load persistent activity from localStorage
  useEffect(() => {
    if (typeof window === "undefined") return;
    try {
      const key = publicKey
        ? `walnut_act_${publicKey.toBase58()}`
        : "walnut_act_guest";
      const raw = localStorage.getItem(key);
      if (raw) {
        const items = JSON.parse(raw);
        if (Array.isArray(items)) {
          setActivity(items);
        }
      } else {
        setActivity([]);
      }
    } catch (e) {
      console.warn("Failed to load activity from storage:", e);
    }
  }, [publicKey]);

  const persistActivity = useCallback(
    (items: ActivityItem[]) => {
      if (typeof window === "undefined") return;
      try {
        const key = publicKey
          ? `walnut_act_${publicKey.toBase58()}`
          : "walnut_act_guest";
        localStorage.setItem(key, JSON.stringify(items.slice(0, 50)));
      } catch (e) {
        console.warn("Failed to persist activity:", e);
      }
    },
    [publicKey],
  );

  const clearActivity = useCallback(() => {
    setActivity([]);
    if (typeof window !== "undefined") {
      const key = publicKey
        ? `walnut_act_${publicKey.toBase58()}`
        : "walnut_act_guest";
      localStorage.removeItem(key);
    }
    showToast({
      type: "info",
      title: "Activity Cleared",
      message: "Your transaction history has been cleared.",
      duration: 3500,
    });
  }, [publicKey, showToast]);

  // ---- protocol and price (no wallet needed)
  useEffect(() => {
    let cancelled = false;
    loadProtocol(chain)
      .then((p) => {
        if (!cancelled) setProtocol(p);
      })
      .catch((err: unknown) => {
        if (!cancelled) setProtocolError(friendlyError(err));
      });
    const loadPrice = () =>
      fetchSolPrice(chain.connection).then((p) => {
        if (!cancelled) setPrice(p);
      });
    void loadPrice();
    const timer = setInterval(() => void loadPrice(), 60_000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [chain]);

  // ---- lending pool (no wallet needed)
  useEffect(() => {
    if (protocol === null) return;
    let cancelled = false;
    const load = () =>
      Promise.all([
        loadLendingView(chain, protocol),
        detectPrivateMode(chain, protocol).catch(() => false),
      ])
        .then(([view, priv]) => {
          if (cancelled) return;
          setLending(view);
          setPrivateMode(view !== null && priv);
          setLendingChecked(true);
        })
        .catch(() => {
          /* keep the last known pool; the next poll retries */
        });
    void load();
    const timer = setInterval(() => void load(), 30_000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [chain, protocol]);

  // 5-minute timeout for private balance hideout on tab switch
  const HIDE_TIMEOUT_MS = 5 * 60 * 1000;
  const hideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const hiddenTimestampRef = useRef<number | null>(null);

  // ---- session custody
  const clearSession = useCallback(() => {
    if (hideTimerRef.current !== null) {
      clearTimeout(hideTimerRef.current);
      hideTimerRef.current = null;
    }
    hiddenTimestampRef.current = null;
    sessionRef.current?.clear();
    sessionRef.current = null;
    setUnlocked(false);
    setBalances(null);
  }, []);

  useEffect(() => {
    // The session clears itself on wallet disconnect; mirror it.
    const timer = setInterval(() => {
      const session = sessionRef.current;
      if (session !== null && !session.isUnlocked) {
        sessionRef.current = null;
        setUnlocked(false);
        setBalances(null);
      }
    }, 500);
    return () => clearInterval(timer);
  }, []);

  useEffect(() => {
    if (!connected) {
      clearSession();
      setAccountState("loading");
      setWalletBalances(null);
      setStuck(null);
      positionRef.current = null;
    }
  }, [connected, publicKey, clearSession]);

  const adoptSession = useCallback(
    (session: ViewingKeySession) => {
      if (hideTimerRef.current !== null) {
        clearTimeout(hideTimerRef.current);
        hideTimerRef.current = null;
      }
      hiddenTimestampRef.current = null;
      sessionRef.current?.clear();
      sessionRef.current = session;
      const adapter = wallet.wallet?.adapter;
      if (adapter !== undefined) {
        session.attachWallet(adapter as unknown as WalletEventSource);
      }
      setUnlocked(true);
    },
    [wallet.wallet],
  );

  // ---- reading
  const refresh = useCallback(async () => {
    if (publicKey === null || protocol === null) return;
    setRefreshing(true);
    try {
      const owner = publicKey;
      const wsolAddress = ataAddress(owner, WSOL_MINT);
      void Promise.all([
        loadLendingView(chain, protocol),
        detectPrivateMode(chain, protocol).catch(() => false),
      ])
        .then(([view, priv]) => {
          setLending(view);
          setPrivateMode(view !== null && priv);
          setLendingChecked(true);
        })
        .catch(() => undefined);
      const [position, lamports, usdc, wsol] = await Promise.all([
        fetchAccount<PositionAccount>(
          chain.program,
          "position",
          positionAddress(owner),
        ),
        chain.connection.getBalance(owner, "confirmed"),
        tokenBalanceOf(
          chain.connection,
          ataAddress(owner, protocol.market.usdcMint),
        ),
        tokenBalanceOf(chain.connection, wsolAddress),
      ]);
      setWalletBalances({
        sol: BigInt(lamports),
        usdc: usdc ?? 0n,
        wsol: wsol ?? 0n,
      });
      positionRef.current = position;
      if (position === null) {
        setAccountState("none");
        setStuck(null);
        return;
      }
      const status = variantName(position.status);
      setAccountState(
        status === "active"
          ? "active"
          : status === "initFailed"
            ? "failed"
            : status === "initializing"
              ? "initializing"
              : "none",
      );
      setStuck(await findStuckOperation(chain, position));
      const session = sessionRef.current;
      if (status === "active" && session !== null && session.isUnlocked) {
        setBalances(
          session.decryptPosition(
            {
              nonce: BigInt(position.encryptedState.nonce.toString()),
              ciphertexts: position.encryptedState.ciphertexts,
            },
            protocol.mxePublicKey,
          ),
        );
      }
    } catch (err) {
      setError(friendlyError(err));
    } finally {
      setRefreshing(false);
    }
  }, [chain, protocol, publicKey]);

  // Keep private balances unlocked for 5 minutes when switching browser tabs
  useEffect(() => {
    if (typeof window === "undefined" || typeof document === "undefined") return;

    const onVisibilityChange = () => {
      if (document.visibilityState === "hidden") {
        // Tab hidden: start 5-minute countdown
        hiddenTimestampRef.current = Date.now();
        if (sessionRef.current !== null && sessionRef.current.isUnlocked) {
          if (hideTimerRef.current !== null) {
            clearTimeout(hideTimerRef.current);
          }
          hideTimerRef.current = setTimeout(() => {
            clearSession();
            hideTimerRef.current = null;
            hiddenTimestampRef.current = null;
          }, HIDE_TIMEOUT_MS);
        }
      } else if (document.visibilityState === "visible") {
        // Returned to tab
        const hiddenAt = hiddenTimestampRef.current;
        hiddenTimestampRef.current = null;

        if (hideTimerRef.current !== null) {
          clearTimeout(hideTimerRef.current);
          hideTimerRef.current = null;
        }

        if (hiddenAt !== null && Date.now() - hiddenAt >= HIDE_TIMEOUT_MS) {
          clearSession();
        } else if (sessionRef.current !== null && sessionRef.current.isUnlocked) {
          // Within 5 minutes: session remains active and unlocked; refresh state
          void refresh();
        }
      }
    };

    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      document.removeEventListener("visibilitychange", onVisibilityChange);
      if (hideTimerRef.current !== null) {
        clearTimeout(hideTimerRef.current);
      }
    };
  }, [clearSession, refresh, HIDE_TIMEOUT_MS]);

  useEffect(() => {
    if (!connected) return;
    void refresh();
    const timer = setInterval(() => {
      if (document.visibilityState === "visible" && !busyRef.current) {
        void refresh();
      }
    }, 30_000);
    return () => clearInterval(timer);
  }, [connected, refresh]);

  // ---- signing context
  const opsContext = useCallback((): OpsContext => {
    if (publicKey === null || wallet.signTransaction === undefined) {
      throw new Error("Connect a wallet that can sign transactions.");
    }
    if (protocol === null) throw new Error("Market data is still loading.");
    const signTransaction = wallet.signTransaction;
    return {
      connection: chain.connection,
      program: chain.program,
      protocol,
      signer: {
        publicKey,
        signTransaction: (tx: Transaction) => signTransaction(tx),
      },
    };
  }, [chain, protocol, publicKey, wallet.signTransaction]);

  // ---- activity tracking
  const track = useCallback(
    async <T,>(
      label: string,
      run: (report: Report, ctx: OpsContext) => Promise<T>,
    ): Promise<T | undefined> => {
      if (busyRef.current) return undefined;
      busyRef.current = true;
      setBusy(true);
      setError(null);
      const id = (activityId.current += 1);
      const toastId = `toast-act-${id}-${Date.now()}`;

      // Show initial pending toast
      showToast({
        id: toastId,
        type: "pending",
        title: label,
        message: "Please sign and approve the transaction in your wallet...",
      });

      const newItem: ActivityItem = {
        id,
        label,
        stage: "signing",
        links: [],
        startedAt: Date.now(),
      };

      setActivity((items) => {
        const updated = [newItem, ...items].slice(0, 50);
        persistActivity(updated);
        return updated;
      });

      const patch = (fn: (item: ActivityItem) => ActivityItem) =>
        setActivity((items) => {
          const updated = items.map((i) => (i.id === id ? fn(i) : i));
          persistActivity(updated);
          return updated;
        });

      let lastSignature: string | undefined;

      const report: Report = (p: Progress) =>
        patch((item) => {
          if (p.signature) lastSignature = p.signature;
          const links =
            p.signature !== undefined &&
            !item.links.some((l) => l.signature === p.signature)
              ? [
                  ...item.links,
                  { label: p.label ?? label, signature: p.signature },
                ]
              : item.links;
          const next: ActivityItem = {
            ...item,
            stage: p.stage,
            links,
          };
          if (p.detail !== undefined) next.detail = p.detail;
          else if (p.label !== undefined && p.label !== label) {
            next.detail = p.label;
          } else delete next.detail;

          // Sync toast notifications in real-time
          if (p.stage === "submitted") {
            updateToast(toastId, {
              type: "pending",
              title: label,
              message: "Transaction submitted to Solana. Awaiting confirmation...",
              signature: p.signature,
            });
          } else if (p.stage === "computing") {
            updateToast(toastId, {
              type: "pending",
              title: label,
              message: p.detail ?? "Arcium MPC network is computing confidential state...",
              signature: p.signature,
            });
          } else if (p.stage === "applied") {
            updateToast(toastId, {
              type: "success",
              title: `${label} Successful`,
              message: p.detail ?? "Operation applied and verified privately onchain.",
              signature: p.signature,
              duration: 6500,
            });
          } else if (p.stage === "rejected") {
            updateToast(toastId, {
              type: "error",
              title: `${label} Refused`,
              message: p.detail ?? "Refused by the MPC circuit. State unchanged.",
              signature: p.signature,
              duration: 8000,
            });
          } else if (p.stage === "failed") {
            updateToast(toastId, {
              type: "error",
              title: `${label} Failed`,
              message: p.detail ?? "Operation failed. Balances unchanged.",
              signature: p.signature,
              duration: 8000,
            });
          }

          return next;
        });

      try {
        const result = await run(report, opsContext());
        // For non-queued L1 actions (e.g. deposit SOL wrap), complete with success if not already terminal
        updateToast(toastId, {
          type: "success",
          title: `${label} Completed`,
          message: "Transaction confirmed on Solana.",
          signature: lastSignature,
          duration: 6000,
        });
        return result;
      } catch (err) {
        const errMsg = friendlyError(err);
        const isCancelled =
          errMsg.toLowerCase().includes("user rejected") ||
          errMsg.toLowerCase().includes("cancelled") ||
          errMsg.toLowerCase().includes("canceled");

        patch((item) => ({
          ...item,
          stage: "failed",
          detail: isCancelled ? "Cancelled by user" : errMsg,
        }));

        if (isCancelled) {
          updateToast(toastId, {
            type: "cancelled",
            title: `${label} Cancelled`,
            message: "Transaction request was cancelled in your wallet.",
            duration: 5000,
          });
        } else {
          updateToast(toastId, {
            type: "error",
            title: `${label} Failed`,
            message: errMsg,
            duration: 8500,
          });
        }
        setError(errMsg);
        return undefined;
      } finally {
        busyRef.current = false;
        setBusy(false);
        await refresh();
      }
    },
    [opsContext, persistActivity, refresh, showToast, updateToast],
  );

  // ---- actions
  const unlock = useCallback(async () => {
    setError(null);
    try {
      if (publicKey === null || protocol === null) return;
      if (wallet.signMessage === undefined) {
        throw new Error(
          "This wallet cannot sign messages, so it cannot unlock private balances.",
        );
      }
      showToast({
        id: "toast-unlock",
        type: "pending",
        title: "Unlock Private Balances",
        message: "Please sign the zero-gas Viewing Key message in your wallet...",
      });
      const position = await fetchAccount<PositionAccount>(
        chain.program,
        "position",
        positionAddress(publicKey),
      );
      const epoch =
        position === null ? 0n : BigInt(position.viewingKeyEpoch.toString());
      const message = viewingKeyMessageBytes({
        programId: PROGRAM_ID,
        network: NETWORK,
        positionPda: positionAddress(publicKey),
        epoch,
      });
      const signature = await wallet.signMessage(message);
      const session = await ViewingKeySession.unlock(signature);
      if (
        position !== null &&
        variantName(position.status) === "active" &&
        !equalBytes(session.publicKey, position.viewingPubkey)
      ) {
        session.clear();
        throw new Error(
          "This signature does not match the Viewing Key of this account.",
        );
      }
      adoptSession(session);
      await refresh();
      updateToast("toast-unlock", {
        type: "success",
        title: "Private Balances Unlocked",
        message: "Viewing Key derived. Session stays active for 5 minutes.",
        duration: 5000,
      });
    } catch (err) {
      const errMsg = friendlyError(err);
      const isCancelled =
        errMsg.toLowerCase().includes("user rejected") ||
        errMsg.toLowerCase().includes("cancelled") ||
        errMsg.toLowerCase().includes("canceled");

      if (isCancelled) {
        updateToast("toast-unlock", {
          type: "cancelled",
          title: "Unlock Cancelled",
          message: "Viewing Key signature was cancelled in your wallet.",
          duration: 4500,
        });
      } else {
        updateToast("toast-unlock", {
          type: "error",
          title: "Unlock Failed",
          message: errMsg,
          duration: 7500,
        });
        setError(errMsg);
      }
    }
  }, [adoptSession, chain, protocol, publicKey, refresh, showToast, updateToast, wallet]);

  const createAccount = useCallback(async () => {
    if (sessionRef.current === null || !sessionRef.current.isUnlocked) {
      await unlock();
    }
    const session = sessionRef.current;
    if (session === null || !session.isUnlocked) return;
    await track("Create private account", async (report, ctx) => {
      return initializePosition(ctx, session.publicKey, report);
    });
  }, [track, unlock]);

  const requireLending = useCallback((): LendingView => {
    if (lending === null) {
      throw new Error("The lending market is being initialized.");
    }
    return lending;
  }, [lending]);

  const parseAddress = (text: string): PublicKey => {
    try {
      return new PublicKey(text.trim());
    } catch {
      throw new Error("Enter a valid wallet address.");
    }
  };

  const rotate = useCallback(async () => {
    await track("Rotate Viewing Key", async (report, ctx) => {
      const session = sessionRef.current;
      if (session === null || !session.isUnlocked) {
        throw new Error("Unlock private balances first.");
      }
      if (wallet.signMessage === undefined) {
        throw new Error("This wallet cannot sign messages.");
      }
      const position = await fetchAccount<PositionAccount>(
        chain.program,
        "position",
        positionAddress(ctx.signer.publicKey),
      );
      if (position === null) throw new Error("No private account found.");
      const oldEpoch = BigInt(position.viewingKeyEpoch.toString());
      const expectedBalances = session.decryptPosition(
        {
          nonce: BigInt(position.encryptedState.nonce.toString()),
          ciphertexts: position.encryptedState.ciphertexts,
        },
        ctx.protocol.mxePublicKey,
      );
      const signature = await wallet.signMessage(
        viewingKeyMessageBytes({
          programId: PROGRAM_ID,
          network: NETWORK,
          positionPda: positionAddress(ctx.signer.publicKey),
          epoch: oldEpoch + 1n,
        }),
      );
      const newKey = await deriveViewingKey(signature, {
        wipeSignature: true,
      });
      try {
        return await rotateKey(
          ctx,
          {
            newKey,
            oldEpoch,
            oldPublicKey: session.publicKey,
            expectedBalances,
            commitKey: (key) => adoptSession(new ViewingKeySession(key)),
          },
          report,
        );
      } catch (err) {
        newKey.wipe();
        throw err;
      }
    });
  }, [adoptSession, chain, track, wallet]);

  const value: WalnutState = {
    connection: chain.connection,
    protocol,
    protocolError,
    lending,
    lendingChecked,
    privateMode,
    price,
    accountState,
    unlocked,
    balances,
    walletBalances,
    stuck,
    pendingLiquidation,
    activity,
    clearActivity,
    busy,
    error,
    refreshing,
    unlock,
    lock: () => {
      sessionRef.current?.lock();
      clearSession();
    },
    createAccount,
    deposit: async (asset, amount) => {
      await track(
        asset === "sol" ? "Deposit SOL" : "Deposit USDC",
        (report, ctx) => deposit(ctx, asset, amount, report),
      );
    },
    withdraw: async (asset, amount) => {
      await track(
        asset === "sol" ? "Withdraw SOL" : "Withdraw USDC",
        (report, ctx) => withdraw(ctx, asset, amount, report),
      );
    },
    lendOp: async (op, label, amount) => {
      await track(label, (report, ctx) =>
        lendOp(ctx, requireLending(), op, label, amount, report),
      );
    },
    privatePoolOp: async (op, label, amount, shareCap) => {
      const outcome = await track(label, (report, ctx) =>
        privatePoolOp(
          ctx,
          requireLending(),
          op,
          label,
          amount,
          report,
          shareCap,
        ),
      );
      return outcome === "applied";
    },
    privateCollateral: async (op, label, lamports) => {
      const outcome = await track(label, (report, ctx) =>
        privateCollateral(ctx, requireLending(), op, label, lamports, report),
      );
      return outcome === "applied";
    },
    privateLiquidate: async (borrower, usdcAmount) => {
      const result = await track("Liquidate", (report, ctx) =>
        privateLiquidate(
          ctx,
          requireLending(),
          parseAddress(borrower),
          usdcAmount,
          report,
        ),
      );
      return result?.outcome === "applied";
    },
    estimateSeize: async (usdcAmount) => {
      if (protocol === null || lending === null) return null;
      try {
        return await estimateSeize(
          { connection: chain.connection, protocol },
          lending,
          usdcAmount,
        );
      } catch {
        return null;
      }
    },
    checkHealth: (borrower) =>
      track("Check health", (report, ctx) =>
        checkHealth(ctx, requireLending(), parseAddress(borrower), report),
      ),
    liquidate: async (borrower, repayAmount) => {
      const result = await track("Liquidate", (report, ctx) =>
        liquidate(
          ctx,
          requireLending(),
          parseAddress(borrower),
          repayAmount,
          report,
        ),
      );
      if (result !== undefined) setPendingLiquidation(result.pending);
    },
    abandonLiquidation: async () => {
      const pending = pendingLiquidation;
      if (pending === null) return;
      await track("Release liquidation", async (report, ctx) => {
        await abandonLiquidation(ctx, pending, report);
        setPendingLiquidation(null);
      });
    },
    rotateKey: rotate,
    disclose: async (auditorPublicKey, mask) => {
      const outcome = await track("Disclose to auditor", (report, ctx) =>
        disclose(ctx, auditorPublicKey, mask, report),
      );
      return outcome === "applied";
    },
    unwrapPending: async () => {
      await track("Unwrap to SOL", (report, ctx) => unwrapWsol(ctx, report));
    },
    abandonStuck: async () => {
      await track("Release stuck operation", async (report, ctx) => {
        const position = positionRef.current;
        if (stuck === null || position === null) return;
        await abandonStuck(ctx, stuck, position, report);
      });
    },
    refreshBand: async () => {
      await track("Refresh rate curve", async (report, ctx) => {
        await refreshPoolBand(ctx, report);
        await refresh();
      });
    },
    refresh,
  };

  return (
    <WalnutContext.Provider value={value}>{children}</WalnutContext.Provider>
  );
}
function equalBytes(a: Uint8Array, b: ArrayLike<number>): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i += 1) if (a[i] !== b[i]) return false;
  return true;
}
