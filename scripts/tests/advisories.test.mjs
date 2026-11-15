import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import {
  DEFAULT_EXCEPTIONS_FILE,
  cargoFindings,
  evaluate,
  main,
  npmFindings,
  parseExceptions,
  validateExceptions,
} from "../advisories.mjs";

const TODAY = "2026-11-15";
const REASON =
  "Dev-only dependency reached only by local test tooling; no untrusted input.";

function entry(overrides = {}) {
  const fields = {
    id: "GHSA-aaaa-bbbb-cccc",
    ecosystem: "npm",
    package: "left-pad",
    reason: REASON,
    owner: "maintainer",
    added: "2026-11-01",
    expires: "2026-12-31",
    ...overrides,
  };
  const body = Object.entries(fields)
    .filter(([, v]) => v !== undefined)
    .map(([k, v]) => `${k} = "${v}"`)
    .join("\n");
  return `[[exception]]\n${body}\n`;
}

function errorsFor(text, today = TODAY) {
  const { exceptions, errors } = parseExceptions(text);
  return [...errors, ...validateExceptions(exceptions, today)];
}

describe("exception file validation", () => {
  it("accepts a complete, justified, unexpired exception", () => {
    assert.deepEqual(errorsFor(entry()), []);
  });

  it("accepts an exception on its last valid day", () => {
    assert.deepEqual(errorsFor(entry({ expires: TODAY })), []);
  });

  it("rejects an expired exception", () => {
    const errors = errorsFor(entry({ expires: "2026-11-14" }));
    assert.equal(errors.length, 1);
    assert.match(errors[0], /expired on 2026-11-14/);
  });

  it("rejects missing or placeholder justification", () => {
    assert.match(
      errorsFor(entry({ reason: undefined }))[0],
      /missing required field "reason"/,
    );
    assert.match(
      errorsFor(entry({ reason: "low risk" }))[0],
      /at least 40 characters/,
    );
    assert.match(
      errorsFor(entry({ reason: `${REASON} TODO confirm` }))[0],
      /placeholder/,
    );
  });

  it("rejects missing owner and expiry", () => {
    const errors = errorsFor(entry({ owner: undefined, expires: undefined }));
    assert.ok(errors.some((e) => /"owner"/.test(e)));
    assert.ok(errors.some((e) => /"expires"/.test(e)));
  });

  it("rejects an expiry window longer than 90 days", () => {
    assert.match(
      errorsFor(entry({ added: "2026-11-01", expires: "2027-02-01" }))[0],
      /more than 90 days/,
    );
  });

  it("rejects invalid dates, ids, ecosystems, and future added dates", () => {
    assert.match(
      errorsFor(entry({ expires: "2026-02-30" }))[0],
      /valid YYYY-MM-DD/,
    );
    assert.match(errorsFor(entry({ id: "SNYK-1" }))[0], /id must be/);
    assert.match(
      errorsFor(entry({ ecosystem: "pip" }))[0],
      /ecosystem must be/,
    );
    assert.match(errorsFor(entry({ added: "2026-11-20" }))[0], /in the future/);
  });

  it("rejects unknown fields, other tables, non-string values, and duplicates", () => {
    assert.match(
      errorsFor(`${entry()}severity = "high"\n`)[0],
      /unknown field "severity"/,
    );
    assert.match(
      errorsFor(`[other]\n${entry()}`)[0],
      /only \[\[exception\]\] tables/,
    );
    assert.match(
      errorsFor(`${entry()}`.replace('owner = "maintainer"', "owner = 1"))[0],
      /unsupported syntax/,
    );
    assert.match(
      errorsFor(entry() + entry()).at(-1),
      /duplicates the exception/,
    );
  });

  it("validates the committed exceptions file", () => {
    const { exceptions, errors } = parseExceptions(
      fs.readFileSync(DEFAULT_EXCEPTIONS_FILE, "utf8"),
    );
    assert.deepEqual(errors, []);
    for (const ex of exceptions) assert.ok(ex.reason.length >= 40, ex.id);
  });
});

const cargoReport = {
  vulnerabilities: {
    found: true,
    count: 1,
    list: [
      {
        advisory: {
          id: "RUSTSEC-2026-0001",
          package: "evil-crate",
          title: "Memory corruption",
          aliases: ["CVE-2026-1111"],
        },
        package: { name: "evil-crate", version: "1.0.0" },
      },
    ],
  },
  warnings: {
    unmaintained: [
      {
        kind: "unmaintained",
        advisory: {
          id: "RUSTSEC-2026-0002",
          package: "old-crate",
          title: "Unmaintained",
        },
        package: { name: "old-crate", version: "0.1.0" },
      },
    ],
  },
};

const npmReport = {
  advisories: {
    1: {
      id: 1,
      github_advisory_id: "GHSA-aaaa-bbbb-cccc",
      module_name: "left-pad",
      severity: "high",
      cves: ["CVE-2026-2222"],
      title: "Prototype pollution",
      findings: [{ version: "1.0.0", paths: [". > left-pad@1.0.0"] }],
    },
    2: {
      id: 2,
      github_advisory_id: "GHSA-dddd-eeee-ffff",
      module_name: "right-pad",
      severity: "moderate",
      cves: [],
      title: "ReDoS",
      findings: [],
    },
  },
};

describe("report normalization", () => {
  it("treats every cargo vulnerability as blocking and warnings as informational", () => {
    const findings = cargoFindings(cargoReport);
    assert.deepEqual(
      findings.map((f) => [f.id, f.blocking]),
      [
        ["RUSTSEC-2026-0001", true],
        ["RUSTSEC-2026-0002", false],
      ],
    );
  });

  it("blocks high and critical npm advisories only", () => {
    const findings = npmFindings(npmReport);
    assert.deepEqual(
      findings.map((f) => [f.id, f.blocking]),
      [
        ["GHSA-aaaa-bbbb-cccc", true],
        ["GHSA-dddd-eeee-ffff", false],
      ],
    );
  });

  it("rejects failed or malformed audit output", () => {
    assert.throws(
      () =>
        npmFindings({
          error: { code: "ERR_PNPM_AUDIT_BAD_RESPONSE", message: "500" },
        }),
      /pnpm audit failed/,
    );
    assert.throws(() => npmFindings({}), /no advisories/);
    assert.throws(() => cargoFindings({}), /no vulnerabilities.list/);
  });
});

describe("policy evaluation", () => {
  const exception = (overrides) => ({
    id: "GHSA-aaaa-bbbb-cccc",
    ecosystem: "npm",
    package: "left-pad",
    reason: REASON,
    owner: "maintainer",
    added: "2026-11-01",
    expires: "2026-12-31",
    line: 1,
    ...overrides,
  });

  it("blocks an advisory without an exception", () => {
    const result = evaluate(npmFindings(npmReport), []);
    assert.deepEqual(
      result.blocking.map((f) => f.id),
      ["GHSA-aaaa-bbbb-cccc"],
    );
  });

  it("accepts a matching exception by id or alias", () => {
    for (const id of ["GHSA-aaaa-bbbb-cccc", "CVE-2026-2222"]) {
      const result = evaluate(npmFindings(npmReport), [exception({ id })]);
      assert.equal(result.blocking.length, 0, id);
      assert.equal(result.excepted.length, 1, id);
    }
  });

  it("does not apply an exception to a different package or ecosystem", () => {
    const findings = npmFindings(npmReport);
    assert.equal(
      evaluate(findings, [exception({ package: "other" })]).blocking.length,
      1,
    );
    assert.equal(
      evaluate(findings, [exception({ ecosystem: "cargo" })]).blocking.length,
      1,
    );
  });

  it("reports exceptions that match nothing as unused", () => {
    const result = evaluate([], [exception()]);
    assert.equal(result.unused.length, 1);
  });
});

describe("CLI", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "walnut-advisories-"));
  const write = (name, content) => {
    const file = path.join(tmp, name);
    fs.writeFileSync(
      file,
      typeof content === "string" ? content : JSON.stringify(content),
    );
    return file;
  };
  const quiet = () => {};
  const npm = write("npm.json", npmReport);
  const cargo = write("cargo.json", {
    vulnerabilities: { list: [] },
    warnings: {},
  });

  it("fails when a blocking advisory has no exception", () => {
    const file = write("none.toml", "# no exceptions\n");
    assert.equal(
      main(
        [
          "evaluate",
          "--npm",
          npm,
          "--cargo",
          cargo,
          "--file",
          file,
          "--today",
          TODAY,
        ],
        quiet,
        quiet,
      ),
      1,
    );
  });

  it("passes when every blocking advisory has a valid exception", () => {
    const file = write("ok.toml", entry());
    assert.equal(
      main(
        [
          "evaluate",
          "--npm",
          npm,
          "--cargo",
          cargo,
          "--file",
          file,
          "--today",
          TODAY,
        ],
        quiet,
        quiet,
      ),
      0,
    );
  });

  it("fails once the exception expires", () => {
    const file = write("ok2.toml", entry());
    assert.equal(
      main(
        ["evaluate", "--npm", npm, "--file", file, "--today", "2027-01-01"],
        quiet,
        quiet,
      ),
      1,
    );
    assert.equal(
      main(["check", "--file", file, "--today", "2027-01-01"], quiet, quiet),
      1,
    );
  });

  it("returns 2 for unreadable reports and bad arguments", () => {
    const file = write("ok3.toml", entry());
    const bad = write("bad.json", "not json");
    assert.equal(
      main(
        ["evaluate", "--npm", bad, "--file", file, "--today", TODAY],
        quiet,
        quiet,
      ),
      2,
    );
    assert.equal(main(["evaluate", "--file", file], quiet, quiet), 2);
    assert.equal(main(["unknown"], quiet, quiet), 2);
  });
});
