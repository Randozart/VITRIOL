// PROVENANCE: original Officina advisory watchdog. Deterministic monitoring
// survives unknown/unavailable semantic judgments; this extension never aborts.
import { mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import {
  buildJudgePrompt,
  MAX_JUDGMENTS_PER_CALL,
  REJUDGE_EVERY_S,
  JUDGE_S_DEFAULT,
  observeToolOutput,
  parseVerdict,
  pickJudgeModel,
  SOFT_S_DEFAULT,
  TICK_S,
  tickCall,
  type TickAction,
  type WatchedCall,
} from "./judge.ts";
import { judgeWithDecision } from "./decision.ts";
import {
  resolveDecisionConfig,
  readDecisionConfig,
  writeDecisionConfig,
} from "./config.ts";
import {
  extractCatalogModels,
  freeToolCandidates,
} from "../caduceus/catalog.ts";

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
  if (process.env.OFFICINA_CADUCEUS_URL)
    return process.env.OFFICINA_CADUCEUS_URL.replace(/\/+$/, "");
  try {
    return (
      readFileSync(join(directory(), "gateway-url"), "utf8")
        .trim()
        .replace(/\/+$/, "") || "http://127.0.0.1:20128/v1"
    );
  } catch {
    return "http://127.0.0.1:20128/v1";
  }
}

function bestFreeLane(): string | null {
  try {
    const raw = JSON.parse(
      readFileSync(join(directory(), "caduceus-models.json"), "utf8"),
    );
    return pickJudgeModel(
      freeToolCandidates(extractCatalogModels(raw)).map((c) => c.id),
    );
  } catch {
    return null;
  }
}

/** A bounded separate completion, never the busy actor's turn or local slot. */
async function judgeWithLane(
  call: WatchedCall,
  signal: AbortSignal,
): Promise<string | null> {
  const model = bestFreeLane();
  const key = process.env.OFFICINA_CADUCEUS_API_KEY?.trim();
  if (!model || !key) return null;
  try {
    const now = Date.now();
    const prompt = buildJudgePrompt(
      call.command,
      Math.floor((now - call.startedAt) / 1000),
      Math.floor((now - (call.stalledSince ?? now)) / 1000),
      call.tail ?? "",
    );
    const res = await fetch(`${gatewayBase()}/chat/completions`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${key}`,
      },
      body: JSON.stringify({
        model,
        messages: [{ role: "user", content: prompt }],
        max_tokens: 80,
        temperature: 0,
      }),
      signal,
    });
    if (!res.ok) return null;
    const doc = (await res.json()) as {
      choices?: Array<{ message?: { content?: unknown } }>;
    };
    const content = doc.choices?.[0]?.message?.content;
    return typeof content === "string" ? content : null;
  } catch {
    return null;
  }
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
  verdict: "JUDGING" | "ALIVE" | "HUNG" | "UNKNOWN" | "NO-JUDGE" | "";
  reason: string;
  /** Where the verdict came from (2026-11-16): typed decision lane, free
   *  cloud lane, or unavailable when neither could answer. Absent on
   *  notices/ended docs where no judge ran. */
  source?: "decision" | "lane" | "unavailable";
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
  } catch {
    /* Observability must not stop the actor. */
  }
}

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_NO_HANG_WATCHDOG === "1") return;
  const softS = envInt("OFFICINA_HANG_SOFT_S", SOFT_S_DEFAULT);
  const judgeS = envInt("OFFICINA_HANG_JUDGE_S", JUDGE_S_DEFAULT);
  const watched = new Map<string, WatchedCall>();
  const pending = new Map<string, AbortController>();
  let session = `process-${process.pid}`;
  let ui:
    | { notify(message: string, level?: "info" | "warning" | "error"): unknown }
    | undefined;
  let timer: ReturnType<typeof setInterval> | undefined;

  const publish = (
    call: WatchedCall,
    status: Pick<
      WatchdogDoc,
      "kind" | "verdict" | "reason" | "source" | "next_check_at" | "phase"
    >,
    message: string,
  ) => {
    const now = Date.now();
    call.surfaced = true;
    const doc: WatchdogDoc = {
      version: 1,
      ts: now,
      valid_until: Math.min(
        now + 360_000,
        Math.max(now + 120_000, status.next_check_at + 30_000),
      ),
      session_id: session,
      tool_call_id: call.toolCallId,
      command: call.command,
      elapsed_s: Math.floor((now - call.startedAt) / 1000),
      stalled_s: Math.floor((now - (call.stalledSince ?? now)) / 1000),
      ...status,
    };
    writeWatchdog(doc);
    try {
      ui?.notify(
        `${message}\n<officina-watchdog-v1>${JSON.stringify(doc)}`,
        status.verdict === "ALIVE" ? "info" : "warning",
      );
    } catch {
      /* advisory */
    }
  };

  const runJudge = async (call: WatchedCall): Promise<void> => {
    const controller = new AbortController();
    pending.set(call.toolCallId, controller);
    call.judging = true;
    const generation = call.generation ?? 0;
    publish(
      call,
      {
        kind: "judge",
        verdict: "JUDGING",
        reason: "",
        next_check_at: Date.now() + JUDGE_TIMEOUT_MS,
        phase: "judge",
      },
      `hang-watchdog: judging "${call.command.slice(0, 60)}"`,
    );
    const current = () =>
      watched.get(call.toolCallId) === call &&
      (call.generation ?? 0) === generation;
    const deadline = setTimeout(() => controller.abort(), JUDGE_TIMEOUT_MS);
    let onAbort: () => void = () => {};
    const cancelled = new Promise<null>((resolve) => {
      onAbort = () => resolve(null);
      controller.signal.addEventListener("abort", onAbort, { once: true });
    });
    try {
      const request = async () => {
        const decision = await judgeWithDecision(call, controller.signal);
        if (decision) return decision; // carries source "decision"
        const reply = await judgeWithLane(call, controller.signal);
        if (reply === null) {
          return {
            alive: null,
            reason: "judge unavailable — liveness unknown",
            source: "unavailable" as const,
          };
        }
        return { ...parseVerdict(reply), source: "lane" as const };
      };
      const result = await Promise.race([request(), cancelled]);
      if (!current()) return;
      const verdict = result ?? {
        alive: null,
        reason: "judge deadline exceeded — liveness unknown (NO-JUDGE)",
        unparsed: true,
        source: "unavailable" as const,
      };
      // `unparsed` only exists on an unparsable reply: a decision-model verdict
      // is structurally parsed, so the field is absent there.
      const broken = "unparsed" in verdict && verdict.unparsed === true;
      const label =
        verdict.alive === null
          ? broken
            ? "NO-JUDGE"
            : "UNKNOWN"
          : verdict.alive
            ? "ALIVE"
            : "HUNG";
      call.lastVerdict = label;
      const capped = call.judgments >= MAX_JUDGMENTS_PER_CALL;
      if (capped) call.monitorPublishedAt = Date.now();
      publish(
        call,
        {
          kind: verdict.alive === null ? "unavailable" : "verdict",
          verdict: label,
          reason: verdict.reason,
          next_check_at: capped
            ? 0
            : (call.judgedAt ?? Date.now()) + REJUDGE_EVERY_S * 1000,
          phase: capped ? "monitor" : "rejudge",
          source: ("source" in verdict ? verdict.source : "lane") as
            "decision" | "lane" | "unavailable",
        },
        `hang-watchdog verdict for "${call.command.slice(0, 60)}": ${label} — ${verdict.reason} (advisory; monitoring continues)`,
      );
    } finally {
      clearTimeout(deadline);
      controller.signal.removeEventListener("abort", onAbort);
      if (pending.get(call.toolCallId) === controller) {
        pending.delete(call.toolCallId);
        call.judging = false;
      }
    }
  };

  const handleTick = (call: WatchedCall, action: TickAction) => {
    if (action.kind === "none" || action.kind === "alive") return;
    if (action.kind === "notice") {
      const capped = call.judgments >= MAX_JUDGMENTS_PER_CALL;
      publish(
        call,
        {
          kind: "notice",
          verdict: "UNKNOWN",
          reason: capped
            ? "judge budget exhausted; deterministic monitoring continues"
            : "no new output",
          next_check_at: capped
            ? 0
            : (call.stalledSince ?? Date.now()) + judgeS * 1000,
          phase: capped ? "monitor" : "soft",
        },
        action.message,
      );
      return;
    }
    void runJudge(call);
  };

  const reset = () => {
    for (const controller of pending.values()) controller.abort();
    watched.clear();
    pending.clear();
    if (timer) clearInterval(timer);
    timer = undefined;
  };

  pi.on("session_start", (_event, ctx) => {
    reset();
    session = ctx.sessionManager.getSessionId();
    ui = ctx.ui; // RPC notifications work even when a caller reports hasUI=false.
    timer = setInterval(() => {
      for (const call of watched.values())
        handleTick(
          call,
          tickCall(call, call.lastOutputRevision, call.tail ?? "", {
            now: Date.now(),
            softS,
            judgeS,
          }),
        );
    }, TICK_S * 1000);
    timer.unref?.();
  });
  pi.on("session_shutdown", () => {
    reset();
    ui = undefined;
  });
  pi.on("tool_execution_start", (event) => {
    if (event.toolName.toLowerCase() !== "bash") return;
    const command = String(event.args.command ?? event.args.cmd ?? "");
    if (!command) return;
    watched.set(event.toolCallId, {
      toolCallId: event.toolCallId,
      command,
      startedAt: Date.now(),
      lastOutputRevision: 0,
      stalledSince: null,
      judgments: 0,
      lastVerdict: null,
    });
  });
  pi.on("tool_execution_update", (event) => {
    const call = watched.get(event.toolCallId);
    if (!call || !observeToolOutput(call, event.partialResult, Date.now()))
      return;
    pending.get(call.toolCallId)?.abort();
    if (call.surfaced)
      publish(
        call,
        {
          kind: "notice",
          verdict: "ALIVE",
          reason: "new output from this tool",
          next_check_at: Date.now() + softS * 1000,
          phase: "soft",
        },
        `hang-watchdog: "${call.command.slice(0, 60)}" output resumed`,
      );
  });
  pi.on("tool_execution_end", (event) => {
    const call = watched.get(event.toolCallId);
    watched.delete(event.toolCallId);
    pending.get(event.toolCallId)?.abort();
    if (call?.surfaced)
      publish(
        call,
        {
          kind: "ended",
          verdict: "",
          reason: "tool finished",
          next_check_at: 0,
          phase: "done",
        },
        `hang-watchdog: "${call.command.slice(0, 60)}" finished`,
      );
  });

  // /decision — configure the typed SystemOne judge (2026-11-16).
  // `status` shows the current lane; `set <url>` / `key <token>` /
  // `floor <0-1>` write the 0600 config; `off` clears it. Env vars still
  // override the file for throwaway runs.
  pi.registerCommand?.("decision", {
    description:
      "Decision lane: status | test [hung|alive] | mode heuristic|http|gemini | set <url> | key <token> | model <id> | floor <0-1> | margin <0-1> | redact on|off | off",
    handler: async (
      args: string,
      ctx: {
        ui?: { notify?: (m: string, l?: "error" | "info" | "warning") => void };
      },
    ) => {
      const cfg = readDecisionConfig() ?? {
        version: 1 as const,
        url: "",
        key: "",
        floor: 0.9,
      };
      const a = args.trim();
      const show = (
        text: string,
        level: "error" | "info" | "warning" = "info",
      ) => {
        try {
          ctx.ui?.notify?.(text, level);
        } catch {
          /* advisory */
        }
      };
      const redact = (k: string) =>
        k ? `${k.slice(0, 6)}…${k.length > 6 ? " (set)" : ""}` : "";
      if (!a || a === "status") {
        const current = resolveDecisionConfig();
        if (current.mode === "heuristic") {
          show(
            "decision: heuristics only (local) — `/decision mode gemini` or `set <url>` to add a judge",
          );
          return;
        }
        if (current.mode === "gemini") {
          show(
            `decision: gemini ${current.model} · floor ${current.floor} · margin ${current.margin} · ` +
              `key ${redact(current.key) || "(none)"} · evidence ${current.redact ? "redacted, leaves the box" : "UNREDACTED, leaves the box"} · ~$0.00003/call`,
          );
          return;
        }
        show(
          `decision: ${current.endpoint} · floor ${current.floor} · margin ${current.margin} · key ${redact(current.key)}`,
        );
        return;
      }
      if (a === "off") {
        writeDecisionConfig({ version: 1, url: "", key: "", floor: 0.9 });
        show("decision: cleared — watchdog uses the free lane");
        return;
      }
      const setUrl = a.match(/^set\s+(\S+)/);
      if (setUrl) {
        const url = setUrl[1];
        if (!/^https?:\/\//.test(url)) {
          show(`decision: URL must be http(s): ${url}`, "warning");
          return;
        }
        const ok = writeDecisionConfig({
          ...cfg,
          url,
          key: cfg.key,
          floor: cfg.floor,
        });
        show(
          ok ? `decision: set ${url}` : "decision: write failed",
          ok ? "info" : "warning",
        );
        return;
      }
      const setKey = a.match(/^key\s+(\S+)/);
      if (setKey) {
        const ok = writeDecisionConfig({ ...cfg, key: setKey[1] });
        show(
          ok ? "decision: key set" : "decision: write failed",
          ok ? "info" : "warning",
        );
        return;
      }
      const setFloor = a.match(/^floor\s+([0-9.]+)/);
      if (setFloor) {
        const floor = Number(setFloor[1]);
        if (!Number.isFinite(floor) || floor < 0 || floor > 1) {
          show("decision: floor must be 0..1", "warning");
          return;
        }
        const ok = writeDecisionConfig({ ...cfg, floor });
        show(
          ok ? `decision: floor ${floor}` : "decision: write failed",
          ok ? "info" : "warning",
        );
        return;
      }
      // `test` — run one real decision on synthetic stalled evidence. The
      // fastest way to answer "is the judge actually being used?" without
      // hunting through facts files (2026-11-16).
      if (a === "test" || a === "test hung" || a === "test alive") {
        const mode = cfg.mode ?? (cfg.url ? "http" : "heuristic");
        if (mode === "heuristic") {
          show("decision: heuristics only — no judge to call (`/decision mode gemini`)", "warning");
          return;
        }
        const now = Date.now();
        const hung = a !== "test alive";
        const started = Date.now();
        const verdict = await judgeWithDecision(
          {
            toolCallId: "decision-test",
            command: hung ? "cat" : "cargo build --release",
            startedAt: now - 300_000,
            lastOutputRevision: 1,
            stalledSince: hung ? now - 280_000 : null,
            judgments: 0,
            lastVerdict: null,
            tail: hung ? "" : "Compiling officina-tui v0.2.0 — Finished release profile",
            generation: 1,
          },
          new AbortController().signal,
        );
        const ms = Date.now() - started;
        if (!verdict) {
          show("decision: lane returned nothing (no judge configured?)", "warning");
          return;
        }
        const label = verdict.alive === null ? "UNKNOWN" : verdict.alive ? "ALIVE" : "HUNG";
        show(
          `decision[${verdict.source}] ${label} in ${ms}ms — ${verdict.reason}`,
          verdict.alive === null ? "warning" : "info",
        );
        return;
      }
      const setMode = a.match(/^mode\s+(heuristic|http|gemini)$/);
      if (setMode) {
        const mode = setMode[1] as "heuristic" | "http" | "gemini";
        if (mode === "http" && !cfg.url) {
          show("decision: mode http needs an endpoint first (`/decision set <url>`)", "warning");
          return;
        }
        const ok = writeDecisionConfig({ ...cfg, mode });
        show(
          ok ? `decision: mode ${mode}` : "decision: write failed",
          ok ? "info" : "warning",
        );
        return;
      }
      const setModel = a.match(/^model\s+(\S+)$/);
      if (setModel) {
        const ok = writeDecisionConfig({ ...cfg, model: setModel[1] });
        show(
          ok ? `decision: model ${setModel[1]}` : "decision: write failed",
          ok ? "info" : "warning",
        );
        return;
      }
      const setMargin = a.match(/^margin\s+([0-9.]+)$/);
      if (setMargin) {
        const margin = Number(setMargin[1]);
        if (!Number.isFinite(margin) || margin < 0 || margin > 1) {
          show("decision: margin must be 0..1", "warning");
          return;
        }
        const ok = writeDecisionConfig({ ...cfg, margin });
        show(
          ok ? `decision: margin ${margin}` : "decision: write failed",
          ok ? "info" : "warning",
        );
        return;
      }
      const setRedact = a.match(/^redact\s+(on|off)$/);
      if (setRedact) {
        const redactOn = setRedact[1] === "on";
        const ok = writeDecisionConfig({ ...cfg, redact: redactOn });
        show(
          ok
            ? `decision: redaction ${redactOn ? "on" : "OFF — raw evidence will leave the box"}`
            : "decision: write failed",
          ok ? "info" : "warning",
        );
        return;
      }
      show(
        "decision: unknown arguments — try `status`, `test`, `mode heuristic|http|gemini`, `set <url>`, `key <token>`, `model <id>`, `floor <0-1>`, `margin <0-1>`, `redact on|off`, `off`",
        "warning",
      );
    },
  });
}
