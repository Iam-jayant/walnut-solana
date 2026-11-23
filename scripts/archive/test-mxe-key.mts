import { getMXEPublicKey } from '@arcium-hq/client';
import { Connection, Keypair, PublicKey } from '@solana/web3.js';
import { AnchorProvider, Wallet } from '@anchor-lang/core';
import { readFileSync } from 'fs';
import { homedir } from 'os';
import { createRequire } from 'module';
const require = createRequire(import.meta.url);
const { BN } = require('@anchor-lang/core');

async function main() {
  const programId = new PublicKey('5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea');
  const rpcUrl = 'https://devnet.helius-rpc.com/?api-key=24db9681-90e5-4d92-bd8c-829d2db5f29e';
  
  const conn = new Connection(rpcUrl, 'confirmed');
  const kp = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(readFileSync(homedir() + '/.config/solana/walnut-deployer.json', 'utf-8')))
  );
  const wallet = new Wallet(kp);
  const provider = new AnchorProvider(conn, wallet, { commitment: 'confirmed' });

  try {
    const key = await getMXEPublicKey(provider, programId);
    console.log('MXE public key:', key ? Buffer.from(key).toString('hex').slice(0, 16) + '...' : 'null');
  } catch (e) {
    console.error('Error:', e.message);
  }
}

main().catch(console.error);
