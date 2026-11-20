import { Connection, Keypair, PublicKey } from '@solana/web3.js';
import * as anchor from '@anchor-lang/core';
import { readFileSync } from 'fs';
import { homedir } from 'os';
import { createRequire } from 'module';
const require = createRequire(import.meta.url);
const idl = require('/mnt/d/solana/walnut-solana/target/idl/walnut.json');

async function main() {
  const programId = new PublicKey('5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea');
  const conn = new Connection('https://devnet.helius-rpc.com/?api-key=24db9681-90e5-4d92-bd8c-829d2db5f29e', 'confirmed');
  const kp = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(readFileSync(homedir() + '/.config/solana/walnut-deployer.json', 'utf-8')))
  );
  const wallet = new anchor.Wallet(kp);
  const provider = new anchor.AnchorProvider(conn, wallet, { commitment: 'confirmed' });

  console.log('Creating Program...');
  const program = new anchor.Program(idl, provider, programId);
  console.log('Program created successfully!');
  console.log('Program ID:', program.programId.toBase58());
}

main().catch(console.error);
