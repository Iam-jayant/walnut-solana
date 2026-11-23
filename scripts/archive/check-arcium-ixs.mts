import { readFileSync } from "fs";
const idl = JSON.parse(readFileSync("./node_modules/@arcium-hq/client/build/ARCIUM_IDL.json", "utf8"));
console.log("Arcium IDL instructions:");
for (const i of idl.instructions || []) {
  console.log(`  ${i.name}`);
}
