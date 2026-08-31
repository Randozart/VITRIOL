// context-relay — structured handoff across model switches (OmniRoute §4.3,
// REPORT-02 step 20). A raw transcript re-send costs 5-10K tokens; the relay
// card is ~500 tokens of goal/constraints/decisions/paths/errors, generated
// by the OUTGOING model while its engine is still warm (KV reuse) and
// injected once into the incoming model's first call.
//
// SHIPS DARK (config gateway.hermes.context_relay.enabled=false): armed with
// TRIS_RELAY=1. Card file survives engine restarts (switching VITRIOL models
// = restarting serve), which is exactly when the relay matters.
// Kill switch: TRIS_NO_RELAY=1 (double guard when armed globally).

export interface RelayCard {
  from_model: string;
  to_model: string;
  session: string;
  at: number;
  card: string; // the handoff text (<= budget)
  injected: boolean;
}

export const RELAY_BUDGET_CHARS = 1750; // ~500 tok at 3.5 chars/tok (§R2.8)

/** Build the outgoing model's generation prompt from condensed transcript. */
export function buildRelayPrompt(transcript: string): string {
  const t = transcript.slice(0, 12_000);
  return (
    `You are writing a HANDOFF CARD for the next model to continue this session. ` +
    `Output EXACTLY these five sections, <=5 bullet lines each, telegraphic style, no prose:\n` +
    `GOAL: ...\nCONSTRAINTS: ...\nDECISIONS: ...\nFILES/PATHS: ...\nOPEN ERRORS: ...\n\n` +
    `Session transcript (condensed):\n${t}`
  );
}

/** Condense a session transcript: tail user/assistant texts, capped each. */
export function condenseTranscript(
  messages: Array<{ role?: string; content?: unknown }>,
  tailCount = 12,
  perMsg = 400,
): string {
  const keep: string[] = [];
  const relevant = messages.filter((m) => m.role === "user" || m.role === "assistant");
  for (const m of relevant.slice(-tailCount)) {
    const text =
      typeof m.content === "string"
        ? m.content
        : Array.isArray(m.content)
          ? m.content.map((p: { type?: string; text?: string }) => (p?.type === "text" ? p.text ?? "" : "")).join(" ")
          : "";
    const clean = text.replace(/\s+/g, " ").trim().slice(0, perMsg);
    if (clean) keep.push(`${m.role}: ${clean}`);
  }
  return keep.join("\n");
}

/** Validate + cap a generated card; null when unusable. */
export function parseCard(raw: string): string | null {
  const t = (raw ?? "").trim();
  if (!t) return null;
  if (!/GOAL/i.test(t)) return null; // structural contract
  return t.length <= RELAY_BUDGET_CHARS ? t : t.slice(0, RELAY_BUDGET_CHARS) + "\n…[relay card capped]";
}

/** Render the injected tail for the incoming model. */
export function renderRelayTail(card: RelayCard): string {
  return `\n\n## Context relay (handed off from ${card.from_model})\n${card.card}\n[~500-token relay replaces a 5-10K transcript re-send]`;
}

/** True when the card should be injected: for THIS model, not yet consumed. */
export function shouldInject(card: RelayCard, currentModel: string, armed: boolean): boolean {
  if (!armed || card.injected) return false;
  if (!currentModel || !card.to_model) return false;
  return currentModel === card.to_model;
}

/** Relay store path (workspace-relative, survives sessions). */
export function relayPath(dir: string, session: string): string {
  const safe = session.replace(/[^a-zA-Z0-9._-]/g, "_").replace(/\.{2,}/g, "_");
  return `${dir}/${safe || "default"}.json`;
}

/** Engine /v1 completion (fetch), bounded; null on any failure (relay is best-effort). */
export async function generateCard(
  endpoint: string,
  model: string,
  prompt: string,
  fetchImpl: typeof fetch = fetch,
  timeoutMs = 180_000,
): Promise<string | null> {
  try {
    const res = await fetchImpl(`${endpoint.replace(/\/+$/, "")}/v1/chat/completions`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        model,
        messages: [{ role: "user", content: prompt }],
        max_tokens: 700,
        temperature: 0.2,
      }),
      signal: AbortSignal.timeout(timeoutMs),
    });
    if (!res.ok) return null;
    const data = (await res.json()) as { choices?: Array<{ message?: { content?: string; reasoning_content?: string } }> };
    const msg = data.choices?.[0]?.message;
    const text = (msg?.content || msg?.reasoning_content || "").toString(); // || : empty string means "no content" here
    return parseCard(text);
  } catch {
    return null;
  }
}
