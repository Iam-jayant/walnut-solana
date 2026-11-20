// Shim for @anchor-lang/core CJS interop under ESM (tsx/node).
// @anchor-lang/core is a CJS package; named ESM imports like `import { BN } from "@anchor-lang/core"`
// fail because Node does not auto-expose CJS `module.exports` properties as ESM named exports.
// Use `createRequire` to access the real CJS exports.
import { createRequire } from "module";
const require = createRequire(import.meta.url);
const anchor = require("@anchor-lang/core");

export const BN = anchor.BN;
export const AnchorProvider = anchor.AnchorProvider;
export const Wallet = anchor.Wallet;
export const Provider = anchor.Provider;
export const setProvider = anchor.setProvider;
export const getProvider = anchor.getProvider;
export type { Program, Account, Idl } from "@anchor-lang/core";

export default anchor;
