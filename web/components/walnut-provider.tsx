"use client";

// Application state: protocol data, the unlocked Viewing Key session, the
// decrypted Position, wallet balances, and the operation actions with live status.
import "../lib/polyfill.ts";
import {
  PositionOp,
  viewingKeyMessageBytes,
  ViewingKeySession,
  type PositionBalances,
  type WalletEventSource,
} from "@walnut/sdk";
import { useWallet } from "@solana/wallet-adapter-react";
import type { Transaction } from "@solana/web3.js";
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
  fetchAccount,
  fetchSolPrice,
  loadProtocol,
  positionAddress,
  tokenBalanceOf,
  variantName,
  type Chain,
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
  abandonStuck,
  deposit,
  findStuckOperation,
  friendlyError,
  initializePosition,
  positionOp,
  unwrapWsol,
  withdraw,
  type Asset,
  type OpOutcome,
  type OpsContext,
  type Progress,
  type Report,
  type Stage,
  type StuckOperation,
} from "../lib/ops.ts";
import walnutIdl from "../lib/idl/walnut.json";

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
  protocol: Protocol | null;
  protocolError: string | null;
  price: PriceInfo;
  accountState: AccountState;
  unlocked: boolean;
  balances: PositionBalances | null;
  walletBalances: WalletBalances | null;
  stuck: StuckOperation | null;
  activity: ActivityItem[];
  busy: boolean;
  error: string | null;
  refreshing: boolean;
  unlock(): Promise<void>;
  lock(): void;
  createAccount(): Promise<void>;
  deposit(asset: Asset, amount: bigint): Promise<void>;
  withdraw(asset: Asset, amount: bigint): Promise<void>;
  positionOp(op: PositionOp, label: string, amount: bigint): Promise<void>;
  unwrapPending(): Promise<void>;
  abandonStuck(): Promise<void>;
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
  const [activity, setActivity] = useState<ActivityItem[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const sessionRef = useRef<ViewingKeySession | null>(null);
  const positionRef = useRef<PositionAccount | null>(null);
  const activityId = useRef(0);
  const busyRef = useRef(false);

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

  // ---- session custody
  const clearSession = useCallback(() => {
    sessionRef.current?.clear();
    sessionRef.current = null;
    setUnlocked(false);
    setBalances(null);
  }, []);

  useEffect(() => {
    // The session clears itself on wallet disconnect and page hide; mirror it.
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
      sessionRef.current?.clear();
      sessionRef.current = session;
      session.attachPage();
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
    async (
      label: string,
      run: (report: Report, ctx: OpsContext) => Promise<OpOutcome | void>,
    ) => {
      if (busyRef.current) return;
      busyRef.current = true;
      setBusy(true);
      setError(null);
      const id = (activityId.current += 1);
      setActivity((items) => [
        { id, label, stage: "signing", links: [], startedAt: Date.now() },
        ...items,
      ]);
      const patch = (fn: (item: ActivityItem) => ActivityItem) =>
        setActivity((items) => items.map((i) => (i.id === id ? fn(i) : i)));
      const report: Report = (p: Progress) =>
        patch((item) => {
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
          return next;
        });
      try {
        await run(report, opsContext());
      } catch (err) {
        patch((item) => ({
          ...item,
          stage: "failed",
          detail: friendlyError(err),
        }));
      } finally {
        busyRef.current = false;
        setBusy(false);
        await refresh();
      }
    },
    [opsContext, refresh],
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
    } catch (err) {
      setError(friendlyError(err));
    }
  }, [adoptSession, chain, protocol, publicKey, refresh, wallet]);

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

  const value: WalnutState = {
    protocol,
    protocolError,
    price,
    accountState,
    unlocked,
    balances,
    walletBalances,
    stuck,
    activity,
    busy,
    error,
    refreshing,
    unlock,
    lock: () => {
      sessionRef.current?.lock();
      clearSession();
    },
    createAccount,
    deposit: (asset, amount) =>
      track(asset === "sol" ? "Deposit SOL" : "Deposit USDC", (report, ctx) =>
        deposit(ctx, asset, amount, report),
      ),
    withdraw: (asset, amount) =>
      track(asset === "sol" ? "Withdraw SOL" : "Withdraw USDC", (report, ctx) =>
        withdraw(ctx, asset, amount, report),
      ),
    positionOp: (op, label, amount) =>
      track(label, (report, ctx) =>
        positionOp(ctx, op, label, amount, price.price, report),
      ),
    unwrapPending: () =>
      track("Unwrap to SOL", (report, ctx) => unwrapWsol(ctx, report)),
    abandonStuck: () =>
      track("Release stuck operation", async (report, ctx) => {
        const position = positionRef.current;
        if (stuck === null || position === null) return;
        await abandonStuck(ctx, stuck, position, report);
      }),
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
