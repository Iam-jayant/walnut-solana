import { Connection } from '@solana/web3.js';
import * as anchor from '@anchor-lang/core';

async function main() {
  const conn = new Connection('https://api.devnet.solana.com');
  
  // Test 1: IDL with accounts but using @anchor-lang/core Program
  const idl = {
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
    address: "11111111111111111111111111111111",
  };

  console.log('Test: Program with accounts...');
  try {
    const program = new anchor.Program(idl, conn);
    console.log('Program created OK');
    console.log('coder.accounts:', typeof program.coder.accounts);
    console.log('coder.accounts.size:', typeof program.coder.accounts.size);
    if (program.coder.accounts && program.coder.accounts.size) {
      try {
        const s = program.coder.accounts.size("TestAccount");
        console.log('size("TestAccount"):', s);
      } catch(e) {
        console.log('size() error:', e.message);
      }
    }
  } catch(e) {
    console.log('Program creation failed:', e.message);
  }
}

main().catch(console.error);
