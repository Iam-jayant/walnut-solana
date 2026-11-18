"use client";

import { explorerTx } from "../lib/config.ts";
import type { Stage } from "../lib/ops.ts";
import { Badge, Card, CardTitle, Spinner } from "./ui.tsx";
import { useWalnut } from "./walnut-provider.tsx";

const STAGE: Record<
  Stage,
  {
    label: string;
    tone: "neutral" | "good" | "warn" | "bad" | "dark";
    live: boolean;
  }
> = {
  signing: { label: "Signing", tone: "neutral", live: true },
  submitted: { label: "Submitted", tone: "neutral", live: true },
  computing: { label: "Computing (MPC)", tone: "warn", live: true },
  applied: { label: "Applied", tone: "good", live: false },
  rejected: { label: "Rejected (state unchanged)", tone: "warn", live: false },
  failed: { label: "Failed", tone: "bad", live: false },
};

export function ActivityPanel() {
  const { activity } = useWalnut();
  return (
    <Card className="h-full">
      <CardTitle>Activity</CardTitle>
      {activity.length === 0 ? (
        <p className="text-small text-walnut-ink/60">
          Operations from this session appear here with their live status.
        </p>
      ) : (
        <ul className="space-y-3">
          {activity.map((item) => {
            const stage = STAGE[item.stage];
            return (
              <li key={item.id} className="rounded-2xl bg-walnut-mist p-4">
                <div className="flex items-center justify-between gap-3">
                  <p className="text-small font-[800]">{item.label}</p>
                  <Badge tone={stage.tone}>
                    {stage.live ? <Spinner /> : null} {stage.label}
                  </Badge>
                </div>
                {item.detail !== undefined ? (
                  <p className="mt-2 text-caption text-walnut-ink/70">
                    {item.detail}
                  </p>
                ) : null}
                {item.links.length > 0 ? (
                  <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1">
                    {item.links.map((link) => (
                      <a
                        key={link.signature}
                        href={explorerTx(link.signature)}
                        target="_blank"
                        rel="noreferrer"
                        className="text-caption font-[800] underline underline-offset-4"
                      >
                        {link.label} · View on Explorer
                      </a>
                    ))}
                  </div>
                ) : null}
              </li>
            );
          })}
        </ul>
      )}
    </Card>
  );
}
