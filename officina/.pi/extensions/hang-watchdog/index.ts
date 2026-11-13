// hang-watchdog (2026-11-12). Watches long-running bash calls; when output
// stalls it escalates: a notice first, then the SESSION'S LANE judges
// liveliness — a tiny free-lane completion over the gateway (never the busy
// agent loop). INVARIANT: it never kills, never aborts, never injects into
// the turn — the operator reads the verdict and presses Esc themselves.
// Kill switch: OFFICINA_NO_HANG_WATCHDOG=1.

import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import {
  buildJudgePrompt,
  MAX_JUDGMENTS_PER_CALL,
  REJUDGE_EVERY_S,
  JUDGE_S_DEFAULT,
  parseVerdict,
  pickJudgeModel,
  SOFT_S_DEFAULT,
  TICK_S,
  tickCall,
  type TickAction,
  type WatchedCall,
} from "./judge.ts";
import { extractCatalogModels, freeToolCandidates } from "../caduceus/catalog.ts";

const DEFAULT_BASE_URL = "http://127.0.0.1:20128/v1";
const CATALOG = join(homedir(), ".vitriol", "officina", "caduceus-models.json");
const JUDGE_TIMEOUT_MS = 30_000;
const TAIL_LINES = 40;

function envInt(name: string, fallback: number): number {
  const n = Number(process.env[name]);
  return Number.isFinite(n) && n > 0 ? n : fallback;
}

function gatewayBase(): string {
  const fromEnv = String(process.env.OFFICINA_CADUCEUS_URL ?? "").trim();
  if (fromEnv) return fromEnv.replace(/\/+$/, "");
  return DEFAULT_BASE_URL;
}

function resolveBaseUrl(): string {
  // env wins (TUI parity), then the gateway-url file, then the default.
  if (process.env.OFFICINA_CADUCEUS_URL) return gatewayBase();
  try {
    const path = join(homedir(), ".vitriol", "officina", "gateway-url");
    if (existsSync(path)) {
      const fromFile = readFileSync(path, "utf8").trim();
      if (fromFile) return fromFile.replace(/\/+$/, "");
    }
  } catch {
    // fall through to default
  }
  return gatewayBase();
}

/** The judgment completion: best free lane, tiny budget, strict timeout.
 *  Returns the raw reply text or null (judge unavailable → fail-open). */
async function judgeWithLane(model: string, prompt: string): Promise<string | null> {
  const key = String(process.env.OFFICINA_CADUCEUS_API_KEY ?? "").trim();
  if (!key) return null;
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), JUDGE_TIMEOUT_MS);
  try {
    const res = await fetch(`${resolveBaseUrl()}/chat/completions`, {
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
      signal: controller.signal,
    });
    if (!res.ok) return null;
    const doc = (await res.json()) as {
      choices?: Array<{ message?: { content?: string } }>;
    };
    return doc.choices?.[0]?.message?.content ?? null;
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}

function bestFreeLane(): string | null {
  try {
    const raw = JSON.parse(readFileSync(CATALOG, "utf8"));
    const ids = freeToolCandidates(extractCatalogModels(raw)).map((c) => c.id);
    return pickJudgeModel(ids);
  } catch {
    return null;
  }
}

/** Structured watchdog facts for the TUI card + countdown (2026-11-13).
 *  Written beside every notify; the TUI renders fields, not prose, and
 *  shows "watchdog in Ns" until next_check_at. */
const WATCHDOG_FILE = join(homedir(), ".vitriol", "officina", "watchdog.json");

interface WatchdogDoc {
  version: 1;
  ts: number;
  kind: "notice" | "judge" | "verdict" | "unavailable";
  command: string;
  elapsed_s: number;
  stalled_s: number;
  verdict: "JUDGING" | "ALIVE" | "HUNG" | "";
  reason: string;
  next_check_at: number;
  phase: "soft" | "judge" | "rejudge" | "done";
}

function writeWatchdog(doc: Omit<WatchdogDoc, "version" | "ts">): void {
  try {
    mkdirSync(join(homedir(), ".vitriol", "officina"), { recursive: true });
    const full: WatchdogDoc = { version: 1, ts: Date.now(), ...doc };
    const tmp = `${WATCHDOG_FILE}.tmp-${process.pid}`;
    writeFileSync(tmp, `${JSON.stringify(full, null, 2)}\n`);
    renameSync(tmp, WATCHDOG_FILE);
  } catch {
    // decoration contract
  }
}

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_NO_HANG_WATCHDOG === "1") return;

  const softS = envInt("OFFICINA_HANG_SOFT_S", SOFT_S_DEFAULT);
  const judgeS = envInt("OFFICINA_HANG_JUDGE_S", JUDGE_S_DEFAULT);
  const watched = new Map<string, WatchedCall>();
  const tails = new Map<string, string>();
  let ui: any = null;

  const surface = (message: string, level: string): void => {
    try {
      ui?.notify?.(message, level);
    } catch {
      // decoration contract
    }
  };

  const handleTick = async (call: WatchedCall, action: TickAction): Promise<void> => {
    if (action.kind === "none" || action.kind === "alive") return;
    const elapsedS = Math.floor((Date.now() - call.startedAt) / 1000);
    const stalledS = Math.floor((Date.now() - (call.stalledSince ?? Date.now())) / 1000);
    if (action.kind === "notice") {
      writeWatchdog({
        kind: "notice",
        command: call.command,
        elapsed_s: elapsedS,
        stalled_s: stalledS,
        verdict: "",
        reason: "",
        next_check_at: Date.now() + Math.max(0, judgeS - stalledS) * 1000,
        phase: "soft",
      });
      surface(action.message, "warning");
      return;
    }
    // judge — never the busy agent loop: a separate tiny completion on a
    // free lane. Fail-open: an unavailable judge changes nothing.
    writeWatchdog({
      kind: "judge",
      command: call.command,
      elapsed_s: elapsedS,
      stalled_s: stalledS,
      verdict: "JUDGING",
      reason: "",
      next_check_at: Date.now() + 30_000,
      phase: "judge",
    });
    const model = bestFreeLane();
    if (!model) {
      writeWatchdog({
        kind: "unavailable",
        command: call.command,
        elapsed_s: elapsedS,
        stalled_s: stalledS,
        verdict: "",
        reason: "no free lane in catalog cache",
        next_check_at: 0,
        phase: "done",
      });
      surface(`${action.message} · (judge unavailable — no free lane in catalog cache)`, "warning");
      return;
    }
    const prompt = buildJudgePrompt(
      call.command,
      Math.floor((Date.now() - call.startedAt) / 1000),
      Math.floor((Date.now() - (call.stalledSince ?? Date.now())) / 1000),
      action.tail,
    );
    const reply = await judgeWithLane(model, prompt);
    if (reply === null) {
      writeWatchdog({
        kind: "unavailable",
        command: call.command,
        elapsed_s: elapsedS,
        stalled_s: stalledS,
        verdict: "",
        reason: "judge unreachable (assuming alive)",
        next_check_at: 0,
        phase: "done",
      });
      surface(`${action.message} · (judge unreachable — assuming alive)`, "warning");
      return;
    }
    const verdict = parseVerdict(reply);
    call.lastVerdict = verdict.alive ? "ALIVE" : "HUNG";
    writeWatchdog({
      kind: "verdict",
      command: call.command,
      elapsed_s: Math.floor((Date.now() - call.startedAt) / 1000),
      stalled_s: Math.floor((Date.now() - (call.stalledSince ?? Date.now())) / 1000),
      verdict: verdict.alive ? "ALIVE" : "HUNG",
      reason: verdict.reason,
      next_check_at:
        call.judgments >= MAX_JUDGMENTS_PER_CALL ? 0 : Date.now() + REJUDGE_EVERY_S * 1000,
      phase: call.judgments >= MAX_JUDGMENTS_PER_CALL ? "done" : "rejudge",
    });
    surface(
      `hang-watchdog verdict for "${call.command.slice(0, 60)}": ${verdict.alive ? "ALIVE" : "HUNG"} — ${verdict.reason} ` +
        "(no auto-kill; Esc aborts if you agree)",
      verdict.alive ? "info" : "warning",
    );
  };

  pi.on("session_start", (_event, ctx) => {
    ui = (ctx as { hasUI?: boolean; ui?: unknown }).hasUI ? (ctx as { ui: any }).ui : null;
    watched.clear();
    tails.clear();
  });

  pi.on("tool_execution_start", (event) => {
    const e = event as { toolCallId?: string; toolName?: string; args?: Record<string, unknown> };
    const name = String(e.toolName ?? "").toLowerCase();
    if (name !== "bash") return;
    const command = String(e.args?.command ?? e.args?.cmd ?? "");
    if (!command) return;
    watched.set(String(e.toolCallId), {
      toolCallId: String(e.toolCallId),
      command,
      startedAt: Date.now(),
      lastOutputChars: 0,
      stalledSince: null,
      judgments: 0,
      lastVerdict: null,
    });
    tails.set(String(e.toolCallId), "");
  });

  pi.on("tool_execution_update", (event) => {
    const e = event as { toolCallId?: string; partialResult?: unknown };
    const id = String(e.toolCallId ?? "");
    if (!watched.has(id)) return;
    const pr = e.partialResult;
    const text =
      typeof pr === "string"
        ? pr
        : Array.isArray(pr)
          ? pr
              .map((b) => (typeof b === "string" ? b : ((b as { text?: string })?.text ?? "")))
              .join("\n")
          : "";
    tails.set(id, String(text).split("\n").slice(-TAIL_LINES).join("\n"));
  });

  pi.on("tool_execution_end", (event) => {
    const id = String((event as { toolCallId?: string }).toolCallId ?? "");
    watched.delete(id);
    tails.delete(id);
  });

  const timer = setInterval(() => {
    const now = Date.now();
    for (const call of watched.values()) {
      const tail = tails.get(call.toolCallId) ?? "";
      const action = tickCall(call, tail.length, tail, {
        now,
        softS,
        judgeS,
      });
      void handleTick(call, action);
    }
  }, TICK_S * 1000);
  timer.unref?.();
}
