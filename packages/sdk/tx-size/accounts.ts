// Account inventories for the Task 2.5 transaction-envelope prototypes.
//
// These describe, per v1 operation, the full ordered account list and
// instruction-data size of the QUEUE transaction (user/keeper -> Arcium) and
// the CALLBACK transaction (Arcium node -> Walnut program) that the real
// operation will carry. They are the faithful account *inventory* the size
// model consumes; the pubkeys themselves are placeholders because serialized
// size depends only on the account count, the signer/writable split, the
// instruction data length, and address-lookup-table membership -- never on the
// actual 32-byte value of a key.
//
// Standard Arcium account sets are modeled on the @arcium-hq/client 0.16.0
// account layout proven in tests/walnut.ts and tests/failure-matrix.ts (the
// queue path lists payer, MXE, mempool, executing pool, computation account,
// comp-def, cluster, fee pool, clock, system program, and the Arcium program;
// callbacks additionally carry the instructions sysvar and the Arcium signer
// PDA). Custom accounts come from design.md (Position/Pool/Intent/PendingShield
// /LiquidationRecord/RecoveryClaim PDAs, vaults, token accounts, Pyth price
// accounts, treasury accounts). Receipt ciphertexts are included in the
// liquidation callback data because design.md requires the size tests to
// include both ciphertexts.

/** A single account reference inside an instruction. */
export interface AccountRef {
  /** Stable logical name; identical names dedupe into one transaction key. */
  name: string;
  signer: boolean;
  writable: boolean;
  /**
   * True when this key is an invoked program id (system, Arcium, SPL token,
   * compute budget, Walnut). Program ids and signers can never be loaded from
   * an address lookup table, so the model keeps them in the static key list.
   */
  program?: boolean;
}

/** One extra instruction carried in the same transaction as the Arcium ix. */
export interface ExtraIx {
  label: string;
  programName: string;
  accounts: AccountRef[];
  dataLen: number;
}

/** The queue or callback envelope for one operation. */
export interface Envelope {
  /** Accounts on the Walnut (MXE) instruction, in order. */
  accounts: AccountRef[];
  /** Serialized Anchor instruction data length in bytes (incl. discriminator). */
  dataLen: number;
  /** Additional instructions in the same transaction (token CPIs, createAccount). */
  extraIxs: ExtraIx[];
  /** Simulated callback compute-unit limit (callback only; documents the CU model). */
  computeUnitLimit: number;
}

export interface OperationPrototype {
  key: string;
  title: string;
  /** "real" = built from an existing/known instruction; "modeled" = from design account list. */
  provenance: "real" | "modeled";
  queue: Envelope;
  callback: Envelope;
  notes: string;
}

// ---------------------------------------------------------------------------
// Instruction-data size model (bytes).
//
// Anchor discriminator .......................................... 8
// computation offset (u64) ...................................... 8
// x25519 public key ............................................. 32
// RescueCipher nonce (u128) ..................................... 16
// one RescueCipher ciphertext field ............................. 32
// u64 amount / slot ............................................. 8
// u128 shares ................................................... 16
// bool / u8 band index .......................................... 1
//
// Encrypted Position payload (EncGroup over PositionSecret's 5 fields):
//   enc pubkey(32) + nonce(16) + 5*32 = 208 bytes.
// Encrypted Pool payload (Enc<Mxe> over PoolSecret's 4 fields):
//   nonce(16) + 4*32 = 144 bytes (no per-user pubkey).
// Encrypted RecoveryClaim payload (4 fields): nonce(16) + 4*32 = 144 bytes.
// Fixed-size Receipt ciphertext (3 fields): pubkey(32) + nonce(16) + 3*32 = 144.
// ---------------------------------------------------------------------------

const DISC = 8;
const COMP_OFFSET = 8;
const PUBKEY = 32;
const NONCE = 16;
const FIELD = 32;
const U64 = 8;
const U128 = 16;
const FLAG = 1;

const ENC_POSITION = PUBKEY + NONCE + 5 * FIELD; // 208
const ENC_POOL = NONCE + 4 * FIELD; // 144
const ENC_CLAIM = NONCE + 4 * FIELD; // 144
const RECEIPT = PUBKEY + NONCE + 3 * FIELD; // 144
const ENC_INPUT = PUBKEY + NONCE + FIELD; // 80: one sealed input + its key/nonce

// Shared account refs -------------------------------------------------------

const market = (writable: boolean): AccountRef => ({
  name: "market",
  signer: false,
  writable,
});
const assetState: AccountRef = {
  name: "asset-state",
  signer: false,
  writable: false,
};
const pool = (writable: boolean): AccountRef => ({
  name: "pool-state",
  signer: false,
  writable,
});
const position = (writable: boolean): AccountRef => ({
  name: "position",
  signer: false,
  writable,
});
const intent: AccountRef = {
  name: "operation-intent",
  signer: false,
  writable: true,
};
const tokenProgram: AccountRef = {
  name: "token-program",
  signer: false,
  writable: false,
  program: true,
};
const pythSol: AccountRef = {
  name: "pyth-sol-usd",
  signer: false,
  writable: false,
};
const pythUsdc: AccountRef = {
  name: "pyth-usdc-usd",
  signer: false,
  writable: false,
};
const vaultAuthority: AccountRef = {
  name: "vault-authority",
  signer: false,
  writable: false,
};

/** Standard Arcium queue accounts, excluding the payer (added per envelope). */
export function arciumQueueStd(): AccountRef[] {
  return [
    { name: "arc-mxe", signer: false, writable: false },
    { name: "arc-mempool", signer: false, writable: true },
    { name: "arc-executing-pool", signer: false, writable: true },
    { name: "arc-computation", signer: false, writable: true },
    { name: "arc-comp-def", signer: false, writable: false },
    { name: "arc-cluster", signer: false, writable: true },
    { name: "arc-fee-pool", signer: false, writable: true },
    { name: "arc-clock", signer: false, writable: false },
    { name: "system-program", signer: false, writable: false, program: true },
    { name: "arcium-program", signer: false, writable: false, program: true },
  ];
}

/** Standard Arcium callback accounts, excluding the node payer. */
export function arciumCallbackStd(): AccountRef[] {
  return [
    { name: "arc-computation", signer: false, writable: true },
    { name: "arc-cluster", signer: false, writable: true },
    { name: "arc-fee-pool", signer: false, writable: true },
    { name: "arc-executing-pool", signer: false, writable: true },
    { name: "arc-instructions-sysvar", signer: false, writable: false },
    { name: "arc-signer", signer: false, writable: false },
    { name: "arcium-program", signer: false, writable: false, program: true },
  ];
}

/** SPL token transfer instruction (3 accounts, 9 data bytes). */
function tokenTransfer(
  label: string,
  source: string,
  dest: string,
  authority: AccountRef,
): ExtraIx {
  return {
    label,
    programName: "token-program",
    accounts: [
      { name: source, signer: false, writable: true },
      { name: dest, signer: false, writable: true },
      authority,
    ],
    dataLen: 9,
  };
}

// Compute-unit limits simulated for each callback. These reflect the heaviest
// realistic circuit-output verification + state writes per operation and are
// documented in docs/adr inputs. They change only the 4-byte argument of the
// compute-budget instruction, not the account layout.
const CU = {
  light: 200_000, // reveal-only boolean/band writes
  position: 400_000, // single encrypted Position rewrite
  pool: 600_000, // Position + Pool interest checkpoint + rewrite
  cpi: 700_000, // pool write + SPL token CPI
  liquidation: 1_200_000, // two Positions + Pool + record + two Receipts
  absorb: 1_000_000, // Position + Pool + Recovery Claim
};

/**
 * Every queue and callback transaction includes a compute-unit limit and a
 * compute-unit price instruction. Both target the compute-budget program and
 * carry fixed-size data (limit = 9 bytes, price = 9 bytes).
 */
export function computeBudgetIxs(limit: number): ExtraIx[] {
  return [
    {
      label: `setComputeUnitLimit(${limit})`,
      programName: "compute-budget",
      accounts: [],
      dataLen: 9,
    },
    {
      label: "setComputeUnitPrice",
      programName: "compute-budget",
      accounts: [],
      dataLen: 9,
    },
  ];
}

export const OPERATIONS: OperationPrototype[] = [
  {
    key: "position-init",
    title: "Position initialization",
    provenance: "real",
    queue: {
      accounts: [market(true), assetState, position(true), intent],
      dataLen: DISC + COMP_OFFSET + PUBKEY + NONCE, // viewing pubkey + nonce
      extraIxs: computeBudgetIxs(CU.light),
      computeUnitLimit: 0,
    },
    callback: {
      accounts: [position(true), intent, market(true)],
      dataLen: DISC + ENC_POSITION,
      extraIxs: computeBudgetIxs(CU.position),
      computeUnitLimit: CU.position,
    },
    notes:
      "Built from the real init_position instruction; owner is the payer. Writes encrypted zero-state.",
  },
  {
    key: "shield",
    title: "Shield (USDC/SOL deposit)",
    provenance: "real",
    // Task 5.3 `shield` account list, in instruction order after the payer
    // and the standard Arcium queue accounts. The SPL `transfer_checked`
    // into the vault is a CPI inside `shield`, so it adds no instruction.
    // The SOL envelope is modeled because it is the larger one: the client
    // wraps in the same transaction (System transfer into the owner's wSOL
    // token account, then SPL `SyncNative`).
    queue: {
      accounts: [
        { name: "arc-sign-pda", signer: false, writable: true },
        market(false),
        assetState,
        position(true),
        { name: "operation-counter", signer: false, writable: true },
        intent,
        { name: "pending-shield", signer: false, writable: true },
        { name: "asset-mint", signer: false, writable: false },
        { name: "vault-token-account", signer: false, writable: true },
        { name: "user-token-account", signer: false, writable: true },
        tokenProgram,
      ],
      // computation offset, Intent sequence, shield sequence, asset, amount
      dataLen: DISC + COMP_OFFSET + U64 + U64 + FLAG + U64,
      extraIxs: [
        {
          label: "system transfer owner->wSOL account",
          programName: "system-program",
          accounts: [
            { name: "queue-payer", signer: true, writable: true },
            { name: "user-token-account", signer: false, writable: true },
          ],
          dataLen: 12,
        },
        {
          label: "spl SyncNative",
          programName: "token-program",
          accounts: [
            { name: "user-token-account", signer: false, writable: true },
          ],
          dataLen: 1,
        },
        ...computeBudgetIxs(CU.position),
      ],
      computeUnitLimit: 0,
    },
    // `shield_credit` callback custom accounts: Market (read-only), Position,
    // Pending Shield, Operation Intent. Output: the re-sealed Position state
    // plus the revealed "credit fits" flag.
    callback: {
      accounts: [
        market(false),
        position(true),
        { name: "pending-shield", signer: false, writable: true },
        intent,
      ],
      dataLen: DISC + ENC_POSITION + FLAG,
      extraIxs: computeBudgetIxs(CU.position),
      computeUnitLimit: CU.position,
    },
    notes:
      "Real Task 5.3 shield: wrap (SOL) + shield with transfer_checked CPI in the queue tx; encrypted credit in the callback.",
  },
  {
    key: "unshield",
    title: "Unshield (+ token CPI)",
    provenance: "real",
    // Task 5.4 `unshield` account list, in instruction order after the payer
    // and the standard Arcium queue accounts. The destination is read-only
    // here; the asset mint, vault, and vault authority are not queue accounts
    // (the Market supplies their addresses to the callback).
    queue: {
      accounts: [
        { name: "arc-sign-pda", signer: false, writable: true },
        market(false),
        assetState,
        position(true),
        { name: "operation-counter", signer: false, writable: true },
        intent,
        { name: "user-token-account", signer: false, writable: false },
      ],
      // computation offset, Intent sequence, asset, amount
      dataLen: DISC + COMP_OFFSET + U64 + FLAG + U64,
      extraIxs: computeBudgetIxs(CU.position),
      computeUnitLimit: 0,
    },
    // `unshield` callback custom accounts, in struct order: Market, Position,
    // Operation Intent, asset mint, vault, destination, vault authority, SPL
    // Token program. Output: the re-sealed Position state plus the revealed
    // approved and requested amounts. The vault transfer is a CPI inside the
    // callback, so it adds no instruction.
    callback: {
      accounts: [
        market(false),
        position(true),
        intent,
        { name: "asset-mint", signer: false, writable: false },
        { name: "vault-token-account", signer: false, writable: true },
        { name: "user-token-account", signer: false, writable: true },
        vaultAuthority,
        tokenProgram,
      ],
      dataLen: DISC + ENC_POSITION + U64 + U64,
      extraIxs: computeBudgetIxs(CU.cpi),
      computeUnitLimit: CU.cpi,
    },
    notes:
      "Real Task 5.4 unshield: queue binds the destination; the callback transfers the approved SPL amount with the vault PDA authority (CPI).",
  },
  {
    key: "supply",
    title: "Supply USDC",
    provenance: "modeled",
    queue: {
      accounts: [market(false), assetState, pool(true), position(true), intent],
      dataLen: DISC + COMP_OFFSET + U64 + ENC_INPUT,
      extraIxs: computeBudgetIxs(CU.light),
      computeUnitLimit: 0,
    },
    callback: {
      accounts: [pool(true), position(true), intent],
      dataLen: DISC + ENC_POSITION + ENC_POOL,
      extraIxs: computeBudgetIxs(CU.pool),
      computeUnitLimit: CU.pool,
    },
    notes:
      "Modeled: interest checkpoint + share mint; writes Position and Pool.",
  },
  {
    key: "withdraw-supply",
    title: "Withdraw supply",
    provenance: "modeled",
    queue: {
      accounts: [market(false), assetState, pool(true), position(true), intent],
      dataLen: DISC + COMP_OFFSET + U128 + ENC_INPUT,
      extraIxs: computeBudgetIxs(CU.light),
      computeUnitLimit: 0,
    },
    callback: {
      accounts: [pool(true), position(true), intent],
      dataLen: DISC + ENC_POSITION + ENC_POOL,
      extraIxs: computeBudgetIxs(CU.pool),
      computeUnitLimit: CU.pool,
    },
    notes: "Modeled: NAV claim from encrypted supplier equity; internal only.",
  },
  {
    key: "borrow",
    title: "Borrow USDC",
    provenance: "modeled",
    queue: {
      accounts: [
        market(false),
        assetState,
        pool(true),
        position(true),
        intent,
        pythSol,
        pythUsdc,
      ],
      dataLen: DISC + COMP_OFFSET + U64 + ENC_INPUT,
      extraIxs: computeBudgetIxs(CU.light),
      computeUnitLimit: 0,
    },
    callback: {
      accounts: [pool(true), position(true), intent, market(true)],
      dataLen: DISC + ENC_POSITION + ENC_POOL + FLAG,
      extraIxs: computeBudgetIxs(CU.pool),
      computeUnitLimit: CU.pool,
    },
    notes:
      "Modeled: fresh Pyth + utilization band, caps and projected-risk health; reveals only zero->nonzero debt transition.",
  },
  {
    key: "repay",
    title: "Repay",
    provenance: "modeled",
    queue: {
      accounts: [market(false), assetState, pool(true), position(true), intent],
      dataLen: DISC + COMP_OFFSET + FLAG + U64 + ENC_INPUT,
      extraIxs: computeBudgetIxs(CU.light),
      computeUnitLimit: 0,
    },
    callback: {
      accounts: [
        pool(true),
        position(true),
        intent,
        market(true),
        { name: "liquidation-record", signer: false, writable: true },
      ],
      dataLen: DISC + ENC_POSITION + ENC_POOL + FLAG,
      extraIxs: computeBudgetIxs(CU.pool),
      computeUnitLimit: CU.pool,
    },
    notes:
      "Modeled: full repay resolves a matching Liquidation Record; reveals only nonzero->zero transition.",
  },
  {
    key: "reserve-withdrawal",
    title: "Reserve withdrawal (+ treasury CPI)",
    provenance: "modeled",
    queue: {
      accounts: [
        market(false),
        assetState,
        pool(true),
        intent,
        { name: "treasury-usdc-account", signer: false, writable: false },
      ],
      dataLen: DISC + COMP_OFFSET + U64,
      extraIxs: computeBudgetIxs(CU.light),
      computeUnitLimit: 0,
    },
    callback: {
      accounts: [
        pool(true),
        intent,
        { name: "vault-usdc-account", signer: false, writable: true },
        { name: "treasury-usdc-account", signer: false, writable: true },
        vaultAuthority,
        tokenProgram,
      ],
      dataLen: DISC + ENC_POOL + U64,
      extraIxs: [
        tokenTransfer(
          "spl transfer vault->treasury",
          "vault-usdc-account",
          "treasury-usdc-account",
          vaultAuthority,
        ),
        ...computeBudgetIxs(CU.cpi),
      ],
      computeUnitLimit: CU.cpi,
    },
    notes:
      "Modeled: all-or-zero public request; equal Cash and Reserve reduction leaves supplier equity unchanged.",
  },
  {
    key: "utilization-band",
    title: "Utilization-band update",
    provenance: "modeled",
    queue: {
      accounts: [market(false), assetState, pool(false), intent],
      dataLen: DISC + COMP_OFFSET,
      extraIxs: computeBudgetIxs(CU.light),
      computeUnitLimit: 0,
    },
    callback: {
      accounts: [
        { name: "asset-state", signer: false, writable: true },
        intent,
      ],
      dataLen: DISC + FLAG, // band index only
      extraIxs: computeBudgetIxs(CU.light),
      computeUnitLimit: CU.light,
    },
    notes:
      "Modeled: read-only-generation computation; reveals and stores only the band index.",
  },
  {
    key: "health",
    title: "Health check callback",
    provenance: "modeled",
    queue: {
      accounts: [
        market(false),
        assetState,
        position(false),
        intent,
        pythSol,
        pythUsdc,
      ],
      dataLen: DISC + COMP_OFFSET,
      extraIxs: computeBudgetIxs(CU.light),
      computeUnitLimit: 0,
    },
    callback: {
      accounts: [position(true), intent, market(true)],
      dataLen: DISC + FLAG, // unhealthy boolean only
      extraIxs: computeBudgetIxs(CU.light),
      computeUnitLimit: CU.light,
    },
    notes:
      "Modeled: non-locking health; freeze only on current generation and no active writer Lock.",
  },
  {
    key: "liquidation",
    title: "Two-Position partial liquidation (+2 Receipts)",
    provenance: "real",
    queue: {
      accounts: [
        market(false),
        assetState,
        pool(true),
        { name: "borrower-position", signer: false, writable: true },
        { name: "liquidator-position", signer: false, writable: true },
        { name: "liquidation-record", signer: false, writable: true },
        intent,
        pythSol,
        pythUsdc,
      ],
      dataLen: DISC + COMP_OFFSET + ENC_INPUT, // encrypted max repayment
      extraIxs: computeBudgetIxs(CU.light),
      computeUnitLimit: 0,
    },
    callback: {
      accounts: [
        { name: "borrower-position", signer: false, writable: true },
        { name: "liquidator-position", signer: false, writable: true },
        pool(true),
        { name: "liquidation-record", signer: false, writable: true },
        intent,
        market(true),
      ],
      // two Position rewrites + Pool + two fixed-size Receipt ciphertexts.
      dataLen: DISC + 2 * ENC_POSITION + ENC_POOL + 2 * RECEIPT,
      extraIxs: computeBudgetIxs(CU.liquidation),
      computeUnitLimit: CU.liquidation,
    },
    notes:
      "Built from the real two-owner liquidation receipt flow (Task 2.3). Receipt ciphertexts are included per design.md. This is the tightest envelope.",
  },
  {
    key: "absorb",
    title: "Absorb (+ Recovery Claim pre-create)",
    provenance: "modeled",
    queue: {
      accounts: [
        market(true),
        assetState,
        pool(true),
        { name: "borrower-position", signer: false, writable: true },
        { name: "recovery-position", signer: false, writable: true },
        { name: "liquidation-record", signer: false, writable: true },
        { name: "recovery-claim", signer: false, writable: true },
        intent,
        pythSol,
        pythUsdc,
      ],
      dataLen: DISC + COMP_OFFSET,
      extraIxs: computeBudgetIxs(CU.light),
      computeUnitLimit: 0,
    },
    callback: {
      accounts: [
        { name: "borrower-position", signer: false, writable: true },
        { name: "recovery-position", signer: false, writable: true },
        pool(true),
        { name: "liquidation-record", signer: false, writable: true },
        { name: "recovery-claim", signer: false, writable: true },
        intent,
        market(true),
      ],
      dataLen: DISC + ENC_POSITION + ENC_POOL + ENC_CLAIM,
      extraIxs: computeBudgetIxs(CU.absorb),
      computeUnitLimit: CU.absorb,
    },
    notes:
      "Modeled: Recovery Claim is pre-created in the queue tx (PDA init); callback populates it. Increments unsettled_recovery_claim_count.",
  },
  {
    key: "recovery-settlement",
    title: "Recovery Claim settlement (+ token CPIs)",
    provenance: "modeled",
    queue: {
      accounts: [
        market(false),
        assetState,
        pool(true),
        { name: "recovery-position", signer: false, writable: true },
        { name: "recovery-claim", signer: false, writable: true },
        { name: "pending-settlement", signer: false, writable: true },
        intent,
        { name: "vault-usdc-account", signer: false, writable: true },
        { name: "buyer-usdc-account", signer: false, writable: true },
        tokenProgram,
        pythSol,
        pythUsdc,
      ],
      dataLen: DISC + COMP_OFFSET + U64 + U64,
      extraIxs: [
        tokenTransfer(
          "spl transfer buyer->vault (escrow)",
          "buyer-usdc-account",
          "vault-usdc-account",
          { name: "queue-payer", signer: true, writable: true },
        ),
        ...computeBudgetIxs(CU.light),
      ],
      computeUnitLimit: 0,
    },
    callback: {
      accounts: [
        { name: "recovery-position", signer: false, writable: true },
        { name: "recovery-claim", signer: false, writable: true },
        pool(true),
        { name: "pending-settlement", signer: false, writable: true },
        intent,
        market(true),
        { name: "vault-wsol-account", signer: false, writable: true },
        { name: "buyer-wsol-account", signer: false, writable: true },
        vaultAuthority,
        tokenProgram,
      ],
      dataLen: DISC + ENC_CLAIM + ENC_POOL + U64,
      extraIxs: [
        tokenTransfer(
          "spl transfer vault->buyer (wSOL)",
          "vault-wsol-account",
          "buyer-wsol-account",
          vaultAuthority,
        ),
        ...computeBudgetIxs(CU.cpi),
      ],
      computeUnitLimit: CU.cpi,
    },
    notes:
      "Modeled: public USDC escrow in the queue tx, public wSOL out in the callback; allocation across supplier loss, reserves, treasury.",
  },
  {
    key: "disclosure",
    title: "Selective disclosure",
    provenance: "modeled",
    queue: {
      accounts: [
        market(false),
        position(false),
        { name: "disclosure", signer: false, writable: true },
        intent,
      ],
      dataLen: DISC + COMP_OFFSET + PUBKEY + U64, // auditor key + field mask
      extraIxs: computeBudgetIxs(CU.light),
      computeUnitLimit: 0,
    },
    callback: {
      accounts: [{ name: "disclosure", signer: false, writable: true }, intent],
      dataLen: DISC + ENC_POSITION, // re-encrypted selected fields to auditor key
      extraIxs: computeBudgetIxs(CU.position),
      computeUnitLimit: CU.position,
    },
    notes:
      "Modeled: Disclosure account pre-created with full space; callback re-encrypts selected fields, no resize.",
  },
  {
    key: "key-rotation",
    title: "Viewing Key rotation",
    provenance: "real",
    // Task 5.5 `rotate_position_key` account list, in instruction order after
    // the payer and the standard Arcium queue accounts. No Market account:
    // rotation moves no value and is not pausable.
    queue: {
      accounts: [
        { name: "arc-sign-pda", signer: false, writable: true },
        position(true),
        { name: "operation-counter", signer: false, writable: true },
        intent,
      ],
      // computation offset, Intent sequence, old and new epoch, old and new
      // Viewing public key
      dataLen: DISC + COMP_OFFSET + U64 + U64 + U64 + PUBKEY + PUBKEY,
      extraIxs: computeBudgetIxs(CU.light),
      computeUnitLimit: 0,
    },
    // `rotate_position_key` callback custom accounts, in struct order:
    // Position and Operation Intent. Output: the re-sealed Position state.
    callback: {
      accounts: [position(true), intent],
      dataLen: DISC + ENC_POSITION,
      extraIxs: computeBudgetIxs(CU.position),
      computeUnitLimit: CU.position,
    },
    notes:
      "Real Task 5.5 rotate_position_key: the queue commits old/new epochs and keys; the callback atomically stores the re-sealed payload, new pubkey, and epoch + 1.",
  },
];
