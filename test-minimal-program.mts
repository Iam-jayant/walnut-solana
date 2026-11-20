import { Connection, Keypair, PublicKey } from '@solana/web3.js';
import * as anchor from '@anchor-lang/core';
import { readFileSync } from 'fs';
import { homedir } from 'os';

async function main() {
  const programId = new PublicKey('5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea');
  const conn = new Connection('https://devnet.helius-rpc.com/?api-key=24db9681-90e5-4d92-bd8c-829d2db5f29e', 'confirmed');
  const kp = Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(readFileSync(homedir() + '/.config/solana/walnut-deployer.json', 'utf-8')))
  );
  const wallet = new anchor.Wallet(kp);
  const provider = new anchor.AnchorProvider(conn, wallet, { commitment: 'confirmed' });

  // Minimal IDL
  const minimalIdl = {
    version: "0.1.0",
    name: "test",
    instructions: [],
    accounts: [
      {
        name: "TestAccount",
        discriminator: [1,2,3,4,5,6,7,8],
        size: 100,
      },
    ],
    types: [
      {
        name: "TestAccount",
        type: {
          kind: "struct",
          fields: [
            { name: "field1", type: "u64" },
          ],
        },
      },
    ],
    address: programId.toBase58(),
  };

  console.log('Creating Program with minimal IDL...');
  const program = new anchor.Program(minimalIdl, provider, programId);
  console.log('Program created successfully!');
  console.log('Program ID:', program.programId.toBase58());
}

main().catch(console.error);
