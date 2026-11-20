import {
  getArciumProgram,
  getMXEAccAddress,
  getMXEPublicKey,
  initMxePart1,
  initMxePart2,
} from '@arcium-hq/client';
import { PublicKey, Connection, Keypair } from '@solana/web3.js';
import * as anchor from '@anchor-lang/core';
const { AnchorProvider, Wallet } = anchor;
import { readFileSync } from 'fs';
import { homedir } from 'os';

async function main() {
  const programId = new PublicKey('AWEGaPVJwaCkrzraWNxGQDtH2k6NdUYL8xJDBAkLakUn');
  const clusterOffset = 456;
  const rpcUrl = process.env.RPC_URL ?? process.env.ANCHOR_PROVIDER_URL ?? 'https://api.devnet.solana.com';

  console.log('Initializing MXE for program', programId.toBase58());
  console.log('Cluster offset:', clusterOffset);
  console.log('RPC:', rpcUrl);

  const conn = new Connection(rpcUrl);
  const kp = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(readFileSync(homedir() + '/.config/solana/walnut-deployer.json', 'utf-8')))
  );
  const wallet = new Wallet(kp);
  const provider = new AnchorProvider(conn, wallet, { commitment: 'confirmed' });

  // recoveryPeers: array of 100 node offsets (0 = unused). At least 4 non-zero if any.
  // Matches `arcium deploy --recovery-set-size 4`.
  const recoveryPeers = [1, 2, 3, 4];
  while (recoveryPeers.length < 100) recoveryPeers.push(0);

  // Computation offsets stored on the MXE account. These reference
  // keygen / key_recovery_init computations on the Arcium program.
  const keygenOffset = 0;
  const keyRecoveryInitOffset = 1;
  const lutOffset = 0;

  const mxeAddress = getMXEAccAddress(programId);
  console.log('Expected MXE address:', mxeAddress.toBase58());

  try {
    // Check if MXE already exists
    const existing = await conn.getAccountInfo(mxeAddress, 'confirmed');
    if (existing !== null) {
      console.log('MXE account already exists at', mxeAddress.toBase58());
      // Check if keygen has finalized
      try {
        const mxeKey = await getMXEPublicKey(provider, programId);
        if (mxeKey) {
          console.log('MXE public key fetched: keygen has finalized.');
        } else {
          console.log('MXE exists but keygen not yet finalized.');
        }
      } catch {
        console.log('MXE exists but keygen not yet finalized.');
      }
      process.exit(0);
    }

    console.log('Step 1: initMxePart1...');
    const sig1 = await initMxePart1(provider, programId, recoveryPeers);
    console.log('Part 1 done:', sig1);

    console.log('Step 2: initMxePart2...');
    const sig2 = await initMxePart2(
      provider,
      clusterOffset,
      programId,
      keygenOffset,
      keyRecoveryInitOffset,
      recoveryPeers,
      lutOffset
    );
    console.log('Part 2 done:', sig2);

    console.log('MXE initialized successfully at', mxeAddress.toBase58());

    // Wait briefly and check if keygen finalized
    console.log('Waiting for MXE keygen to finalize...');
    let mxeKey = null;
    for (let attempt = 1; attempt <= 10; attempt += 1) {
      try {
        mxeKey = await getMXEPublicKey(provider, programId);
        if (mxeKey) break;
      } catch {}
      await new Promise((r) => setTimeout(r, 2000));
    }
    if (mxeKey) {
      console.log('MXE public key fetched: keygen has finalized on this cluster.');
    } else {
      console.log('MXE keygen not yet finalized. This is normal; wait and re-check.');
    }
  } catch (e) {
    const msg = e.message || String(e);
    if (msg.includes('already in use') || msg.includes('AlreadyInUse')) {
      console.log('MXE already exists - this is fine');
      process.exit(0);
    } else {
      console.error('Error:', msg);
      throw e;
    }
  }
}

main().catch(console.error);
