// Session-panel telemetry rows (2026-11-16): the session's own facts, not the
// workshop's engine.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  apiRowParts,
  cachedJsonFile,
  fmtCost,
  fmtTokens,
  laneRowParts,
  shortModel,
  type CaduceusFacts,
  type HealthFacts,
} from "./telemetry.ts";

const NOW = 1_800_000_000_000;

/** The exact shape the live ledger recorded for a caduceus turn. */
const ledgerSample = {
  provider: "caduceus",
  model: "dahl/MiniMaxAI/MiniMax-M2.7",
  tokensIn: 69_481,
  tokensOut: 976,
  latencyMs: 44_706,
  costEstimate: 0,
  outcome: "ok",
};

describe("apiRowParts", () => {
  it("describes this session's own routed request", () => {
    expect(apiRowParts(ledgerSample, null, NOW)).toEqual({
      model: "MiniMax-M2.7",
      fullModel: "dahl/MiniMaxAI/MiniMax-M2.7",
      usage: "69.5k→976 tok",
      latency: "44.7s",
      cost: "free",
      outcome: "ok",
      note: undefined,
    });
  });

  it("adds a cooling or failing note from provider health", () => {
    const health: HealthFacts = {
      models: {
        "caduceus/dahl/MiniMaxAI/MiniMax-M2.7": {
          cooldownUntil: NOW + 120_000,
          failureCount: 3,
        },
      },
    };
    expect(apiRowParts(ledgerSample, health, NOW)?.note).toBe("cooling 2m");
    expect(
      apiRowParts({ ...ledgerSample, outcome: "error" }, health, NOW + 600_000)
        ?.note,
    ).toBe("failing");
  });

  it("stays silent without an entry (P4)", () => {
    expect(apiRowParts(null, null, NOW)).toBeNull();
    expect(apiRowParts({ provider: "caduceus" }, null, NOW)).toBeNull();
  });
});

describe("laneRowParts", () => {
  const facts: CaduceusFacts = {
    gateway_up: true,
    cooling: 2,
    candidates: [
      {
        id: "dahl/MiniMaxAI/MiniMax-M2.7",
        context_window: 204_800,
        free: true,
      },
      {
        id: "freeinference/deepseek-v4-flash",
        context_window: 1_000_000,
        free: true,
      },
      { id: "paid/big", context_window: 2_000_000, free: false },
    ],
    budget: { daily_tokens: 12_000, daily_cap: 2_000_000, allowed: true },
  };

  it("counts the pool and picks the next lane that fits the context", () => {
    const row = laneRowParts(facts, 500_000, null, NOW);
    expect(row).toMatchObject({
      lanes: 3,
      cooling: 0,
      next: "freeinference/deepseek-v4-flash",
      gatewayUp: true,
      budget: "12.0k/2.0M today",
    });
    // A small context fits the first free lane.
    expect(laneRowParts(facts, 10_000, null, NOW)?.next).toBe(
      "dahl/MiniMaxAI/MiniMax-M2.7",
    );
    // Nothing fits → an explicit null, never a wrong pick.
    expect(laneRowParts(facts, 5_000_000, null, NOW)?.next).toBeNull();
  });

  it("skips cooling lanes and never counts a paid lane as next", () => {
    const health: HealthFacts = {
      models: {
        "caduceus/freeinference/deepseek-v4-flash": {
          cooldownUntil: NOW + 60_000,
        },
      },
    };
    const row = laneRowParts(facts, 500_000, health, NOW);
    expect(row?.cooling).toBe(1);
    expect(row?.next).toBeNull(); // the only other fit is paid
  });

  it("stays silent with no candidates (P4)", () => {
    expect(laneRowParts(null, 0, null, NOW)).toBeNull();
    expect(laneRowParts({ candidates: [] }, 0, null, NOW)).toBeNull();
  });
});

describe("shortModel", () => {
  it("keeps the last segment and clips to the sidebar width", () => {
    expect(shortModel("dahl/MiniMaxAI/MiniMax-M2.7")).toBe("MiniMax-M2.7");
    expect(shortModel("Lapis Occultus")).toBe("Lapis Occultus");
    expect(shortModel("owner/" + "x".repeat(40)).endsWith("…")).toBe(true);
    expect(shortModel("")).toBe("");
  });
});

describe("formatters", () => {
  it("round tokens and cost honestly", () => {
    expect(fmtTokens(999)).toBe("999");
    expect(fmtTokens(1234)).toBe("1.2k");
    expect(fmtTokens(2_500_000)).toBe("2.5M");
    expect(fmtCost(0)).toBe("free");
    expect(fmtCost(0.00048)).toBe("€0.0005");
    expect(fmtCost(1.2)).toBe("€1.20");
  });
});

describe("cachedJsonFile", () => {
  let dir: string;
  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), "telemetry-cache-"));
  });
  afterEach(() => rmSync(dir, { recursive: true, force: true }));

  it("caches by mtime and tolerates missing/corrupt files", () => {
    const path = join(dir, "facts.json");
    writeFileSync(path, JSON.stringify({ a: 1 }));
    let reads = 0;
    const read = (p: string) => {
      reads += 1;
      return readFileSync(p, "utf8");
    };
    expect(cachedJsonFile<{ a: number }>(path, 2000, read, () => NOW)).toEqual({
      a: 1,
    });
    expect(
      cachedJsonFile<{ a: number }>(path, 2000, read, () => NOW + 100),
    ).toEqual({ a: 1 });
    expect(reads).toBe(1); // second render served from cache (P5)
    writeFileSync(path, "{corrupt");
    expect(cachedJsonFile(path, 0, read, () => NOW + 200)).toBeNull();
    expect(
      cachedJsonFile(join(dir, "absent.json"), 2000, read, () => NOW),
    ).toBeNull();
  });
});
