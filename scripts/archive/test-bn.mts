import { createRequire } from 'module';
const require = createRequire(import.meta.url);
const { BN } = require('@anchor-lang/core');
console.log("BN type:", typeof BN);
console.log("BN instance:", new BN(123).toString());
