// code-tool report + diff (2026-11-14). Pure: no I/O, unit-testable.

export interface FormatOutcome {
  file: string;
  tool: string;
  changed: boolean;
  diff?: string;
  error?: string;
}

export interface SyntaxOutcome {
  file: string;
  ok: boolean;
  output?: string;
}

export interface GateOutcomeView {
  id: string;
  label: string;
  ok: boolean;
  message?: string;
}

export interface PraetorOutcome {
  file: string;
  line: number;
  message: string;
}

export interface CodeReport {
  formatted: FormatOutcome[];
  syntax: SyntaxOutcome[];
  gates: { ran: boolean; ok: boolean; outcomes: GateOutcomeView[] };
  praetor: PraetorOutcome[];
  budgetBytes: number;
}

/** Render the compact report the model sees (and the TUI colors by glyph). */
export function renderReport(r: CodeReport): string {
  const lines: string[] = [];
  const changed = r.formatted.filter((f) => f.changed).length;
  if (r.formatted.length > 0) {
    lines.push(
      `code: ${r.formatted.length} file(s) formatted — ${changed} changed`,
    );
    for (const f of r.formatted) {
      if (f.error) {
        lines.push(`  ✗ ${f.file} — ${f.tool}: ${f.error}`);
      } else {
        lines.push(`  ${f.changed ? "✓" : "·"} ${f.tool} ${f.file}${f.changed ? " (reformatted)" : ""}`);
      }
      if (f.changed && f.diff) {
        for (const dl of f.diff.split("\n").slice(0, 40)) lines.push(`    ${dl}`);
      }
    }
  } else {
    lines.push("code: no formatter applies to the given files");
  }

  if (r.syntax.length > 0) {
    for (const s of r.syntax) {
      if (s.ok) {
        lines.push(`  ✓ syntax ${s.file}`);
      } else {
        lines.push(`  ✗ syntax ${s.file}`);
        for (const l of String(s.output ?? "").split("\n").slice(0, 8)) lines.push(`    ${l}`);
      }
    }
  }

  if (r.gates.ran) {
    if (r.gates.ok) {
      lines.push(`  ✓ gates (${r.gates.outcomes.length}) green`);
    } else {
      const bad = r.gates.outcomes.filter((g) => !g.ok);
      lines.push(`  ✗ ${bad.length} gate(s) failing`);
      for (const g of bad.slice(0, 6)) {
        lines.push(`    ✗ ${g.id}: ${String(g.message ?? "").slice(0, 200)}`);
      }
    }
  } else {
    lines.push("  · no project gates detected");
  }

  if (r.praetor.length > 0) {
    lines.push(`  ⚠ praetor: ${r.praetor.length} advisory diagnostic(s)`);
    for (const p of r.praetor.slice(0, 5)) {
      lines.push(`    ⚠ ${p.file}:${p.line} ${p.message.slice(0, 160)}`);
    }
  }

  let text = lines.join("\n");
  if (text.length > r.budgetBytes) {
    text = `${text.slice(0, Math.max(0, r.budgetBytes - 40))}\n… [report truncated; request a single path for the rest]`;
  }
  return text;
}

/** Line-based unified diff (LCS), bounded. Empty string when identical. */
export function unifiedDiff(
  oldText: string,
  newText: string,
  path: string,
  opts: { context?: number; maxLines?: number } = {},
): string {
  if (oldText === newText) return "";
  const context = opts.context ?? 2;
  const maxLines = opts.maxLines ?? 120;
  const a = oldText.split("\n");
  const b = newText.split("\n");
  if (a.length > 1200 || b.length > 1200) {
    return `--- ${path}\n+++ ${path}\n(file too large to diff: ${a.length} → ${b.length} lines)`;
  }
  // LCS table (n*m; bounded by the guard above).
  const n = a.length;
  const m = b.length;
  const dp: Uint32Array[] = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i]![j] = a[i] === b[j] ? dp[i + 1]![j + 1]! + 1 : Math.max(dp[i + 1]![j]!, dp[i]![j + 1]!);
    }
  }
  type Row = { kind: " " | "-" | "+"; text: string };
  const rows: Row[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      rows.push({ kind: " ", text: a[i]! });
      i++;
      j++;
    } else if (dp[i + 1]![j]! >= dp[i]![j + 1]!) {
      rows.push({ kind: "-", text: a[i]! });
      i++;
    } else {
      rows.push({ kind: "+", text: b[j]! });
      j++;
    }
  }
  while (i < n) rows.push({ kind: "-", text: a[i++]! });
  while (j < m) rows.push({ kind: "+", text: b[j++]! });

  // Keep only hunks with ±context of a change.
  const keep = new Array(rows.length).fill(false);
  rows.forEach((row, idx) => {
    if (row.kind === " ") return;
    for (let k = Math.max(0, idx - context); k <= Math.min(rows.length - 1, idx + context); k++) {
      keep[k] = true;
    }
  });
  const out: string[] = [`--- ${path}`, `+++ ${path}`];
  let pendingGap = false;
  for (let idx = 0; idx < rows.length && out.length < maxLines; idx++) {
    if (!keep[idx]) {
      pendingGap = true;
      continue;
    }
    if (pendingGap && out.length > 2) out.push("@@");
    pendingGap = false;
    out.push(`${rows[idx]!.kind}${rows[idx]!.text}`);
  }
  if (out.length >= maxLines) out.push("… [diff truncated]");
  return out.length > 2 ? out.join("\n") : "";
}
