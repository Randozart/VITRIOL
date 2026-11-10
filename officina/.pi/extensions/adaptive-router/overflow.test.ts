import { describe, expect, it } from "vitest";
import {
  configFromEnv,
  decideTier,
  localBusy,
  parseModelSpec,
  type DecideInput,
  type LoadSample,
  type OverflowConfig,
  type RouterState,
} from "./overflow.ts";

const cfg: OverflowConfig = {
  queueWaitMs: 1500,
  gpuLoadCutoff: 0.85,
  minDwell: 3,
  maxSwitches: 2,
  returnIdleMs: 60_000,
  returnTokenBudget: 16_000,
  cloudTokenBudget: 2_000_000,
  sensitiveLocal: true,
  failover: true,
};

const load = (over: Partial<LoadSample> = {}): LoadSample => ({
  processing: 0,
  deferred: 0,
  queueWaitMs: 0,
  stalled: false,
  gpuLoad: 0.1,
  ...over,
});

const state = (over: Partial<RouterState> = {}): RouterState => ({
  tier: "local",
  switches: 0,
  turnsOnTier: 1,
  cloudTokens: 0,
  ...over,
});

const input = (over: Partial<DecideInput> = {}): DecideInput => ({
  state: state(),
  load: load(),
  privacy: "safe",
  cloudFits: true,
  budgetLeft: true,
  localIdleForMs: 0,
  localDown: false,
  failoverActive: false,
  ...over,
});

describe("localBusy", () => {
  it("detects processing, queue, stall, and foreign GPU load", () => {
    expect(localBusy(load(), cfg)).toBe(false);
    expect(localBusy(load({ processing: 1 }), cfg)).toBe(true);
    expect(localBusy(load({ deferred: 1 }), cfg)).toBe(true);
    expect(localBusy(load({ queueWaitMs: 1500 }), cfg)).toBe(true);
    expect(localBusy(load({ stalled: true }), cfg)).toBe(true);
    expect(localBusy(load({ gpuLoad: 0.9 }), cfg)).toBe(true);
  });
});

describe("decideTier", () => {
  it("fails over to cloud when the local engine is DOWN", () => {
    const d = decideTier(input({ localDown: true }), cfg);
    expect(d.tier).toBe("cloud");
    expect(d.reason).toContain("emergency");
  });

  it("stays local when DOWN and no cloud candidate fits", () => {
    expect(decideTier(input({ localDown: true, cloudFits: false }), cfg).tier).toBe("local");
  });

  it("does not fall back to a dead local engine from cloud", () => {
    expect(
      decideTier(
        input({
          state: state({ tier: "cloud", switches: 1, turnsOnTier: 2 }),
          localDown: true,
          cloudFits: false,
        }),
        cfg,
      ).tier,
    ).toBe("cloud");
  });

  it("privacy is not traded for availability", () => {
    expect(
      decideTier(input({ localDown: true, privacy: "confidential" }), cfg).tier,
    ).toBe("local");
  });

  it("returns to local quickly after a failover recovery", () => {
    const d = decideTier(
      input({
        state: state({ tier: "cloud", switches: 0, turnsOnTier: 1, cloudTokens: 999_999 }),
        failoverActive: true,
        localIdleForMs: 10_000,
        load: load(),
      }),
      cfg,
    );
    expect(d.tier).toBe("local");
    expect(d.reason).toContain("recovered");
  });

  it("keeps an idle local tier", () => {
    expect(decideTier(input(), cfg).tier).toBe("local");
  });

  it("spills to cloud when local is busy and budget allows", () => {
    const d = decideTier(input({ load: load({ processing: 1 }) }), cfg);
    expect(d.tier).toBe("cloud");
    expect(d.reason).toContain("spill");
  });

  it("stays local when cloud cannot fit or budget is gone", () => {
    expect(
      decideTier(input({ load: load({ processing: 1 }), cloudFits: false }), cfg).tier,
    ).toBe("local");
    expect(
      decideTier(input({ load: load({ processing: 1 }), budgetLeft: false }), cfg).tier,
    ).toBe("local");
  });

  it("privacy always returns/stays local", () => {
    expect(
      decideTier(
        input({ state: state({ tier: "cloud", turnsOnTier: 9 }), privacy: "confidential" }),
        cfg,
      ).tier,
    ).toBe("local");
    expect(
      decideTier(input({ load: load({ processing: 1 }), privacy: "sensitive" }), cfg).tier,
    ).toBe("local");
  });

  it("cloud honours minimum dwell", () => {
    const d = decideTier(
      input({
        state: state({ tier: "cloud", switches: 1, turnsOnTier: 1 }),
        localIdleForMs: 120_000,
      }),
      cfg,
    );
    expect(d.tier).toBe("cloud");
    expect(d.reason).toContain("dwell");
  });

  it("returns to local only when idle and cloud delta is small", () => {
    const base = {
      state: state({ tier: "cloud", switches: 1, turnsOnTier: 4, cloudTokens: 5000 }),
      localIdleForMs: 120_000,
    };
    expect(decideTier(input(base), cfg).tier).toBe("local");
    expect(
      decideTier(
        input({ ...base, state: state({ tier: "cloud", switches: 1, turnsOnTier: 4, cloudTokens: 50_000 }) }),
        cfg,
      ).tier,
    ).toBe("cloud");
  });

  it("never exceeds the switch cap when returning", () => {
    const d = decideTier(
      input({
        state: state({ tier: "cloud", switches: 2, turnsOnTier: 9, cloudTokens: 1000 }),
        localIdleForMs: 120_000,
      }),
      cfg,
    );
    expect(d.tier).toBe("cloud");
    expect(d.reason).toContain("cap");
  });
});

describe("config + model spec", () => {
  it("parses env overrides with sane defaults", () => {
    const c = configFromEnv({
      OFFICINA_OVERFLOW_QUEUE_MS: "900",
      OFFICINA_OVERFLOW_MAX_SWITCHES: "1",
      OFFICINA_OVERFLOW_MIN_DWELL: "0",
    });
    expect(c.queueWaitMs).toBe(900);
    expect(c.maxSwitches).toBe(1);
    expect(c.minDwell).toBe(1);
    expect(configFromEnv({}).cloudTokenBudget).toBe(2_000_000);
    expect(configFromEnv({}).failover).toBe(true);
    expect(configFromEnv({ OFFICINA_OVERFLOW_FAILOVER: "0" }).failover).toBe(false);
  });

  it("keeps slashes in the model id", () => {
    expect(parseModelSpec("caduceus/opencode-go/deepseek-v4-flash")).toEqual({
      provider: "caduceus",
      id: "opencode-go/deepseek-v4-flash",
    });
    expect(parseModelSpec("llamacpp/Lapis Occultus")).toEqual({
      provider: "llamacpp",
      id: "Lapis Occultus",
    });
    expect(parseModelSpec("nope")).toBeNull();
  });
});
