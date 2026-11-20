#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
cat > check-signpda-addr.mts << 'ENDSCRIPT'
import { PublicKey } from "@solana/web3.js";
const seeds = [Buffer.from("ArciumSignerAccount")];
const walnutProgramId = new PublicKey("5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea");
const arciumProgramId = new PublicKey("Arcj82pX7HxYKLR92qvgZUAd7vGS1k4hQvAFcPATFdEQ");

const [walnutPda, walnutBump] = PublicKey.findProgramAddressSync(seeds, walnutProgramId);
const [arciumPda, arciumBump] = PublicKey.findProgramAddressSync(seeds, arciumProgramId);

console.log("sign_pda_account under Walnut:", walnutPda.toBase58(), "bump:", walnutBump);
console.log("sign_pda_account under Arcium:", arciumPda.toBase58(), "bump:", arciumBump);
ENDSCRIPT
tsx check-signpda-addr.mts 2>&1
rm -f check-signpda-addr.mts
