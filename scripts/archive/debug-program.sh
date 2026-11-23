#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana/packages/sdk
export ANCHOR_PROVIDER_URL="https://devnet.helius-rpc.com/?api-key=24db9681-90e5-4d92-bd8c-829d2db5f29e"
export ANCHOR_WALLET="$HOME/.config/solana/walnut-deployer.json"
export WALNUT_BENCH_CLUSTER_OFFSET=456
export WALNUT_PROGRAM_ID="5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea"
cat > /tmp/debug-prog.mts << 'ENDSCRIPT'
import { createContext, resolveOptions } from "./bench/config.ts";
const ctx = await createContext(resolveOptions([]));
console.log("idl.address:", ctx.program.idl.address);
console.log("program.programId:", ctx.program.programId.toBase58());
console.log("Match:", ctx.program.idl.address === ctx.program.programId.toBase58());
console.log("Expected: 5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea");
await ctx.connection.close();
process.exit(0);
ENDSCRIPT
tsx /tmp/debug-prog.mts 2>&1
