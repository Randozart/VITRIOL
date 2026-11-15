// PROVENANCE: original Officina advisory watchdog. Deterministic monitoring
// survives unknown/unavailable semantic judgments; this extension never aborts.
import { mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import {
  buildJudgePrompt, MAX_JUDGMENTS_PER_CALL, REJUDGE_EVERY_S, JUDGE_S_DEFAULT,
  observeToolOutput, parseVerdict, pickJudgeModel, SOFT_S_DEFAULT, TICK_S, tickCall,
  type TickAction, type WatchedCall,
} from "./judge.ts";
import { judgeWithDecision } from "./decision.ts";
import { extractCatalogModels, freeToolCandidates } from "../caduceus/catalog.ts";

const JUDGE_TIMEOUT_MS = 30_000;
const directory = () => join(homedir(), ".vitriol", "officina");

/** Session-scoped facts prevent one TUI from displaying another session's verdict. */
export function watchdogPath(session: string): string {
  const safe = session.replace(/[^A-Za-z0-9._-]/g, "_").slice(0, 120);
  return join(directory(), "watchdog", `${safe || "unknown"}.json`);
}

function envInt(name: string, fallback: number): number {
  const n = Number(process.env[name]);
  return Number.isFinite(n) && n > 0 ? n : fallback;
}

function gatewayBase(): string {
  if (process.env.OFFICINA_CADUCEUS_URL) return process.env.OFFICINA_CADUCEUS_URL.replace(/\/+$/, "");
  try { return readFileSync(join(directory(), "gateway-url"), "utf8").trim().replace(/\/+$/, "") || "http://127.0.0.1:20128/v1"; }
  catch { return "http://127.0.0.1:20128/v1"; }
}

function bestFreeLane(): string | null {
  try {
    const raw = JSON.parse(readFileSync(join(directory(), "caduceus-models.json"), "utf8"));
    return pickJudgeModel(freeToolCandidates(extractCatalogModels(raw)).map((c) => c.id));
  } catch { return null; }
}

/** A bounded separate completion, never the busy actor's turn or local slot. */
async function judgeWithLane(call: WatchedCall, signal: AbortSignal): Promise<string | null> {
  const model = bestFreeLane();
  const key = process.env.OFFICINA_CADUCEUS_API_KEY?.trim();
  if (!model || !key) return null;
  try {
    const now = Date.now();
    const prompt = buildJudgePrompt(call.command, Math.floor((now - call.startedAt) / 1000), Math.floor((now - (call.stalledSince ?? now)) / 1000), call.tail ?? "");
    const res = await fetch(`${gatewayBase()}/chat/completions`, {
      method: "POST", headers: { "content-type": "application/json", authorization: `Bearer ${key}` },
      body: JSON.stringify({ model, messages: [{ role: "user", content: prompt }], max_tokens: 80, temperature: 0 }), signal,
    });
    if (!res.ok) return null;
    const doc = await res.json() as { choices?: Array<{ message?: { content?: unknown } }> };
    const content = doc.choices?.[0]?.message?.content;
    return typeof content === "string" ? content : null;
  } catch { return null; }
}

interface WatchdogDoc {
  version: 1;
  ts: number;
  valid_until: number;
  session_id: string;
  tool_call_id: string;
  kind: "notice" | "judge" | "verdict" | "unavailable" | "ended";
  command: string;
  elapsed_s: number;
  stalled_s: number;
  verdict: "JUDGING" | "ALIVE" | "HUNG" | "UNKNOWN" | "";
  reason: string;
  next_check_at: number;
  phase: "soft" | "judge" | "rejudge" | "monitor" | "done";
}

function writeWatchdog(doc: WatchdogDoc): void {
  try {
    const path = watchdogPath(doc.session_id);
    mkdirSync(join(directory(), "watchdog"), { recursive: true });
    const tmp = `${path}.tmp-${process.pid}`;
    writeFileSync(tmp, JSON.stringify(doc) + "\n", { mode: 0o600 });
    renameSync(tmp, path);
  } catch { /* Observability must not stop the actor. */ }
}

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_NO_HANG_WATCHDOG === "1") return;
  const softS = envInt("OFFICINA_HANG_SOFT_S", SOFT_S_DEFAULT);
  const judgeS = envInt("OFFICINA_HANG_JUDGE_S", JUDGE_S_DEFAULT);
  const watched = new Map<string, WatchedCall>();
  const pending = new Map<string, AbortController>();
  let session = `process-${process.pid}`;
  let ui: { notify(message: string, level?: "info" | "warning" | "error"): unknown } | undefined;
  let timer: ReturnType<typeof setInterval> | undefined;

  const publish = (call: WatchedCall, status: Pick<WatchdogDoc, "kind" | "verdict" | "reason" | "next_check_at" | "phase">, message: string) => {
    const now = Date.now();
    call.surfaced = true;
    const doc: WatchdogDoc = { version: 1, ts: now, valid_until: Math.min(now + 360_000, Math.max(now + 120_000, status.next_check_at + 30_000)), session_id: session, tool_call_id: call.toolCallId, command: call.command,
      elapsed_s: Math.floor((now - call.startedAt) / 1000), stalled_s: Math.floor((now - (call.stalledSince ?? now)) / 1000), ...status };
    writeWatchdog(doc);
    try { ui?.notify(`${message}\n<officina-watchdog-v1>${JSON.stringify(doc)}`, status.verdict === "ALIVE" ? "info" : "warning"); } catch { /* advisory */ }
  };

  const runJudge = async (call: WatchedCall): Promise<void> => {
    const controller = new AbortController();
    pending.set(call.toolCallId, controller);
    call.judging = true;
    const generation = call.generation ?? 0;
    publish(call, { kind: "judge", verdict: "JUDGING", reason: "", next_check_at: Date.now() + JUDGE_TIMEOUT_MS, phase: "judge" }, `hang-watchdog: judging "${call.command.slice(0, 60)}"`);
    const current = () => watched.get(call.toolCallId) === call && (call.generation ?? 0) === generation;
    const deadline = setTimeout(() => controller.abort(), JUDGE_TIMEOUT_MS);
    let onAbort: () => void = () => {};
    const cancelled = new Promise<null>((resolve) => { onAbort = () => resolve(null); controller.signal.addEventListener("abort", onAbort, { once: true }); });
    try {
      const request = async () => {
        const decision = await judgeWithDecision(call, controller.signal);
        if (decision) return decision;
        const reply = await judgeWithLane(call, controller.signal);
        return reply === null ? { alive: null, reason: "judge unavailable — liveness unknown" } : parseVerdict(reply);
      };
      const result = await Promise.race([request(), cancelled]);
      if (!current()) return;
      const verdict = result ?? { alive: null, reason: "judge deadline exceeded — liveness unknown" };
      const label = verdict.alive === null ? "UNKNOWN" : verdict.alive ? "ALIVE" : "HUNG";
      call.lastVerdict = label;
      const capped = call.judgments >= MAX_JUDGMENTS_PER_CALL;
      publish(call, { kind: verdict.alive === null ? "unavailable" : "verdict", verdict: label, reason: verdict.reason,
        next_check_at: (call.judgedAt ?? Date.now()) + REJUDGE_EVERY_S * 1000, phase: capped ? "monitor" : "rejudge" },
      `hang-watchdog verdict for "${call.command.slice(0, 60)}": ${label} — ${verdict.reason} (advisory; monitoring continues)`);
    } finally {
      clearTimeout(deadline);
      controller.signal.removeEventListener("abort", onAbort);
      if (pending.get(call.toolCallId) === controller) { pending.delete(call.toolCallId); call.judging = false; }
    }
  };

  const handleTick = (call: WatchedCall, action: TickAction) => {
    if (action.kind === "none" || action.kind === "alive") return;
    if (action.kind === "notice") {
      const capped = call.judgments >= MAX_JUDGMENTS_PER_CALL;
      publish(call, { kind: "notice", verdict: "UNKNOWN", reason: capped ? "judge budget exhausted; deterministic monitoring continues" : "no new output",
        next_check_at: capped ? Date.now() + REJUDGE_EVERY_S * 1000 : (call.stalledSince ?? Date.now()) + judgeS * 1000,
        phase: capped ? "monitor" : "soft" }, action.message);
      return;
    }
    void runJudge(call);
  };

  const reset = () => {
    for (const controller of pending.values()) controller.abort();
    watched.clear(); pending.clear();
    if (timer) clearInterval(timer);
    timer = undefined;
  };

  pi.on("session_start", (_event, ctx) => {
    reset();
    session = ctx.sessionManager.getSessionId();
    ui = ctx.ui; // RPC notifications work even when a caller reports hasUI=false.
    timer = setInterval(() => {
      for (const call of watched.values()) handleTick(call, tickCall(call, call.lastOutputRevision, call.tail ?? "", { now: Date.now(), softS, judgeS }));
    }, TICK_S * 1000);
    timer.unref?.();
  });
  pi.on("session_shutdown", () => { reset(); ui = undefined; });
  pi.on("tool_execution_start", (event) => {
    if (event.toolName.toLowerCase() !== "bash") return;
    const command = String(event.args.command ?? event.args.cmd ?? "");
    if (!command) return;
    watched.set(event.toolCallId, { toolCallId: event.toolCallId, command, startedAt: Date.now(), lastOutputRevision: 0, stalledSince: null, judgments: 0, lastVerdict: null });
  });
  pi.on("tool_execution_update", (event) => {
    const call = watched.get(event.toolCallId);
    if (!call || !observeToolOutput(call, event.partialResult, Date.now())) return;
    pending.get(call.toolCallId)?.abort();
    if (call.surfaced) publish(call, { kind: "notice", verdict: "ALIVE", reason: "new output from this tool", next_check_at: Date.now() + softS * 1000, phase: "soft" },
      `hang-watchdog: "${call.command.slice(0, 60)}" output resumed`);
  });
  pi.on("tool_execution_end", (event) => {
    const call = watched.get(event.toolCallId);
    watched.delete(event.toolCallId);
    pending.get(event.toolCallId)?.abort();
    if (call?.surfaced) publish(call,
      { kind: "ended", verdict: "", reason: "tool finished", next_check_at: 0, phase: "done" }, `hang-watchdog: "${call.command.slice(0, 60)}" finished`);
  });
}
