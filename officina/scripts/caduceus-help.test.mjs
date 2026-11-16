// Help/dispatch drift guard (2026-11-16).
//
// The operator could not find `signup` because the two help screens had drifted
// from the dispatcher. This test parses the dispatch table and the help text out
// of the CLI source, so a command can never ship undocumented (or documented
// after being removed).
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const SOURCE = readFileSync(new URL("./caduceus.mjs", import.meta.url), "utf8");

/** Every `command === "x"` the dispatcher handles. */
function dispatchedCommands() {
  const out = new Set();
  for (const m of SOURCE.matchAll(/command === "([a-z][a-z-]*)"/g))
    out.add(m[1]);
  return out;
}

/** The help text body (inside printHelp's template literal). */
function helpText() {
  const start = SOURCE.indexOf("function printHelp() {");
  expect(start, "printHelp must exist").toBeGreaterThan(-1);
  const end = SOURCE.indexOf("async function main() {", start);
  expect(end, "printHelp must precede main()").toBeGreaterThan(start);
  return SOURCE.slice(start, end);
}

const ALWAYS_RENAMED_AWAY = new Set(["help"]);

describe("caduceus help", () => {
  it("documents every dispatched command", () => {
    const help = helpText();
    const missing = [...dispatchedCommands()]
      .filter((cmd) => !ALWAYS_RENAMED_AWAY.has(cmd))
      .filter((cmd) => !new RegExp(`\\b${cmd}\\b`).test(help));
    expect(missing, "undocumented commands").toEqual([]);
  });

  it("keeps the provider wizard discoverable", () => {
    // The specific regression: `signup` existed but was absent from the help the
    // operator saw.
    const help = helpText();
    expect(help).toMatch(/^\s+signup\s/m);
    expect(help).toMatch(/accounts[\s\S]{0,200}signup/);
  });

  it("is sectioned rather than one flat list", () => {
    const help = helpText();
    for (const section of [
      "SETUP AND LIFECYCLE",
      "PROVIDERS AND ACCOUNTS",
      "ROUTING AND CACHE",
      "SUPERVISION",
      "DIAGNOSTICS",
    ]) {
      expect(help, `missing section ${section}`).toContain(section);
    }
  });

  it("does not leave trailing prose comments in the command column", () => {
    // Old help carried `   # health probe; restarts a wedged gateway` on the
    // same line as a command, which broke the alignment.
    for (const line of helpText().split("\n")) {
      if (!line.trim().startsWith("vitriol") && !/^\s{2}[a-z]/.test(line))
        continue;
      expect(line, `inline comment: ${line}`).not.toMatch(/\s#\s/);
    }
  });
});
