import { getArciumProgram, initMxePart1, initMxePart2 } from '@arcium-hq/client';
import { PublicKey, Connection, Keypair } from '@solana/web3.js';
import { AnchorProvider, Wallet } from '@coral-xyz/anchor';
import { readFileSync } from 'fs';
import { homedir } from 'os';

async function main() {
  console.log('Initializing MXE for program 5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea...');
  console.log('Cluster offset: 456');

  // Create minimal provider since createContext will fail without MXE
  const conn = new Connection('https://api.devnet.solana.com');
  const kp = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(readFileSync(homedir() + '/.config/solana/walnut-deployer.json', 'utf-8')))
  );
  const wallet = new Wallet(kp);
  const provider = new AnchorProvider(conn, wallet, { commitment: 'confirmed' });

  const arciumProgram = getArciumProgram(provider);
  const programId = new PublicKey('5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea');
  const clusterOffset = 456;

  try {
    console.log('Step 1: initMxePart1...');
    const sig1 = await initMxePart1(provider, arciumProgram, programId, clusterOffset);
    console.log('Part 1 done:', sig1);

    console.log('Step 2: initMxePart2...');
    const sig2 = await initMxePart2(provider, arciumProgram, programId, clusterOffset);
    console.log('Part 2 done:', sig2);

    console.log('MXE initialized successfully!');
  } catch (e) {
    if (e.message?.includes('already in use') || e.logs?.join(' ').includes('already in use')) {
      console.log('MXE already exists - this is fine');
      process.exit(0);
    } else {
      console.error('Error:', e.message || e);
      throw e;
    }
  }
}

main().catch(console.error);
