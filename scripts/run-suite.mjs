#!/usr/bin/env node
// Run a named test suite in every workspace package that defines it.
//
// Usage: node scripts/run-suite.mjs <script-name>
//
// Runs `pnpm -r --if-present run <script-name>` and first lists the packages
// that define the script. If none do yet, prints that zero suites ran and
// exits 0, so CI wiring can exist before the first test of that kind.
// Suite names and conventions are documented in docs/CI.md#test-suites.

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const name = process.argv[2];
if (!name || !/^[A-Za-z0-9:._-]+$/.test(name) || process.argv.length > 3) {
  console.error("usage: node scripts/run-suite.mjs <script-name>");
  process.exit(2);
}

const shell = process.platform === "win32";
const run = (args, opts = {}) =>
  spawnSync("pnpm", args, { shell, encoding: "utf8", ...opts });

const listed = run(["-r", "ls", "--depth", "-1", "--json"]);
if (listed.status !== 0) {
  console.error(
    `run-suite: pnpm ls failed:\n${listed.stderr || listed.stdout}`,
  );
  process.exit(2);
}
let packages;
try {
  packages = JSON.parse(listed.stdout);
} catch (e) {
  console.error(`run-suite: cannot parse pnpm ls output: ${e.message}`);
  process.exit(2);
}

const root = path.resolve(process.cwd());
const defining = packages
  .filter((p) => path.resolve(p.path) !== root)
  .filter((p) => {
    try {
      const pkg = JSON.parse(
        fs.readFileSync(path.join(p.path, "package.json"), "utf8"),
      );
      return Boolean(pkg.scripts && pkg.scripts[name]);
    } catch {
      return false;
    }
  })
  .map((p) => p.name ?? path.relative(root, p.path));

if (defining.length === 0) {
  console.log(
    `Suite "${name}": no workspace package defines it yet; 0 suites run.`,
  );
  process.exit(0);
}

console.log(`Suite "${name}": running in ${defining.join(", ")}`);
const result = run(["-r", "--if-present", "run", name], { stdio: "inherit" });
process.exit(result.status ?? 1);
