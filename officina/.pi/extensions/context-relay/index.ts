import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import {
  buildRelayPrompt,
  condenseTranscript,
  generateCard,
  parseCard,
  relayPath,
  renderRelayTail,
  shouldInject,
  type RelayCard,
} from "./relay.ts";

// context-relay — handoff card across model switches (§4.3, step 20).
// On model_select the OUTGOING model (engine still warm) generates a
// structured GOAL/CONSTRAINTS/DECISIONS/PATHS/ERRORS card from the condensed
// transcript; the INCOMING model gets it injected once as a tail message.
// Raw transcript re-send: 5-10K tokens; relay: ~500. Card file survives
// engine restarts (VITRIOL model switch = serve restart).
//
// SHIPS DARK: TRIS_RELAY=1 arms (config context_relay.enabled default false).
// Kill: TRIS_NO_RELAY=1. Layer: scaffold owns context entry (Rule 2) — relay
// injection is a tail append, cache-safe by construction (Rule 7).

export default function (pi: ExtensionAPI) {
  const armed = process.env.TRIS_RELAY === "1" && process.env.TRIS_NO_RELAY !== "1";
  if (!armed) return;

  const dir = process.env.TRIS_RELAY_DIR || ".pi/relay";
  const endpoint = (process.env.VITRIOL_BASE_URL || "http://127.0.0.1:8279/v1").replace(/\/v1$/, "");
  let sessionFile = "";
  let sessionStem = "session";
  let generating = false;

  function readTranscript(): Array<{ role?: string; content?: unknown }> {
    if (!sessionFile) return [];
    try {
      const out: Array<{ role?: string; content?: unknown }> = [];
      for (const line of readFileSync(sessionFile, "utf8").split("\n")) {
        if (!line.trim()) continue;
        try {
          const e = JSON.parse(line) as { message?: { role?: string; content?: unknown } };
          if (e.message?.role) out.push(e.message);
        } catch {
          // torn jsonl line — skip, relay is best-effort
        }
      }
      return out;
    } catch {
      return [];
    }
  }

  function loadCard(): RelayCard | null {
    try {
      const c = JSON.parse(readFileSync(relayPath(dir, sessionStem), "utf8")) as RelayCard;
      return parseCard(c.card ?? "") ? c : null;
    } catch {
      return null;
    }
  }

  function saveCard(c: RelayCard): void {
    try {
      mkdirSync(dirname(relayPath(dir, sessionStem)), { recursive: true });
      writeFileSync(relayPath(dir, sessionStem), JSON.stringify(c, null, 2));
    } catch {
      // store trouble: no relay this switch, conversation continues
    }
  }

  pi.on("session_start", async (_event, ctx) => {
    const sm = (ctx as { sessionManager?: { getSessionFile?: () => string | null } }).sessionManager;
    sessionFile = sm?.getSessionFile?.() ?? "";
    sessionStem = sessionFile.split("/").pop()?.replace(/\.jsonl$/, "") || "session";
  });

  pi.on("model_select", async (event, ctx) => {
    const e = event as { model?: { id?: string }; previousModel?: { id?: string } };
    const from = e.previousModel?.id;
    const to = e.model?.id;
    if (!from || !to || from === to || generating) return;
    generating = true;
    try {
      const transcript = condenseTranscript(readTranscript());
      if (!transcript) return;
      const card = await generateCard(endpoint, from, buildRelayPrompt(transcript));
      if (!card) {
        ctx.ui.notify("context-relay: outgoing model produced no card — cold start for the new model", "warning");
        return;
      }
      saveCard({ from_model: from, to_model: to, session: sessionStem, at: Date.now(), card, injected: false });
      emitHarnessEvent(harnessEvent("lc-relay", "card-generated", { detail: `${from}->${to} ${card.length}ch`, session: sessionStem }));
      ctx.ui.notify(`context-relay: handoff card generated (${card.length} chars) ${from} -> ${to}`, "info");
    } finally {
      generating = false;
    }
  });

  pi.on("context", async (event, ctx) => {
    const card = loadCard();
    if (!card) return undefined;
    const current = (ctx as { model?: { id?: string } }).model?.id ?? "";
    if (!shouldInject(card, current, armed)) return undefined;
    const tail = {
      role: "custom" as const,
      customType: "lc-relay",
      content: renderRelayTail(card),
      display: false,
      details: {},
      timestamp: Date.now(),
    };
    saveCard({ ...card, injected: true }); // consumed exactly once
    emitHarnessEvent(harnessEvent("lc-relay", "card-injected", { detail: current, session: sessionStem }));
    return { messages: [...event.messages, tail] };
  });
}
