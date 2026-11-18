"use client";

import { useState } from "react";
import { USDC_DECIMALS } from "../lib/config.ts";
import { parseUnits } from "../lib/format.ts";
import type { HealthResult } from "../lib/ops.ts";
import { Badge, Button, Card, CardTitle, Notice, Spinner } from "./ui.tsx";
import { useWalnut } from "./walnut-provider.tsx";

/** Health check and liquidation of any borrower's private account. */
export function LiquidationsPanel() {
  const w = useWalnut();
  const { lending, lendingChecked, protocol, busy, stuck } = w;
  const [borrower, setBorrower] = useState("");
  const [amountText, setAmountText] = useState("");
  const [health, setHealth] = useState<{
    borrower: string;
    result: HealthResult;
  } | null>(null);

  if (lending === null) {
    return (
      <Card>
        <CardTitle>Liquidations</CardTitle>
        {!lendingChecked ? (
          <p className="flex items-center gap-2 text-small text-walnut-ink/70">
            <Spinner /> Loading the lending market...
          </p>
        ) : (
          <Notice tone="info">
            <p className="font-[800]">Lending market is being initialized</p>
            <p className="mt-1">
              Health checks and liquidations open together with the market.
            </p>
          </Notice>
        )}
      </Card>
    );
  }

  const amount = parseUnits(amountText, USDC_DECIMALS);
  const amountValid = amount !== null && amount > 0n;
  const address = borrower.trim();
  const blocked = busy || stuck !== null;
  const shown =
    health !== null && health.borrower === address ? health.result : null;

  const runCheck = async () => {
    const result = await w.checkHealth(address);
    if (result !== undefined) setHealth({ borrower: address, result });
  };

  return (
    <Card>
      <CardTitle>Liquidations</CardTitle>
      <p className="text-small text-walnut-ink/70">
        Anyone can check whether a borrower is below the liquidation threshold.
        The MPC network answers with a single yes or no; balances stay private.
        {protocol !== null
          ? ` A liquidator repays up to ${Number(protocol.closeFactorBps) / 100}% of the debt in USDC and receives the borrower's SOL collateral plus a ${Number(protocol.liquidationBonusBps) / 100}% bonus.`
          : ""}
      </p>
      <label className="mt-4 block text-caption font-[800] uppercase text-walnut-ink/70">
        Borrower wallet address
        <input
          value={borrower}
          onChange={(e) => setBorrower(e.target.value)}
          autoComplete="off"
          spellCheck={false}
          placeholder="Wallet address"
          className="mt-1.5 w-full rounded-card bg-walnut-mist px-4 py-3 text-small font-[400] normal-case outline-none ring-1 ring-transparent focus:ring-walnut-black"
        />
      </label>
      <div className="mt-3 flex flex-wrap items-center gap-3">
        <Button
          variant="dark"
          disabled={address === "" || blocked}
          onClick={() => void runCheck()}
        >
          {busy ? <Spinner /> : null} Check health
        </Button>
        {shown !== null && shown.unhealthy !== null ? (
          shown.unhealthy ? (
            <Badge tone="bad">Below liquidation threshold</Badge>
          ) : (
            <Badge tone="good">Healthy</Badge>
          )
        ) : null}
      </div>
      <div className="mt-5">
        <label className="block text-caption font-[800] uppercase text-walnut-ink/70">
          USDC to repay
          <div className="mt-1.5 flex items-center rounded-card bg-walnut-mist px-4 py-3 ring-1 ring-transparent focus-within:ring-walnut-black">
            <input
              inputMode="decimal"
              autoComplete="off"
              placeholder="0.00"
              value={amountText}
              onChange={(e) => setAmountText(e.target.value)}
              className="min-w-0 flex-1 bg-transparent text-lead font-[800] outline-none placeholder:text-walnut-ink/30"
            />
            <span className="text-small font-[800] text-walnut-ink/70">
              USDC
            </span>
          </div>
        </label>
        <p className="mt-2 text-caption text-walnut-ink/60">
          Your USDC is taken with the transaction and returned if the
          liquidation is refused.
        </p>
        <Button
          className="mt-3 w-full py-3"
          disabled={address === "" || !amountValid || blocked}
          onClick={() =>
            void w
              .liquidate(address, amount ?? 0n)
              .then(() => setAmountText(""))
          }
        >
          {busy ? <Spinner /> : null} Liquidate
        </Button>
      </div>
      {w.pendingLiquidation !== null ? (
        <div className="mt-4">
          <Notice tone="warn">
            <p className="font-[800]">A liquidation is still processing.</p>
            <p className="mt-1">
              If it does not finish before its lock expires, release it to get
              your USDC back.
            </p>
            <Button
              className="mt-3 py-1.5"
              variant="dark"
              disabled={busy}
              onClick={() => void w.abandonLiquidation()}
            >
              Release liquidation
            </Button>
          </Notice>
        </div>
      ) : null}
    </Card>
  );
}
