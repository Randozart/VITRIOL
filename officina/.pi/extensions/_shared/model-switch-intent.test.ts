// Programmatic-switch intent (2026-11-18).
//
// The router latches "the operator chose this model" from pi's model_select
// event, but pi reports source "set" identically for an operator pick and for
// an extension's own setModel call. The marker separates the two; these tests
// pin the window semantics and the globalThis contract (jiti isolation).

import { afterEach, describe, expect, it, vi } from "vitest";
import {
  clearProgrammaticSwitch,
  markProgrammaticSwitch,
  wasProgrammaticSwitch,
} from "./model-switch-intent.ts";

describe("model-switch intent", () => {
  afterEach(() => clearProgrammaticSwitch());

  it("is silent until someone announces a switch", () => {
    expect(wasProgrammaticSwitch(1_000)).toBe(false);
  });

  it("reports a switch announced moments ago", () => {
    markProgrammaticSwitch(1_000);
    expect(wasProgrammaticSwitch(1_050)).toBe(true);
  });

  it("expires once the event has surely landed", () => {
    // A human pick minutes later must not inherit a stale mark.
    markProgrammaticSwitch(1_000);
    expect(wasProgrammaticSwitch(1_000 + 5_000)).toBe(false);
  });

  it("honours a tighter window when the caller asks for one", () => {
    markProgrammaticSwitch(1_000);
    expect(wasProgrammaticSwitch(1_040, 20)).toBe(false);
    expect(wasProgrammaticSwitch(1_040, 1_000)).toBe(true);
  });

  it("survives a second module instance (jiti isolation)", async () => {
    // pi loads each extension through its own jiti instance with
    // moduleCache: false — the writer and the reader are different module
    // objects. Only the globalThis cell makes the contract hold.
    markProgrammaticSwitch(2_000);
    vi.resetModules();
    const fresh = await import("./model-switch-intent.ts");
    expect(fresh.wasProgrammaticSwitch(2_010)).toBe(true);
  });
});
