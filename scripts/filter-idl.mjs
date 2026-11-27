// Removes the `legacy`-gated instructions from a full IDL, giving the IDL of
// the slim mainnet program. `anchor idl build` cannot build without the
// `legacy` feature (its type collection ignores `cfg` on instructions), so the
// full IDL is built and filtered. Discriminators are unchanged.
//
//   node scripts/filter-idl.mjs <full-idl.json> <out.json>
import { readFileSync, writeFileSync } from "node:fs";

const [input, output] = process.argv.slice(2);
if (input === undefined || output === undefined) {
  throw new Error("usage: filter-idl.mjs <full-idl.json> <out.json>");
}

const source = readFileSync(
  new URL("../programs/walnut/src/lib.rs", import.meta.url),
  "utf8",
);
// Each gated instruction: the cfg line, its docs and attributes, then `pub fn`.
const gated = new Set(
  [
    ...source.matchAll(
      /^ {4}#\[cfg\(feature = "legacy"\)\]\r?\n(?:(?! {4}pub fn ).*\r?\n)*? {4}pub fn (\w+)/gm,
    ),
  ].map((m) => m[1]),
);
if (gated.size === 0) throw new Error("no gated instructions found in lib.rs");

const idl = JSON.parse(readFileSync(input, "utf8"));
const names = new Set(idl.instructions.map((i) => i.name));
const unknown = [...gated].filter((n) => !names.has(n));
if (unknown.length > 0) {
  throw new Error(`gated instructions missing from IDL: ${unknown.join(", ")}`);
}
idl.instructions = idl.instructions.filter((i) => !gated.has(i.name));
writeFileSync(output, `${JSON.stringify(idl, null, 2)}\n`);
console.log(
  `${output}: ${idl.instructions.length} instructions (${gated.size} removed)`,
);
