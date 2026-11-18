"use client";

import { useWallet } from "@solana/wallet-adapter-react";
import { useState } from "react";
import { DISCLOSURE_FIELDS, parseKey32 } from "../lib/audit.ts";
import { positionAddress } from "../lib/chain.ts";
import { Button, Card, CardTitle, LockIcon, Notice, Spinner } from "./ui.tsx";
import { useWalnut } from "./walnut-provider.tsx";

/** Viewing Key rotation and disclosure of selected fields to an auditor. */
export function PrivacyPanel() {
  const w = useWalnut();
  const { publicKey } = useWallet();
  const positionText =
    publicKey === null ? null : positionAddress(publicKey).toBase58();
  const { busy, unlocked, stuck } = w;
  const [auditorText, setAuditorText] = useState("");
  const [selected, setSelected] = useState<ReadonlySet<number>>(new Set());
  const [formError, setFormError] = useState<string | null>(null);
  const [done, setDone] = useState(false);

  const blocked = busy || stuck !== null || !unlocked;
  const mask = Array.from(selected).reduce((m, bit) => m | bit, 0);

  const toggle = (bit: number) => {
    setDone(false);
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(bit)) next.delete(bit);
      else next.add(bit);
      return next;
    });
  };

  const submit = async () => {
    setFormError(null);
    setDone(false);
    let key: Uint8Array;
    try {
      key = parseKey32(auditorText, "auditor public key");
    } catch (err) {
      setFormError(err instanceof Error ? err.message : String(err));
      return;
    }
    if (mask === 0) {
      setFormError("Choose at least one field to disclose.");
      return;
    }
    setDone(await w.disclose(key, mask));
  };

  return (
    <Card>
      <CardTitle>Privacy controls</CardTitle>
      {!unlocked ? (
        <p className="mb-4 text-small text-walnut-ink/70">
          Unlock private balances to rotate your Viewing Key or disclose to an
          auditor.
        </p>
      ) : null}

      <div className="rounded-card bg-walnut-mist p-5">
        <h3 className="text-lead font-[800]">Rotate Viewing Key</h3>
        <p className="mt-1 text-small text-walnut-ink/70">
          Creates a new Viewing Key and re-encrypts your account to it through
          the MPC network. Your balances are decrypted with the new key and
          compared with the current ones. Only when they match does this browser
          switch to the new key; otherwise the current key stays in use. You
          sign one message and confirm one transaction.
        </p>
        <Button
          className="mt-3"
          variant="dark"
          disabled={blocked}
          onClick={() => void w.rotateKey()}
        >
          {busy ? <Spinner /> : <LockIcon />} Rotate Viewing Key
        </Button>
      </div>

      <div className="mt-5 rounded-card bg-walnut-mist p-5">
        <h3 className="text-lead font-[800]">Disclose to auditor</h3>
        <p className="mt-1 text-small text-walnut-ink/70">
          Seals the fields you choose to an auditor&apos;s public key. Only the
          holder of the matching secret key can read them, and only the chosen
          fields. Disclosure is a snapshot of your account now.
        </p>
        {positionText !== null ? (
          <p className="mt-2 break-all text-caption text-walnut-ink/70">
            Your position address:{" "}
            <span className="font-[800]">{positionText}</span>
          </p>
        ) : null}
        <label className="mt-4 block text-caption font-[800] uppercase text-walnut-ink/70">
          Auditor public key (base58 or hex, 32 bytes)
          <input
            value={auditorText}
            onChange={(e) => {
              setAuditorText(e.target.value);
              setDone(false);
            }}
            autoComplete="off"
            spellCheck={false}
            className="mt-1.5 w-full rounded-card bg-walnut-white px-4 py-3 text-small font-[400] normal-case outline-none ring-1 ring-black/10 focus:ring-walnut-black"
          />
        </label>
        <fieldset className="mt-4">
          <legend className="text-caption font-[800] uppercase text-walnut-ink/70">
            Fields to disclose
          </legend>
          <div className="mt-2 grid gap-2 sm:grid-cols-2">
            {DISCLOSURE_FIELDS.map((f) => (
              <label
                key={f.key}
                className="flex items-center gap-2 rounded-2xl bg-walnut-white px-3 py-2 text-small"
              >
                <input
                  type="checkbox"
                  checked={selected.has(f.bit)}
                  onChange={() => toggle(f.bit)}
                />
                {f.label}
              </label>
            ))}
          </div>
        </fieldset>
        {formError !== null ? (
          <p className="mt-3 text-small text-red-700">{formError}</p>
        ) : null}
        {done ? (
          <div className="mt-3">
            <Notice tone="info">
              Disclosure sealed. Share your position address with the auditor so
              they can open it on the Audit page.
            </Notice>
          </div>
        ) : null}
        <Button
          className="mt-4"
          disabled={blocked || auditorText.trim() === "" || mask === 0}
          onClick={() => void submit()}
        >
          {busy ? <Spinner /> : null} Disclose
        </Button>
      </div>
    </Card>
  );
}
