import { createContext, resolveOptions } from "./bench/config.ts";
import { buildSetLendingRateSourceInstruction, buildUpdatePoolBandInstruction, derivePoolStatePda, deriveLendingPoolPda, derivePoolCounterPda, deriveIntentPda, RATE_SOURCE_BAND } from "./src/index.ts";
import { Connection, Keypair, PublicKey, Transaction } from "@solana/web3.js";
import { AnchorProvider, Wallet } from "@anchor-lang/core";
import { readFileSync } from "fs";
import { homedir } from "os";

async function main() {
  const options = resolveOptions([]);
  const ctx = await createContext(options);
  console.log("program.programId:", ctx.program.programId.toBase58());
  console.log("idl.address:", ctx.program.idl.address);
  console.log("Expected: 5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea");
  console.log("Match:", ctx.program.idl.address === ctx.program.programId.toBase58());

  const program = ctx.program;
  const provider = ctx.provider;
  const connection = ctx.connection;
  const payer = ctx.payer;
  const programId = ctx.programId;

  const market = PublicKey.findProgramAddressSync([Buffer.from("market")], programId)[0];
  const accounts = program.account as any;
  const marketAcc = await accounts["market"].fetchNullable(market);
  if (!marketAcc) throw new Error("Market not found");
  console.log("Market found, assetState:", marketAcc.assetState);

  const assetState = marketAcc.assetState;
  const poolState = derivePoolStatePda(programId, market);
  const lendingPool = deriveLendingPoolPda(programId, market);

  const lpAcc = await accounts["lendingPool"].fetchNullable(lendingPool);
  console.log("LendingPool rateSource:", lpAcc?.rateSource, "(1 = band)");

  if (lpAcc?.rateSource !== RATE_SOURCE_BAND) {
    console.log("Setting lending rate source to band...");
    const ix = await buildSetLendingRateSourceInstruction({
      program,
      admin: payer.publicKey,
      market,
      assetState,
      rateSource: RATE_SOURCE_BAND,
    });
    console.log("Instruction built");
    const tx = new Transaction().add(ix);
    const sig = await provider.sendAndConfirm(tx, [payer], { commitment: "confirmed" });
    console.log("Rate source set, sig:", sig);
  } else {
    console.log("Rate source already band");
  }

  const counter = await accounts["operationCounter"].fetchNullable(derivePoolCounterPda(programId, poolState));
  const sequence = BigInt(String((counter as any)?.nextSequence ?? 0));
  const offset = BigInt(Math.floor(Math.random() * 2 ** 48));
  console.log("Building update_pool_band with sequence:", sequence, "offset:", offset);
  const ix2 = await buildUpdatePoolBandInstruction({
    program,
    payer: payer.publicKey,
    market,
    assetState,
    poolState,
    sequence,
    computationOffset: offset,
    clusterOffset: 456,
  });
  console.log("Instruction built successfully");
  const tx2 = new Transaction().add(ix2);
  const sig2 = await provider.sendAndConfirm(tx2, [payer], { commitment: "confirmed" });
  console.log("update_pool_band sig:", sig2);

  await connection.close();
  process.exit(0);
}
main().catch(e => {
  console.error("Error:", e.message);
  console.error("Stack:", e.stack?.split("\n").slice(0, 10).join("\n"));
  process.exit(1);
});
