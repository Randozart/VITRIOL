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

/**
 * Resolves the executable to run for `file` (`cmd` is the formatter name).
 * Returns null when unavailable. Injected so tests need no real binaries.
 * (2026-11-14: project-local `node_modules/.bin` discovery lives in the
 * extension; this module stays pure.)
 */
export type Resolver = (file: string, cmd: string) => string | null;

/** Rusfmt edition from the nearest Cargo.toml, default 2021. Pure-ish (reads
 *  through the injected reader so tests are deterministic). */
export type ReadText = (path: string) => string | null;

function nearestEdition(file: string, readText: ReadText | undefined, dirnameOf: (p: string) => string): string {
  if (!readText) return "2021";
  let dir = dirnameOf(file);
  for (let i = 0; i < 8; i++) {
    const cargo = `${dir}/Cargo.toml`;
    const text = readText(cargo);
    if (text) {
      const m = /^\s*edition\s*=\s*"([0-9]{4})"/m.exec(text);
      if (m) return m[1]!;
      return "2021";
    }
    const parent = dirnameOf(dir);
    if (!parent || parent === dir) break;
    dir = parent;
  }
  return "2021";
}

/**
 * Path-aware formatter selection (2026-11-14): same policy as `formatterFor`,
 * but the resolved executable is injected per file so project-local binaries
 * (`node_modules/.bin/prettier`) and the local Rust edition work.
 */
export function formatterForPath(
  file: string,
  resolve: Resolver,
  opts: { readText?: ReadText; dirname?: (p: string) => string } = {},
): FormatterCommand | null {
  const dirname = opts.dirname ?? ((p: string) => { const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\")); return i <= 0 ? "." : p.slice(0, i); });
  const pick = (cmd: string, argv: (exe: string) => string[], label: string): FormatterCommand | null => {
    const exe = resolve(file, cmd);
    return exe ? { argv: argv(exe), label } : null;
  };
  if (file.endsWith(".py")) {
    return pick("ruff", (e) => [e, "format", file], "ruff format")
      ?? pick("black", (e) => [e, "--quiet", file], "black");
  }
  if (PRETTIER_EXTS.test(file)) {
    return pick("prettier", (e) => [e, "--write", "--log-level", "silent", file], "prettier");
  }
  if (file.endsWith(".go")) {
    return pick("gofmt", (e) => [e, "-w", file], "gofmt");
  }
  if (file.endsWith(".rs")) {
    const edition = nearestEdition(file, opts.readText, dirname);
    return pick("rustfmt", (e) => [e, "--edition", edition, file], "rustfmt");
  }
  if (/\.(c|h|cc|cpp|hpp)$/.test(file)) {
    return pick("clang-format", (e) => [e, "-i", file], "clang-format");
  }
  return null;
}

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
