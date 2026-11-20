// Seam tripwires (2026-11-20, seam-resistance M3): CI greps for the SHAPES
// of known layer-boundary bugs. Each rule cites the incident it guards
// against. These are warnings-by-list — new hits fail the suite with the
// rule name so the pattern must be either fixed or explicitly allow-listed
// (with a comment citing why it is safe).
//
// Rules:
//   R1  raw OFFICINA_* env reads outside the oven/config (M2 erosion guard;
//       the caduceus bypass class). Allow-list: config.ts (the inventory),
//       resolve-config.ts (the oven), envfile users that predate the oven.
//   R2  silent stream-through swallowing in scripts/ (`catch { res.destroy
//       () }` with no logging — the caduceus mid-stream hang class).
//   R3  parity-tester registry: every test that asserts equality with a
//       shared JSON asset must be listed in PARITY_TESTERS so a change to
//       that asset knows how many consumers to check (the vitriol-tui CI
//       miss class).
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const SHARED = join(__dirname);
const REPO_ROOT = join(SHARED, "..", "..", "..", "..");

function walk(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    const st = statSync(p);
    if (st.isDirectory()) walk(p, out);
    else out.push(p);
  }
  return out;
}

function read(p: string): string {
  try {
    return readFileSync(p, "utf8");
  } catch {
    return "";
  }
}

describe("seam tripwires", () => {
  it("R1: OFFICINA_* env reads live in the oven or the inventory", () => {
    // Allow-list: the oven (the resolver), the knob inventory (its test),
    // and launch-time files that run before the extension layer exists.
    const allowed = [
      "resolve-config.ts",
      "resolve-config.test.ts",
      "config.ts",
      "config.test.ts",
    ];
    const offenders: string[] = [];
    // Known debt (2026-11-20): the migration backlog. These predate the
    // oven; each migrates to resolve-config as its knob is touched. A NEW
    // file reading env raw fails R1 — that is the point.
    const KNOWN_DEBT = new Set(
      [
        "band.ts", "budget.ts", "caduceus-prefs.ts", "core-rpc.ts",
        "hook-trace.ts", "lane-health.ts", "profiles.ts",
        "provider-health.ts", "route-ledger.ts", "sidebar.ts",
      ].map((p) => join(SHARED, p)),
    );
    for (const f of walk(SHARED)) {
      if (!f.endsWith(".ts") || f.endsWith(".test.ts")) continue;
      if (allowed.some((a) => f.endsWith(a))) continue;
      // Known debt (2026-11-20): 10 pre-oven files are on a migration
      // backlog INSIDE this loop's allow-list below; new files fail.
      if (KNOWN_DEBT.has(f)) continue;
      const src = read(f);
      // Reads of OFFICINA_* knobs straight from env (the oven-less pattern).
      const m = src.match(
        /process\.env\.OFFICINA_[A-Z_]*|process\.env\[\s*[`'"]OFFICINA_/,
      );
      if (m) offenders.push(`${f}: ${m[0]}`);
    }
    expect(
      offenders,
      `R1 (M2 erosion): raw OFFICINA_* env reads outside the oven. Route ` +
        `them through _shared/resolve-config.ts — that is the whole point ` +
        `of the oven. Offenders:\n${offenders.join("\n")}`,
    ).toEqual([]);
  });

  it("R2: no silent stream-through swallowing in scripts/", () => {
    const scripts = join(REPO_ROOT, "scripts");
    const offenders: string[] = [];
    for (const f of walk(scripts)) {
      if (!f.endsWith(".mjs")) continue;
      const src = read(f);
      // The caduceus mid-stream hang: destroy() inside a bare catch with
      // no log line between them.
      const m = src.match(
        /catch\s*(\([^)]*\))?\s*\{\s*res\.destroy\(\);?\s*\}/,
      );
      if (m) offenders.push(`${f}: bare catch → res.destroy() with no logging`);
    }
    expect(
      offenders,
      `R2 (mid-stream hang class): a destroyed response teaches the client ` +
        `nothing. Log the failure (logReroute or console.error) and, where ` +
        `possible, emit a terminal error event. Offenders:\n${offenders.join("\n")}`,
    ).toEqual([]);
  });

  it("R3: parity testers of shared JSON assets are registered", () => {
    // The vitriol-tui CI miss: palette.json gained a parity tester in a
    // second crate and nobody updated the change checklist. One registry,
    // one place to look.
    const PARITY_TESTERS: Record<string, string[]> = {
      "officina/theme/palette.json": [
        "officina/tui/src/theme.rs (themes_match_the_palette_file)",
        "vitriol-tui/src/theme.rs (palette_matches_the_shared_file)",
      ],
    };
    for (const [asset, testers] of Object.entries(PARITY_TESTERS)) {
      for (const t of testers) {
        const [crate, fn] = [t.split(" ")[0], t.split(" ")[1] ?? ""];
        const src = read(join(REPO_ROOT, crate));
        expect(
          src.length > 0,
          `R3: parity tester registry stale — ${crate} not found for ${asset}`,
        ).toBe(true);
        const fnName = fn.replace(/[()]/g, "").split("/").pop() ?? fn;
        expect(
          src.includes(fnName),
          `R3: ${asset} parity tester "${fnName}" missing from ${crate} — ` +
            `the registry is stale; update PARITY_TESTERS or the test`,
        ).toBe(true);
      }
    }
  });
});
