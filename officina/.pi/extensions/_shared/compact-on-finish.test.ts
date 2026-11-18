// Compact-on-finish (2026-11-18): the policy is three numbers and four guards,
// so the tests are about the guards, not about pi's compaction machinery.
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  compactOnFinish,
  lastCompactRequest,
  resolveCompactPolicy,
  shouldCompactOnFinish,
  stampCompactRequest,
} from "./compact-on-finish.ts";

const POLICY = { enabled: true, minTokens: 80_000, cooldownMs: 10 * 60_000 };

function home(): string {
  return mkdtempSync(join(tmpdir(), "officina-compact-"));
}

describe("resolveCompactPolicy", () => {
  it("defaults to enabled with an 80k floor", () => {
    const p = resolveCompactPolicy({});
    expect(p.enabled).toBe(true);
    expect(p.minTokens).toBe(80_000);
  });

  it("honours the kill switch and the numeric overrides", () => {
    expect(
      resolveCompactPolicy({ OFFICINA_NO_COMPACT_ON_FINISH: "1" }).enabled,
    ).toBe(false);
    const p = resolveCompactPolicy({
      OFFICINA_COMPACT_ON_FINISH_MIN_TOKENS: "12000",
      OFFICINA_COMPACT_ON_FINISH_COOLDOWN_MS: "0",
    });
    expect(p.minTokens).toBe(12_000);
    expect(p.cooldownMs).toBe(0);
  });
});

describe("shouldCompactOnFinish", () => {
  const base = { policy: POLICY, lastRequestAt: null, now: 1_000_000 };

  it("asks when a finished run left a fat context", () => {
    expect(shouldCompactOnFinish({ ...base, fillTokens: 150_000 })).toEqual({
      ok: true,
      reason: "finished run",
    });
  });

  it("skips a session too small to be worth summarising", () => {
    const d = shouldCompactOnFinish({ ...base, fillTokens: 20_000 });
    expect(d.ok).toBe(false);
    expect(d.reason).toContain("min 80000");
  });

  it("skips when the context size is unknown", () => {
    expect(shouldCompactOnFinish({ ...base, fillTokens: null }).ok).toBe(false);
  });

  it("skips inside the cooldown — two loops must not compact twice", () => {
    const d = shouldCompactOnFinish({
      ...base,
      fillTokens: 150_000,
      lastRequestAt: base.now - 60_000,
    });
    expect(d.ok).toBe(false);
    expect(d.reason).toBe("cooldown");
  });

  it("skips when the feature is switched off", () => {
    const d = shouldCompactOnFinish({
      policy: { ...POLICY, enabled: false },
      fillTokens: 150_000,
      lastRequestAt: null,
      now: 1,
    });
    expect(d.ok).toBe(false);
    expect(d.reason).toBe("disabled");
  });
});

describe("compactOnFinish", () => {
  it("stamps the request so a re-entered settled handler cannot fire twice", () => {
    const h = home();
    try {
      let calls = 0;
      const ctx = {
        getContextUsage: () => ({ tokens: 120_000, contextWindow: 262_144 }),
        compact: () => {
          calls += 1;
        },
      };
      const first = compactOnFinish(ctx, "grind done", {
        home: h,
        now: 10_000_000,
      });
      expect(first.requested).toBe(true);
      expect(calls).toBe(1);
      expect(lastCompactRequest(h)).toBe(10_000_000);
      expect(
        JSON.parse(
          readFileSync(
            join(h, ".vitriol/officina/compact-on-finish.json"),
            "utf8",
          ),
        ).reason,
      ).toBe("grind done");

      // Same run again: the cooldown stamp refuses.
      const second = compactOnFinish(ctx, "grind done", {
        home: h,
        now: 10_000_500,
      });
      expect(second.requested).toBe(false);
      expect(calls).toBe(1);
    } finally {
      rmSync(h, { recursive: true, force: true });
    }
  });

  it("does not stamp when the guard refuses", () => {
    const h = home();
    try {
      const ctx = {
        getContextUsage: () => ({ tokens: 5_000, contextWindow: 262_144 }),
        compact: () => {
          throw new Error("must not be called");
        },
      };
      expect(
        compactOnFinish(ctx, "grind done", { home: h, now: 1 }).requested,
      ).toBe(false);
      expect(lastCompactRequest(h)).toBeNull();
    } finally {
      rmSync(h, { recursive: true, force: true });
    }
  });

  it("survives a host without compact()", () => {
    const h = home();
    try {
      const ctx = {
        getContextUsage: () => ({ tokens: 200_000, contextWindow: 262_144 }),
      };
      expect(
        compactOnFinish(ctx, "plan done", { home: h, now: 5 }).requested,
      ).toBe(true);
    } finally {
      rmSync(h, { recursive: true, force: true });
    }
  });
});
