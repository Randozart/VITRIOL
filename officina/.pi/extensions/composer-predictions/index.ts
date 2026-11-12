// composer-predictions (codex-qol stage 8 → voice-suggestions stages 1+3,
// 2026-11-12). After each turn: the advisory widget line (heuristic rules),
// plus a voice-matched next-prompt staged for the TUI's ghost text, plus
// slow-cadence voice learning. Advisory only; kill switches:
//   OFFICINA_NO_PREDICTIONS=1    removes everything
//   OFFICINA_NO_PROMPT_SUGGEST=1 keeps the widget, stops staging drafts
//   OFFICINA_NO_VOICE_LEARN=1    stops distillation (suggestions keep working)

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
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
import { classifyTurn } from "../auto-route/classifier.ts";
import {
  acceptanceBlock,
  buildDistillPrompt,
  parseProfile,
  readProfile,
  shouldDistill,
} from "./voice.ts";
import { extractCatalogModels, freeToolCandidates } from "../caduceus/catalog.ts";

const WIDGET_KEY = "composer-prediction";
const OFFICINA_DIR = join(homedir(), ".vitriol", "officina");
const CATALOG = join(OFFICINA_DIR, "caduceus-models.json");
const PROFILE_PATH = join(OFFICINA_DIR, "voice-profile.json");
const EVENTS_PATH = join(OFFICINA_DIR, "prompt-suggest-events.jsonl");
const DEFAULT_BASE_URL = "http://127.0.0.1:20128/v1";
const JUDGE_TIMEOUT_MS = 20_000;

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
    const doc = JSON.parse(readFileSync(PROFILE_PATH, "utf8")) as { profile?: string };
    return typeof doc.profile === "string" ? doc.profile : null;
  } catch {
    return null;
  }
}

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_NO_PREDICTIONS === "1") return;

  const suggestEnabled = process.env.OFFICINA_NO_PROMPT_SUGGEST !== "1";
  const learnEnabled = process.env.OFFICINA_NO_VOICE_LEARN !== "1";
  let cwd = process.cwd();
  let ui: any = null;
  let lastLine = "";
  let lastAssistantTail = "";
  let inRun = false;
  // I4: only privacy-SAFE prompts are kept, and ONLY they reach a cloud lane
  // at distill time. Session-scoped; never persisted raw.
  const safePrompts: string[] = [];
  const acceptedSuggestions: string[] = [];

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

  /** Slow-cadence voice distillation (stage 3). Free lane, day-capped by
   *  shouldDistill; failures are silent — suggestions keep working from the
   *  last profile. */
  const maybeDistill = async (): Promise<void> => {
    if (process.env.OFFICINA_NO_VOICE_LEARN === "1") return;
    try {
      const profile = readProfile(PROFILE_PATH);
      const gate = shouldDistill(profile, safePrompts.length);
      if (!gate.ok) return;
      const model = bestFreeLane();
      if (!model) return;
      let events: Array<{ verdict?: string; source?: string }> = [];
      try {
        events = readFileSync(EVENTS_PATH, "utf8")
          .trim()
          .split("\n")
          .slice(-200)
          .map((l) => {
            try {
              return JSON.parse(l) as { verdict?: string; source?: string };
            } catch {
              return {} as { verdict?: string; source?: string };
            }
          });
      } catch {
        events = [];
      }
      const prompt = buildDistillPrompt(profile, safePrompts, acceptedSuggestions);
      const reply = await generateDraft(model, prompt);
      const updated = parseProfile(reply);
      if (!updated) return;
      const doc = {
        version: 1 as const,
        updatedAt: new Date().toISOString(),
        profile: updated,
        samples: safePrompts.length,
        acceptance: acceptanceBlock(events),
      };
      const tmp = join(tmpdir(), `voice-profile-${process.pid}.json`);
      writeFileSync(tmp, `${JSON.stringify(doc, null, 2)}\n`);
      renameSync(tmp, PROFILE_PATH);
    } catch {
      // learning is decoration
    }
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

  pi.on("message_end", (event) => {
    const message = (event as { message?: { role?: string; content?: unknown } }).message;
    if (!message) return;
    const content = message.content;
    const textOf = (): string => {
      if (typeof content === "string") return content;
      if (Array.isArray(content)) {
        let t = "";
        for (const b of content as Array<{ type?: string; text?: unknown }>) {
          if (b?.type === "text" && typeof b.text === "string") t += b.text;
        }
        return t;
      }
      return "";
    };
    if (message.role === "user" && learnEnabled) {
      const text = textOf().trim();
      if (text && !text.startsWith("/")) {
        try {
          const cls = classifyTurn({
            promptText: text,
            recentErrorCount: 0,
            churnLoops: 0,
            scratchpadContextLines: [],
            filesTouched: 0,
            turnCount: 0,
          });
          // I4 fail-closed: classifier failure = exclude from learning.
          if (cls.privacy === "safe" && safePrompts.length < 200) {
            safePrompts.push(text.slice(0, 400));
          }
        } catch {
          // exclude
        }
      }
      return;
    }
    if (message.role === "assistant") {
      const text = textOf().trim();
      if (text) lastAssistantTail = text.slice(-500);
    }
  });

  pi.on("before_agent_start", () => {
    inRun = true;
  });

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
        // Learn taste from what the lane produced (acceptance events land
        // separately from the TUI when the operator accepts).
        if (resolved.source === "llm" && acceptedSuggestions.length < 100) {
          acceptedSuggestions.push(resolved.text);
        }
        await maybeDistill();
      } catch {
        // decoration contract
      }
    })();
  });
}
