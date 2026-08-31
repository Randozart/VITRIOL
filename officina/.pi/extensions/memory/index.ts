import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "@sinclair/typebox";
import { existsSync, readdirSync, mkdirSync, appendFileSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { injectionResult } from "../_shared/inject.ts";

// memory — the Officina-native persistent memory store (2026-08-31).
//
// SS2 gateway fold-in (docs/SELF-SUFFICIENCY-2026-08-31.md): the workshop
// stops renting its memory from hermes-agent. This extension OWNS memory:
//
//   project store  <cwd>/.officina/MEMORY.md — project facts, decisions,
//                  conventions (the goal is programming: memory belongs to
//                  the project, versionable, reviewable in diffs)
//   global store   ~/.vitriol/officina/memory/USER.md — owner facts that
//                  travel across projects (preferences, hardware, taste)
//   tools     memory_read / memory_write / memory_search (project store is
//             the default target; file:"USER.md" targets the global store)
//
// memory_search scans OFFICINA'S OWN past session files (JSONL under the
// live session dir reported by ctx.sessionManager) — case-insensitive
// substring scan, capped — so cross-session recall needs no external
// database. Successor to the hermes-bridge FTS contract (trismegistus
// REPORT-02 step 16) and the hermes memory-extractor concept; provenance:
// trismegistus hermes-plugins @ 237e424 (owner-authored, MIT).
//
// Kill switch: OFFICINA_MEMORY=0. Injection cap: MEMORY_INJECT_LINES (50).

const MEM_DIR_DEFAULT = ".vitriol/officina/memory";
const INJECT_LINES = Number(process.env.MEMORY_INJECT_LINES) || 50;
const SEARCH_MAX_HITS = 8;
const SEARCH_SNIPPET = 240;

function globalDir(env: NodeJS.ProcessEnv = process.env): string {
  if (env.OFFICINA_MEMORY_DIR) return env.OFFICINA_MEMORY_DIR;
  return join(env.HOME || homedir(), MEM_DIR_DEFAULT);
}

function projectDir(cwd: string): string {
  return join(cwd, ".officina");
}

function headLines(text: string, n: number): string {
  return text.split("\n").slice(0, n).join("\n");
}

function readFacts(dirs: string[]): string {
  const parts: string[] = [];
  for (const dir of dirs) {
    if (!dir || !existsSync(dir)) continue;
    const label = dir.includes(".officina") ? "project" : "global";
    const p = join(dir, "MEMORY.md");
    if (!existsSync(p)) continue;
    try {
      parts.push(`### MEMORY.md (${label})
${headLines(readFileSync(p, "utf8"), INJECT_LINES)}`);
    } catch {
      // unreadable store is absence of knowledge, not an error
    }
  }
  const ug = join(globalDir(), "USER.md");
  if (existsSync(ug)) {
    try {
      parts.push(`### USER.md (owner)
${headLines(readFileSync(ug, "utf8"), INJECT_LINES)}`);
    } catch {
      // as above
    }
  }
  return parts.join("\n\n");
}

function listDir(d: string): import("node:fs").Dirent[] {
  try {
    return readdirSync(d, { withFileTypes: true });
  } catch {
    return [];
  }
}

function walkJsonl(out: string[], d: string, left: number): void {
  for (const e of listDir(d)) {
    const p = join(d, e.name);
    if (e.isDirectory()) {
      if (left > 0) walkJsonl(out, p, left - 1);
      continue;
    }
    if (e.isFile() && e.name.endsWith(".jsonl")) out.push(p);
  }
}

function sessionJsonlFiles(root: string, depth: number): string[] {
  const out: string[] = [];
  walkJsonl(out, root, Math.max(1, depth));
  return out;
}

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_MEMORY === "0") return; // Rule 15

  const globalMemoryDir = globalDir();
  let projectMemoryDir = "";
  let sessionDir: string | null = null;

  pi.registerTool({
    name: "memory_read",
    label: "Memory read",
    description:
      "Read persistent memory (MEMORY.md facts and USER.md owner facts). Use when past decisions, preferences, or project context might matter.",
    parameters: Type.Object({}),
    async execute() {
      const text = readFacts([projectMemoryDir, globalMemoryDir]);
      return { content: [{ type: "text", text: text || "memory: empty (nothing stored yet)." }], details: undefined as never };
    },
  });

  pi.registerTool({
    name: "memory_write",
    label: "Memory write",
    description:
      "Append a durable fact to persistent memory (MEMORY.md). Use SPARINGLY for facts that matter beyond this session: decisions, preferences, project conventions. One line per fact.",
    parameters: Type.Object({
      fact: Type.String({ description: "One durable fact, single line" }),
      file: Type.Optional(Type.String({ description: "MEMORY.md (default) or USER.md" })),
    }),
    async execute(_id, params) {
      // default target: the PROJECT store (programming memory lives with
      // the project); USER.md routes to the global owner store
      const target = params.file === "USER.md"
        ? { dir: globalMemoryDir, file: "USER.md" }
        : { dir: projectMemoryDir, file: "MEMORY.md" };
      const file = target.file;
      const dir = target.dir;
      mkdirSync(dir, { recursive: true });
      const line = `- ${params.fact.replace(/\s+/g, " ").trim().slice(0, 300)}`;
      appendFileSync(join(dir, file), line + "\n");
      return { content: [{ type: "text", text: `remembered (${file}): ${line.slice(2)}` }], details: undefined as never };
    },
  });

  pi.registerTool({
    name: "memory_search",
    label: "Memory search",
    description:
      "Search past Officina sessions and memory files for a keyword. Returns capped snippets. Use for cross-session recall.",
    parameters: Type.Object({
      query: Type.String({ description: "Case-insensitive keyword or phrase" }),
    }),
    async execute(_id, params) {
      const q = params.query.trim().slice(0, 200);
      if (!q) return { content: [{ type: "text", text: "memory_search: empty query." }], details: undefined as never };
      const needle = q.toLowerCase();
      const hits: string[] = [];

      const scan = (text: string, label: string) => {
        for (const line of text.split("\n")) {
          if (hits.length >= SEARCH_MAX_HITS) return;
          if (line.toLowerCase().includes(needle)) {
            hits.push(`• [${label}] ${line.trim().slice(0, SEARCH_SNIPPET)}`);
          }
        }
      };

      const memDirs: Array<[string, string]> = [
        [projectMemoryDir || "(no project)", "project/MEMORY.md"],
        [globalMemoryDir, "global/USER.md"],
      ];
      for (const [d, label] of memDirs) {
        const p = join(d, "MEMORY.md");
        if (existsSync(p)) {
          try {
            scan(readFileSync(p, "utf8"), label);
          } catch {
            // skip unreadable
          }
        }
      }
      const ug = join(globalMemoryDir, "USER.md");
      if (existsSync(ug)) {
        try {
          scan(readFileSync(ug, "utf8"), "global/USER.md");
        } catch {
          // skip unreadable
        }
      }

      if (sessionDir && existsSync(sessionDir)) {
        for (const f of sessionJsonlFiles(sessionDir, 2)) {
          if (hits.length >= SEARCH_MAX_HITS) break;
          try {
            scan(readFileSync(f, "utf-8"), f.slice(-44));
          } catch {
            // skip unreadable session files
          }
        }
      }

      const text =
        hits.length === 0 ? `memory_search: no matches for '${q}'.` : hits.join("\n");
      return { content: [{ type: "text", text }], details: undefined as never };
    },
  });

  // per-turn injection of persistent facts (cache-safe hidden tail message)
  pi.on("before_agent_start", () => {
    const facts = readFacts([projectMemoryDir, globalMemoryDir]);
    if (!facts) return;
    return injectionResult(
      "officina-memory",
      `Persistent memory (facts that carry across sessions):\n${facts}`,
    );
  });

  pi.on("session_start", (_event, ctx) => {
    projectMemoryDir = projectDir(ctx.cwd);
    try {
      sessionDir = ctx.sessionManager.getSessionDir();
    } catch {
      sessionDir = null;
    }
  });
}
