// diagnostics-loop — post-edit auto-checks injected back to the model
// (OpenCode §3.3 / REPORT-02 step 11). Turns the quality gate from "check"
// into "check → auto-repair → re-check": the model repairs its own edit
// without a human round-trip.
// Pure module: command selection + result shaping; execution is injected.

export interface DiagConfig {
  enabled: boolean;
  /** ~300-token injection budget (§3.3): ~1,050 chars at 3.5 chars/token. */
  budgetChars: number;
  /** Praetor pass (needs a DIRECTORY target — a file target silently passes). */
  praetor: boolean;
  praetorTimeoutMs: number;
}

export function diagConfig(env: NodeJS.ProcessEnv = process.env): DiagConfig {
  return {
    enabled: env.TRIS_NO_DIAGNOSTICS !== "1",
    budgetChars: 1050,
    praetor: env.TRIS_DIAG_PRAETOR === "1",
    praetorTimeoutMs: 60_000,
  };
}

/** A checker command for one edited file. */
export interface CheckCommand {
  argv: string[];
}

/**
 * Pick fast, file-local syntax checks by extension. Deliberately cheap:
 * this runs on EVERY edit; anything slower belongs in a pre-commit gate.
 * Returns null for files we do not check. Pure.
 */
export function checkFor(file: string, cfg: DiagConfig = diagConfig({})): CheckCommand | null {
  if (file.endsWith(".py")) return { argv: ["python3", "-m", "py_compile", file] };
  if (file.endsWith(".sh") || file.endsWith(".bash")) return { argv: ["bash", "-n", file] };
  if (/\.(m|c)?js$/.test(file)) return { argv: ["node", "--check", file] };
  if (file.endsWith(".json")) return { argv: ["node", "-e", `JSON.parse(require('fs').readFileSync(${JSON.stringify(file)},'utf8'))`] };
  if (cfg.praetor && /\.(ts|tsx|js|py|rs|c|h|go|rb)$/.test(file)) {
    return { argv: ["praetor-diag", file] }; // sentinel — runner copies to tmp dir
  }
  return null;
}

/** One file's check outcome. */
export interface FileDiag {
  file: string;
  ok: boolean;
  /** stderr/stdout from the checker, already tail-capped. */
  output: string;
}

/**
 * Render the injected diagnostics block. Only failures are reported
 * (clean checks stay silent — zero tokens). Capped to budgetChars.
 */
export function renderDiagnostics(diags: FileDiag[], budgetChars: number): string {
  const bad = diags.filter((d) => !d.ok);
  if (bad.length === 0) return "";
  const head = `[diagnostics: ${bad.length} file(s) failed their syntax check after your edit — fix before continuing]`;
  let out = head;
  for (const d of bad) {
    const block = `\n--- ${d.file} ---\n${d.output}`;
    if (out.length + block.length > budgetChars) {
      out += `\n… [truncated to budget; re-run the check on ${d.file} for the rest]`;
      break;
    }
    out += block;
  }
  return out;
}

/** Tail-cap a checker's output — errors usually live at the end. Pure. */
export function tailCap(text: string, maxChars = 400): string {
  const t = text.trim();
  if (t.length <= maxChars) return t;
  return "…" + t.slice(t.length - maxChars);
}
