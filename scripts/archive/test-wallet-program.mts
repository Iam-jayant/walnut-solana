import { Connection } from '@solana/web3.js';
import * as anchor from '@anchor-lang/core';
import { createRequire } from 'module';
const require = createRequire(import.meta.url);
const idl = require('/mnt/d/solana/walnut-solana/target/idl/walnut.json');

async function main() {
  const conn = new Connection('https://api.devnet.solana.com');
  
  console.log('IDL accounts:', idl.accounts?.length);
  console.log('IDL types:', idl.types?.length);
  
  console.log('Creating Program with wallet IDL...');
  try {
    const program = new anchor.Program(idl, conn);
    console.log('Program created OK!');
    
    // Check coder
    console.log('coder.accounts:', typeof program.coder.accounts);
    console.log('coder.accounts.size:', typeof program.coder.accounts?.size);
    
    // Try size on each account
    if (idl.accounts && program.coder.accounts) {
      for (const acc of idl.accounts) {
        try {
          const s = program.coder.accounts.size(acc.name);
          console.log(`  size(${acc.name}):`, s);
        } catch(e) {
          console.log(`  size(${acc.name}) ERROR:`, e.message);
        }
      }
    }
  } catch(e) {
    console.log('Program creation FAILED:', e.message);
    console.log('Stack:', e.stack.split('\n').slice(0, 8).join('\n'));
  }
}

main().catch(console.error);
