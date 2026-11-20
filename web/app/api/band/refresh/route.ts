import { NextRequest, NextResponse } from "next/server";
import {
  Connection,
  Keypair,
  PublicKey,
  ComputeBudgetProgram,
  Transaction,
} from "@solana/web3.js";
import { AnchorProvider, Program } from "@anchor-lang/core";
import {
  buildUpdatePoolBandInstruction,
  derivePoolStatePda,
  derivePoolCounterPda,
  deriveIntentPda,
} from "@walnut/sdk";
import bs58 from "bs58";
import walnutIdl from "@/lib/idl/walnut.json";
import {
  fetchAccount,
  type MarketAccount,
  type AssetStateAccount,
  type OperationCounterAccount,
  type IntentAccount,
} from "@/lib/chain";
import {
  PROGRAM_ID,
  MARKET_ADDRESS,
  CLUSTER_OFFSET,
  QUEUE_COMPUTE_UNITS,
  rpcUrl,
} from "@/lib/config";

// Fallback dedicated devnet keeper keypair funded on Solana Devnet (pubkey: 9FV3uQ7bEQgyMMt4rs6Hj23Kd6JCzE3hsQwiU2CFLPsK)
const DEVNET_KEEPER_BYTES = [
  100, 186, 244, 1, 137, 116, 40, 52, 70, 182, 255, 167, 236, 109, 243, 239,
  203, 112, 107, 212, 227, 206, 12, 113, 124, 249, 193, 17, 74, 207, 115, 36,
  122, 147, 87, 209, 132, 4, 147, 23, 92, 203, 94, 218, 170, 155, 51, 88, 116,
  133, 87, 241, 7, 82, 60, 245, 230, 231, 232, 13, 44, 188, 52, 246,
];

function getKeeperKeypair(): Keypair {
  const envKey = process.env.BAND_KEEPER_KEY || process.env.DEPLOYER_KEY;
  if (envKey) {
    try {
      if (envKey.startsWith("[")) {
        return Keypair.fromSecretKey(Uint8Array.from(JSON.parse(envKey)));
      }
      return Keypair.fromSecretKey(bs58.decode(envKey));
    } catch (e) {
      console.warn("Failed to parse BAND_KEEPER_KEY env, using default devnet keeper:", e);
    }
  }
  return Keypair.fromSecretKey(Uint8Array.from(DEVNET_KEEPER_BYTES));
}

async function handleRefresh(req: NextRequest) {
  try {
    const force =
      req.nextUrl.searchParams.get("force") === "true" ||
      req.nextUrl.searchParams.get("force") === "1";

    const connection = new Connection(rpcUrl(), "confirmed");
    const keeper = getKeeperKeypair();
    const dummyWallet = {
      publicKey: keeper.publicKey,
      signTransaction: async (tx: Transaction) => {
        tx.sign(keeper);
        return tx;
      },
      signAllTransactions: async (txs: Transaction[]) => {
        txs.forEach((tx) => tx.sign(keeper));
        return txs;
      },
    };
    const provider = new AnchorProvider(connection, dummyWallet as any, {
      commitment: "confirmed",
    });

    const idlCopy = { ...walnutIdl, address: PROGRAM_ID.toBase58() };
    const program = new Program(idlCopy as any, provider);

    const marketAcc = await fetchAccount<MarketAccount>(
      program,
      "market",
      MARKET_ADDRESS,
    );
    if (!marketAcc) throw new Error("Market account not found");
    const assetStateKey = marketAcc.assetState;
    const poolStateKey = derivePoolStatePda(PROGRAM_ID, MARKET_ADDRESS);

    const assetAcc = await fetchAccount<AssetStateAccount>(
      program,
      "assetState",
      assetStateKey,
    );
    if (!assetAcc) throw new Error("AssetState account not found");

    const currentSlot = await connection.getSlot("confirmed");
    const observedSlot = Number(
      assetAcc.utilizationObservationSlot?.toString() ?? 0,
    );
    const maxAge = Number(assetAcc.rates?.bandMaxAgeSlots?.toString() ?? 9000);
    const age = currentSlot - observedSlot;

    // Refresh if age exceeds 2,500 slots (~17 minutes) or if force requested
    const shouldRefresh = force || age > 2500;

    if (!shouldRefresh) {
      return NextResponse.json({
        success: true,
        refreshed: false,
        message: "Utilization band is fresh",
        currentSlot,
        observedSlot,
        age,
        maxAge,
      });
    }

    const poolCounter = derivePoolCounterPda(PROGRAM_ID, poolStateKey);
    const counter = await fetchAccount<OperationCounterAccount>(
      program,
      "operationCounter",
      poolCounter,
    );
    const sequence = BigInt(counter?.nextSequence?.toString() ?? 0);
    const offset = BigInt(Math.floor(Math.random() * 2 ** 48));

    const ix = await buildUpdatePoolBandInstruction({
      program,
      payer: keeper.publicKey,
      market: MARKET_ADDRESS,
      assetState: assetStateKey,
      poolState: poolStateKey,
      sequence,
      computationOffset: offset,
      clusterOffset: CLUSTER_OFFSET,
    });

    const { blockhash, lastValidBlockHeight } =
      await connection.getLatestBlockhash("confirmed");
    const tx = new Transaction({
      feePayer: keeper.publicKey,
      blockhash,
      lastValidBlockHeight,
    });
    tx.add(
      ComputeBudgetProgram.setComputeUnitLimit({ units: QUEUE_COMPUTE_UNITS }),
      ix,
    );
    tx.sign(keeper);

    const signature = await connection.sendRawTransaction(tx.serialize(), {
      preflightCommitment: "confirmed",
      maxRetries: 5,
    });

    // Await Arcium MPC callback up to 12 seconds
    const intentKey = deriveIntentPda(PROGRAM_ID, poolStateKey, sequence);
    let finalStatus = "queued";
    for (let i = 0; i < 6; i++) {
      await new Promise((r) => setTimeout(r, 2000));
      const intentAcc = await fetchAccount<IntentAccount>(
        program,
        "operationIntent",
        intentKey,
      );
      if (intentAcc?.status) {
        finalStatus = Object.keys(intentAcc.status)[0] ?? "unknown";
        if (finalStatus === "applied") break;
      }
    }

    return NextResponse.json({
      success: true,
      refreshed: true,
      signature,
      status: finalStatus,
      ageBefore: age,
      currentSlot,
      observedSlotBefore: observedSlot,
      keeper: keeper.publicKey.toBase58(),
    });
  } catch (err: any) {
    console.error("[Band Refresh API] Error:", err);
    return NextResponse.json(
      {
        success: false,
        error: err.message || String(err),
      },
      { status: 500 },
    );
  }
}

export async function GET(req: NextRequest) {
  return handleRefresh(req);
}

export async function POST(req: NextRequest) {
  return handleRefresh(req);
}
