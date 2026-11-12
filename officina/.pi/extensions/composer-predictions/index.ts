// composer-predictions v2 (voice-suggestions stage 1, 2026-11-12). After a
// turn ends: heuristic next-step (existing rules, priority I3) or an LLM
// draft in the operator's voice, staged as prompt-suggestion.json for the
// TUI ghost text. Advisory only; kill switches independent:
//   OFFICINA_NO_PREDICTIONS=1    removes the whole extension (widget + staging)
//   OFFICINA_NO_PROMPT_SUGGEST=1 keeps the widget, stops staging drafts

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { dirtyCount, predict, readVerifyLast } from "./predict.ts";
import {
  buildGenPrompt,
  clearSuggestion,
  makeSuggestionId,
  resolveSuggestion,
  SUGGESTION_TTL_MS,
  writeSuggestion,
  type GenSignals,
} from "./suggest.ts";
import { pickJudgeModel } from "../hang-watchdog/judge.ts";
import { extractCatalogModels, freeToolCandidates } from "../caduceus/catalog.ts";

const OFFICINA_DIR = join(homedir(), ".vitriol", "officina");
const CATALOG = join(OFFICINA_DIR, "caduceus-models.json");
const DEFAULT_BASE_URL = "http://127.0.0.1:20128/v1";
const JUDGE_TIMEOUT_MS = 20_000;
const WIDGET_KEY = "composer-prediction";

function resolveBaseUrl(): string {
  if (process.env.OFFICINA_CADUCEUS_URL) {
    return String(process.env.OFFICINA_CADUCEUS_URL).replace(/\/+$/, "");
  }
  try {
    const path = join(OFFICINA_DIR, "gateway-url");
    if (existsSync(path)) {
      const fromFile = readFileSync(path, "utf8").trim();
      if (fromFile) return fromFile.replace(/\/+$/, "");
    }
  } catch {
    // fall through
  }
  return DEFAULT_BASE_URL;
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

async function generateDraft(model: string, prompt: string): Promise<string | null> {
  const key = String(process.env.OFFICINA_CADUCEUS_API_KEY ?? "").trim();
  if (!key) return null;
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), JUDGE_TIMEOUT_MS);
  try {
    const res = await fetch(`${resolveBaseUrl()}/chat/completions`, {
      method: "POST",
      headers: { "content-type": "application/json", authorization: `Bearer ${key}` },
      body: JSON.stringify({
        model,
        messages: [{ role: "user", content: prompt }],
        max_tokens: 120,
        temperature: 0.3,
      }),
      signal: controller.signal,
    });
    if (!res.ok) return null;
    const doc = (await res.json()) as { choices?: Array<{ message?: { content?: string } }> };
    return doc.choices?.[0]?.message?.content ?? null;
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}

function readVoiceProfile(): string | null {
  try {
    const doc = JSON.parse(readFileSync(join(OFFICINA_DIR, "voice-profile.json"), "utf8")) as {
      profile?: string;
    };
    return typeof doc.profile === "string" ? doc.profile : null;
  } catch {
    return null;
  }
}

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_NO_PREDICTIONS === "1") return;

  const suggestEnabled = process.env.OFFICINA_NO_PROMPT_SUGGEST !== "1";
  let cwd = process.cwd();
  let ui: any = null;
  let lastLine = "";
  let lastAssistantTail = "";
  let inRun = false;

  const setPrediction = (text: string | null): void => {
    const line = text ?? "";
    if (line === lastLine) return;
    lastLine = line;
    try {
      ui?.setWidget?.(WIDGET_KEY, line ? [line] : [], { placement: "belowEditor" });
    } catch {
      // decoration contract: never break the workshop
    }
  };

  const refresh = (): number => {
    const verify = readVerifyLast(join(cwd, ".officina"));
    const dirty = dirtyCount((args) => {
      try {
        const res = spawnSync("git", args, { cwd, encoding: "utf8", timeout: 5_000 });
        if (res.error || res.status !== 0) return "";
        return String(res.stdout ?? "");
      } catch {
        return "";
      }
    });
    setPrediction(predict(verify, dirty, Date.now()));
    return dirty;
  };

  pi.on("session_start", (_event, ctx) => {
    const c = (ctx as { cwd?: string; hasUI?: boolean }).cwd;
    if (c) cwd = c;
    ui = (ctx as { hasUI?: boolean; ui?: unknown }).hasUI ? (ctx as { ui: any }).ui : null;
    lastLine = "§";
    setPrediction(null);
    lastAssistantTail = "";
    clearSuggestion(OFFICINA_DIR); // stale ghost from a dead session must not haunt
  });

  // Track the assistant's closing words so the draft knows where the work
  // actually landed (message_end carries role + content).
  pi.on("message_end", (event) => {
    const message = (event as { message?: { role?: string; content?: unknown } }).message;
    if (!message || message.role !== "assistant") return;
    const content = message.content;
    let text = "";
    if (typeof content === "string") text = content;
    else if (Array.isArray(content)) {
      for (const b of content as Array<{ type?: string; text?: unknown }>) {
        if (b?.type === "text" && typeof b.text === "string") text += b.text;
      }
    }
    if (text.trim()) lastAssistantTail = text.trim().slice(-500);
  });

  pi.on("before_agent_start", () => {
    inRun = true;
  });

  const countDirty = (): number => {
    try {
      const res = spawnSync("git", ["status", "--porcelain"], {
        cwd,
        encoding: "utf8",
        timeout: 5_000,
      });
      if (res.error || res.status !== 0) return -1;
      return String(res.stdout ?? "").split("\n").filter((l) => l.trim()).length;
    } catch {
      return -1;
    }
  };

  pi.on("agent_end", () => {
    inRun = false;
    const verify = readVerifyLast(join(cwd, ".officina"));
    const dirty = countDirty();
    const heuristic = predict(verify, dirty, Date.now());
    setPrediction(heuristic);
    if (!suggestEnabled) return;
    void (async () => {
      try {
        const signals: GenSignals = {
          lastAssistantTail,
          verifyKnown: !!verify,
          verifyOk: verify?.ok,
          dirtyFiles: dirty >= 0 ? dirty : undefined,
        };
        const model = bestFreeLane();
        const draft = model
          ? await generateDraft(model, buildGenPrompt(signals, readVoiceProfile()))
          : null;
        const resolved = resolveSuggestion(heuristic, draft);
        if (!resolved || inRun) return;
        writeSuggestion(OFFICINA_DIR, {
          version: 1,
          id: makeSuggestionId(),
          text: resolved.text,
          source: resolved.source,
          created: new Date().toISOString(),
          expires: Date.now() + SUGGESTION_TTL_MS,
        });
      } catch {
        // decoration contract
      }
    })();
  });
}
