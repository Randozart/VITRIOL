// Programmatic model switches (2026-11-18).
//
// `pi.setModel()` is the only way an extension changes the model, and pi
// reports the change to extensions as a `model_select` event whose `source` is
// "set" for BOTH an operator pick and an extension's own call. Routers that
// latch "the operator chose this" need to tell the two apart:
//
//   operator picks DeepSeek in /model   -> latch a pin (never demote silently)
//   llama-cpp-provider syncs the alias  -> do NOT latch (that is startup glue)
//
// The switch moves so fast that a timestamp window is the honest signal:
// an extension announces its intent, calls setModel, and the router checks the
// window inside the resulting model_select. Anything outside the window is a
// human.
//
// globalThis-backed (2026-09-06 jiti isolation rule): each extension loads in
// its own jiti instance, so a module-level `let` would be a different variable
// in the writer and the reader.

const KEY = "__officinaProgrammaticSwitch";
const DEFAULT_WINDOW_MS = 250;

interface SwitchMark {
  at: number;
}

type SwitchHost = { [KEY]?: SwitchMark };

const host = globalThis as unknown as SwitchHost;

/** Announces that the calling extension is about to change the model itself. */
export function markProgrammaticSwitch(now: number = Date.now()): void {
  host[KEY] = { at: now };
}

/**
 * True when a `model_select` arriving now was caused by an extension rather
 * than the operator. `windowMs` tolerates the async hop through pi's event bus.
 */
export function wasProgrammaticSwitch(
  now: number = Date.now(),
  windowMs: number = DEFAULT_WINDOW_MS,
): boolean {
  const mark = host[KEY];
  if (!mark) return false;
  return windowMs < 0 ? false : now - mark.at <= windowMs;
}

/** Test seam: forget the mark so one test cannot leak into the next. */
export function clearProgrammaticSwitch(): void {
  delete host[KEY];
}
