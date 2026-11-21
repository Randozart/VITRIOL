// perms-report — read-only renderer for the permission surfaces (2026-11-21,
// seam plan permission-visibility-2026-11-21.md). Owner report: rules lived
// in three places with no viewer.
//
// Surfaces, in report order:
//   1. active mode + where it came from (mode file / env / default)
//   2. global path rules        ~/.config/trismegistus/permissions.json
//      (GENERATED mirror — patterns shown verbatim; I4 forbids truncation,
//      patterns ARE the safety information)
//   3. project allowlist        ~/.vitriol/officina/perms/<slug>-<hash8>.json
//      (command/program rules the operator approved via the dialog)
//   4. protected roots          hardcoded in policy.ts — always ask, cannot be
//      bypassed by any rule or mode
//
// I5: read-only. Nothing in this module writes any store.
import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

export interface ReportLine {
  text: string;
}

/** Load + parse the global mirror; null when missing/corrupt (fail-soft). */
export function loadGlobalRules(
  path = join(homedir(), ".config", "trismegistus", "permissions.json"),
): Array<{ tool: string; pattern: string; action: string }> | null {
  try {
    const doc = JSON.parse(readFileSync(path, "utf8")) as {
      rules?: Array<{ tool?: string; pattern?: string; action?: string }>;
    };
    if (!Array.isArray(doc.rules)) return null;
    return doc.rules.map((r) => ({
      tool: String(r.tool ?? "?"),
      pattern: String(r.pattern ?? "?"),
      action: String(r.action ?? "?"),
    }));
  } catch {
    return null;
  }
}

/** Load a project allowlist; null when missing/corrupt. */
export function loadProjectRules(
  path: string,
): Array<{ tool: string; kind: string; pattern: string }> | null {
  try {
    const doc = JSON.parse(readFileSync(path, "utf8")) as {
      rules?: Array<{ tool?: string; kind?: string; pattern?: string }>;
    };
    if (!Array.isArray(doc.rules)) return null;
    return doc.rules.map((r) => ({
      tool: String(r.tool ?? "?"),
      kind: String(r.kind ?? "?"),
      pattern: String(r.pattern ?? "?"),
    }));
  } catch {
    return null;
  }
}

/** Active mode + its source, mirroring permissions-guard's precedence. */
export function resolveMode(env: NodeJS.ProcessEnv = process.env): {
  mode: string;
  source: string;
} {
  try {
    const modeFile = join(
      homedir(),
      ".vitriol",
      "officina",
      "permissions-mode",
    );
    const fromFile = readFileSync(modeFile, "utf8").trim();
    if (fromFile) return { mode: fromFile, source: modeFile };
  } catch {
    /* no file */
  }
  const fromEnv = env.OFFICINA_PERMISSIONS_MODE?.trim();
  if (fromEnv) return { mode: fromEnv, source: "OFFICINA_PERMISSIONS_MODE" };
  return { mode: "ask", source: "default" };
}

/**
 * The full report, one string per line. Pure: takes the pieces as arguments
 * so tests inject fixtures; the command wrapper resolves the real paths.
 */
export function renderReport(input: {
  mode: string;
  modeSource: string;
  globalRules: Array<{ tool: string; pattern: string; action: string }> | null;
  globalPath: string;
  projectRules: Array<{ tool: string; kind: string; pattern: string }> | null;
  projectPath: string;
  protectedRoots: readonly string[];
}): string[] {
  const out: string[] = [];
  out.push(`mode: ${input.mode}  (source: ${input.modeSource})`);
  out.push("");
  out.push(`global path rules — ${input.globalPath}`);
  if (!input.globalRules) {
    out.push("  (missing or unreadable — mode policy only)");
  } else if (input.globalRules.length === 0) {
    out.push("  (none)");
  } else {
    for (const r of input.globalRules) {
      out.push(`  ${r.action.padEnd(6)} ${r.tool.padEnd(6)} ${r.pattern}`);
    }
  }
  out.push("");
  out.push(`project allowlist — ${input.projectPath}`);
  if (!input.projectRules) {
    out.push("  (none for this project)");
  } else if (input.projectRules.length === 0) {
    out.push("  (empty)");
  } else {
    for (const r of input.projectRules) {
      out.push(`  ${r.tool.padEnd(6)} ${r.kind.padEnd(8)} ${r.pattern}`);
    }
  }
  out.push("");
  out.push("protected roots — always ask; no rule, mode, or YOLO bypasses:");
  for (const p of input.protectedRoots) {
    out.push(`  ${p}`);
  }
  return out;
}
