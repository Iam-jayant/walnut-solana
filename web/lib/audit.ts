// Auditor-side helpers. Keys are generated and used in the browser only; the
// secret key is never sent anywhere.
import "./polyfill.ts";
import { x25519 } from "@arcium-hq/client";
import {
  decodeDisclosure,
  decryptDisclosure,
  deriveDisclosurePda,
  DisclosureField,
  hexToBytes,
  type DisclosedFields,
  type Disclosure,
} from "@walnut/sdk";
import { PublicKey, type Connection } from "@solana/web3.js";
import bs58 from "bs58";
import { PROGRAM_ID } from "./config.ts";
import { withRetry } from "./chain.ts";

export interface AuditorKeypair {
  secret: Uint8Array;
  publicKey: Uint8Array;
}

/** A fresh x25519 keypair for receiving disclosures. */
export function generateAuditorKeypair(): AuditorKeypair {
  const secret = x25519.utils.randomSecretKey();
  return { secret, publicKey: x25519.getPublicKey(secret) };
}

/** Parses a 32-byte key written as base58 or hex (optionally 0x-prefixed). */
export function parseKey32(text: string, what: string): Uint8Array {
  const t = text.trim();
  if (t === "") throw new Error(`Enter the ${what}.`);
  const hex = t.startsWith("0x") || t.startsWith("0X") ? t.slice(2) : t;
  if (/^[0-9a-fA-F]{64}$/.test(hex)) return hexToBytes(hex);
  try {
    const bytes = bs58.decode(t);
    if (bytes.length === 32) return bytes;
  } catch {
    /* fall through */
  }
  throw new Error(`The ${what} must be 32 bytes in base58 or hex.`);
}

export function toHex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

export function toBase58(bytes: Uint8Array): string {
  return bs58.encode(bytes);
}

/** The disclosable fields in mask bit order. */
export const DISCLOSURE_FIELDS: ReadonlyArray<{
  key: keyof DisclosedFields;
  bit: number;
  label: string;
  unit: "SOL" | "USDC" | "USDC shares";
}> = [
  {
    key: "freeSol",
    bit: DisclosureField.FreeSol,
    label: "Free SOL",
    unit: "SOL",
  },
  {
    key: "freeUsdc",
    bit: DisclosureField.FreeUsdc,
    label: "Free USDC",
    unit: "USDC",
  },
  {
    key: "collateralSol",
    bit: DisclosureField.CollateralSol,
    label: "Collateral",
    unit: "SOL",
  },
  {
    key: "supplySharesUsdc",
    bit: DisclosureField.SupplyShares,
    label: "Supply shares",
    unit: "USDC shares",
  },
  {
    key: "debtSharesUsdc",
    bit: DisclosureField.DebtShares,
    label: "Debt shares",
    unit: "USDC shares",
  },
];

export interface FetchedDisclosure {
  address: PublicKey;
  disclosure: Disclosure;
}

/** Reads the Disclosure PDA `["disclosure", position, auditor]`; null if absent. */
export async function fetchDisclosure(
  connection: Connection,
  position: PublicKey,
  auditorPublicKey: Uint8Array,
): Promise<FetchedDisclosure | null> {
  const address = deriveDisclosurePda(
    PROGRAM_ID,
    position,
    new PublicKey(auditorPublicKey),
  );
  const info = await withRetry(() =>
    connection.getAccountInfo(address, "confirmed"),
  );
  if (info === null) return null;
  return { address, disclosure: decodeDisclosure(info.data) };
}

/** Decrypts locally; only the fields selected by the mask are returned. */
export function openDisclosure(
  disclosure: Disclosure,
  auditorSecret: Uint8Array,
  mxePublicKey: Uint8Array,
): DisclosedFields {
  return decryptDisclosure(disclosure, auditorSecret, mxePublicKey);
}
