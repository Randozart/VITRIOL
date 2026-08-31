import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { compressProse, reductionPct } from "./compress.ts";

// caveman wiring (2026-08-31, SS2b port): applies the deterministic
// compressor to compression-ALLOWED text entering context — sub-coder
// reports ("dispatch" tool results) and memory retrieval ("memory_search").
// Never system prompts, schemas, plans, or code (R2.3 forbidden list;
// code spans are protected inside the compressor itself).
//
// Ships DARK exactly like upstream: armed only with TRIS_CAVEMAN=1.
// Off out of the box because compression of agentic text is a measured
// trade (−65% output tokens upstream), not a default.

const ALLOWED_TOOLS = new Set(["dispatch", "memory_search"]);
const MIN_REDUCTION_TO_NOTE = 5;

export default function (pi: ExtensionAPI) {
  if (process.env.TRIS_CAVEMAN !== "1") return; // dark by default (Rule 15)

  pi.on("tool_result", (event) => {
    if (!ALLOWED_TOOLS.has(event.toolName)) return;
    try {
      const content = (event as { content?: Array<{ type?: string; text?: string }> }).content ?? [];
      for (const part of content) {
        if (part.type === "text" && part.text && part.text.length >= 80) {
          const compressed = compressProse(part.text);
          const saved = reductionPct(part.text, compressed);
          if (saved >= MIN_REDUCTION_TO_NOTE) {
            part.text = compressed;
          }
        }
      }
    } catch {
      // compression must never break a tool result
    }
  });
}
