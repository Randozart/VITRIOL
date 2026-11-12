// Tests for the review findings contract (codex-qol stage 6, invariant I2/I3).
import { mkdtempSync, readFileSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  parseFindingsBlock,
  summarizeFindings,
  validateFindings,
  writeFindings,
} from "./findings.ts";

const dirs: string[] = [];
const tmpDir = (): string => {
  const dir = mkdtempSync(join(tmpdir(), "review-findings-"));
  dirs.push(dir);
  return dir;
};
afterEach(() => {
  for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
});

const GOOD = JSON.stringify({
  version: 1,
  summary: "The cache layer races on shutdown.",
  findings: [
    {
      severity: "high",
      file: "src/cache.rs",
      line: "120-140",
      title: "unlock skipped on error path",
      detail: "early return leaves the mutex poisoned; use a guard",
    },
    { severity: "info", file: "src/main.rs", title: "typo in log", detail: "says 'recieve'" },
  ],
});

describe("parseFindingsBlock", () => {
  it("extracts the fenced block from surrounding prose", () => {
    const text = `Here is my review.\n\n\`\`\`officina-review\n${GOOD}\n\`\`\`\nDone.`;
    const findings = parseFindingsBlock(text, "uncommitted");
    expect(findings).not.toBeNull();
    expect(findings?.version).toBe(1);
    expect(findings?.preset).toBe("uncommitted");
    expect(findings?.findings).toHaveLength(2);
    expect(findings?.findings[0].severity).toBe("high");
    expect(findings?.findings[0].line).toBe("120-140");
  });

  it("returns null without a fence, with bad JSON, or a wrong version", () => {
    expect(parseFindingsBlock("no block here")).toBeNull();
    expect(parseFindingsBlock("```officina-review\n{not json}\n```")).toBeNull();
    const v2 = JSON.stringify({ ...JSON.parse(GOOD), version: 2 });
    expect(parseFindingsBlock(`\`\`\`officina-review\n${v2}\n\`\`\``)).toBeNull();
  });

  it("rejects findings with unknown severity or empty fields", () => {
    expect(
      validateFindings({
        version: 1,
        summary: "s",
        findings: [{ severity: "catastrophic", file: "a", title: "t", detail: "d" }],
      }),
    ).toBeNull();
    expect(
      validateFindings({ version: 1, summary: "s", findings: [{ severity: "low", file: "", title: "t", detail: "d" }] }),
    ).toBeNull();
  });
});

describe("writeFindings", () => {
  it("writes atomically, 0600, and round-trips", () => {
    const dir = tmpDir();
    const findings = parseFindingsBlock(`\`\`\`officina-review\n${GOOD}\n\`\`\``, "commit")!;
    writeFindings(dir, findings);
    const path = join(dir, "review.json");
    const mode = statSync(path).mode & 0o777;
    expect(mode).toBe(0o600);
    const back = JSON.parse(readFileSync(path, "utf8"));
    expect(back.version).toBe(1);
    expect(back.preset).toBe("commit");
    expect(back.summary).toContain("races");
  });
});

describe("summarizeFindings", () => {
  it("counts by severity", () => {
    const findings = parseFindingsBlock(`\`\`\`officina-review\n${GOOD}\n\`\`\``)!;
    const text = summarizeFindings(findings);
    expect(text).toContain("2 finding(s)");
    expect(text).toContain("1 high");
    expect(text).toContain("1 info");
  });
});
