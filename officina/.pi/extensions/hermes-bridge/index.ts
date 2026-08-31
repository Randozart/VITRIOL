import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "@sinclair/typebox";
import { bridgeConfig, readPersistentFacts, runSearch } from "./bridge.ts";

// hermes-bridge — the scaffold's ONE sanctioned, READ-ONLY window into Hermes
// memory (step 16 / PLAN §3c). Sub-coders orient with gateway knowledge
// without owning gateway state: writes stay in Hermes (memory tool, step-18
// extractor) so exactly one layer authors memory (Rule 2).
//
// Read-only is structural: the sqlite handle opens readOnly:true and the
// tool surface exposes queries only. Kill switch: TRIS_NO_HERMES_BRIDGE=1.

export default function (pi: ExtensionAPI) {
  const cfg = bridgeConfig();
  if (!cfg.enabled) return;

  pi.registerTool({
    name: "hermes_search",
    label: "Hermes Search",
    description:
      "Full-text search over Hermes session memory (FTS5, bm25-ranked). Read-only. " +
      "Use before re-doing research: past sessions across ALL projects live here.",
    parameters: Type.Object({
      query: Type.String({ description: "Words to search (plain terms; FTS syntax stripped)" }),
    }),
    async execute(_id, { query }) {
      const out = await runSearch(cfg.stateDb, query, cfg);
      const text = out ?? `hermes memory: store not found at ${cfg.stateDb} (is Hermes initialized?)`;
      return { content: [{ type: "text" as const, text: cap(text) }], details: {} };
    },
  });

  pi.registerTool({
    name: "hermes_facts",
    label: "Hermes Facts",
    description: "The gateway's persistent facts (MEMORY.md + USER.md, head-capped). Read-only.",
    parameters: Type.Object({}),
    async execute() {
      const text = readPersistentFacts(cfg.memoriesDir, cfg.memoryHeadLines) || "no persistent memory yet";
      return { content: [{ type: "text" as const, text: cap(text) }], details: {} };
    },
  });
}

/** Entry-side budget guard: bridge reads must stay ~500 tok total (§R2.8). */
function cap(text: string): string {
  const max = 1800;
  if (text.length <= max) return text;
  return text.slice(0, max) + "… [capped]";
}
