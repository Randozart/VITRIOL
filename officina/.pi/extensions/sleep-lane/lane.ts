// sleep-lane — pure module: cadence, gate decision, session window selection.
// Execution (HTTP on slot 1, fs reads, MEMORY.md writes) lives in index.ts.
//
// Policy (plan .opencode/plans/sleep-lane-2026-11-21.md):
//   - runs ONLY on slot 1 (foreground slot 0 is sacred, I2)
//   - idle-only by default: consolidation is minutes-long, not seconds
//   - cadence: OFFICINA_SLEEP_INTERVAL_H hours (default 24) since last run
//   - kill switch: OFFICINA_NO_SLEEP=1
//   - bounded: reads at most maxBytes of session, emits at most maxFacts (I3)

export interface SleepConfig {
  enabled: boolean;
  base: string;
  slotId: number;
  /** Hours between consolidation runs. */
  intervalH: number;
  /** Engine must be idle this long (ms) before a run. */
  idleMs: number;
  /** Max bytes of session transcript the run reads. */
  maxBytes: number;
  /** Max facts emitted per run. */
  maxFacts: number;
}

export const DEFAULTS: SleepConfig = {
  enabled: true,
  base: "http://127.0.0.1:8279",
  slotId: 1,
  intervalH: 24,
  idleMs: 5 * 60_000,
  maxBytes: 2_000_000,
  maxFacts: 10,
};

export function sleepConfig(env: NodeJS.ProcessEnv = process.env): SleepConfig {
  const num = (v: string | undefined, d: number): number => {
    const n = Number(v);
    return Number.isFinite(n) && n > 0 ? n : d;
  };
  return {
    enabled: env.OFFICINA_NO_SLEEP !== "1",
    base:
      env.OFFICINA_CADUCEUS_URL?.replace(/\/v1\/?$/, "") ||
      env.VITRIOL_BASE_URL ||
      DEFAULTS.base,
    slotId: num(env.OFFICINA_SLEEP_SLOT, DEFAULTS.slotId),
    intervalH: num(env.OFFICINA_SLEEP_INTERVAL_H, DEFAULTS.intervalH),
    idleMs: num(env.OFFICINA_SLEEP_IDLE_MS, DEFAULTS.idleMs),
    maxBytes: num(env.OFFICINA_SLEEP_MAX_BYTES, DEFAULTS.maxBytes),
    maxFacts: num(env.OFFICINA_SLEEP_MAX_FACTS, DEFAULTS.maxFacts),
  };
}

export interface EngineState {
  up: boolean;
  /** Number of slots reported by /slots (0 when unknown). */
  slots: number;
  /** Slots currently processing. */
  busy: number;
  /** True while a foreground turn is streaming. */
  foregroundActive: boolean;
}

export type GateReason = string;

/**
 * Should a sleep run start now? Pure. Fail-closed: every unknown blocks.
 * (I2: the foreground turn is sacred; the run is worth waiting for.)
 */
export function shouldRun(
  engine: EngineState,
  cfg: SleepConfig,
  lastRunMs: number | null,
  now: number,
): { run: boolean; reason: GateReason } {
  if (!cfg.enabled)
    return { run: false, reason: "kill switch OFFICINA_NO_SLEEP" };
  if (!engine.up) return { run: false, reason: "engine down" };
  if (engine.slots < 2) {
    return {
      run: false,
      reason: "needs 2 slots for foreground isolation (I2)",
    };
  }
  if (engine.foregroundActive)
    return { run: false, reason: "foreground active" };
  if (engine.busy > 0) return { run: false, reason: "engine busy" };
  if (lastRunMs !== null && now - lastRunMs < cfg.intervalH * 3_600_000) {
    return { run: false, reason: "ran recently" };
  }
  return { run: true, reason: `idle past ${cfg.idleMs}ms, interval elapsed` };
}

/** Session lines eligible for consolidation: bounded by bytes (I3). */
export function selectSessionFiles(
  files: Array<{ path: string; bytes: number; mtimeMs: number }>,
  maxBytes: number,
): Array<{ path: string; bytes: number; mtimeMs: number }> {
  const sorted = [...files].sort((a, b) => b.mtimeMs - a.mtimeMs);
  const out: Array<{ path: string; bytes: number; mtimeMs: number }> = [];
  let total = 0;
  for (const f of sorted) {
    if (total + f.bytes > maxBytes) continue;
    out.push(f);
    total += f.bytes;
  }
  return out;
}
