// Browser replacement for the Node `crypto` module, as imported by
// @arcium-hq/client. Only the pieces the Web App reaches are real: SHA-256 and
// SHA3-256 hashing (computation definition offsets) and random bytes. The AES
// helpers are never used by the Rescue cipher path and throw if called.
import { sha256 } from "@noble/hashes/sha256";
import { sha3_256 } from "@noble/hashes/sha3";
import { Buffer } from "buffer";

export function randomBytes(length: number): Buffer {
  const bytes = new Uint8Array(length);
  globalThis.crypto.getRandomValues(bytes);
  return Buffer.from(bytes);
}

interface Hasher {
  update(data: Uint8Array | string): Hasher;
  digest(): Buffer;
}

export function createHash(algorithm: string): Hasher {
  const chunks: Uint8Array[] = [];
  const hasher: Hasher = {
    update(data: Uint8Array | string): Hasher {
      chunks.push(
        typeof data === "string" ? new TextEncoder().encode(data) : data,
      );
      return hasher;
    },
    digest(): Buffer {
      const all = Buffer.concat(chunks);
      if (algorithm === "sha256") return Buffer.from(sha256(all));
      if (algorithm === "sha3-256") return Buffer.from(sha3_256(all));
      throw new Error(`unsupported hash algorithm: ${algorithm}`);
    },
  };
  return hasher;
}

export function createCipheriv(): never {
  throw new Error("AES is not available in the browser build");
}

export function createDecipheriv(): never {
  throw new Error("AES is not available in the browser build");
}
