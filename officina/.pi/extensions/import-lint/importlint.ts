// import-lint — cheap, toolchain-free unused-import detection for Python and
// JS/TS. Sits one rung below diagnostics-loop: this works when no linter or
// typechecker is installed, which is exactly the small-model workshop case.
//
// Deliberately a word-boundary usage count, NOT scope analysis — it only
// claims "unused", never "undefined" (that needs a real resolver). Pure.
//
// Provenance: original work, this repo (First-Party Mandate). Plan:
// .opencode/plans/officina-algorithmic-support-2026-08-31.md (P3).

export interface ImportLintConfig {
  enabled: boolean;
  /** Report at most this many names per file. */
  maxNames: number;
}

export function importLintConfig(env: NodeJS.ProcessEnv = process.env): ImportLintConfig {
  return {
    enabled: env.OFFICINA_NO_IMPORT_LINT !== "1",
    maxNames: 6,
  };
}

/** Names imported by one Python file's import lines. Pure. */
export function pyImportedNames(src: string): string[] {
  const names = new Set<string>();
  for (const line of src.split("\n")) {
    const t = line.trim();
    let m = /^import\s+([\w.]+)(\s+as\s+(\w+))?\s*(#.*)?$/.exec(t);
    if (m) {
      names.add(m[3] ?? m[1].split(".")[0]);
      continue;
    }
    m = /^from\s+[\w.]+\s+import\s+(.+?)(\s*#.*)?$/.exec(t);
    if (m) {
      for (const part of m[1].split(",")) {
        const p = part.trim();
        if (!p || p === "(" || p === ")") continue;
        const as = /^(\w+)(\s+as\s+(\w+))?$/.exec(p);
        if (as) names.add(as[3] ?? as[1]);
      }
    }
  }
  return [...names];
}

/** Names imported by one JS/TS file's import declarations. Pure. */
export function jsImportedNames(src: string): string[] {
  const names = new Set<string>();
  const re = /import\s+(.+?)\s+from\s*['"][^'"]+['"]|import\s*['"][^'"]+['"]/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    const clause = m[1];
    if (!clause) continue;
    const def = /^(\w+)(\s*,\s*)?/.exec(clause);
    if (def) names.add(def[1]);
    const braces = /\{([^}]*)\}/.exec(clause);
    if (braces) {
      for (const part of braces[1].split(",")) {
        const as = /^\s*(\w+)(\s+as\s+(\w+))?\s*$/.exec(part);
        if (as) names.add(as[3] ?? as[1]);
      }
    }
    const ns = /\*\s+as\s+(\w+)/.exec(clause);
    if (ns) names.add(ns[1]);
  }
  return [...names];
}

/**
 * Which imported names are never referenced elsewhere in the source?
 * Counts word-boundary occurrences of each name outside import lines
 * (Python) / import statements (JS). Pure.
 */
export function unusedImports(src: string, names: string[], lang: "py" | "js"): string[] {
  const body =
    lang === "py"
      ? src
          .split("\n")
          .filter((l) => !/^\s*(import\s|from\s+[\w.]+\s+import\s)/.test(l))
          .join("\n")
      : src.replace(/import\s+(.+?)\s+from\s*['"][^'"]+['"]|import\s*['"][^'"]+['"]/g, "");
  return names.filter((n) => !new RegExp(`\\b${n}\\b`).test(body));
}

/** Render the injected notice. Empty when clean (zero tokens). Pure. */
export function renderImportNotice(file: string, unused: string[], maxNames: number): string {
  if (unused.length === 0) return "";
  const shown = unused.slice(0, maxNames).join(", ");
  const extra = unused.length > maxNames ? ` (+${unused.length - maxNames} more)` : "";
  return `import-lint: ${file}: unused import(s): ${shown}${extra} — remove them; they are dead weight and mislead readers.`;
}
