#!/usr/bin/env node
// Dependency-advisory policy gate (Requirement 1.5).
//
// Usage:
//   node scripts/advisories.mjs check [--file F] [--today YYYY-MM-DD]
//       Validate security/advisory-exceptions.toml only.
//   node scripts/advisories.mjs evaluate [--cargo REPORT] [--npm REPORT] [--file F] [--today YYYY-MM-DD]
//       Validate exceptions, then apply the policy to cargo-audit --json and
//       pnpm audit --json reports.
//
// Policy:
//   - Every RustSec vulnerability (cargo-audit "vulnerabilities") blocks.
//     RustSec entries often carry no CVSS score, so all are treated as high.
//   - npm advisories with severity "high" or "critical" block.
//   - cargo-audit warnings (unmaintained, unsound, yanked) and lower-severity
//     npm advisories are reported but do not block.
//   - A blocking advisory passes only if an exception with the same ecosystem,
//     package, and ID (or alias) is valid and unexpired.
//
// Exit status: 0 pass, 1 policy failure, 2 usage or unreadable input.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
export const DEFAULT_EXCEPTIONS_FILE = path.join(
  ROOT,
  "security",
  "advisory-exceptions.toml",
);
export const MAX_EXCEPTION_DAYS = 90;
export const MIN_REASON_LENGTH = 40;
export const BLOCKING_NPM_SEVERITIES = new Set(["high", "critical"]);

const FIELDS = [
  "id",
  "ecosystem",
  "package",
  "reason",
  "owner",
  "added",
  "expires",
];
const ECOSYSTEMS = new Set(["cargo", "npm"]);
const ID_PATTERN =
  /^(RUSTSEC-\d{4}-\d{4}|GHSA-[0-9a-z]{4}-[0-9a-z]{4}-[0-9a-z]{4}|CVE-\d{4}-\d{4,})$/;
const PLACEHOLDER_PATTERN = /\b(TODO|TBD|FIXME|XXX)\b|<[^>]*>/i;
const DAY_MS = 24 * 60 * 60 * 1000;

// ---------------------------------------------------------------------------
// Exceptions file: a strict TOML subset ([[exception]] tables, string values).

function unescapeBasicString(raw) {
  return raw.replace(/\\(u[0-9A-Fa-f]{4}|["\\nt])/g, (_, e) => {
    if (e === "n") return "\n";
    if (e === "t") return "\t";
    if (e.startsWith("u"))
      return String.fromCharCode(Number.parseInt(e.slice(1), 16));
    return e;
  });
}

/** Parse the exceptions file. Returns { exceptions, errors }. */
export function parseExceptions(text, file = "advisory-exceptions.toml") {
  const exceptions = [];
  const errors = [];
  let current = null;
  const lines = text.split(/\r?\n/);
  lines.forEach((rawLine, index) => {
    const lineNo = index + 1;
    const line = rawLine.trim();
    if (line === "" || line.startsWith("#")) return;
    if (/^\[\[exception\]\]\s*(#.*)?$/.test(line)) {
      current = { line: lineNo };
      exceptions.push(current);
      return;
    }
    if (line.startsWith("[")) {
      errors.push(
        `${file}:${lineNo}: only [[exception]] tables are allowed, found ${line}`,
      );
      return;
    }
    const kv = /^([A-Za-z0-9_-]+)\s*=\s*"((?:[^"\\]|\\.)*)"\s*(#.*)?$/.exec(
      line,
    );
    if (!kv) {
      errors.push(
        `${file}:${lineNo}: unsupported syntax; use key = "string value"`,
      );
      return;
    }
    const [, key, value] = kv;
    if (!current) {
      errors.push(
        `${file}:${lineNo}: "${key}" appears before any [[exception]] table`,
      );
      return;
    }
    if (!FIELDS.includes(key)) {
      errors.push(
        `${file}:${lineNo}: unknown field "${key}" (allowed: ${FIELDS.join(", ")})`,
      );
      return;
    }
    if (Object.hasOwn(current, key)) {
      errors.push(`${file}:${lineNo}: duplicate field "${key}"`);
      return;
    }
    current[key] = unescapeBasicString(value);
  });
  return { exceptions, errors };
}

/** Parse YYYY-MM-DD as a UTC day number, or return null. */
export function parseDay(value) {
  if (typeof value !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(value))
    return null;
  const ms = Date.parse(`${value}T00:00:00Z`);
  if (Number.isNaN(ms) || new Date(ms).toISOString().slice(0, 10) !== value)
    return null;
  return Math.round(ms / DAY_MS);
}

/** Validate parsed exceptions against the policy. Returns a list of errors. */
export function validateExceptions(
  exceptions,
  today,
  file = "advisory-exceptions.toml",
) {
  const errors = [];
  const todayDay = parseDay(today);
  if (todayDay === null) throw new Error(`invalid --today date: ${today}`);
  const seen = new Map();
  for (const ex of exceptions) {
    const where = `${file}:${ex.line}${ex.id ? ` (${ex.id})` : ""}`;
    for (const field of FIELDS) {
      if (typeof ex[field] !== "string" || ex[field].trim() === "") {
        errors.push(`${where}: missing required field "${field}"`);
      }
    }
    if (ex.id && !ID_PATTERN.test(ex.id)) {
      errors.push(
        `${where}: id must be RUSTSEC-YYYY-NNNN, GHSA-xxxx-xxxx-xxxx, or CVE-YYYY-NNNN`,
      );
    }
    if (ex.ecosystem && !ECOSYSTEMS.has(ex.ecosystem)) {
      errors.push(
        `${where}: ecosystem must be "cargo" or "npm", found "${ex.ecosystem}"`,
      );
    }
    if (typeof ex.reason === "string" && ex.reason.trim() !== "") {
      if (ex.reason.trim().length < MIN_REASON_LENGTH) {
        errors.push(
          `${where}: reason must explain the exception in at least ${MIN_REASON_LENGTH} characters`,
        );
      }
      if (PLACEHOLDER_PATTERN.test(ex.reason)) {
        errors.push(
          `${where}: reason contains a placeholder (TODO, TBD, FIXME, XXX, or <...>)`,
        );
      }
    }
    if (typeof ex.owner === "string" && PLACEHOLDER_PATTERN.test(ex.owner)) {
      errors.push(`${where}: owner contains a placeholder`);
    }
    const added = ex.added === undefined ? undefined : parseDay(ex.added);
    const expires = ex.expires === undefined ? undefined : parseDay(ex.expires);
    if (added === null)
      errors.push(`${where}: added must be a valid YYYY-MM-DD date`);
    if (expires === null)
      errors.push(`${where}: expires must be a valid YYYY-MM-DD date`);
    if (typeof added === "number" && added > todayDay) {
      errors.push(`${where}: added date ${ex.added} is in the future`);
    }
    if (typeof expires === "number") {
      if (expires < todayDay) {
        errors.push(
          `${where}: expired on ${ex.expires}; re-review the advisory and renew or remove the exception`,
        );
      }
      if (typeof added === "number") {
        if (expires < added) {
          errors.push(
            `${where}: expires ${ex.expires} is before added ${ex.added}`,
          );
        } else if (expires - added > MAX_EXCEPTION_DAYS) {
          errors.push(
            `${where}: expires ${ex.expires} is more than ${MAX_EXCEPTION_DAYS} days after added ${ex.added}`,
          );
        }
      }
    }
    if (ex.ecosystem && ex.id) {
      const key = `${ex.ecosystem}:${ex.package ?? ""}:${ex.id}`;
      if (seen.has(key))
        errors.push(
          `${where}: duplicates the exception at line ${seen.get(key)}`,
        );
      else seen.set(key, ex.line);
    }
  }
  return errors;
}

// ---------------------------------------------------------------------------
// Audit reports -> normalized findings.

/** Findings from `cargo audit --json`. */
export function cargoFindings(report) {
  if (
    !report ||
    typeof report !== "object" ||
    !report.vulnerabilities ||
    !Array.isArray(report.vulnerabilities.list)
  ) {
    throw new Error(
      "cargo-audit report has no vulnerabilities.list; did cargo audit fail?",
    );
  }
  const findings = [];
  for (const v of report.vulnerabilities.list) {
    const adv = v.advisory ?? {};
    findings.push({
      ecosystem: "cargo",
      kind: "vulnerability",
      id: adv.id,
      aliases: Array.isArray(adv.aliases) ? adv.aliases : [],
      package: adv.package ?? v.package?.name,
      version: v.package?.version,
      severity: "vulnerability",
      title: adv.title ?? "",
      url:
        adv.url ?? (adv.id ? `https://rustsec.org/advisories/${adv.id}` : ""),
      blocking: true,
    });
  }
  for (const [kind, list] of Object.entries(report.warnings ?? {})) {
    if (!Array.isArray(list)) continue;
    for (const w of list) {
      const adv = w.advisory ?? {};
      findings.push({
        ecosystem: "cargo",
        kind,
        id: adv.id ?? `${kind}:${w.package?.name}@${w.package?.version}`,
        aliases: Array.isArray(adv.aliases) ? adv.aliases : [],
        package: adv.package ?? w.package?.name,
        version: w.package?.version,
        severity: kind,
        title: adv.title ?? kind,
        url: adv.url ?? "",
        blocking: false,
      });
    }
  }
  return findings;
}

/** Findings from `pnpm audit --json` (npm v6 audit format). */
export function npmFindings(report) {
  if (report && typeof report === "object" && report.error) {
    const e = report.error;
    throw new Error(
      `pnpm audit failed: ${e.code ?? ""} ${e.message ?? e.summary ?? JSON.stringify(e)}`.trim(),
    );
  }
  if (
    !report ||
    typeof report !== "object" ||
    !report.advisories ||
    typeof report.advisories !== "object"
  ) {
    throw new Error(
      "pnpm audit report has no advisories object; did pnpm audit fail?",
    );
  }
  return Object.values(report.advisories).map((a) => {
    const id = a.github_advisory_id || String(a.id);
    const aliases = [
      String(a.id),
      ...(Array.isArray(a.cves) ? a.cves : []),
    ].filter((x) => x && x !== id);
    const severity = String(a.severity ?? "unknown").toLowerCase();
    return {
      ecosystem: "npm",
      kind: "advisory",
      id,
      aliases,
      package: a.module_name,
      version: [...new Set((a.findings ?? []).map((f) => f.version))].join(
        ", ",
      ),
      severity,
      title: a.title ?? "",
      url: a.url ?? "",
      paths: (a.findings ?? []).flatMap((f) => f.paths ?? []),
      // Unknown severities block rather than pass silently.
      blocking:
        BLOCKING_NPM_SEVERITIES.has(severity) ||
        !["info", "low", "moderate"].includes(severity),
    };
  });
}

// ---------------------------------------------------------------------------
// Policy evaluation.

function matches(exception, finding) {
  return (
    exception.ecosystem === finding.ecosystem &&
    exception.package === finding.package &&
    (exception.id === finding.id || finding.aliases.includes(exception.id))
  );
}

/** Apply exceptions to findings. Exceptions must already be valid. */
export function evaluate(findings, exceptions) {
  const blocking = [];
  const excepted = [];
  const informational = [];
  const used = new Set();
  for (const finding of findings) {
    const ex = exceptions.find((e) => matches(e, finding));
    if (ex) used.add(ex);
    if (!finding.blocking) informational.push(finding);
    else if (ex) excepted.push({ finding, exception: ex });
    else blocking.push(finding);
  }
  const unused = exceptions.filter((e) => !used.has(e));
  return { blocking, excepted, informational, unused };
}

export function exceptionTemplate(finding, today) {
  const todayDay = parseDay(today);
  const max = new Date((todayDay + MAX_EXCEPTION_DAYS) * DAY_MS)
    .toISOString()
    .slice(0, 10);
  return [
    "[[exception]]",
    `id = "${finding.id}"`,
    `ecosystem = "${finding.ecosystem}"`,
    `package = "${finding.package}"`,
    `reason = "<why it is not exploitable here or cannot be fixed yet, and what removes it>"`,
    `owner = "<GitHub handle>"`,
    `added = "${today}"`,
    `expires = "<no later than ${max}>"`,
  ].join("\n");
}

// ---------------------------------------------------------------------------
// CLI.

function parseArgs(argv) {
  const [command, ...rest] = argv;
  const opts = {
    command,
    file: DEFAULT_EXCEPTIONS_FILE,
    today: new Date().toISOString().slice(0, 10),
  };
  for (let i = 0; i < rest.length; i++) {
    const flag = rest[i];
    const value = rest[i + 1];
    const take = (name) => {
      if (value === undefined || value.startsWith("--"))
        throw new Error(`${flag} needs a value`);
      opts[name] = value;
      i++;
    };
    if (flag === "--file") take("file");
    else if (flag === "--today") take("today");
    else if (flag === "--cargo") take("cargo");
    else if (flag === "--npm") take("npm");
    else throw new Error(`unknown argument: ${flag}`);
  }
  if (command !== "check" && command !== "evaluate")
    throw new Error("command must be check or evaluate");
  if (parseDay(opts.today) === null)
    throw new Error(`invalid --today date: ${opts.today}`);
  if (command === "evaluate" && !opts.cargo && !opts.npm)
    throw new Error("evaluate needs --cargo and/or --npm");
  return opts;
}

function readJson(file, label) {
  let text;
  try {
    text = fs.readFileSync(file, "utf8");
  } catch (e) {
    throw new Error(`cannot read ${label} report ${file}: ${e.message}`, {
      cause: e,
    });
  }
  try {
    return JSON.parse(text.replace(/^\uFEFF/, ""));
  } catch (e) {
    throw new Error(
      `${label} report ${file} is not valid JSON (${e.message}); first bytes: ${JSON.stringify(text.slice(0, 120))}`,
      { cause: e },
    );
  }
}

function describe(f) {
  const version = f.version ? `@${f.version}` : "";
  return `${f.ecosystem} ${f.package}${version} ${f.id} [${f.severity}] ${f.title}${f.url ? ` (${f.url})` : ""}`;
}

export function main(argv, out = console.log, err = console.error) {
  let opts;
  try {
    opts = parseArgs(argv);
  } catch (e) {
    err(`advisories: ${e.message}`);
    err(
      "usage: node scripts/advisories.mjs check|evaluate [--cargo F] [--npm F] [--file F] [--today YYYY-MM-DD]",
    );
    return 2;
  }
  const rel = path.relative(process.cwd(), opts.file) || opts.file;
  let text;
  try {
    text = fs.readFileSync(opts.file, "utf8");
  } catch (e) {
    err(`advisories: cannot read exceptions file ${rel}: ${e.message}`);
    return 2;
  }
  const { exceptions, errors: parseErrors } = parseExceptions(text, rel);
  const errors = [
    ...parseErrors,
    ...validateExceptions(exceptions, opts.today, rel),
  ];
  out(
    `Advisory exceptions: ${exceptions.length} in ${rel} (today ${opts.today} UTC)`,
  );
  for (const e of errors) err(`  FAIL  ${e}`);

  if (opts.command === "check") {
    if (errors.length > 0) {
      err(
        `${errors.length} exception problem(s). See docs/CI.md#advisory-exceptions.`,
      );
      return 1;
    }
    out("All exceptions are justified, owned, and unexpired.");
    return 0;
  }

  let findings = [];
  try {
    if (opts.cargo)
      findings = findings.concat(
        cargoFindings(readJson(opts.cargo, "cargo-audit")),
      );
    if (opts.npm)
      findings = findings.concat(npmFindings(readJson(opts.npm, "pnpm audit")));
  } catch (e) {
    err(`advisories: ${e.message}`);
    return 2;
  }
  // Only valid, unexpired exceptions may suppress findings.
  const valid = exceptions.filter(
    (ex) => validateExceptions([ex], opts.today, rel).length === 0,
  );
  const result = evaluate(findings, valid);

  out(
    `\nFindings: ${findings.length} (${result.blocking.length} blocking, ${result.excepted.length} excepted, ${result.informational.length} informational)`,
  );
  for (const { finding, exception } of result.excepted) {
    out(
      `  excepted  ${describe(finding)}\n            owner ${exception.owner}, expires ${exception.expires}`,
    );
  }
  for (const f of result.informational) out(`  info      ${describe(f)}`);
  for (const ex of result.unused) {
    out(
      `  warn      exception ${ex.ecosystem} ${ex.package} ${ex.id} (line ${ex.line}) matches no current advisory; remove it`,
    );
  }
  for (const f of result.blocking) {
    err(`  BLOCKING  ${describe(f)}`);
    if (f.paths?.length)
      err(`            via ${f.paths.slice(0, 3).join("; ")}`);
  }
  if (result.blocking.length > 0) {
    err(
      "\nUpgrade the dependency, or add a reviewed exception to security/advisory-exceptions.toml:",
    );
    for (const f of result.blocking)
      err(`\n${exceptionTemplate(f, opts.today)}`);
  }
  if (errors.length > 0 || result.blocking.length > 0) {
    err(
      `\nAdvisory policy failed: ${errors.length} exception problem(s), ${result.blocking.length} blocking advisory(ies).`,
    );
    return 1;
  }
  out("\nAdvisory policy passed.");
  return 0;
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  process.exitCode = main(process.argv.slice(2));
}
