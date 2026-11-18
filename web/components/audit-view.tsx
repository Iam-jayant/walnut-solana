"use client";

import {
  ViewingKeyDecryptError,
  type DisclosedFields,
  type Disclosure,
} from "@walnut/sdk";
import { PublicKey } from "@solana/web3.js";
import { useEffect, useRef, useState } from "react";
import {
  DISCLOSURE_FIELDS,
  fetchDisclosure,
  generateAuditorKeypair,
  openDisclosure,
  parseKey32,
  toBase58,
  toHex,
  type AuditorKeypair,
} from "../lib/audit.ts";
import { SOL_DECIMALS, USDC_DECIMALS } from "../lib/config.ts";
import { friendlyError } from "../lib/ops.ts";
import { formatUnits } from "../lib/format.ts";
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
import { indexesAt } from "../lib/lend.ts";
import { useNowSeconds } from "./use-now.ts";

function fieldValue(
  key: keyof DisclosedFields,
  value: bigint,
  supplyIndex: bigint,
  borrowIndex: bigint,
): { text: string; unit: string; note?: string } {
  if (key === "freeSol" || key === "collateralSol") {
    return { text: formatUnits(value, SOL_DECIMALS), unit: "SOL" };
  }
  if (key === "freeUsdc") {
    return { text: formatUnits(value, USDC_DECIMALS), unit: "USDC" };
  }
  const index = key === "supplySharesUsdc" ? supplyIndex : borrowIndex;
  const usdc = (value * index) / 1_000_000_000n;
  return {
    text: formatUnits(usdc, USDC_DECIMALS),
    unit: "USDC",
    note: `${value.toString()} shares at the current index`,
  };
}

export function AuditView() {
  const { protocol, lending, connection } = useWalnut();
  const now = useNowSeconds(5000);
  const idx = indexesAt(lending, protocol, now);
  const [keys, setKeys] = useState<AuditorKeypair | null>(null);
  const [positionText, setPositionText] = useState("");
  const [secretText, setSecretText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<{
    disclosure: Disclosure;
    fields: DisclosedFields;
  } | null>(null);
  const secretRef = useRef<Uint8Array | null>(null);

  // Drop decrypted values and key material when leaving the page.
  useEffect(
    () => () => {
      secretRef.current?.fill(0);
      keys?.secret.fill(0);
    },
    [],
  );

  const generate = () => {
    keys?.secret.fill(0);
    setKeys(generateAuditorKeypair());
    setResult(null);
  };

  const open = async () => {
    setError(null);
    setResult(null);
    if (protocol === null) {
      setError("Market data is still loading. Try again in a moment.");
      return;
    }
    setBusy(true);
    let secret: Uint8Array | null = null;
    try {
      let position: PublicKey;
      try {
        position = new PublicKey(positionText.trim());
      } catch {
        throw new Error("Enter a valid position address.");
      }
      secret = parseKey32(secretText, "auditor secret key");
      const { x25519 } = await import("@arcium-hq/client");
      const auditorPublic = x25519.getPublicKey(secret);
      const found = await fetchDisclosure(connection, position, auditorPublic);
      if (found === null) {
        throw new Error(
          "No disclosure was found for this position and auditor key.",
        );
      }
      const fields = openDisclosure(
        found.disclosure,
        secret,
        protocol.mxePublicKey,
      );
      setResult({ disclosure: found.disclosure, fields });
    } catch (err) {
      setError(
        err instanceof ViewingKeyDecryptError
          ? "The disclosure could not be decrypted with this secret key."
          : friendlyError(err),
      );
    } finally {
      secret?.fill(0);
      setBusy(false);
    }
  };

  return (
    <div className="bg-walnut-mist">
      <div className="mx-auto mt-8 flex max-w-(--container-page) flex-col gap-6 px-6 py-10 md:px-10">
        <div>
          <h1 className="text-h2 font-[200]">Auditor</h1>
          <p className="mt-2 max-w-2xl text-body text-walnut-ink/80">
            Receive and read a disclosure from a Walnut account holder. Keys are
            generated and used in this browser only. Decryption happens locally,
            and nothing you enter here is sent to a server.
          </p>
        </div>

        <Card>
          <CardTitle
            right={
              <Badge tone="dark">
                <LockIcon /> Local only
              </Badge>
            }
          >
            1. Create an auditor key
          </CardTitle>
          <p className="text-small text-walnut-ink/70">
            Give the public key to the account holder. Keep the secret key
            private; you need it to read the disclosure.
          </p>
          <Button className="mt-3" variant="dark" onClick={generate}>
            Generate key pair
          </Button>
          {keys !== null ? (
            <dl className="mt-4 space-y-3 text-small">
              <div>
                <dt className="text-caption font-[800] uppercase text-walnut-ink/70">
                  Public key (share)
                </dt>
                <dd className="mt-1 break-all rounded-2xl bg-walnut-mist px-3 py-2">
                  {toBase58(keys.publicKey)}
                  <span className="block text-caption text-walnut-ink/60">
                    hex: {toHex(keys.publicKey)}
                  </span>
                </dd>
              </div>
              <div>
                <dt className="text-caption font-[800] uppercase text-walnut-ink/70">
                  Secret key (keep private)
                </dt>
                <dd className="mt-1 break-all rounded-2xl bg-walnut-mist px-3 py-2">
                  {toHex(keys.secret)}
                </dd>
              </div>
            </dl>
          ) : null}
        </Card>

        <Card>
          <CardTitle>2. Open a disclosure</CardTitle>
          <label className="block text-caption font-[800] uppercase text-walnut-ink/70">
            Position address
            <input
              value={positionText}
              onChange={(e) => setPositionText(e.target.value)}
              autoComplete="off"
              spellCheck={false}
              className="mt-1.5 w-full rounded-card bg-walnut-mist px-4 py-3 text-small font-[400] normal-case outline-none ring-1 ring-transparent focus:ring-walnut-black"
            />
          </label>
          <label className="mt-4 block text-caption font-[800] uppercase text-walnut-ink/70">
            Auditor secret key (hex or base58)
            <input
              type="password"
              value={secretText}
              onChange={(e) => setSecretText(e.target.value)}
              autoComplete="off"
              spellCheck={false}
              className="mt-1.5 w-full rounded-card bg-walnut-mist px-4 py-3 text-small font-[400] normal-case outline-none ring-1 ring-transparent focus:ring-walnut-black"
            />
          </label>
          <Button
            className="mt-4"
            disabled={
              busy || positionText.trim() === "" || secretText.trim() === ""
            }
            onClick={() => void open()}
          >
            {busy ? <Spinner /> : <LockIcon />} Fetch and decrypt
          </Button>
          {error !== null ? (
            <div className="mt-4">
              <Notice tone="bad">{error}</Notice>
            </div>
          ) : null}
          {result !== null ? (
            <div className="mt-5">
              <p className="text-caption text-walnut-ink/60">
                Owner {result.disclosure.owner.toBase58()} · snapshot at slot{" "}
                {result.disclosure.slot.toString()}
              </p>
              <div className="mt-3 grid gap-4 sm:grid-cols-2">
                {DISCLOSURE_FIELDS.map((f) => {
                  const value = result.fields[f.key];
                  if (value === undefined) return null;
                  const shown = fieldValue(
                    f.key,
                    value,
                    idx.supply,
                    idx.borrow,
                  );
                  return (
                    <div
                      key={f.key}
                      className="rounded-card bg-walnut-mist p-5"
                    >
                      <p className="text-caption font-[800] uppercase text-walnut-ink/70">
                        {f.label}
                      </p>
                      <p className="mt-2 break-all text-h3 font-[800]">
                        {shown.text}{" "}
                        <span className="text-small font-[400] text-walnut-ink/70">
                          {shown.unit}
                        </span>
                      </p>
                      {shown.note !== undefined ? (
                        <p className="mt-1 text-caption text-walnut-ink/60">
                          {shown.note}
                        </p>
                      ) : null}
                    </div>
                  );
                })}
              </div>
              <p className="mt-3 text-caption text-walnut-ink/60">
                Only the fields the account holder chose are shown.
              </p>
            </div>
          ) : null}
        </Card>
      </div>
    </div>
  );
}
