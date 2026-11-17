/**
 * Task 6.5: localnet Pyth `PriceUpdateV2` fixture (Requirement 6.1).
 *
 * The Pyth receiver program is not deployed on localnet, but an account can be
 * pre-loaded into `solana-test-validator` with ANY owner, so the fixture
 * writes accounts that carry the genuine `PriceUpdateV2` layout and are owned
 * by the real receiver program id. The program's `oracle` module checks
 * exactly that: receiver owner, feed id, age, confidence, exponent (see
 * `programs/walnut/src/oracle/mod.rs`). No program change or test-only
 * feature is needed; the receiver id stays the on-chain constant.
 *
 * This test (CI only):
 *   1. builds the SOL/USD and USDC/USD fixture accounts,
 *   2. checks the encoder against a golden byte vector shared with the SDK test
 *      (`packages/sdk/test/oracle.test.ts`) so the layout cannot drift,
 *   3. writes `target/oracle-fixture/<name>.json` in the
 *      `solana-test-validator --account` format, rewriting a file only when its
 *      bytes change (idempotent), and
 *   4. re-reads each file and decodes it back.
 *
 * Wiring a localnet run (once Task 9 adds the price-sensitive instruction):
 *   ORACLE_FIXTURE_PUBLISH_TIME=$(date +%s) pnpm exec ts-mocha -p ./tsconfig.json \
 *     tests/oracle-fixture.ts
 *   solana-test-validator --account <SOL addr> target/oracle-fixture/sol-usd.json \
 *     --account <USDC addr> target/oracle-fixture/usdc-usd.json ...
 * The addresses are printed by this test. Publish time defaults to "now" so the
 * 60 s age bound holds when the validator starts straight after generation.
 */
import { Keypair, PublicKey } from "@solana/web3.js";
import { createHash } from "crypto";
import * as fs from "fs";
import * as path from "path";
import { expect } from "chai";

/** The Pyth Solana receiver program id (owner of every PriceUpdateV2). */
const PYTH_RECEIVER_PROGRAM_ID = new PublicKey(
  "rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ",
);
const DISCRIMINATOR = Buffer.from([34, 241, 35, 99, 157, 126, 244, 205]);
const PRICE_UPDATE_V2_LEN = 134;
/** Rent-exempt minimum for 134 bytes of data (6.96e-3 SOL class; generous). */
const FIXTURE_LAMPORTS = 1_461_600;

const SOL_USD_FEED = Buffer.from(
  "ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d",
  "hex",
);
const USDC_USD_FEED = Buffer.from(
  "eaa020c61cc479712813461ce153894a96a6c00b21ed0cfc2798d1f9a9e9c94a",
  "hex",
);

/** Golden bytes, identical to `GOLDEN_PRICE_UPDATE_HEX` in the SDK test. */
const GOLDEN_HEX =
  "22f123639d7ef4cd0707070707070707070707070707070707070707070707070707070707070707" +
  "01" +
  "1111111111111111111111111111111111111111111111111111111111111111" +
  "00d6117e0300000020a1070000000000f8ffffffe803000000000000e703000000000000" +
  "00d6117e0300000020a10700000000002a0000000000000000";

interface FixtureFields {
  writeAuthority: PublicKey;
  feedId: Buffer;
  price: bigint;
  conf: bigint;
  exponent: number;
  publishTime: bigint;
  emaPrice: bigint;
  emaConf: bigint;
  postedSlot: bigint;
}

/** Encodes a fully verified (`Full`) `PriceUpdateV2` account body. */
export function encodePriceUpdate(f: FixtureFields): Buffer {
  const out = Buffer.alloc(PRICE_UPDATE_V2_LEN);
  DISCRIMINATOR.copy(out, 0);
  f.writeAuthority.toBuffer().copy(out, 8);
  out[40] = 1; // VerificationLevel::Full
  f.feedId.copy(out, 41);
  out.writeBigInt64LE(f.price, 73);
  out.writeBigUInt64LE(f.conf, 81);
  out.writeInt32LE(f.exponent, 89);
  out.writeBigInt64LE(f.publishTime, 93);
  out.writeBigInt64LE(f.publishTime - 1n, 101); // prev_publish_time
  out.writeBigInt64LE(f.emaPrice, 109);
  out.writeBigUInt64LE(f.emaConf, 117);
  out.writeBigUInt64LE(f.postedSlot, 125);
  return out;
}

/** Decodes the fields the program-side checks read. */
export function decodePriceUpdate(data: Buffer): {
  feedId: string;
  price: bigint;
  conf: bigint;
  exponent: number;
  publishTime: bigint;
} {
  expect(data.length).to.equal(PRICE_UPDATE_V2_LEN);
  expect(data.subarray(0, 8).equals(DISCRIMINATOR)).to.equal(true);
  return {
    feedId: data.subarray(41, 73).toString("hex"),
    price: data.readBigInt64LE(73),
    conf: data.readBigUInt64LE(81),
    exponent: data.readInt32LE(89),
    publishTime: data.readBigInt64LE(93),
  };
}

/** A deterministic fixture address derived from a label. */
function fixtureAddress(label: string): PublicKey {
  const seed = createHash("sha256").update(`walnut-oracle-fixture:${label}`);
  return Keypair.fromSeed(seed.digest()).publicKey;
}

interface Fixture {
  name: string;
  address: PublicKey;
  data: Buffer;
}

function buildFixtures(publishTime: bigint): Fixture[] {
  const writeAuthority = fixtureAddress("write-authority");
  const common = { writeAuthority, postedSlot: 1n, publishTime };
  return [
    {
      name: "sol-usd",
      address: fixtureAddress("sol-usd"),
      data: encodePriceUpdate({
        ...common,
        feedId: SOL_USD_FEED,
        price: 150_00000000n,
        conf: 5_00000n,
        exponent: -8,
        emaPrice: 150_00000000n,
        emaConf: 5_00000n,
      }),
    },
    {
      name: "usdc-usd",
      address: fixtureAddress("usdc-usd"),
      data: encodePriceUpdate({
        ...common,
        feedId: USDC_USD_FEED,
        price: 1_00000000n,
        conf: 10_000n,
        exponent: -8,
        emaPrice: 1_00000000n,
        emaConf: 10_000n,
      }),
    },
  ];
}

/** The `solana-test-validator --account` JSON for a fixture. */
function validatorJson(fixture: Fixture): string {
  return (
    JSON.stringify(
      {
        pubkey: fixture.address.toBase58(),
        account: {
          lamports: FIXTURE_LAMPORTS,
          data: [fixture.data.toString("base64"), "base64"],
          owner: PYTH_RECEIVER_PROGRAM_ID.toBase58(),
          executable: false,
          rentEpoch: 0,
          space: fixture.data.length,
        },
      },
      null,
      2,
    ) + "\n"
  );
}

/** Writes `file` only when its content changes. Returns whether it wrote. */
function writeIfChanged(file: string, content: string): boolean {
  if (fs.existsSync(file) && fs.readFileSync(file, "utf8") === content) {
    return false;
  }
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, content);
  return true;
}

describe("oracle fixture (Task 6.5)", () => {
  const publishTime = BigInt(
    process.env.ORACLE_FIXTURE_PUBLISH_TIME ?? Math.floor(Date.now() / 1000),
  );
  const outDir = path.resolve(__dirname, "..", "target", "oracle-fixture");

  it("encodes the golden PriceUpdateV2 bytes", () => {
    const golden = encodePriceUpdate({
      writeAuthority: new PublicKey(Buffer.alloc(32, 7)),
      feedId: Buffer.alloc(32, 0x11),
      price: 15000000000n,
      conf: 500000n,
      exponent: -8,
      publishTime: 1000n,
      emaPrice: 15000000000n,
      emaConf: 500000n,
      postedSlot: 42n,
    });
    expect(golden.toString("hex")).to.equal(GOLDEN_HEX);
  });

  it("writes receiver-owned validator account files idempotently", () => {
    const fixtures = buildFixtures(publishTime);
    for (const fixture of fixtures) {
      const file = path.join(outDir, `${fixture.name}.json`);
      writeIfChanged(file, validatorJson(fixture));
      // A second run with identical inputs must not rewrite the file.
      expect(writeIfChanged(file, validatorJson(fixture))).to.equal(false);

      const parsed = JSON.parse(fs.readFileSync(file, "utf8"));
      expect(parsed.account.owner).to.equal(
        PYTH_RECEIVER_PROGRAM_ID.toBase58(),
      );
      expect(parsed.pubkey).to.equal(fixture.address.toBase58());
      expect(parsed.account.space).to.equal(PRICE_UPDATE_V2_LEN);
      const data = Buffer.from(parsed.account.data[0], "base64");
      const decoded = decodePriceUpdate(data);
      expect(decoded.publishTime).to.equal(publishTime);
      expect(decoded.exponent).to.equal(-8);
      console.log(`      ${fixture.name}: ${fixture.address.toBase58()}`);
    }
  });

  it("uses the two configured feed ids and distinct addresses", () => {
    const [sol, usdc] = buildFixtures(publishTime);
    expect(decodePriceUpdate(sol.data).feedId).to.equal(
      SOL_USD_FEED.toString("hex"),
    );
    expect(decodePriceUpdate(usdc.data).feedId).to.equal(
      USDC_USD_FEED.toString("hex"),
    );
    expect(sol.address.equals(usdc.address)).to.equal(false);
  });
});
