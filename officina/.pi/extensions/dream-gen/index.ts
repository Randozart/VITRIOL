// dream-gen — the gated corpus builder (rung 2, 2026-11-21).
// /dream-gen [maxPairs] walks the pi session transcripts for this
// workspace, extracts question/answer practice pairs, gates them
// (I-P3 provenance), and freezes a versioned corpus under
// ~/.vitriol/officina/dream/corpus-<hash12>.jsonl.
//
// Read-only over MEMORY.md and the eval set — the corpus is a SEPARATE
// artefact (I-E1 family: corpus hash ≠ eval hash by construction).
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import {
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import {
  buildCorpus,
  extractQA,
  type DreamPair,
  type TranscriptLine,
} from "../_shared/dream-generator-core.ts";

const DREAM_DIR = join(homedir(), ".vitriol", "officina", "dream");

/** Resolve this workspace's pi session dir (the path is cwd-encoded). */
export function sessionDirFor(cwd: string): string {
  const enc = cwd.replace(/\//g, "-");
  return join(homedir(), ".pi", "agent", "sessions", enc);
}

/** Parse one pi session JSONL file into transcript lines (bounded by bytes). */
export function readTranscript(
  path: string,
  maxBytes = 4 * 1024 * 1024,
): TranscriptLine[] {
  let text: string;
  try {
    text = readFileSync(path, "utf8");
  } catch {
    return [];
  }
  if (Buffer.byteLength(text) > maxBytes) {
    // I-E2: an over-budget file is skipped whole, not truncated mid-pair.
    return [];
  }
  const out: TranscriptLine[] = [];
  for (const line of text.split("\n")) {
    if (!line.trim()) continue;
    let row;
    try {
      row = JSON.parse(line);
    } catch {
      continue; // custom/malformed rows are not transcript messages
    }
    if (row?.type !== "message") continue;
    const m = row.message;
    if (!m || typeof m.content !== "string") continue;
    const role = m.role;
    if (role !== "user" && role !== "assistant") continue;
    out.push({ role, text: m.content, ts: row.timestamp ?? "" });
  }
  return out;
}

/** Write a frozen corpus (append-only dir; old versions retained). */
export function writeCorpus(corpus: {
  hash: string;
  items: DreamPair[];
}): string {
  mkdirSync(DREAM_DIR, { recursive: true });
  const path = join(DREAM_DIR, `corpus-${corpus.hash.slice(0, 12)}.jsonl`);
  const body =
    corpus.items
      .map((p) => JSON.stringify({ ...p, hash: corpus.hash }))
      .join("\n") + "\n";
  writeFileSync(path, body);
  return path;
}

export default function (pi: ExtensionAPI) {
  pi.registerCommand("dream-gen", {
    description:
      "build the gated practice corpus from this workspace's transcripts (usage: /dream-gen [maxPairs])",
    handler: async (args: string, ctx: unknown) => {
      const c = ctx as { ui?: any; cwd?: string };
      const maxPairs = Math.max(
        1,
        Math.min(2000, Number(parts(args)[0]) || 200),
      );
      const dir = sessionDirFor(c.cwd ?? process.cwd());
      if (!existsSync(dir)) {
        c.ui?.notify?.(`dream-gen: no session dir at ${dir}`, "warning");
        return;
      }
      const files = readdirSync(dir)
        .filter((f) => f.endsWith(".jsonl") || f.includes(".jsonl."))
        .sort()
        .reverse(); // newest sessions first
      const raw: DreamPair[] = [];
      let scanned = 0;
      for (const f of files) {
        const rows = readTranscript(join(dir, f));
        if (rows.length === 0) continue;
        scanned++;
        raw.push(...extractQA(f, rows, maxPairs - raw.length));
        if (raw.length >= maxPairs) break;
      }
      if (raw.length === 0) {
        c.ui?.notify?.(
          `dream-gen: no extractable pairs from ${scanned} session(s)`,
          "warning",
        );
        return;
      }
      const corpus = await buildCorpus(raw);
      const path = writeCorpus(corpus);
      c.ui?.notify?.(
        `dream-gen: ${corpus.items.length} gated pairs (hash ${corpus.hash.slice(0, 12)}…) → ${path}`,
        "info",
      );
    },
  });
}

/** Trim arg parts (defensive: command args arrive pre-split in some hosts). */
function parts(args: string): string[] {
  return args.trim().split(/\s+/).filter(Boolean);
}
