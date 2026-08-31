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
//   store     ~/.vitriol/officina/memory/  (plain markdown, agent-writable)
//   MEMORY.md persistent facts the agent should always know (head-capped,
//             injected cache-safe as a hidden tail message each turn)
//   USER.md   facts about the owner
//   tools     memory_read / memory_write / memory_search
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

function memDir(env: NodeJS.ProcessEnv = process.env): string {
  if (env.OFFICINA_MEMORY_DIR) return env.OFFICINA_MEMORY_DIR;
  return join(env.HOME || homedir(), MEM_DIR_DEFAULT);
}

function headLines(text: string, n: number): string {
  return text.split("\n").slice(0, n).join("\n");
}

function readFacts(dir: string): string {
  const parts: string[] = [];
  for (const f of ["MEMORY.md", "USER.md"]) {
    const p = join(dir, f);
    if (!existsSync(p)) continue;
    try {
      parts.push(`### ${f}\n${headLines(readFileSync(p, "utf8"), INJECT_LINES)}`);
    } catch {
      // unreadable store is absence of knowledge, not an error
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

  const dir = memDir();
  let sessionDir: string | null = null;

  pi.registerTool({
    name: "memory_read",
    label: "Memory read",
    description:
      "Read persistent memory (MEMORY.md facts and USER.md owner facts). Use when past decisions, preferences, or project context might matter.",
    parameters: Type.Object({}),
    async execute() {
      if (!existsSync(dir)) return { content: [{ type: "text", text: "memory: empty (nothing stored yet)." }], details: undefined as never };
      const text = readFacts(dir);
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
      const file = params.file === "USER.md" ? "USER.md" : "MEMORY.md";
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

      for (const f of ["MEMORY.md", "USER.md"]) {
        const p = join(dir, f);
        if (existsSync(p)) {
          try {
            scan(readFileSync(p, "utf8"), f);
          } catch {
            // skip unreadable
          }
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
    if (!existsSync(dir)) return;
    const facts = readFacts(dir);
    if (!facts) return;
    return injectionResult(
      "officina-memory",
      `Persistent memory (facts that carry across sessions):\n${facts}`,
    );
  });

  pi.on("session_start", (_event, ctx) => {
    try {
      sessionDir = ctx.sessionManager.getSessionDir();
    } catch {
      sessionDir = null;
    }
  });
}
