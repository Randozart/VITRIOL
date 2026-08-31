// rtk-output — pure entry-side command-output filtering (OmniRoute §4.1,
// REPORT-02 step 8). "Reduce To Knowledge": bash/test/build output is
// transformed AS IT ARRIVES so bloat never enters context. Complements
// tool-result-clearer (eviction after consumption) and read-guard (file reads).
//
// This module is pure (no I/O, no pi imports) so it is trivially unit-testable;
// index.ts wires it to the tool_result hook and writes the raw log.

/** Extension config. Kill switch per Golden Rule 15. */
export interface RtkConfig {
  enabled: boolean;
  /** Outputs shorter than this are never touched (not worth a summary). */
  thresholdChars: number;
  /** Verbatim tail lines kept in the summary. */
  tailLines: number;
  /** Max error-ish lines surfaced. */
  errorCap: number;
  /** Where full raw output is persisted (recoverable by path). */
  rawDir: string;
}

export function rtkConfig(env: NodeJS.ProcessEnv = process.env): RtkConfig {
  const num = (raw: string | undefined, dflt: number): number => {
    if (raw === undefined || raw.trim() === "") return dflt;
    const n = Number(raw);
    return Number.isFinite(n) && n >= 1 ? Math.floor(n) : dflt;
  };
  return {
    enabled: env.TRIS_NO_RTK_OUTPUT !== "1",
    thresholdChars: num(env.TRIS_RTK_THRESHOLD, 800),
    tailLines: num(env.TRIS_RTK_TAIL, 20),
    errorCap: num(env.TRIS_RTK_ERRCAP, 30),
    rawDir: env.TRIS_RTK_DIR || ".pi/rtk",
  };
}

/**
 * Command shapes whose output is summary-shaped noise (test/build/run logs).
 * Everything else (git status, cat, ls, ...) is left alone — the model asked
 * for that text deliberately.
 */
const RTK_PATTERNS = [
  /^npm (test|run |install|ci)/, /^pnpm (test|run |install|add)/, /^yarn (test|run |install|add)/,
  /^pytest\b/, /^python(3)? -m pytest\b/, /^cargo (build|test|check|clippy|run)/,
  /^make\b/, /^\.\//, /^\.\/scripts\//, /^bash /, /^sh /, /^zsh /,
  /^tsc\b/, /^vitest\b/, /^npx (tsc|vitest|jest|playwright test)/,
  /^go (test|build|run)/, /^pytest/, /^ctest/, /^cmake/, /^gcc/, /^clang/,
  /^docker build/, /^pip install/, /^npm i\b/,
  /^(cargo|npm|pnpm|yarn) (bench|benchmark)/,
];

export function isRtkTarget(command: string): boolean {
  const c = command.trim();
  return RTK_PATTERNS.some((p) => p.test(c));
}

const ERROR_LINE = /(error|fail(ed|ure)?|panic|exception|traceback|undefined reference|cannot find|not found|assert|denied|abort)/i;
const WARN_LINE = /(warn(ing)?|deprecated)/i;

export interface RtkSummary {
  exitNote: string;
  counts: { totalLines: number; errorLines: number; warnLines: number };
  errorLines: string[];
  tail: string[];
  /** Estimated chars saved by the summary vs the raw output. */
  savedChars: number;
}

/**
 * Reduce raw command output to: status line + error lines + verbatim tail.
 * Deterministic, no model, ~60-90% reduction on test/build noise.
 */
export function summarize(command: string, output: string, cfg: RtkConfig): RtkSummary {
  const lines = output.split("\n");
  const errorLines = lines.filter((l) => ERROR_LINE.test(l)).slice(0, cfg.errorCap);
  const warnLines = lines.filter((l) => WARN_LINE.test(l)).length;
  const tail = lines.slice(-cfg.tailLines);
  const exitNote = exitCodeNote(command, output);
  const summaryChars = 200 + errorLines.join("\n").length + tail.join("\n").length;
  return {
    exitNote,
    counts: { totalLines: lines.length, errorLines: errorLines.length, warnLines },
    errorLines,
    tail,
    savedChars: Math.max(0, output.length - summaryChars),
  };
}

/** Exit-code line: pi's bash tool embeds it; fall back to isError-driven text. */
export function exitCodeNote(command: string, output: string): string {
  const m = output.match(/(?:Process exited with code|exit(?:ed)? code)[:\s]+(-?\d+)/i);
  const code = m ? m[1] : "unknown";
  const cmd = command.trim().split("\n")[0].slice(0, 80);
  return `$ ${cmd}  → exit ${code}`;
}

/** Render the summary the model sees instead of the raw payload. */
export function renderSummary(s: RtkSummary, rawPath: string): string {
  const parts: string[] = [
    `[rtk: output reduced to knowledge — ${s.counts.totalLines} lines, ${s.counts.errorLines} error line(s)${s.counts.warnLines ? `, ${s.counts.warnLines} warnings` : ""}]`,
    s.exitNote,
  ];
  if (s.errorLines.length) {
    parts.push("— error lines —", ...s.errorLines);
  }
  parts.push("— verbatim tail —", ...s.tail);
  parts.push(`[full output: ${rawPath} — read targeted slices if needed]`);
  return parts.join("\n");
}

/** Raw-output log path for one tool call. */
export function rawPathFor(dir: string, toolCallId: string): string {
  const safe = toolCallId.replace(/[^a-zA-Z0-9_.-]/g, "_");
  return `${dir}/${safe}.log`;
}
