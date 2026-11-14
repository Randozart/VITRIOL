// Provider guard (2026-11-14) — structural validation of provider payloads.
//
// Part of the "guaranteeing Pi doesn't have bugs" program: malformed provider
// data becomes a typed violation (lc-guard event + one warning per code)
// instead of a crash or a silent mis-parse. Advisory only; never blocks a
// response. Kill switch: OFFICINA_NO_PROVIDER_GUARD=1.

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import {
  looksLikeCompletion,
  renderViolations,
  validateChatCompletion,
  validateStreamChunk,
} from "../_shared/provider-validate.ts";

const NOTICE_WINDOW_MS = 60_000;

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_NO_PROVIDER_GUARD === "1") return;
  const seen = new Map<string, number>();

  const report = (ctx: { ui?: { notify?: (t: string, k?: string) => unknown } }, code: string, detail: string, when: number): void => {
    emitHarnessEvent(harnessEvent("lc-guard", "violation", { detail: `${code}: ${detail}` }));
    const last = seen.get(code) ?? 0;
    if (when - last < NOTICE_WINDOW_MS) return;
    seen.set(code, when);
    try {
      void ctx.ui?.notify?.(`provider payload violation — ${code}: ${detail}`, "warning");
    } catch {
      // decoration contract
    }
  };

  pi.on("provider_stream_event", (event, ctx) => {
    const data = (event as { data?: unknown }).data;
    if (!looksLikeCompletion(data)) return;
    // Streaming chunks carry choices[].delta; complete payloads carry
    // choices[].message. Validate whichever shape it is.
    const doc = data as { choices?: Array<{ delta?: unknown; message?: unknown }> };
    const first = Array.isArray(doc.choices) ? doc.choices[0] : undefined;
    const validation = first && "message" in first
      ? validateChatCompletion(data)
      : validateStreamChunk(data);
    if (!validation.ok) {
      report(ctx as never, validation.violations[0]?.code ?? "unknown", renderViolations(validation.violations), Date.now());
    }
  });
}
