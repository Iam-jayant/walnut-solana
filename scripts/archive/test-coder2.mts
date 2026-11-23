import { Connection } from '@solana/web3.js';
import * as anchor from '@anchor-lang/core';

async function main() {
  const conn = new Connection('https://api.devnet.solana.com');
  
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

  const program = new anchor.Program(idl, conn);
  
  // Try camelCased name
  try {
    const s = program.coder.accounts.size("testAccount");
    console.log('size("testAccount"):', s);
  } catch(e) {
    console.log('size("testAccount") error:', e.message);
  }
  
  // Check what account names look like after camelCase
  console.log('idl.accounts[0].name:', (idl as any).accounts[0].name);
  const camelIdl = anchor.convertIdlToCamelCase ? anchor.convertIdlToCamelCase(idl) : idl;
  console.log('camelIdl.accounts[0].name:', camelIdl.accounts[0].name);
}

main().catch(console.error);
