import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "@sinclair/typebox";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import { requestSidebarUpdate } from "../_shared/sidebar.ts";
import {
  applyUpdate,
  emptyDoc,
  parseScratchpad,
  renderScratchpadBlock,
  scratchpadConfig,
  serializeScratchpad,
  totalLines,
  type ScratchpadDoc,
} from "./state.ts";

// scratchpad — the project-scoped hot notebook (detective-notebook lane).
//
// Sibling of task-state (work items) and memory (curated long-term): this
// holds the VOLATILE working register — evidence, open leads, ruled-out
// dead ends — that mid-run compaction would otherwise destroy and force
// expensive re-reads to recover. Same survival mechanism: persisted by a
// tool call, parsed from disk and re-injected as a tail message before
// EVERY LLM call, so it survives compaction by construction.
//
// Cache safety (Rule 7, same as task-state): we only ever APPEND to
// event.messages — the existing prefix stays byte-identical, KV cache
// intact. Injection is skipped when the last copy already carries the
// identical block. Pruning is model-driven: a section named in a write is
// replaced wholesale, so "no longer relevant" means "leave it out of the
// next write of that section".
//
// Kill switch: OFFICINA_SCRATCHPAD=0. Cap: OFFICINA_SCRATCHPAD_CAP (512).

const CUSTOM_TYPE = "lc-scratchpad";
const FILE_NAME = "SCRATCHPAD.md";

const cfg = scratchpadConfig();
const currentFile = join(cfg.dir, FILE_NAME);

function readDoc(): ScratchpadDoc {
  try {
    if (!existsSync(currentFile)) return emptyDoc();
    return parseScratchpad(readFileSync(currentFile, "utf8"));
  } catch {
    return emptyDoc(); // corrupt file — start fresh rather than block the lane
  }
}

/** Sidebar/v2 export: null when the notebook is empty. */
export function getScratchpadSummary(): { lines: number; cap: number; facts: number; context: number; leads: number; dead: number } | null {
  try {
    const doc = existsSync(currentFile) ? parseScratchpad(readFileSync(currentFile, "utf8")) : emptyDoc();
    if (totalLines(doc) === 0) return null;
    return { lines: totalLines(doc), cap: cfg.cap, facts: doc.facts.length, context: doc.context.length, leads: doc.leads.length, dead: doc.dead.length };
  } catch {
    return null;
  }
}

/** Scratchpad CONTENT for sidebar display: the actual open lines —
 *  facts (evidence) and leads (open threads), each already one line.
 *  Null when the notebook is empty. */
export function getScratchpadItems(): { facts: string[]; leads: string[] } | null {
  try {
    const doc = readDoc();
    if (totalLines(doc) === 0) return null;
    return { facts: doc.facts, leads: doc.leads };
  } catch {
    return null;
  }
}

export default function (pi: ExtensionAPI) {
  if (!cfg.enabled) return;

  pi.registerTool({
    name: "scratchpad_write",
    label: "Scratchpad Write",
    description:
      "Update the project hot notebook (external state, survives compaction, re-injected every turn). " +
      "Detective-notebook discipline: evidence in `facts` (numbers verbatim), structured working data in `context` " +
      "(error lists, file excerpts, intermediate results), open hypotheses in `leads`, " +
      `ruled-out ideas in \`dead\`. Hard cap ${cfg.cap} lines total. ` +
      "A section you name is REPLACED wholesale — omit stale lines to prune them. Not history.",
    parameters: Type.Object({
      facts: Type.Optional(Type.Array(Type.String(), { description: "Replace the facts section (evidence: numbers, shapes, argv, observed behavior)" })),
      context: Type.Optional(Type.Array(Type.String(), { description: "Replace the context section (working-set data: error lists, file excerpts in progress, intermediate results that bridge compaction gaps)" })),
      leads: Type.Optional(Type.Array(Type.String(), { description: "Replace the leads section (open hypotheses, next attempts)" })),
      dead: Type.Optional(Type.Array(Type.String(), { description: "Replace the dead section (ruled out, stated briefly; prune when cold)" })),
      reset: Type.Optional(Type.Boolean({ description: "Wipe the entire notebook before applying (true = start fresh)" })),
    }),
    async execute(_id, update) {
      const before = readDoc();
      const v = applyUpdate(before, update, cfg);
      if (v.error || !v.doc) {
        const counts = `current: facts=${before.facts.length} context=${before.context.length} leads=${before.leads.length} dead=${before.dead.length}`;
        return { content: [{ type: "text" as const, text: `scratchpad_write rejected: ${v.error} (${counts})` }], details: {}, isError: true };
      }
      try {
        mkdirSync(cfg.dir, { recursive: true });
        writeFileSync(currentFile, serializeScratchpad(v.doc));
      } catch (e) {
        return { content: [{ type: "text" as const, text: `scratchpad_write could not persist: ${(e as Error).message}` }], details: {}, isError: true };
      }
      const total = totalLines(v.doc);
      emitHarnessEvent(harnessEvent("lc-scratchpad", "updated", { detail: `${total}/${cfg.cap} lines` }));
      requestSidebarUpdate(); // live refresh (2026-09-04) — see task-state note
      return {
        content: [{
          type: "text" as const,
          text: `scratchpad saved: ${total}/${cfg.cap} lines (facts=${v.doc.facts.length} context=${v.doc.context.length} leads=${v.doc.leads.length} dead=${v.doc.dead.length}) -> ${currentFile}`,
        }],
        details: {},
      };
    },
  });

  pi.on("context", async (event) => {
    const block = renderScratchpadBlock(readDoc(), cfg.cap);
    if (!block) return undefined;
    if (lastCopyIsCurrent(event.messages, block)) return undefined;
    const tail = {
      role: "custom" as const,
      customType: CUSTOM_TYPE,
      content: block,
      display: false,
      details: {},
      timestamp: Date.now(),
    };
    return { messages: [...event.messages, tail] };
  });
}

/** True when the most recent injected copy already carries exactly `block`. */
export function lastCopyIsCurrent(messages: unknown[], block: string): boolean {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i] as { role?: string; customType?: string; content?: unknown };
    if (m.role === "custom" && m.customType === CUSTOM_TYPE) {
      return String(m.content ?? "") === block;
    }
  }
  return false; // never injected this session yet
}
