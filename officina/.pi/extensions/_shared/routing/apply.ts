// The ONE place a model switch happens (2026-11-18).
//
// Nine call sites used to call `pi.setModel()` directly: adaptive-router (three,
// demotion/promotion/disable), coupling (two), phase-model, hang-watchdog,
// llama-cpp-provider's startup sync, and resilient-default's virtual route.
// Each carried its own idea of "is this the operator's choice?", which is how
// the same bug class kept coming back:
//
//   - a resumed session never re-latched its pin (restore early-return),
//   - an explicit local pick pinned nothing (returned before the latch),
//   - a non-pool cloud lane was never adopted (candidate-gated latch),
//   - the engine's startup sync looked exactly like an operator pick.
//
// Every switch now goes through `applyModel`, which:
//   1. announces the intent (`markProgrammaticSwitch`) so a router never mistakes
//      an extension's own switch for the operator's — pi reports both as
//      source "set", so the marker is the only signal that exists;
//   2. performs the only `pi.setModel` call in the tree;
//   3. records `lc-route/switch` (owner + reason) so every switch is nameable
//      after the fact, including the ones that fail.
//
// Decision POLICY still lives with the routers (a virtual-model route hook and a
// tier engine have genuinely different inputs); the ACTUATION is single-owner.

import { emitHarnessEvent, harnessEvent } from "../events.ts";
import { markProgrammaticSwitch } from "../model-switch-intent.ts";

export interface SwitchRequest {
  /** Who wanted the switch: "adaptive-router", "coupling", "phase-model", … */
  owner: string;
  /** Why, in the operator's words: "manual pin", "overflow demotion", … */
  reason: string;
}

/** Minimal shape of the model we actually need to name in events. */
interface Switchable {
  provider?: string;
  id?: string;
}

interface SetModelHost {
  // Kept deliberately loose (`any`): pi's real signature is generic over its own
  // Model type, and the callers hold Model | undefined. The module's job is
  // ordering and recording, not type narrowing.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  setModel?: (model: any) => Promise<boolean> | boolean;
}

/** "caduceus/opencode-go/deepseek-v4.1-flash", or "?" when unknown. */
export function modelLabel(model: unknown): string {
  const m = (model ?? {}) as Switchable;
  const provider = (m.provider ?? "").trim();
  const id = (m.id ?? "").trim();
  if (!provider && !id) return "?";
  return id ? `${provider}/${id}` : provider;
}

/**
 * Switch the session's model, with intent announced and the switch recorded.
 *
 * Returns whether pi accepted it (false also when `pi.setModel` throws or is
 * absent — a refused switch must never break a turn).
 */
export async function applyModel(
  pi: SetModelHost,
  model: unknown,
  request: SwitchRequest,
): Promise<boolean> {
  if (!pi.setModel) return false;
  markProgrammaticSwitch();
  let ok = false;
  try {
    ok = !!(await pi.setModel(model));
  } catch {
    ok = false;
  }
  emitHarnessEvent(
    harnessEvent("lc-route", "switch", {
      detail: `${request.owner}: ${modelLabel(model)} — ${request.reason}${
        ok ? "" : " (refused)"
      }`,
    }),
  );
  return ok;
}
