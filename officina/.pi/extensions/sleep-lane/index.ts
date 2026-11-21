// sleep-lane — scheduled session consolidation on the engine's second slot.
//
// The feasible half of Google's SLEEP framework (research:
// .opencode/plans/sleep-framework-research-2026-11-21.md): wake/sleep
// alternation where the sleep phase re-reads recent session history, extracts
// durable facts, VERIFIES them by self-quizzing, and only then persists.
// Weight modification is explicitly out of scope — consolidation output is
// prose in MEMORY.md + curator queue.
//
// Policy (plan sleep-lane-2026-11-21.md): slot 1 only (I2), idle-only
// default, OFFICINA_SLEEP_INTERVAL_H cadence, OFFICINA_NO_SLEEP=1 kill
// switch, bounded reads/writes (I3), provenance-stamped appends (I4).
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import {
  existsSync,
  readdirSync,
  statSync,
  appendFileSync,
  readFileSync,
} from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import {
  startEnginePolling,
  getEngineSnapshot,
  markLocalContended,
} from "../_shared/engine.ts";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import { setBandRow } from "../_shared/band.ts";
import {
  extractPrompt,
  quizPrompt,
  parseQuizReply,
  verifyAnswer,
  shapeFact,
  capFacts,
  type ConsolidatedFact,
} from "../_shared/sleep-consolidate.ts";
import { shouldRun, sleepConfig, type SleepConfig } from "./lane.ts";

// jiti rule: shared state via globalThis.
interface SleepState {
  lastRunMs: number | null;
  running: boolean;
}
const g = globalThis as unknown as { __officinaSleepState?: SleepState };
function state(): SleepState {
  g.__officinaSleepState ??= { lastRunMs: null, running: false };
  return g.__officinaSleepState;
}

const MEMORY_PROJECT = join(process.cwd(), ".officina", "MEMORY.md");
const CURATOR_QUEUE = join(
  homedir(),
  ".vitriol",
  "officina",
  "memory",
  "curator-queue.jsonl",
);

/** Sessions dir for the current project (pi layout). */
function sessionsDir(): string | null {
  const base = join(homedir(), ".pi", "agent", "sessions");
  const dir = join(base, `--${process.cwd().replaceAll("/", "-").slice(1)}`);
  return existsSync(dir) ? dir : null;
}

function listSessionFiles(
  maxAgeDays: number,
): Array<{ path: string; bytes: number; mtimeMs: number }> {
  const dir = sessionsDir();
  if (!dir) return [];
  const cutoff = Date.now() - maxAgeDays * 86_400_000;
  const out: Array<{ path: string; bytes: number; mtimeMs: number }> = [];
  for (const f of readdirSync(dir)) {
    if (!f.endsWith(".jsonl")) continue;
    const p = join(dir, f);
    try {
      const st = statSync(p);
      if (st.mtimeMs < cutoff) continue;
      out.push({ path: p, bytes: st.size, mtimeMs: st.mtimeMs });
    } catch {
      /* raced file — skip */
    }
  }
  return out;
}

/** One chat completion on the sleep slot. Returns text, or null on failure. */
async function complete(
  cfg: SleepConfig,
  prompt: string,
  maxTokens: number,
): Promise<string | null> {
  try {
    const res = await fetch(`${cfg.base}/completion`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        prompt,
        n_predict: maxTokens,
        slot_id: cfg.slotId,
        cache_prompt: false,
        temperature: 0.4,
      }),
      signal: AbortSignal.timeout(600_000),
    });
    if (!res.ok) return null;
    const doc = (await res.json()) as { content?: string };
    return typeof doc.content === "string" ? doc.content : null;
  } catch {
    return null;
  }
}

function setBand(stateText: string): void {
  setBandRow("sleep", "state", [stateText]);
}

/** Emit to the audit log with the sleep-lane source id. */
function emit(kind: string, detail: string): void {
  emitHarnessEvent(harnessEvent("lc-sleep", kind, { detail }));
}

/** One consolidation run: select → extract → quiz-verify → persist. */
export async function triggerSleep(reason: string): Promise<void> {
  await runConsolidation(undefined as never, undefined, reason);
}

async function runConsolidation(
  pi: ExtensionAPI | undefined,
  ctx: { ui?: { notify?: (msg: string, level?: string) => void } } | undefined,
  triggerReason?: string,
): Promise<void> {
  const st = state();
  if (st.running) return;
  const cfg = sleepConfig();
  const now = Date.now();
  const gate = shouldRun(getEngineSnapshot() as never, cfg, st.lastRunMs, now);
  if (!gate.run) {
    if (triggerReason) emit("skipped", `${gate.reason} (${triggerReason})`);
    return;
  }
  st.running = true;
  setBand("sleep · consolidating…");
  try {
    const files = listSessionFiles(7);
    if (files.length === 0) {
      setBand("sleep · no recent sessions");
      return;
    }
    // Bounded window (I3).
    const selected = [];
    let total = 0;
    for (const f of [...files].sort((a, b) => b.mtimeMs - a.mtimeMs)) {
      if (total + f.bytes > cfg.maxBytes) continue;
      selected.push(f);
      total += f.bytes;
    }
    if (selected.length === 0) {
      setBand("sleep · window exceeds budget");
      return;
    }

    // Read + truncate each file into the transcript window.
    const parts: string[] = [];
    for (const f of selected) {
      const text = readFileSync(f.path, "utf8");
      parts.push(`--- ${f.path.split("/").pop()} ---\n${text.slice(-200_000)}`);
    }
    const sessionId = selected[0]!.path.split("/").pop() ?? "unknown";

    const extraction = await complete(
      cfg,
      extractPrompt(parts.join("\n\n")),
      1200,
    );
    if (extraction === null || extraction.includes("NONE")) {
      setBand(`sleep · nothing durable (${selected.length} files read)`);
      emit("run-done", "no durable facts");
      return;
    }

    const candidates = capFacts(
      extraction
        .split("\n")
        .filter((l) => l.trim().startsWith("-"))
        .map((l) => l.replace(/^-\s*/, "").trim())
        .filter(Boolean),
    );

    // Verify each candidate (I1): quiz from alternate phrasing, judge content.
    const dateIso = new Date().toISOString();
    const results: ConsolidatedFact[] = [];
    for (const fact of candidates) {
      const reply = await complete(cfg, quizPrompt(fact), 400);
      const quiz = reply ? parseQuizReply(reply) : null;
      results.push(
        quiz
          ? shapeFact(fact, verifyAnswer(fact, quiz), sessionId, dateIso)
          : shapeFact(fact, false, sessionId, dateIso),
      );
    }

    // Persist (I1 + I4): verified → MEMORY.md; rest → curator queue.
    let verified = 0;
    let curated = 0;
    for (const r of results) {
      if (r.memoryLine) {
        appendFileSync(MEMORY_PROJECT, `${r.memoryLine}\n`);
        verified++;
      } else if (r.curatorLine) {
        appendFileSync(CURATOR_QUEUE, `${r.curatorLine}\n`);
        curated++;
      }
    }
    st.lastRunMs = Date.now();
    const summary = `sleep · ${verified} verified, ${curated} queued (${selected.length} sessions)`;
    setBand(summary);
    emit("run-done", summary);
    ctx?.ui?.notify?.(`sleep lane: ${summary}`, "info");
  } catch (err) {
    setBand(`sleep · failed: ${(err as Error).message.slice(0, 60)}`);
    emit("run-error", String(err));
  } finally {
    st.running = false;
  }
}

export default function (pi: ExtensionAPI) {
  const cfg = sleepConfig();
  if (!cfg.enabled) return;

  startEnginePolling();

  setBand("sleep · idle");

  pi.on("agent_start", () => {
    // Periodic opportunity check; the gate decides (idle + interval).
    const timer = setInterval(() => {
      void runConsolidation(pi, undefined);
    }, 10 * 60_000);
    timer.unref?.();
  });

  pi.registerCommand("sleep-now", {
    description: "run session consolidation now (slot 1, bounded)",
    handler: async (_args: string, ctx: unknown) => {
      setBand("sleep · manual run");
      await runConsolidation(pi, ctx as { ui?: { notify?: (m: string, l?: string) => void } });
    },
  });
}
