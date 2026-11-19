import { describe, it, expect, afterEach, vi } from "vitest";
import setupProvider from "./index.ts";
import { streamLocalWithProgress } from "./progress.ts";

// End-to-end test of the issue #54 swap-time re-probe: drive the real default
// export (which reads the shipped models.json, so the engine providers are
// present) with a fake pi and a stubbed global fetch, then fire a model_select
// to confirm the provider is re-registered with the fresh window and the user
// is notified.
//
// A2 alias period (2026-11-19): the engine now ships under BOTH `vitriol`
// (canonical) and `llamacpp` (legacy alias), so every registration happens
// twice-once per spelling-and these tests assert the INVARIANT rather than a
// spelling: both engine names carry the same fresh window and the same stream
// handler. When the legacy id is deleted, these keep passing.

function fakePi() {
  const registers: {
    name: string;
    ctx: number | undefined;
    stream: unknown;
  }[] = [];
  let modelSelect: ((event: any, ctx: any) => any) | undefined;
  const pi = {
    registerProvider(name: string, config: any) {
      registers.push({
        name,
        ctx: config.models?.[0]?.contextWindow,
        stream: config.streamSimple,
      });
    },
    on(event: string, handler: (event: any, ctx: any) => any) {
      if (event === "model_select") modelSelect = handler;
    },
  };
  return { pi, registers, fire: (e: any, c: any) => modelSelect?.(e, c) };
}

// Return a stub `fetch` that yields a different n_ctx on each successive call,
// so the startup probe and the swap probe see different windows.
function fetchReturning(...nctxSequence: number[]) {
  let i = 0;
  return vi.fn(async () => {
    const n_ctx = nctxSequence[Math.min(i, nctxSequence.length - 1)];
    i++;
    return {
      ok: true,
      json: async () => ({ default_generation_settings: { n_ctx } }),
    } as any;
  });
}

describe("llama-cpp-provider swap re-probe (issue #54)", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    delete process.env.LITTLE_CODER_NO_CTX_PROBE;
  });

  it("re-registers llamacpp with the new window and notifies on swap", async () => {
    vi.stubGlobal("fetch", fetchReturning(32768, 131072));
    const { pi, registers, fire } = fakePi();
    await setupProvider(pi as any);

    // Startup registered the engine at the first probed window, under BOTH
    // spellings (canonical vitriol + legacy llamacpp). Registration ORDER is not
    // a contract (models.json key order feeds it), so assert the invariant:
    // every engine registration carries the startup window, and both names exist.
    const startup = registers.filter(
      (r) => r.name === "vitriol" || r.name === "llamacpp",
    );
    expect(startup.length).toBeGreaterThanOrEqual(2);
    expect(startup.every((r) => r.ctx === 32768)).toBe(true);
    expect(new Set(startup.map((r) => r.name)).size).toBe(2);

    const notes: string[] = [];
    await fire(
      {
        model: { provider: "llamacpp", id: "m2" },
        previousModel: { provider: "llamacpp", id: "m1" },
        source: "cycle",
      },
      { ui: { notify: (m: string) => notes.push(m) } },
    );

    // Swap re-registered the engine at the new window, under BOTH spellings,
    // with a notice. Startup registrations at the old window remain in the log
    // (append-only), so filter to the LAST registration per engine name — the
    // invariant is "the newest registration under every engine spelling carries
    // the fresh window and the real stream handler". Non-engine providers are
    // untouched (stream stays undefined).
    const after = registers.filter(
      (r) => r.name === "vitriol" || r.name === "llamacpp",
    );
    const latestPerName = new Map<
      string,
      { ctx: number | undefined; stream: unknown }
    >();
    for (const r of after) latestPerName.set(r.name, r);
    expect([...latestPerName.values()].every((r) => r.ctx === 131072)).toBe(
      true,
    );
    expect(
      [...latestPerName.values()].every(
        (r) => r.stream === streamLocalWithProgress,
      ),
    ).toBe(true);
    expect(
      registers
        .filter((r) => r.name !== "vitriol" && r.name !== "llamacpp")
        .every((r) => r.stream === undefined),
    ).toBe(true);
    expect(notes).toEqual(["context window updated 32k → 128k"]);
  });

  it("does nothing when the window is unchanged after a swap", async () => {
    vi.stubGlobal("fetch", fetchReturning(65536, 65536));
    const { pi, registers, fire } = fakePi();
    await setupProvider(pi as any);
    // Both engine spellings register at startup; neither may gain a registration
    // when the probed window is unchanged.
    const before = registers.filter(
      (r) => r.name === "vitriol" || r.name === "llamacpp",
    ).length;

    const notes: string[] = [];
    await fire(
      {
        model: { provider: "llamacpp", id: "m2" },
        previousModel: { provider: "llamacpp", id: "m1" },
        source: "cycle",
      },
      { ui: { notify: (m: string) => notes.push(m) } },
    );

    expect(
      registers.filter((r) => r.name === "vitriol" || r.name === "llamacpp").length,
    ).toBe(before);
    expect(notes).toEqual([]);
  });

  it("ignores the initial selection (previousModel undefined) and non-llamacpp models", async () => {
    vi.stubGlobal("fetch", fetchReturning(32768, 131072));
    const { pi, registers, fire } = fakePi();
    await setupProvider(pi as any);
    const before = registers.filter((r) => r.name === "llamacpp").length;
    const notes: string[] = [];
    const ctx = { ui: { notify: (m: string) => notes.push(m) } };

    // Initial selection of a llamacpp model — previousModel undefined → skip.
    await fire(
      {
        model: { provider: "llamacpp", id: "m1" },
        previousModel: undefined,
        source: "set",
      },
      ctx,
    );
    // Swap to a non-llamacpp model → skip.
    await fire(
      {
        model: { provider: "ollama", id: "q" },
        previousModel: { provider: "llamacpp", id: "m1" },
        source: "cycle",
      },
      ctx,
    );

    expect(registers.filter((r) => r.name === "llamacpp").length).toBe(before);
    expect(notes).toEqual([]);
  });

  it("does not register a model_select hook when probing is disabled", async () => {
    process.env.LITTLE_CODER_NO_CTX_PROBE = "1";
    vi.stubGlobal("fetch", fetchReturning(131072));
    const { pi, fire } = fakePi();
    await setupProvider(pi as any);
    // No handler captured → fire is a no-op returning undefined.
    expect(
      await fire(
        {
          model: { provider: "llamacpp", id: "m2" },
          previousModel: { provider: "llamacpp", id: "m1" },
          source: "cycle",
        },
        { ui: { notify: () => {} } },
      ),
    ).toBeUndefined();
  });
});
