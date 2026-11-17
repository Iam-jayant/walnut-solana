// Task 6.5: real Pyth price update, decoded and validated.
//
// Reads the REAL, current SOL/USD and USDC/USD `PriceUpdateV2` accounts that
// Pyth's push oracle keeps on devnet (shard 0 sponsored feeds, owned by the
// Pyth receiver program), parses them with the SDK parser, and runs the
// TypeScript mirror of the Rust `validate_price` with the launch parameters.
// It prints the conservative normalized prices: SOL collateral low
// (price - confidence, floor) and USDC debt high (price + confidence, ceil).
// This needs only a keyless public devnet RPC.
//
// Optional: if HERMES_API_KEY is set, it also fetches a signed update from
// Hermes (which now requires a Pyth API key), decodes the accumulator payload,
// and validates that too. Posting an update to devnet needs the Pyth receiver
// SDK (@pythnetwork/pyth-solana-receiver, a new dependency that is not approved
// yet); see docs/ORACLE_CHECKPOINT.md.
//
// Usage:
//   pnpm --filter @walnut/sdk demo:oracle
// ANCHOR_PROVIDER_URL (optional) overrides the RPC (default: the public devnet
// RPC from config/networks/devnet.json). It is never printed; api-keys are
// redacted from all output.
import { Connection, PublicKey } from "@solana/web3.js";
import {
  bytesToHex,
  decodeAccumulatorUpdate,
  encodePriceUpdateV2,
  hexToBytes,
  LAUNCH_ORACLE_PARAMS,
  parsePriceUpdateV2,
  PYTH_FEED_IDS,
  PYTH_RECEIVER_PROGRAM_ID,
  toPriceView,
  validatePrice,
  type PriceFeedMessage,
  type PriceUse,
} from "../src/oracle.ts";

const HERMES = process.env.HERMES_URL ?? "https://hermes.pyth.network";
const DEFAULT_RPC = "https://api.devnet.solana.com";
/** Pyth push-oracle program: owns the PDA that points a feed at its update. */
const PUSH_ORACLE_PROGRAM_ID = new PublicKey(
  "pythWSnswVUd12oZpeFP8e9CVaEqJg25g1Vtc2biRsT",
);
const CASES: ReadonlyArray<{
  name: keyof typeof PYTH_FEED_IDS;
  use: PriceUse;
}> = [
  { name: "SOL/USD", use: "collateral" },
  { name: "USDC/USD", use: "debt" },
];

function redact(text: string): string {
  return text
    .replace(/api-key=[^&\s"')]+/gi, "api-key=REDACTED")
    .replace(/https?:\/\/[^\s"')]*helius[^\s"')]*/gi, "<rpc redacted>");
}

function log(message: string): void {
  console.log(redact(message));
}

/** Formats a USD_SCALE (1e8) value as a decimal string. */
function usd(value: bigint): string {
  const whole = value / 100_000_000n;
  const frac = (value % 100_000_000n).toString().padStart(8, "0");
  return `${whole}.${frac}`;
}

/** The shard-0 sponsored-feed account address for a feed id. */
function sponsoredFeedAddress(feedIdHex: string): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from([0, 0]), Buffer.from(hexToBytes(feedIdHex))],
    PUSH_ORACLE_PROGRAM_ID,
  )[0];
}

interface Observed {
  readonly source: string;
  readonly owner: PublicKey;
  readonly message: PriceFeedMessage;
}

async function readOnChain(
  connection: Connection,
  feedIdHex: string,
): Promise<Observed> {
  const address = sponsoredFeedAddress(feedIdHex);
  const info = await connection.getAccountInfo(address, "confirmed");
  if (!info) throw new Error(`no account at ${address.toBase58()}`);
  const parsed = parsePriceUpdateV2(info.data);
  return {
    source: `devnet account ${address.toBase58()} (verification ${parsed.verificationLevel})`,
    owner: info.owner,
    message: parsed,
  };
}

interface HermesResponse {
  binary: { encoding: string; data: string[] };
}

async function readHermes(apiKey: string): Promise<PriceFeedMessage[]> {
  const params = Object.values(PYTH_FEED_IDS)
    .map((id) => `ids[]=${id}`)
    .join("&");
  const response = await fetch(
    `${HERMES}/v2/updates/price/latest?${params}&encoding=hex`,
    { headers: { Authorization: `Bearer ${apiKey}` } },
  );
  if (!response.ok) throw new Error(`Hermes responded ${response.status}`);
  const body = (await response.json()) as HermesResponse;
  return body.binary.data.flatMap((hex) =>
    decodeAccumulatorUpdate(hexToBytes(hex)),
  );
}

function report(
  name: string,
  use: PriceUse,
  feedId: Uint8Array,
  observed: Observed,
  now: bigint,
): boolean {
  const { message } = observed;
  const account = encodePriceUpdateV2({
    writeAuthority: PublicKey.default,
    verificationLevel: "full",
    ...message,
    prevPublishTime: message.prevPublishTime,
    postedSlot: 0n,
  });
  const view = toPriceView(parsePriceUpdateV2(account), observed.owner);
  const strict = validatePrice(view, feedId, LAUNCH_ORACLE_PARAMS, now, use);
  // Age-neutral run: evaluates every other rule as of the publish time, so a
  // stale-but-real devnet print still proves owner, feed, confidence, exponent,
  // normalization, and deviation behave on real data.
  const neutral = validatePrice(
    view,
    feedId,
    LAUNCH_ORACLE_PARAMS,
    message.publishTime,
    use,
  );
  const price = Number(message.price) * 10 ** message.exponent;
  const conf = Number(message.conf) * 10 ** message.exponent;
  const confPct = (Number(message.conf) / Number(message.price)) * 100;
  log(`${name} [${observed.source}]`);
  log(
    `  owner=${observed.owner.toBase58()} price=${price} conf=${conf} (${confPct.toFixed(4)}%) exponent=${message.exponent} publish_time=${message.publishTime} age=${now - message.publishTime}s`,
  );
  log(
    strict.ok
      ? `  strict (age <= ${LAUNCH_ORACLE_PARAMS.maxAgeSeconds}s): PASS`
      : `  strict (age <= ${LAUNCH_ORACLE_PARAMS.maxAgeSeconds}s): rejected, ${strict.error}`,
  );
  if (neutral.ok) {
    const label =
      use === "collateral"
        ? "collateral low (price-conf, floor)"
        : "debt high (price+conf, ceil)";
    log(`  age-neutral: PASS, ${label} = ${usd(neutral.normalized)} USD`);
    return true;
  }
  log(`  age-neutral: FAILED, ${neutral.error}`);
  return false;
}
async function main(): Promise<void> {
  const connection = new Connection(
    process.env.ANCHOR_PROVIDER_URL ?? DEFAULT_RPC,
    "confirmed",
  );
  const slot = await connection.getSlot();
  const blockTime = await connection.getBlockTime(slot);
  const now =
    blockTime === null
      ? BigInt(Math.floor(Date.now() / 1000))
      : BigInt(blockTime);
  log(`Devnet slot ${slot}, cluster time ${now}`);

  let allPass = true;
  for (const { name, use } of CASES) {
    const id = PYTH_FEED_IDS[name];
    const observed = await readOnChain(connection, id);
    allPass = report(name, use, hexToBytes(id), observed, now) && allPass;
  }

  const apiKey = process.env.HERMES_API_KEY;
  if (apiKey) {
    const messages = await readHermes(apiKey);
    for (const { name, use } of CASES) {
      const id = PYTH_FEED_IDS[name];
      const message = messages.find((m) => bytesToHex(m.feedId) === id);
      if (!message) throw new Error(`${name} missing from the Hermes update`);
      const observed = {
        source: "Hermes signed update",
        owner: PYTH_RECEIVER_PROGRAM_ID,
        message,
      };
      allPass = report(name, use, hexToBytes(id), observed, now) && allPass;
    }
  } else {
    log("HERMES_API_KEY not set: skipping the signed Hermes update");
  }

  log(allPass ? "RESULT: PASS" : "RESULT: FAIL");
  process.exitCode = allPass ? 0 : 1;
}

main()
  .then(() => process.exit(process.exitCode ?? 0))
  .catch((error: unknown) => {
    const message = error instanceof Error ? error.message : String(error);
    console.error(redact(`Oracle demo failed: ${message}`));
    process.exit(1);
  });
