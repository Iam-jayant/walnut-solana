#!/usr/bin/env bash
set -euo pipefail
export NVM_DIR="$HOME/.nvm"
. "$NVM_DIR/nvm.sh"
nvm use 20
cd /mnt/d/solana/walnut-solana
node --input-type=commonjs -e '
const c = require("@arcium-hq/client");
const { BN } = require("@anchor-lang/core");
const { PublicKey } = require("@solana/web3.js");
const names = ["keygen", "key_recovery_init", "mxe_keygen", "mxe_key_recovery_init"];
for (const n of names) {
  const off = Buffer.from(c.getCompDefAccOffset(n)).readUInt32LE(0);
  console.log(n, "->", off);
}
const arciumProgramId = new PublicKey("Arcj82pX7HxYKLR92qvgZUAd7vGS1k4hQvAFcPATFdEQ");
const mxeAddr = c.getMXEAccAddress(arciumProgramId);
console.log("MXE from arcium ID:", mxeAddr.toBase58());
const walnutProgramId = new PublicKey("5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea");
const mxeAddr2 = c.getMXEAccAddress(walnutProgramId);
console.log("MXE from walnut ID:", mxeAddr2.toBase58());
console.log("Expected MXE: G66Kuw86rMipmThR5EYejGoHfFcN1rFxrhyVVhSHiRkJ");
'
