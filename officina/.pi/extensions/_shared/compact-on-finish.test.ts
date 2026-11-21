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
  safeStampKey,
  sessionStampId,
  shouldCompactOnFinish,
  stampCompactRequest,
  stampPath,
} from "./compact-on-finish.ts";

const POLICY = { enabled: true, minTokens: 80_000, cooldownMs: 10 * 60_000 };

function home(): string {
  return mkdtempSync(join(tmpdir(), "officina-compact-"));
}

describe("resolveCompactPolicy", () => {
  it("defaults to enabled with a zero floor (2026-11-21: predictability)", () => {
    const policy = resolveCompactPolicy({});
    expect(policy.enabled).toBe(true);
    expect(policy.minTokens).toBe(0);
    expect(policy.cooldownMs).toBe(10 * 60_000);
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
        sessionId: "s1",
      });
      expect(first.requested).toBe(true);
      expect(calls).toBe(1);
      expect(lastCompactRequest(h, "s1")).toBe(10_000_000);
      expect(JSON.parse(readFileSync(stampPath(h, "s1"), "utf8")).reason).toBe(
        "grind done",
      );

      // Same run again: the cooldown stamp refuses.
      const second = compactOnFinish(ctx, "grind done", {
        home: h,
        now: 10_000_500,
        sessionId: "s1",
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

  // 2026-11-18: the stamp used to be ONE global file, so with several sessions
  // in one project the first session to finish silenced every other session's
  // compaction for the whole cooldown. Each session owns its stamp.
  it("keys the stamp per session so one session cannot silence another", () => {
    const h = home();
    try {
      const ctx = {
        getContextUsage: () => ({ tokens: 200_000, contextWindow: 262_144 }),
      };
      expect(
        compactOnFinish(ctx, "grind done", {
          home: h,
          now: 1_000_000,
          sessionId: "session-a.jsonl",
        }).requested,
      ).toBe(true);
      // Same instant, different session: a stamp of its own, so it may run.
      expect(
        compactOnFinish(ctx, "grind done", {
          home: h,
          now: 1_000_000,
          sessionId: "session-b.jsonl",
        }).requested,
      ).toBe(true);
      expect(lastCompactRequest(h, "session-a.jsonl")).toBe(1_000_000);
      expect(lastCompactRequest(h, "session-b.jsonl")).toBe(1_000_000);
      expect(stampPath(h, "session-a.jsonl")).not.toBe(
        stampPath(h, "session-b.jsonl"),
      );
    } finally {
      rmSync(h, { recursive: true, force: true });
    }
  });

  it("keeps the stamp key filesystem-safe for long session names", () => {
    // Session file basenames carry `.patched-<ts>` chains and can exceed the
    // 255-byte filename limit, so the key is a hash, never the raw name.
    const key = safeStampKey(
      "2026-11-15T20-24-26-146Z_01a1e0e1-f11d-75d7-b9a7-683f2304996f.jsonl.patched-1794774774853.jsonl.patched-1794775128941.jsonl",
    );
    expect(key).toMatch(/^[0-9a-f]{16}$/);
    expect(sessionStampId({})).toMatch(/^tui-\d+$/);
    expect(
      sessionStampId({
        sessionManager: { getSessionFile: () => "/a/b/s.jsonl" },
      }),
    ).toBe("s.jsonl");
  });
});
