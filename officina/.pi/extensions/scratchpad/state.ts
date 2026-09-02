// scratchpad — project-scoped hot notebook that survives compaction.
//
// The detective-notebook lane: current evidence (numbers, shapes, argv,
// observed behavior), open leads, and recently ruled-out dead ends. Not
// history — anything no longer load-bearing is pruned on the next write.
// Same survival mechanism as task-state: persisted by a tool call, parsed
// from disk and re-injected as a tail message before every LLM call.
//
// Pure module: config/parse/apply/render — no I/O, no pi imports.

export type SectionName = "facts" | "leads" | "dead";

export const SECTION_NAMES: SectionName[] = ["facts", "leads", "dead"];

export interface ScratchpadConfig {
  enabled: boolean;
  /** Total lines across sections; the tool enforces this on write. */
  cap: number;
  /** Directory under cwd holding SCRATCHPAD.md. */
  dir: string;
  /** Max characters per entry line (budget guard). */
  maxLineChars: number;
}

export function scratchpadConfig(env: NodeJS.ProcessEnv = process.env): ScratchpadConfig {
  const capNum = Number(env.OFFICINA_SCRATCHPAD_CAP);
  const cap = Number.isFinite(capNum) && capNum >= 0 ? capNum : 60;
  return {
    enabled: env.OFFICINA_SCRATCHPAD !== "0",
    cap: Math.max(10, Math.floor(cap)),
    dir: env.OFFICINA_SCRATCHPAD_DIR || ".officina",
    maxLineChars: 200,
  };
}

export interface ScratchpadDoc {
  facts: string[];
  leads: string[];
  dead: string[];
}

export function emptyDoc(): ScratchpadDoc {
  return { facts: [], leads: [], dead: [] };
}

export function totalLines(doc: ScratchpadDoc): number {
  return doc.facts.length + doc.leads.length + doc.dead.length;
}

/** Parse persisted markdown back into sections. Unknown sections are dropped. */
export function parseScratchpad(text: string): ScratchpadDoc {
  const doc = emptyDoc();
  let current: SectionName | null = null;
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    const m = /^## (facts|leads|dead)$/.exec(line);
    if (m) {
      current = m[1] as SectionName;
      continue;
    }
    if (!line || line.startsWith("# ")) continue;
    if (current && line.startsWith("- ")) doc[current].push(line.slice(2));
  }
  return doc;
}

export function serializeScratchpad(doc: ScratchpadDoc): string {
  const parts = ["# Scratchpad (hot notebook - prune stale lines)"];
  for (const name of SECTION_NAMES) {
    if (doc[name].length === 0) continue;
    parts.push(`## ${name}`);
    for (const item of doc[name]) parts.push(`- ${item}`);
  }
  return parts.join("\n") + "\n";
}

export interface ScratchpadUpdate {
  facts?: string[];
  leads?: string[];
  dead?: string[];
  reset?: boolean;
}

/**
 * Apply an update to a doc. A section present in the update REPLACES that
 * section wholesale (pass fewer/empty entries to prune); omitted sections
 * are untouched. Returns an error string instead of throwing — the tool
 * result tells the model what to fix.
 */
export function applyUpdate(
  doc: ScratchpadDoc,
  update: ScratchpadUpdate,
  cfg: ScratchpadConfig,
): { doc?: ScratchpadDoc; error?: string } {
  const next: ScratchpadDoc = update.reset
    ? emptyDoc()
    : { facts: [...doc.facts], leads: [...doc.leads], dead: [...doc.dead] };
  for (const name of SECTION_NAMES) {
    const v = (update as Record<string, unknown>)[name];
    if (v === undefined) continue;
    if (!Array.isArray(v)) return { error: `${name} must be an array of strings` };
    next[name] = []; // replacement semantics: the named section is rewritten wholesale
    for (let i = 0; i < v.length; i++) {
      const s = String(v[i] ?? "").trim();
      if (!s) return { error: `${name}[${i}]: empty entry (omit the line instead)` };
      if (s.length > cfg.maxLineChars) {
        return { error: `${name}[${i}]: ${s.length} chars exceeds ${cfg.maxLineChars} - split or shorten` };
      }
      next[name].push(s);
    }
  }
  const total = totalLines(next);
  if (total > cfg.cap) {
    return {
      error: `cap exceeded: ${total} lines > ${cfg.cap}. Prune: rewrite sections without stale entries, then re-add the essential ones.`,
    };
  }
  return { doc: next };
}

/**
 * Render the injected tail block. Empty notebook renders "" (injection is
 * skipped by lastCopyIsCurrent / the context handler).
 */
export function renderScratchpadBlock(doc: ScratchpadDoc, cap: number): string {
  const total = totalLines(doc);
  if (total === 0) return "";
  const parts = [
    "## Scratchpad (external truth - survives compaction; update with scratchpad_write; PRUNE what is no longer relevant)",
    `[${total}/${cap} lines]`,
  ];
  for (const name of SECTION_NAMES) {
    if (doc[name].length === 0) continue;
    parts.push(`### ${name}`);
    for (const item of doc[name]) parts.push(`- ${item}`);
  }
  return `\n\n${parts.join("\n")}`;
}
