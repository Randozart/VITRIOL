import { describe, expect, it } from "vitest";
import {
  busySlots,
  parseLoadedModel,
  counterDelta,
  fireLoad,
  gpuFireLoad,
  fmtRate,
  fmtTokens,
  parseMetrics,
  parseSlots,
  renderBar,
} from "./decode.ts";

const METRICS = `# HELP llamacpp:prompt_tokens_total Number of prompt tokens processed.
llamacpp:prompt_tokens_total 17500
llamacpp:n_decode_total 812
llamacpp:n_tokens_max 4096
garbage line without number
`;

describe("parseMetrics", () => {
  it("parses the fork's counter names and ignores everything else", () => {
    expect(parseMetrics(METRICS)).toEqual({ promptTokens: 17500, decodeTokens: 812, ejected: 0 });
  });

  it("parses the loaded model from /v1/models shapes", () => {
    expect(parseLoadedModel('{"data":[{"id":"Lapis Occultus"}]}')).toBe("Lapis Occultus");
    expect(parseLoadedModel('{"models":[{"model":"Qwen3.8-27B"}]}')).toBe("Qwen3.8-27B");
    expect(parseLoadedModel("not json")).toBe("");
    expect(parseLoadedModel("{}")).toBe("");
  });

  it("parses the sparse-KV ejected counter when present", () => {
    const m = METRICS + "llamacpp:kv_ejected_total 12000\n";
    expect(parseMetrics(m)?.ejected).toBe(12000);
  });

  it("returns null when a counter is missing (never guesses)", () => {
    expect(parseMetrics("llamacpp:prompt_tokens_total 1")).toBeNull();
    expect(parseMetrics("")).toBeNull();
    expect(parseMetrics("garbage")).toBeNull();
  });

  it("returns null on non-numeric values", () => {
    expect(parseMetrics("llamacpp:prompt_tokens_total NaN\nllamacpp:n_decode_total x")).toBeNull();
  });
});

describe("counterDelta", () => {
  const after = { promptTokens: 18000, decodeTokens: 900 };

  it("computes t/s and token delta from the previous poll", () => {
    const before = { promptTokens: 17500, decodeTokens: 812 };
    expect(counterDelta(before, after, 5)).toEqual({ tps: 17.6, tokens: 88 });
  });

  it("is zero without a previous reading (first poll after boot)", () => {
    expect(counterDelta(null, after, 5)).toEqual({ tps: 0, tokens: 0 });
  });

  it("is zero across an engine restart (counter went backwards)", () => {
    const before = { promptTokens: 999999, decodeTokens: 999999 };
    const d = counterDelta(before, after, 5);
    expect(d.tokens).toBe(0);
    expect(d.tps).toBe(0);
  });
});

describe("renderBar", () => {
  it("renders empty, partial, full, and clamps out-of-range", () => {
    expect(renderBar(0, 4)).toBe("░░░░");
    expect(renderBar(0.5, 4)).toBe("██░░");
    expect(renderBar(1, 4)).toBe("████");
    expect(renderBar(2, 4)).toBe("████");
    expect(renderBar(-1, 4)).toBe("░░░░");
  });
});

describe("formatters", () => {
  it("rates", () => {
    expect(fmtRate(11.94)).toBe("11.9");
    expect(fmtRate(3.21)).toBe("3.21");
    expect(fmtRate(0)).toBe("0");
  });

  it("tokens", () => {
    expect(fmtTokens(812)).toBe("812");
    expect(fmtTokens(81234)).toBe("81.2k");
  });
});

describe("parseSlots", () => {
  it("reads the live engine schema (is_processing, not busy)", () => {
    const raw = JSON.stringify([
      { id: 0, is_processing: false },
      { id: 1, is_processing: true },
    ]);
    const slots = parseSlots(raw);
    expect(slots).toEqual([
      { id: 0, busy: false },
      { id: 1, busy: true },
    ]);
  });

  it("falls back to a busy alias when the schema differs", () => {
    expect(parseSlots(JSON.stringify([{ id: 0, busy: true }]))).toEqual([{ id: 0, busy: true }]);
  });

  it("returns [] on garbage without throwing", () => {
    expect(parseSlots("not json")).toEqual([]);
  });
});

describe("busySlots (two-source busy truth)", () => {
  it("counts flagged slots", () => {
    const slots = [{ id: 0, busy: false }, { id: 1, busy: true }];
    expect(busySlots(slots, 0)).toBe(1);
  });

  it("is NOT idle while tokens flow, even with zero flagged slots (dual-GPU bug)", () => {
    expect(busySlots([{ id: 0, busy: false }], 42)).toBe(1);
  });

  it("idle only when nothing flagged AND no token movement", () => {
    expect(busySlots([{ id: 0, busy: false }], 0)).toBe(0);
    expect(busySlots([], 0)).toBe(0);
  });
});

describe("gpuFireLoad", () => {
  it("reads idle desktop draw as no fire", () => {
    expect(gpuFireLoad(60, 240, 2)).toBeLessThan(0.05);
  });

  it("scales with power draw past the idle baseline", () => {
    const mid = gpuFireLoad(140, 240, 60);
    const high = gpuFireLoad(220, 240, 98);
    expect(mid).toBeGreaterThan(0.2);
    expect(high).toBeGreaterThan(mid);
    expect(high).toBeLessThanOrEqual(1);
  });

  it("clamps and rejects nonsense", () => {
    expect(gpuFireLoad(500, 240, 120)).toBe(1);
    expect(gpuFireLoad(-1, 240, 50)).toBe(0);
    expect(gpuFireLoad(60, 0, 50)).toBe(0);
  });

  it("dead zone: tiny idle draws stay dark", () => {
    expect(gpuFireLoad(8, 180, 4)).toBe(0); // this host's real idle reading
    expect(gpuFireLoad(60, 240, 6)).toBe(0);
  });
});

describe("fireLoad", () => {
  const base = { up: true, busy: 0, slotCount: 2, tps: 0, ingestTps: 0, gpuLoad: null };

  it("engine down = no fire", () => {
    expect(fireLoad({ ...base, up: false, gpuLoad: 0.8 })).toBe(0);
  });

  it("prefers real GPU load when available", () => {
    expect(fireLoad({ ...base, gpuLoad: 0.73 })).toBeCloseTo(0.73);
  });

  it("activity proxy: busy slots light a mid fire", () => {
    expect(fireLoad({ ...base, busy: 1 })).toBeGreaterThanOrEqual(0.55);
  });

  it("activity proxy: prefill spikes fire", () => {
    expect(fireLoad({ ...base, ingestTps: 1200 })).toBe(1);
  });

  it("proxy is idle with no activity", () => {
    expect(fireLoad(base)).toBe(0);
  });
});
