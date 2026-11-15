import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import {
  checkJsSource,
  checkRustSource,
  scanRepository,
} from "../check-fixture-imports.mjs";

const ROOT = path.resolve("/repo");
const FILE = path.join(ROOT, "packages", "sdk", "src", "index.ts");
const js = (text) => checkJsSource(text, FILE, ROOT).map((v) => v.line);

describe("JS/TS imports", () => {
  it("allows ordinary imports", () => {
    assert.deepEqual(
      js(
        'import { PublicKey } from "@solana/web3.js";\nimport { x } from "./math/wad.js";\n',
      ),
      [],
    );
  });

  it("flags fixture, mock, and fake modules", () => {
    assert.deepEqual(
      js(
        [
          'import { price } from "./fixtures/pyth.js";',
          'import mock from "../__mocks__/rpc";',
          'const f = require("./oracle.fixture");',
          'export * from "./fake-rpc/fakes";',
          'const m = await import("./mocks/index.js");',
          'import "./test-utils/setup";',
        ].join("\n"),
      ),
      [1, 2, 3, 4, 5, 6],
    );
  });

  it("flags test-only packages and imports from tests/", () => {
    assert.deepEqual(
      js(
        [
          'import sinon from "sinon";',
          'import fc from "fast-check";',
          'import { a } from "../../../tests/helpers";',
        ].join("\n"),
      ),
      [1, 2, 3],
    );
  });

  it("ignores comments", () => {
    assert.deepEqual(
      js('// import x from "./fixtures/a";\n * from "./mocks"\n'),
      [],
    );
  });
});

describe("Rust items", () => {
  const rust = (text) => checkRustSource(text).map((v) => v.line);

  it("allows production modules and imports", () => {
    assert.deepEqual(
      rust("use anchor_lang::prelude::*;\npub mod state;\nmod oracle;\n"),
      [],
    );
  });

  it("flags fixture modules, imports, includes, and test crates", () => {
    assert.deepEqual(
      rust(
        [
          "mod fixtures;",
          "use crate::mocks::Price;",
          'const B: &[u8] = include_bytes!("../tests/data.bin");',
          "use proptest::prelude::*;",
          'const C: &str = include_str!("price_fixture.json");',
        ].join("\n"),
      ),
      [1, 2, 3, 4, 5],
    );
  });

  it("skips items gated by #[cfg(test)]", () => {
    const text = [
      "#[cfg(test)]",
      "mod fixtures;",
      "#[cfg(test)]",
      "#[allow(dead_code)]",
      "mod tests {",
      "    use super::fixtures::*;",
      "    fn nested() { let _ = 1; }",
      "}",
      "use crate::mocks::After;",
    ].join("\n");
    assert.deepEqual(rust(text), [9]);
  });

  it("does not treat #[cfg(not(test))] as test-only", () => {
    assert.deepEqual(
      rust("#[cfg(not(test))]\nuse crate::mocks::Price;\n"),
      [2],
    );
  });
});

describe("repository scan", () => {
  it("scans production roots and skips test files and directories", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "walnut-fixtures-"));
    const put = (rel, text) => {
      const file = path.join(root, rel);
      fs.mkdirSync(path.dirname(file), { recursive: true });
      fs.writeFileSync(file, text);
    };
    put("packages/sdk/src/index.ts", 'import { p } from "./fixtures/p.js";\n');
    put(
      "packages/sdk/src/index.test.ts",
      'import { p } from "./fixtures/p.js";\n',
    );
    put("packages/sdk/src/__tests__/a.ts", 'import sinon from "sinon";\n');
    put("programs/walnut/src/lib.rs", "use crate::fixtures::Price;\n");
    put("programs/walnut/tests/it.rs", "use crate::fixtures::Price;\n");
    put("tests/walnut.ts", 'import { p } from "./fixtures/p";\n');

    const { files, violations } = scanRepository(root);
    assert.equal(files, 2);
    assert.deepEqual(violations.map((v) => `${v.file}:${v.line}`).sort(), [
      "packages/sdk/src/index.ts:1",
      "programs/walnut/src/lib.rs:1",
    ]);
  });

  it("passes on this repository", () => {
    const repo = path.resolve(import.meta.dirname, "..", "..");
    assert.deepEqual(scanRepository(repo).violations, []);
  });
});
