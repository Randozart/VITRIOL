// Sleep-lane gate tests (2026-11-21, plan sleep-lane-2026-11-21.md).
// I2: the foreground turn is sacred — every unknown/insufficient condition
// blocks, slot-1-only, kill switch honoured.
import { describe, expect, it } from "vitest";
import {
  shouldRun,
  sleepConfig,
  type EngineState,
  type SleepConfig,
} from "./lane.ts";

const cfg: SleepConfig = { ...sleepConfig(), intervalH: 24, idleMs: 300_000 };
const healthy: EngineState = {
  up: true,
  slots: 2,
  busy: 0,
  foregroundActive: false,
};

describe("sleep-lane gate (I2)", () => {
  it("runs when everything is right", () => {
    const g = shouldRun(healthy, cfg, null, 1_000_000);
    expect(g.run).toBe(true);
  });

  it("refuses single-slot engines (the whole point of the fix)", () => {
    const g = shouldRun({ ...healthy, slots: 1 }, cfg, null, 1_000_000);
    expect(g.run).toBe(false);
    expect(g.reason).toContain("2 slots");
  });

  it("refuses zero-slot/unknown telemetry", () => {
    expect(shouldRun({ ...healthy, slots: 0 }, cfg, null, 1).run).toBe(false);
  });

  it("refuses while the engine is down", () => {
    expect(shouldRun({ ...healthy, up: false }, cfg, null, 1).run).toBe(false);
  });

  it("refuses while the foreground turn is active", () => {
    expect(
      shouldRun({ ...healthy, foregroundActive: true }, cfg, null, 1).run,
    ).toBe(false);
  });

  it("refuses while any slot is busy", () => {
    expect(shouldRun({ ...healthy, busy: 1 }, cfg, null, 1).run).toBe(false);
  });

  it("honours the interval since the last run", () => {
    const now = Date.now();
    const g = shouldRun(healthy, cfg, now - 3600_000, now);
    expect(g.run).toBe(false);
    expect(g.reason).toContain("recently");
    const ok = shouldRun(healthy, cfg, now - 25 * 3600_000, now);
    expect(ok.run).toBe(true);
  });

  it("kill switch wins over everything (OFFICINA_NO_SLEEP=1)", () => {
    const c: SleepConfig = { ...cfg, enabled: false };
    const g = shouldRun(healthy, c, null, 1_000_000);
    expect(g.run).toBe(false);
    expect(g.reason).toContain("kill switch");
  });

  it("config defaults: slot 1, 24h, bounded reads", () => {
    const c = sleepConfig({});
    expect(c.slotId).toBe(1);
    expect(c.intervalH).toBe(24);
    expect(c.maxBytes).toBe(2_000_000);
    expect(c.maxFacts).toBe(10);
  });

  it("numeric env overrides parse", () => {
    const c = sleepConfig({
      OFFICINA_SLEEP_SLOT: "3",
      OFFICINA_SLEEP_INTERVAL_H: "6",
    });
    expect(c.slotId).toBe(3);
    expect(c.intervalH).toBe(6);
  });
});
