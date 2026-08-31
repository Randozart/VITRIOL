// format-gate — formatter selection and config. Pure module: given a file
// name and an availability predicate, pick the project-canonical formatter.
// Execution lives in index.ts; this file is unit-testable without any
// formatter installed.
//
// Provenance: original work, this repo (First-Party Mandate). Plan:
// .opencode/plans/officina-algorithmic-support-2026-08-31.md (P1).

export interface FormatConfig {
  enabled: boolean;
  timeoutMs: number;
}

export function formatConfig(env: NodeJS.ProcessEnv = process.env): FormatConfig {
  return {
    enabled: env.OFFICINA_NO_FORMAT !== "1",
    timeoutMs: 30_000,
  };
}

/** One formatter invocation for one file. */
export interface FormatterCommand {
  argv: string[];
  /** Human label used in the injected notice ("prettier", "gofmt", …). */
  label: string;
}

/** Resolves a bare command name to true when it exists on PATH. Injected. */
export type Availability = (cmd: string) => boolean;

const PRETTIER_EXTS = /\.(m|c)?[jt]sx?$|\.jsonc?$|\.css$|\.scss$|\.html?$/;

/**
 * Pick the canonical formatter for a file, or null when none applies (or
 * none is installed — the gate must stay silent rather than error). Pure.
 */
export function formatterFor(file: string, available: Availability): FormatterCommand | null {
  if (file.endsWith(".py")) {
    if (available("ruff")) return { argv: ["ruff", "format", file], label: "ruff format" };
    if (available("black")) return { argv: ["black", "--quiet", file], label: "black" };
    return null;
  }
  if (PRETTIER_EXTS.test(file)) {
    if (available("prettier")) return { argv: ["prettier", "--write", "--log-level", "silent", file], label: "prettier" };
    return null;
  }
  if (file.endsWith(".go")) {
    if (available("gofmt")) return { argv: ["gofmt", "-w", file], label: "gofmt" };
    return null;
  }
  if (file.endsWith(".rs")) {
    if (available("rustfmt")) return { argv: ["rustfmt", "--edition", "2021", file], label: "rustfmt" };
    return null;
  }
  if (/\.(c|h|cc|cpp|hpp)$/.test(file)) {
    if (available("clang-format")) return { argv: ["clang-format", "-i", file], label: "clang-format" };
    return null;
  }
  return null;
}

/** Render the injected reformat notice. Empty when nothing changed. Pure. */
export function renderFormatNotice(reformatted: Array<{ file: string; label: string }>, budgetChars = 600): string {
  if (reformatted.length === 0) return "";
  const names = [...new Set(reformatted.map((r) => r.label))].join(", ");
  let out = `[format-gate: ${reformatted.length} file(s) were reformatted by ${names} after your edit. The ON-DISK content is canonical — your in-context copy may be stale. Do not re-edit for style.]`;
  for (const r of reformatted) {
    const line = `\n  ${r.file}`;
    if (out.length + line.length > budgetChars) {
      out += "\n  …";
      break;
    }
    out += line;
  }
  return out;
}
