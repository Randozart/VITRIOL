// PROVENANCE: original — VITRIOL officina (2026-09-27, parity plan phase 1.2,
// .opencode/plans/officina-parity-2026-09-27.md). No third-party code.
//
// at-mention — officina's autocomplete provider. pi 0.83.0 ships @file fuzzy
// completion (CombinedAutocompleteProvider, fd-backed) and the
// ctx.ui.addAutocompleteProvider wrap point; officina never wired either past
// the stock file completion. This provider WRAPS the current one and merges
// officina-owned targets into the @-space:
//   - skill / knowledge / protocol cards (officina/skills/**, absolute paths —
//     readable from any project cwd)
//   - .officina/ artifacts (SCRATCHPAD.md, MEMORY.md, approved-plan.md,
//     tasks/*.json — relative paths, project-local)
// File completions still come from the stock provider; items are appended,
// never replaced. TUI only (print/json mode has no editor).

import { readdirSync, existsSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import type { AutocompleteItem, AutocompleteProvider } from "@earendil-works/pi-tui";
import { atTokenAt, mergeSuggestions, type CatalogEntry } from "./suggest.ts";

const here = dirname(fileURLToPath(import.meta.url));
const scaffoldRoot = resolve(here, "..", "..", "..");
const skillsDir = join(scaffoldRoot, "skills");

const SKILLS_TTL_MS = 60_000;
const ARTIFACTS_TTL_MS = 5_000;

interface Catalog {
  entries: CatalogEntry[];
  skillsAt: number;
  artifactsAt: number;
}

const catalog: Catalog = { entries: [], skillsAt: 0, artifactsAt: 0 };

/** All .md files under a dir (non-recursive walks per known subdir; skills
 *  tree is shallow: tools/knowledge/protocols/<file>.md). */
function listMarkdown(dir: string): string[] {
  const out: string[] = [];
  if (!existsSync(dir)) return out;
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) out.push(...listMarkdown(p));
    else if (e.name.endsWith(".md")) out.push(p);
  }
  return out;
}

function kindLabel(p: string): string {
  if (p.includes(`${join("skills", "tools")}`)) return "tool card";
  if (p.includes(`${join("skills", "knowledge")}`)) return "knowledge card";
  if (p.includes(`${join("skills", "protocols")}`)) return "protocol";
  const base = p.endsWith(".md") ? p.slice(0, -3) : p;
  if (base.endsWith("SCRATCHPAD")) return "scratchpad";
  if (base.endsWith("MEMORY")) return "memory";
  if (base.endsWith("approved-plan")) return "approved plan";
  if (p.includes(join(".officina", "tasks"))) return "task list";
  return "artifact";
}

function refreshSkills(now: number): void {
  if (now - catalog.skillsAt < SKILLS_TTL_MS) return;
  catalog.skillsAt = now;
  catalog.entries = catalog.entries.filter((e) => !e.id.startsWith("skill:"));
  for (const p of listMarkdown(skillsDir)) {
    catalog.entries.push({
      id: `skill:${p}`,
      value: p,
      label: p.slice(p.lastIndexOf("/") + 1).replace(/\.md$/, ""),
      description: kindLabel(p),
      dir: false,
    });
  }
}

function refreshArtifacts(now: number, cwd: string): void {
  if (now - catalog.artifactsAt < ARTIFACTS_TTL_MS) return;
  catalog.artifactsAt = now;
  catalog.entries = catalog.entries.filter((e) => !e.id.startsWith("artifact:"));
  const off = join(cwd, ".officina");
  const push = (p: string) => {
    try {
      if (!existsSync(p) || !statSync(p).isFile()) return;
    } catch {
      return;
    }
    catalog.entries.push({
      id: `artifact:${p}`,
      value: p.startsWith(cwd + "/") ? p.slice(cwd.length + 1) : p,
      label: p.slice(p.lastIndexOf("/") + 1),
      description: kindLabel(p),
      dir: false,
    });
  };
  for (const name of ["SCRATCHPAD.md", "MEMORY.md", "approved-plan.md"]) push(join(off, name));
  const tasksDir = join(off, "tasks");
  if (existsSync(tasksDir)) {
    try {
      for (const e of readdirSync(tasksDir)) push(join(tasksDir, e));
    } catch {
      /* unreadable tasks dir — skip */
    }
  }
}

export default async function (pi: ExtensionAPI) {
  pi.on("session_start", (_event, ctx) => {
    if (!ctx.hasUI) return;
    ctx.ui.addAutocompleteProvider?.((current: AutocompleteProvider): AutocompleteProvider => {
      const cwd = process.cwd();
      return {
        triggerCharacters: current.triggerCharacters,
        shouldTriggerFileCompletion: current.shouldTriggerFileCompletion?.bind(current),
        async getSuggestions(lines, cursorLine, cursorCol, options) {
          const base = await current.getSuggestions(lines, cursorLine, cursorCol, options);
          const now = Date.now();
          refreshSkills(now);
          refreshArtifacts(now, cwd);
          return mergeSuggestions(base, catalog.entries, lines, cursorLine, cursorCol, atTokenAt);
        },
        applyCompletion(lines, cursorLine, cursorCol, item: AutocompleteItem, prefix) {
          if ((item as AutocompleteItem & { officina?: boolean }).officina) {
            const token = atTokenAt(lines[cursorLine] ?? "", cursorCol);
            const cut = token ? token.raw.length : prefix.length;
            const line = lines[cursorLine] ?? "";
            const before = line.slice(0, cursorCol - cut);
            const after = line.slice(cursorCol);
            const suffix = item.label.endsWith("/") ? "" : " ";
            const next = [...lines];
            next[cursorLine] = before + item.value + suffix + after;
            return {
              lines: next,
              cursorLine,
              cursorCol: before.length + item.value.length + suffix.length,
            };
          }
          return current.applyCompletion(lines, cursorLine, cursorCol, item, prefix);
        },
      };
    });
  });
}
