import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { appendFileSync, mkdirSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

// memory-extractor (TS port, 2026-08-31, SS2b).
//
// Provenance: ported from trismegistus/hermes-plugins/memory-extractor/
// extractor.py @ 237e424 (owner-authored, MIT; OmniRoute §4.4 / step 18).
// Same stance as upstream: candidates are NEVER auto-trusted — wrong facts
// are context poisoning. Every candidate queues to
// ~/.vitriol/officina/memory/curator-queue.jsonl for human sign-off;
// only with TRIS_MEMORY_AUTO=1 do >=0.85-confidence candidates append
// straight to the project MEMORY.md.
//
// Hook: user messages stream through message_end; each is tested against
// the candidate rules. Kill switch: OFFICINA_EXTRACTOR=0.

interface Candidate {
  kind: string;
  text: string;
  confidence: number;
  source_session: string;
}

const MAX_FACT_CHARS = 260;
const AUTOTHRESHOLD = 0.85;

const RULES: Array<{ kind: string; rx: RegExp; conf: number }> = [
  { kind: "correction", rx: /^\s*no[,.]?\s+(?:it'?s|it is|use|we use|the)\s+(.{8,180})$/i, conf: 0.9 },
  { kind: "preference", rx: /\bI (?:prefer|always use|never use|want|like)\s+(.{6,180})$/i, conf: 0.85 },
  {
    kind: "environment",
    rx: /\b(?:the|my) (project|repo|db|database|server|endpoint|port|stack|venv)\b[^.\n]{0,40}?(?:is|uses|runs on|lives at|listens on)\s+(\S{4,120})/i,
    conf: 0.7,
  },
  {
    kind: "convention",
    rx: /\b(?:in this|our) (?:project|repo|codebase)\b[^.\n]{0,60}?(?:we |use |always )(.{6,160})$/i,
    conf: 0.75,
  },
];

/** Highest-confidence candidate in one user message. Pure. */
export function bestCandidateForMessage(msg: string, sessionId = ""): Candidate | null {
  if (!msg || msg.length > 4000) return null;
  let best: Candidate | null = null;
  for (const { kind, rx, conf } of RULES) {
    const m = rx.exec(msg.trim());
    if (m === null) continue;
    const text = m.groups
      ? Object.values(m.groups).filter(Boolean).join(" ").replace(/\s+/g, " ").trim()
      : m.slice(1).filter(Boolean).join(" ").replace(/\s+/g, " ").trim();
    if (text.length < 6 || text.length > MAX_FACT_CHARS) continue;
    const cand: Candidate = { kind, text, confidence: conf, source_session: sessionId };
    if (best === null || cand.confidence > best.confidence) best = cand;
  }
  return best;
}

const q = (s: string) => s.replace(/"/g, "");

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_EXTRACTOR === "0") return; // Rule 15

  const auto = process.env.TRIS_MEMORY_AUTO === "1";
  const queueDir = () => join(process.env.HOME || homedir(), ".vitriol", "officina", "memory");
  let cwd = "";
  let sessionId = "";

  pi.on("session_start", (_event, ctx) => {
    cwd = ctx.cwd;
    try {
      sessionId = ctx.sessionManager.getSessionId();
    } catch {
      sessionId = "";
    }
  });

  pi.on("message_end", (event) => {
    const msg = event.message as { role?: string; content?: unknown };
    if (msg?.role !== "user") return;
    let text = "";
    if (typeof msg.content === "string") text = msg.content;
    else if (Array.isArray(msg.content)) {
      text = (msg.content as Array<{ type?: string; text?: string }>)
        .filter((c) => c.type === "text" && c.text)
        .map((c) => c.text as string)
        .join(" ");
    }
    const cand = bestCandidateForMessage(text, sessionId);
    if (!cand) return;
    cand.text = `${cand.text} (project: ${cwd})`;
    try {
      mkdirSync(queueDir(), { recursive: true });
      appendFileSync(join(queueDir(), "curator-queue.jsonl"), JSON.stringify(cand) + "\n");
      if (auto && cand.confidence >= AUTOTHRESHOLD) {
        mkdirSync(join(cwd, ".officina"), { recursive: true });
        appendFileSync(join(cwd, ".officina", "MEMORY.md"), `- ${cand.text.slice(0, MAX_FACT_CHARS)}\n`);
      }
    } catch {
      // queueing must never break the turn
    }
  });
}
