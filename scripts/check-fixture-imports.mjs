#!/usr/bin/env node
// Fail if production code imports test fixtures, mocks, fakes, or test-only
// libraries (Requirement 1.8).
//
// Usage: node scripts/check-fixture-imports.mjs [--root DIR]
//
// Production code is everything under programs/, encrypted-ixs/, packages/,
// keeper/, and web/, except test files and directories (see SKIP_DIRS and
// isTestFile). Rust items gated by #[cfg(test)] are skipped.
//
// Exit status: 0 clean, 1 violations found, 2 usage error.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const PRODUCTION_ROOTS = [
  "programs",
  "encrypted-ixs",
  "packages",
  "keeper",
  "web",
];

const SKIP_DIRS = new Set([
  "node_modules",
  "dist",
  "build",
  "target",
  "out",
  "coverage",
  ".next",
  ".turbo",
  "tests",
  "test",
  "__tests__",
  "benches",
  "examples",
]);

const JS_EXT = new Set([
  ".ts",
  ".tsx",
  ".mts",
  ".cts",
  ".js",
  ".jsx",
  ".mjs",
  ".cjs",
]);

// Module path segments that indicate fixture or mock code.
const JS_FORBIDDEN_SEGMENT =
  /^(__)?(fixtures?|mocks?|fakes?|stubs?|test-?utils?|test-?helpers?|testing)(__)?$/i;
const JS_FORBIDDEN_SUFFIX = /\.(fixtures?|mocks?|fakes?|stubs?|test|spec)$/i;
// Test-only npm packages that must not appear in production imports.
export const JS_FORBIDDEN_PACKAGES = [
  "sinon",
  "nock",
  "msw",
  "testdouble",
  "jest-mock",
  "@jest/globals",
  "vitest",
  "mocha",
  "chai",
  "fast-check",
  "node:test",
];

const RUST_FORBIDDEN_NAME =
  /^(fixtures?|mocks?|fakes?|stubs?|test_?utils?|test_?helpers?|testing)$/;
export const RUST_FORBIDDEN_CRATES = [
  "mockall",
  "mockito",
  "proptest",
  "quickcheck",
  "rstest",
  "solana_program_test",
  "litesvm",
];

export function isTestFile(name) {
  return (
    /\.(test|spec)\.[cm]?[jt]sx?$/.test(name) ||
    /^(vitest|jest|playwright|karma)\.config\./.test(name) ||
    name.startsWith(".mocharc") ||
    name === "tests.rs"
  );
}

function stripExt(segment) {
  return segment.replace(/\.[cm]?[jt]sx?$/, "");
}

/** Reason a JS/TS module specifier is forbidden, or null. */
export function forbiddenJsSpecifier(spec, fileAbs, root) {
  for (const pkg of JS_FORBIDDEN_PACKAGES) {
    if (spec === pkg || spec.startsWith(`${pkg}/`))
      return `imports test-only package "${pkg}"`;
  }
  if (spec.startsWith(".")) {
    const target = path
      .relative(root, path.resolve(path.dirname(fileAbs), spec))
      .split(path.sep)
      .join("/");
    if (target === "tests" || target.startsWith("tests/"))
      return `imports from the test directory (${target})`;
  }
  for (const raw of spec.split(/[/\\]/)) {
    const segment = stripExt(raw.replace(/^@/, ""));
    if (
      JS_FORBIDDEN_SEGMENT.test(segment) ||
      JS_FORBIDDEN_SUFFIX.test(segment)
    ) {
      return `imports fixture/mock module "${spec}"`;
    }
  }
  return null;
}

const JS_SPECIFIER_PATTERNS = [
  /\bfrom\s*["']([^"']+)["']/g,
  /\bimport\s*\(\s*["']([^"']+)["']\s*\)/g,
  /^\s*import\s+["']([^"']+)["']/g,
  /\brequire\s*\(\s*["']([^"']+)["']\s*\)/g,
];

/** Violations in one JS/TS source. */
export function checkJsSource(text, fileAbs, root) {
  const violations = [];
  text.split(/\r?\n/).forEach((line, index) => {
    const trimmed = line.trim();
    if (
      trimmed.startsWith("//") ||
      trimmed.startsWith("*") ||
      trimmed.startsWith("/*")
    )
      return;
    for (const pattern of JS_SPECIFIER_PATTERNS) {
      for (const match of line.matchAll(pattern)) {
        const reason = forbiddenJsSpecifier(match[1], fileAbs, root);
        if (reason) violations.push({ line: index + 1, reason });
      }
    }
  });
  return violations;
}

function forbiddenRustPath(p) {
  const segments = p.split(/\//);
  if (segments.includes("tests")) return true;
  return segments.some((s) => {
    const stem = s.replace(/\.[A-Za-z0-9]+$/, "");
    return (
      RUST_FORBIDDEN_NAME.test(stem) ||
      /\.(fixtures?|mocks?)$/.test(stem) ||
      /[_-](fixtures?|mocks?)$/.test(stem)
    );
  });
}

/** Violations in one Rust source. Skips items gated by #[cfg(test)]. */
export function checkRustSource(text) {
  const violations = [];
  const lines = text.split(/\r?\n/);
  let pendingTestCfg = false;
  let skipDepth = 0; // >0 while inside a #[cfg(test)] block
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();
    if (skipDepth > 0) {
      skipDepth +=
        (line.match(/\{/g) ?? []).length - (line.match(/\}/g) ?? []).length;
      continue;
    }
    if (trimmed === "" || trimmed.startsWith("//")) continue;
    if (
      /^#\[cfg\((.*\W)?test(\W.*)?\)\]/.test(trimmed) &&
      !/\bnot\s*\(\s*test\b/.test(trimmed)
    ) {
      pendingTestCfg = true;
      continue;
    }
    if (trimmed.startsWith("#[") || trimmed.startsWith("#![")) {
      if (/^#!?\[path\s*=\s*"([^"]+)"\]/.test(trimmed)) {
        const p = /"([^"]+)"/.exec(trimmed)[1];
        if (!pendingTestCfg && forbiddenRustPath(p)) {
          violations.push({
            line: i + 1,
            reason: `#[path] points at fixture/test code "${p}"`,
          });
        }
      }
      continue; // other attributes keep a pending #[cfg(test)] alive
    }
    if (pendingTestCfg) {
      pendingTestCfg = false;
      const depth =
        (line.match(/\{/g) ?? []).length - (line.match(/\}/g) ?? []).length;
      if (depth > 0) skipDepth = depth;
      continue;
    }
    const mod = /^(pub(\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)/.exec(
      trimmed,
    );
    if (mod && RUST_FORBIDDEN_NAME.test(mod[3])) {
      violations.push({
        line: i + 1,
        reason: `declares fixture/mock module "${mod[3]}" outside #[cfg(test)]`,
      });
    }
    const use = /^(pub(\([^)]*\))?\s+)?(use|extern\s+crate)\s+([^;]+)/.exec(
      trimmed,
    );
    if (use) {
      const idents = use[4].split(/[^A-Za-z0-9_]+/).filter(Boolean);
      const crate =
        idents[0] === "crate" || idents[0] === "self" || idents[0] === "super"
          ? null
          : idents[0];
      if (crate && RUST_FORBIDDEN_CRATES.includes(crate)) {
        violations.push({
          line: i + 1,
          reason: `uses test-only crate "${crate}" outside #[cfg(test)]`,
        });
      } else if (idents.some((id) => RUST_FORBIDDEN_NAME.test(id))) {
        violations.push({
          line: i + 1,
          reason: `imports fixture/mock path "${use[4].trim()}" outside #[cfg(test)]`,
        });
      }
    }
    for (const m of line.matchAll(
      /\binclude_(?:bytes|str)!\s*\(\s*"([^"]+)"/g,
    )) {
      if (forbiddenRustPath(m[1]))
        violations.push({
          line: i + 1,
          reason: `includes fixture/test file "${m[1]}"`,
        });
    }
  }
  return violations;
}

function* walk(dir) {
  let entries;
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return;
  }
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (!SKIP_DIRS.has(entry.name) && !entry.name.startsWith("."))
        yield* walk(full);
    } else if (entry.isFile()) {
      yield full;
    }
  }
}

/** Scan the production roots under `root`. Returns { files, violations }. */
export function scanRepository(root) {
  const violations = [];
  let files = 0;
  for (const top of PRODUCTION_ROOTS) {
    for (const file of walk(path.join(root, top))) {
      const name = path.basename(file);
      if (isTestFile(name)) continue;
      const ext = path.extname(name);
      let found;
      if (ext === ".rs") found = checkRustSource(fs.readFileSync(file, "utf8"));
      else if (JS_EXT.has(ext))
        found = checkJsSource(fs.readFileSync(file, "utf8"), file, root);
      else continue;
      files++;
      const rel = path.relative(root, file).split(path.sep).join("/");
      for (const v of found) violations.push({ file: rel, ...v });
    }
  }
  return { files, violations };
}

export function main(argv, out = console.log, err = console.error) {
  let root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--root" && argv[i + 1]) root = path.resolve(argv[++i]);
    else {
      err(`check-fixture-imports: unknown argument ${argv[i]}`);
      err("usage: node scripts/check-fixture-imports.mjs [--root DIR]");
      return 2;
    }
  }
  const { files, violations } = scanRepository(root);
  if (violations.length === 0) {
    out(
      `Fixture-import check passed: ${files} production source file(s) under ${PRODUCTION_ROOTS.join(", ")}.`,
    );
    return 0;
  }
  for (const v of violations) err(`${v.file}:${v.line}: ${v.reason}`);
  err(
    `\nFixture-import check failed: ${violations.length} violation(s) in ${files} production file(s).`,
  );
  err(
    "Production code must not import fixtures, mocks, fakes, or test-only libraries. Move them under tests/.",
  );
  return 1;
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  process.exitCode = main(process.argv.slice(2));
}
