"use client";

import { useWallet } from "@solana/wallet-adapter-react";
import { FAUCET_URL, SOL_DECIMALS, USDC_DECIMALS } from "../lib/config.ts";
import { formatUnits } from "../lib/format.ts";
import { borrowMetrics, healthOf, type Health } from "../lib/metrics.ts";
import { kindLabel } from "../lib/ops.ts";
import { ActionPanel } from "./action-panel.tsx";
import { ActivityPanel } from "./activity-panel.tsx";
import {
  Badge,
  Button,
  Card,
  CardTitle,
  LockIcon,
  Notice,
  Spinner,
} from "./ui.tsx";
import { useWalnut } from "./walnut-provider.tsx";

function Hidden() {
  return (
    <span className="inline-flex items-center gap-2 text-walnut-ink/40">
      <LockIcon className="h-5 w-5" /> ••••••
    </span>
  );
}

function Stat({
  label,
  value,
  unit,
  hint,
}: {
  label: string;
  value: string | null;
  unit: string;
  hint?: string;
}) {
  return (
    <div className="rounded-card bg-walnut-mist p-5">
      <p className="text-caption font-[800] uppercase text-walnut-ink/70">
        {label}
      </p>
      <p className="mt-2 break-all text-h3 font-[800]">
        {value === null ? (
          <Hidden />
        ) : (
          <>
            {value}{" "}
            <span className="text-small font-[400] text-walnut-ink/70">
              {unit}
            </span>
          </>
        )}
      </p>
      {hint !== undefined ? (
        <p className="mt-1 text-caption text-walnut-ink/60">{hint}</p>
      ) : null}
    </div>
  );
}

const HEALTH_STYLE: Record<
  Health,
  { label: string; bar: string; tone: "good" | "warn" | "bad" | "neutral" }
> = {
  none: { label: "No debt", bar: "bg-emerald-500", tone: "neutral" },
  healthy: { label: "Healthy", bar: "bg-emerald-500", tone: "good" },
  moderate: { label: "Moderate", bar: "bg-amber-400", tone: "warn" },
  high: { label: "Close to the limit", bar: "bg-red-500", tone: "bad" },
};

function SetupCard() {
  const {
    accountState,
    unlocked,
    unlock,
    createAccount,
    busy,
    protocol,
    error,
  } = useWalnut();

  if (accountState === "loading") {
    return (
      <Card>
        <p className="flex items-center gap-2 text-small text-walnut-ink/70">
          <Spinner /> Loading your account...
        </p>
      </Card>
    );
  }
  if (accountState === "none") {
    return (
      <Card>
        <CardTitle>Create your private account</CardTitle>
        <p className="max-w-2xl text-body text-walnut-ink/80">
          Your account holds your encrypted balances, collateral and debt. You
          will sign a message to create your Viewing Key, then confirm one
          transaction. Setup takes a few seconds while the MPC network
          initializes your encrypted state.
        </p>
        <Button
          className="mt-5"
          onClick={() => void createAccount()}
          disabled={busy || protocol === null}
        >
          {busy ? <Spinner /> : <LockIcon />} Create private account
        </Button>
        {error !== null ? (
          <p className="mt-3 text-small text-red-700">{error}</p>
        ) : null}
      </Card>
    );
  }
  if (accountState === "initializing") {
    return (
      <Card>
        <p className="flex items-center gap-2 text-small font-[800]">
          <Spinner /> Setting up your private account (MPC computing)...
        </p>
        <p className="mt-2 text-small text-walnut-ink/70">
          This page updates automatically once the account is active.
        </p>
      </Card>
    );
  }
  if (accountState === "failed") {
    return (
      <Notice tone="bad">
        Account setup did not complete. Release the stuck operation below if one
        is shown, then refresh.
      </Notice>
    );
  }
  if (!unlocked) {
    return (
      <Card>
        <CardTitle
          right={
            <Badge tone="dark">
              <LockIcon /> Encrypted on-chain
            </Badge>
          }
        >
          Unlock private balances
        </CardTitle>
        <p className="max-w-2xl text-body text-walnut-ink/80">
          Sign a message with your wallet to derive your Viewing Key. It stays
          in this browser tab&apos;s memory only and is cleared when you lock,
          disconnect or leave the page. No transaction is sent.
        </p>
        <Button className="mt-5" onClick={() => void unlock()}>
          <LockIcon /> Unlock private balances
        </Button>
        {error !== null ? (
          <p className="mt-3 text-small text-red-700">{error}</p>
        ) : null}
      </Card>
    );
  }
  return null;
}

function Notices() {
  const { stuck, walletBalances, abandonStuck, unwrapPending, busy } =
    useWalnut();
  return (
    <>
      {stuck !== null ? (
        <Notice tone="warn">
          <p className="font-[800]">
            A previous {kindLabel(stuck.kind)} is still processing.
          </p>
          <p className="mt-1">
            Your account is locked until it completes.{" "}
            {stuck.expired
              ? "It has timed out and can be released now."
              : `It can be released in about ${stuck.secondsLeft}s if it does not finish.`}
          </p>
          <Button
            className="mt-3 py-1.5"
            variant="dark"
            disabled={!stuck.expired || !stuck.supported || busy}
            onClick={() => void abandonStuck()}
          >
            Release stuck operation
          </Button>
        </Notice>
      ) : null}
      {walletBalances !== null && walletBalances.wsol > 0n ? (
        <Notice tone="info">
          <p className="font-[800]">
            {formatUnits(walletBalances.wsol, SOL_DECIMALS)} wrapped SOL is
            waiting in your wallet.
          </p>
          <Button
            className="mt-3 py-1.5"
            variant="dark"
            disabled={busy}
            onClick={() => void unwrapPending()}
          >
            Unwrap to SOL
          </Button>
        </Notice>
      ) : null}
    </>
  );
}

function Overview() {
  const { balances, price, protocol, walletBalances, refresh, refreshing } =
    useWalnut();
  const m =
    balances !== null && protocol !== null
      ? borrowMetrics(balances, price.price, protocol.ltvBps)
      : null;
  const health =
    m === null ? null : HEALTH_STYLE[healthOf(m.usedPercent, m.debt)];
  const usdc = (v: bigint) => formatUnits(v, USDC_DECIMALS);
  const sol = (v: bigint) => formatUnits(v, SOL_DECIMALS);

  return (
    <div className="grid gap-6 lg:grid-cols-3">
      <Card className="lg:col-span-2">
        <CardTitle
          right={
            <div className="flex items-center gap-2">
              <Badge tone="dark">
                <LockIcon /> Encrypted on-chain
              </Badge>
              <Button
                variant="ghost"
                className="py-1.5"
                onClick={() => void refresh()}
                disabled={refreshing}
              >
                {refreshing ? <Spinner /> : null} Refresh
              </Button>
            </div>
          }
        >
          Private position
        </CardTitle>
        <div className="grid gap-4 sm:grid-cols-2">
          <Stat
            label="Free SOL"
            unit="SOL"
            value={balances && sol(balances.freeSol)}
          />
          <Stat
            label="Free USDC"
            unit="USDC"
            value={balances && usdc(balances.freeUsdc)}
          />
          <Stat
            label="Collateral"
            unit="SOL"
            value={balances && sol(balances.collateralSol)}
          />
          <Stat
            label="Debt"
            unit="USDC"
            value={balances && usdc(balances.debtSharesUsdc)}
          />
        </div>
        <div className="mt-6 rounded-card bg-walnut-mist p-5">
          <div className="flex flex-wrap items-end justify-between gap-3">
            <div>
              <p className="text-caption font-[800] uppercase text-walnut-ink/70">
                Borrow limit
              </p>
              <p className="mt-1 text-h3 font-[800]">
                {m === null ? (
                  <Hidden />
                ) : (
                  <>
                    {usdc(m.limit)}{" "}
                    <span className="text-small font-[400] text-walnut-ink/70">
                      USDC
                    </span>
                  </>
                )}
              </p>
            </div>
            <div className="text-right">
              <p className="text-caption font-[800] uppercase text-walnut-ink/70">
                Available to borrow
              </p>
              <p className="mt-1 text-h3 font-[800]">
                {m === null ? (
                  <Hidden />
                ) : (
                  <>
                    {usdc(m.available)}{" "}
                    <span className="text-small font-[400] text-walnut-ink/70">
                      USDC
                    </span>
                  </>
                )}
              </p>
            </div>
          </div>
          <div className="mt-4 h-2.5 w-full overflow-hidden rounded-full bg-white">
            <div
              className={`h-full rounded-full transition-all ${health?.bar ?? "bg-walnut-ink/20"}`}
              style={{
                width: `${m === null ? 0 : Math.max(m.usedPercent, m.debt > 0n ? 2 : 0)}%`,
              }}
            />
          </div>
          <div className="mt-2 flex items-center justify-between text-caption text-walnut-ink/70">
            <span>
              {m === null
                ? "Unlock to see your position health"
                : `${m.usedPercent.toFixed(1)}% of limit used`}
            </span>
            {health !== null ? (
              <Badge tone={health.tone}>{health.label}</Badge>
            ) : null}
          </div>
          <p className="mt-3 text-caption text-walnut-ink/60">
            SOL price (Pyth): {usdc(price.price)} USDC
            {protocol !== null
              ? ` · Max loan-to-value ${Number(protocol.ltvBps) / 100}%`
              : ""}
          </p>
        </div>
      </Card>

      <Card>
        <CardTitle>Wallet</CardTitle>
        <p className="text-caption text-walnut-ink/60">Public balances</p>
        <dl className="mt-3 space-y-3">
          <div className="flex items-baseline justify-between">
            <dt className="text-small text-walnut-ink/70">SOL</dt>
            <dd className="text-lead font-[800]">
              {walletBalances ? sol(walletBalances.sol) : "-"}
            </dd>
          </div>
          <div className="flex items-baseline justify-between">
            <dt className="text-small text-walnut-ink/70">USDC</dt>
            <dd className="text-lead font-[800]">
              {walletBalances ? usdc(walletBalances.usdc) : "-"}
            </dd>
          </div>
        </dl>
        <a
          href={FAUCET_URL}
          target="_blank"
          rel="noreferrer"
          className="mt-5 inline-block text-small font-[800] underline underline-offset-4 hover:text-walnut-ink"
        >
          Get devnet SOL
        </a>
      </Card>
    </div>
  );
}

export function Dashboard() {
  const { connected } = useWallet();
  const { accountState, protocolError, protocol } = useWalnut();
  if (!connected) return null;
  return (
    <div className="bg-walnut-mist">
      <div className="mx-auto mt-8 flex max-w-(--container-page) flex-col gap-6 px-6 py-10 md:px-10">
        {protocolError !== null ? (
          <Notice tone="bad">
            Could not load market data: {protocolError}
          </Notice>
        ) : protocol === null ? (
          <Notice tone="info">
            <span className="flex items-center gap-2">
              <Spinner /> Loading market data...
            </span>
          </Notice>
        ) : null}
        <SetupCard />
        <Notices />
        {accountState === "active" ? (
          <>
            <Overview />
            <div className="grid gap-6 lg:grid-cols-5">
              <div className="lg:col-span-3">
                <ActionPanel />
              </div>
              <div className="lg:col-span-2">
                <ActivityPanel />
              </div>
            </div>
          </>
        ) : (
          <ActivityPanel />
        )}
      </div>
    </div>
  );
}
