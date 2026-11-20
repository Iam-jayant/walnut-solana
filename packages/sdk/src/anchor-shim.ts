// Shim for @anchor-lang/core CJS interop under ESM and bundlers (Next.js/Turbopack).
import * as anchorModule from "@anchor-lang/core";

// When bundled or under Node/ESM, anchorModule might be { default: { BN, ... }, BN, ... }
const anchor = (anchorModule as any).default?.BN ? (anchorModule as any).default : anchorModule;

export const BN = anchor.BN;
export type BN = InstanceType<typeof anchor.BN>;
export const AnchorProvider = anchor.AnchorProvider;
export type AnchorProvider = InstanceType<typeof anchor.AnchorProvider>;
export const Program = anchor.Program;
export type Program = InstanceType<typeof anchor.Program>;
export const Wallet = anchor.Wallet;
export const Provider = anchor.Provider;
export const setProvider = anchor.setProvider;
export const getProvider = anchor.getProvider;
export type { Idl } from "@anchor-lang/core";

export default anchor;

