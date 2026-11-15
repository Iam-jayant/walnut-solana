// Task 2.5 transaction-envelope size prototypes (Arcium feasibility gate).
//
// Pure Node/TypeScript size measurement: for every v1 operation this builds the
// realistic QUEUE and CALLBACK transactions, serializes them as versioned (v0)
// transactions with and without an address lookup table, and asserts each stays
// under the 1,232-byte hard limit while flagging any over the 1,150-byte target.
//
// No localnet, RPC, or on-chain execution is required -- the transactions are
// built and serialized entirely in memory, so this runs in CI (the rust-arcium
// job's `pnpm test:tx-size` step) without an Arcium cluster. The pubkeys are
// deterministic placeholders; serialized size depends only on account count,
// the signer/writable split, instruction data length, and lookup-table
// membership, never on the actual key bytes. Account inventories live in
// accounts.ts and come from design.md plus the proven @arcium-hq/client 0.16.0
// account layout.

import * as web3 from "@solana/web3.js";
import {
  OPERATIONS,
  arciumCallbackStd,
  arciumQueueStd,
  type AccountRef,
  type Envelope,
  type ExtraIx,
  type OperationPrototype,
} from "./accounts.ts";

const {
  PublicKey,
  TransactionInstruction,
  TransactionMessage,
  AddressLookupTableAccount,
  ComputeBudgetProgram,
  SystemProgram,
} = web3;

const HARD_LIMIT = 1232;
const TARGET = 1150;

// Deterministic distinct placeholder keys, with real ids for known programs.
const keyRegistry = new Map<string, web3.PublicKey>();
let keyCounter = 1;

function key(name: string): web3.PublicKey {
  if (name === "compute-budget") return ComputeBudgetProgram.programId;
  if (name === "system-program") return SystemProgram.programId;
  const existing = keyRegistry.get(name);
  if (existing) return existing;
  const buf = Buffer.alloc(32);
  buf.writeUInt32BE(keyCounter, 28);
  keyCounter += 1;
  const pk = new PublicKey(buf);
  keyRegistry.set(name, pk);
  return pk;
}

const WALNUT_PROGRAM = "walnut-program";
const ALT_KEY = "address-lookup-table";
// A valid 32-byte blockhash placeholder (all-zero base58).
const BLOCKHASH = PublicKey.default.toBase58();

function toMeta(ref: AccountRef): web3.AccountMeta {
  return {
    pubkey: key(ref.name),
    isSigner: ref.signer,
    isWritable: ref.writable,
  };
}

function buildIx(
  programName: string,
  refs: AccountRef[],
  dataLen: number,
): web3.TransactionInstruction {
  return new TransactionInstruction({
    programId: key(programName),
    keys: refs.map(toMeta),
    data: Buffer.alloc(dataLen),
  });
}

function extraToIx(extra: ExtraIx): web3.TransactionInstruction {
  return buildIx(extra.programName, extra.accounts, extra.dataLen);
}

interface Assembled {
  payer: web3.PublicKey;
  instructions: web3.TransactionInstruction[];
  /** All account refs across every instruction, for lookup-table classification. */
  refs: AccountRef[];
}

function assemble(
  payerName: string,
  std: AccountRef[],
  env: Envelope,
): Assembled {
  const payerRef: AccountRef = {
    name: payerName,
    signer: true,
    writable: true,
  };
  const mainRefs = [payerRef, ...std, ...env.accounts];
  const main = buildIx(WALNUT_PROGRAM, mainRefs, env.dataLen);
  const extras = env.extraIxs.map(extraToIx);
  const refs: AccountRef[] = [
    payerRef,
    ...std,
    ...env.accounts,
    ...env.extraIxs.flatMap((e) => e.accounts),
  ];
  // Extra ix program ids participate in the static key set as invoked programs.
  for (const e of env.extraIxs) {
    refs.push({
      name: e.programName,
      signer: false,
      writable: false,
      program: true,
    });
  }
  refs.push({
    name: WALNUT_PROGRAM,
    signer: false,
    writable: false,
    program: true,
  });
  return { payer: key(payerName), instructions: [main, ...extras], refs };
}

/** Names that may be loaded from a lookup table: non-signer, non-program. */
function lookupEligible(refs: AccountRef[]): web3.PublicKey[] {
  const signers = new Set<string>();
  const programs = new Set<string>();
  for (const r of refs) {
    if (r.signer) signers.add(r.name);
    if (r.program) programs.add(r.name);
  }
  const seen = new Set<string>();
  const out: web3.PublicKey[] = [];
  for (const r of refs) {
    if (signers.has(r.name) || programs.has(r.name)) continue;
    if (seen.has(r.name)) continue;
    seen.add(r.name);
    out.push(key(r.name));
  }
  return out;
}

type V0Message = ReturnType<web3.TransactionMessage["compileToV0Message"]>;

/** Byte length of a shortvec (compact-u16) length prefix. */
function compactLen(n: number): number {
  if (n < 0x80) return 1;
  if (n < 0x4000) return 2;
  return 3;
}

/**
 * Exact serialized byte length of a compiled v0 transaction, computed from the
 * compiled message structure. web3's own MessageV0.serialize() encodes into
 * fixed 1,232-byte buffers and throws on an oversized envelope, so measuring
 * an over-limit transaction (which the gate must be able to report rather than
 * crash on) is done analytically from the authoritative compiled message. The
 * formula is the Solana wire format: signatures + version + header + static
 * keys + blockhash + instructions + address-table lookups.
 */
function v0ByteLength(message: V0Message): number {
  const numSigs = message.header.numRequiredSignatures;
  let len = 1 + 3; // version prefix + 3-byte header
  len +=
    compactLen(message.staticAccountKeys.length) +
    32 * message.staticAccountKeys.length;
  len += 32; // recent blockhash
  len += compactLen(message.compiledInstructions.length);
  for (const ix of message.compiledInstructions) {
    len += 1; // program id index
    len +=
      compactLen(ix.accountKeyIndexes.length) + ix.accountKeyIndexes.length;
    len += compactLen(ix.data.length) + ix.data.length;
  }
  len += compactLen(message.addressTableLookups.length);
  for (const lut of message.addressTableLookups) {
    len += 32; // lookup table account key
    len += compactLen(lut.writableIndexes.length) + lut.writableIndexes.length;
    len += compactLen(lut.readonlyIndexes.length) + lut.readonlyIndexes.length;
  }
  return compactLen(numSigs) + 64 * numSigs + len;
}

function serializedSize(
  a: Assembled,
  useAlt: boolean,
): { bytes: number; altAccounts: number } {
  let lookupTables: web3.AddressLookupTableAccount[] = [];
  let altAccounts = 0;
  if (useAlt) {
    const addresses = lookupEligible(a.refs);
    altAccounts = addresses.length;
    lookupTables = [
      new AddressLookupTableAccount({
        key: key(ALT_KEY),
        state: {
          deactivationSlot: BigInt("18446744073709551615"),
          lastExtendedSlot: 0,
          lastExtendedSlotStartIndex: 0,
          addresses,
        },
      }),
    ];
  }
  const message = new TransactionMessage({
    payerKey: a.payer,
    recentBlockhash: BLOCKHASH,
    instructions: a.instructions,
  }).compileToV0Message(lookupTables);
  return { bytes: v0ByteLength(message), altAccounts };
}

interface PhaseResult {
  noAlt: number;
  alt: number;
  altAccounts: number;
  altNeeded: boolean;
  final: number;
  passesLimit: boolean;
  overTarget: boolean;
}

function measurePhase(a: Assembled): PhaseResult {
  const noAltRes = serializedSize(a, false);
  const altRes = serializedSize(a, true);
  const altNeeded = noAltRes.bytes > TARGET;
  const final = altNeeded ? altRes.bytes : noAltRes.bytes;
  return {
    noAlt: noAltRes.bytes,
    alt: altRes.bytes,
    altAccounts: altRes.altAccounts,
    altNeeded,
    final,
    passesLimit: final < HARD_LIMIT,
    overTarget: final > TARGET,
  };
}

interface OpResult {
  op: OperationPrototype;
  queue: PhaseResult;
  callback: PhaseResult;
}

function measureOperation(op: OperationPrototype): OpResult {
  const queue = measurePhase(
    assemble("queue-payer", arciumQueueStd(), op.queue),
  );
  const callback = measurePhase(
    assemble("callback-node", arciumCallbackStd(), op.callback),
  );
  return { op, queue, callback };
}

function pad(value: string, width: number): string {
  return value.length >= width
    ? value
    : value + " ".repeat(width - value.length);
}

function cell(p: PhaseResult): string {
  const chosen = p.altNeeded ? `${p.final} (ALT)` : `${p.final}`;
  const base = p.altNeeded ? `${p.noAlt}->${p.alt}` : `${p.noAlt}`;
  return `${chosen} [${base}]`;
}

function status(p: PhaseResult): string {
  if (!p.passesLimit) return "FAIL >=1232";
  if (p.overTarget) return "OVER TARGET";
  return "ok";
}

function main(): void {
  const results = OPERATIONS.map(measureOperation);

  const rows = results.map((r) => ({
    title: r.op.title,
    src: r.op.provenance,
    queue: cell(r.queue),
    qstatus: status(r.queue),
    callback: cell(r.callback),
    cstatus: status(r.callback),
    cu: r.op.callback.computeUnitLimit.toLocaleString("en-US"),
  }));

  const headers = {
    title: "Operation",
    src: "Source",
    queue: "Queue bytes [base]",
    qstatus: "Queue",
    callback: "Callback bytes [base]",
    cstatus: "Callback",
    cu: "Callback CU",
  };

  const widths = {
    title: Math.max(headers.title.length, ...rows.map((r) => r.title.length)),
    src: Math.max(headers.src.length, ...rows.map((r) => r.src.length)),
    queue: Math.max(headers.queue.length, ...rows.map((r) => r.queue.length)),
    qstatus: Math.max(
      headers.qstatus.length,
      ...rows.map((r) => r.qstatus.length),
    ),
    callback: Math.max(
      headers.callback.length,
      ...rows.map((r) => r.callback.length),
    ),
    cstatus: Math.max(
      headers.cstatus.length,
      ...rows.map((r) => r.cstatus.length),
    ),
    cu: Math.max(headers.cu.length, ...rows.map((r) => r.cu.length)),
  };

  const line = (c: typeof headers): string =>
    [
      pad(c.title, widths.title),
      pad(c.src, widths.src),
      pad(c.queue, widths.queue),
      pad(c.qstatus, widths.qstatus),
      pad(c.callback, widths.callback),
      pad(c.cstatus, widths.cstatus),
      pad(c.cu, widths.cu),
    ].join("  ");

  console.log(
    `Transaction-envelope size prototypes (hard limit ${HARD_LIMIT} B, target ${TARGET} B)`,
  );
  console.log(
    "Cell format: final bytes [serialized without ALT -> with ALT]. (ALT) = lookup table used to fit.\n",
  );
  console.log(line(headers));
  console.log("-".repeat(line(headers).length));
  for (const r of rows) {
    console.log(line(r));
  }

  const altOps = results
    .filter((r) => r.queue.altNeeded || r.callback.altNeeded)
    .map((r) => r.op.title);
  const overTarget = results.filter(
    (r) => r.queue.overTarget || r.callback.overTarget,
  );
  const failures = results.filter(
    (r) => !r.queue.passesLimit || !r.callback.passesLimit,
  );

  console.log(
    `\nOperations that require an address lookup table to meet the target: ${
      altOps.length ? altOps.join(", ") : "none"
    }`,
  );
  if (overTarget.length) {
    console.log(
      `Over the ${TARGET}-byte target (still under the hard limit): ${overTarget
        .map((r) => r.op.title)
        .join(", ")}`,
    );
  }

  if (failures.length) {
    console.error(
      `\nFAIL: ${failures.length} operation(s) exceed the ${HARD_LIMIT}-byte hard limit:`,
    );
    for (const r of failures) {
      if (!r.queue.passesLimit) {
        console.error(`  ${r.op.title} queue = ${r.queue.final} bytes`);
      }
      if (!r.callback.passesLimit) {
        console.error(`  ${r.op.title} callback = ${r.callback.final} bytes`);
      }
    }
    process.exit(1);
  }

  console.log(`\nAll ${results.length} operations are within the hard limit.`);
}

main();
