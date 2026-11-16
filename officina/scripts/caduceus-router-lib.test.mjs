// Tests for the routing proxy's pure policy (2026-11-16).
import { describe, expect, it } from "vitest";
import {
  candidateOrder,
  fallbacksFromEnv,
  fallbacksFromPrefs,
  resolveFallbacks,
  giveUpMessage,
  isParked,
  park,
  parseQuotaHint,
  pruneParked,
  shouldRetry,
} from "./caduceus-router-lib.mjs";

const T0 = Date.parse("2026-11-16T18:51:00.000Z");

describe("parseQuotaHint", () => {
  it("reads a bare retry_after (seconds) — what Caduceus sends", () => {
    expect(
      parseQuotaHint(
        'quota spent (reset after 2m 57s)","retry_after":177,"reset_at":"2026-11-16T18:54:18.506Z"',
        T0,
      ),
    ).toBe(177_000);
  });

  it("reads a quoted retry_after too", () => {
    expect(parseQuotaHint('{"retry_after":"120"}', T0)).toBe(120_000);
  });

  it("falls back to reset_at", () => {
    expect(parseQuotaHint('{"reset_at":"2026-11-16T18:54:18.506Z"}', T0)).toBe(
      198_506,
    );
  });

  it("clamps and rejects nonsense", () => {
    expect(parseQuotaHint('{"retry_after":0}', T0)).toBe(1_000);
    expect(parseQuotaHint('{"retry_after":999999}', T0)).toBe(24 * 3600_000);
    expect(parseQuotaHint("plain error", T0)).toBeUndefined();
    expect(parseQuotaHint('{"reset_at":"not-a-date"}', T0)).toBeUndefined();
    expect(parseQuotaHint(undefined, T0)).toBeUndefined();
  });
});

describe("parking", () => {
  it("parks a lane, remembers why, and expires", () => {
    let state = park(
      { parked: {} },
      "agnes/agnes-3.0-flash",
      T0 + 177_000,
      "429",
    );
    expect(isParked(state, "agnes/agnes-3.0-flash", T0)).toBe(true);
    expect(isParked(state, "agnes/agnes-3.0-flash", T0 + 178_000)).toBe(false);
    expect(state.reasons["agnes/agnes-3.0-flash"]).toBe("429");
  });

  it("prunes expired entries only", () => {
    let state = park({ parked: {} }, "a/one", T0 + 1_000, "x");
    state = park(state, "a/two", T0 + 900_000, "y");
    const pruned = pruneParked(state, T0 + 60_000);
    expect(Object.keys(pruned.parked)).toEqual(["a/two"]);
  });
});

describe("candidateOrder", () => {
  const state = park({ parked: {} }, "b/parked", T0 + 600_000, "429");

  it("keeps the requested model first and skips parked lanes", () => {
    expect(
      candidateOrder({
        requested: "a/first",
        fallbacks: ["b/parked", "c/third", "a/first"],
        state,
        nowMs: T0,
      }),
    ).toEqual(["a/first", "c/third"]);
  });

  it("drops a parked requested model instead of re-picking it", () => {
    expect(
      candidateOrder({
        requested: "b/parked",
        fallbacks: ["c/third"],
        state,
        nowMs: T0,
      }),
    ).toEqual(["c/third"]);
  });

  it("is empty when everything is parked", () => {
    expect(
      candidateOrder({
        requested: "b/parked",
        fallbacks: ["b/parked"],
        state,
        nowMs: T0,
      }),
    ).toEqual([]);
  });

  it("ignores blanks and duplicates", () => {
    expect(
      candidateOrder({
        requested: "a/x",
        fallbacks: ["", "  ", "a/x", "b/y"],
        nowMs: T0,
      }),
    ).toEqual(["a/x", "b/y"]);
  });
});

describe("shouldRetry", () => {
  it("retries a 429 before anything was forwarded", () => {
    expect(shouldRetry({ status: 429, forwarded: false })).toBe(true);
  });

  it("never retries after bytes reached the client", () => {
    expect(shouldRetry({ status: 429, forwarded: true })).toBe(false);
  });

  it("retries a timeout, but not a 200 or a 400", () => {
    expect(shouldRetry({ status: 0, forwarded: false, timedOut: true })).toBe(
      true,
    );
    expect(shouldRetry({ status: 200, forwarded: false })).toBe(false);
    expect(shouldRetry({ status: 400, forwarded: false })).toBe(false);
  });
});

describe("giveUpMessage", () => {
  it("names the count and the earliest return", () => {
    const state = park({ parked: {} }, "a/x", T0 + 120_000, "429");
    expect(giveUpMessage({ tried: 3, parked: state.parked, nowMs: T0 })).toBe(
      "all 3 candidate(s) unavailable — earliest lane returns in 120s",
    );
  });

  it("still reads sensibly with nothing parked", () => {
    expect(giveUpMessage({ tried: 1, parked: {}, nowMs: T0 })).toContain(
      "no lane is parked",
    );
  });
});

describe("fallbacksFromEnv", () => {
  it("splits, trims and drops blanks", () => {
    expect(fallbacksFromEnv({ CADUCEUS_ROUTER_FALLBACKS: " a , ,b " })).toEqual(
      ["a", "b"],
    );
    expect(fallbacksFromEnv({})).toEqual([]);
  });
});

describe("fallbacks from the Caduceus panel", () => {
  it("reads the prefs `fallback` list", () => {
    expect(fallbacksFromPrefs({ fallback: [" opencode-go/deepseek-v4.1-flash ", ""] })).toEqual([
      "opencode-go/deepseek-v4.1-flash",
    ]);
    expect(fallbacksFromPrefs({})).toEqual([]);
    expect(fallbacksFromPrefs(null)).toEqual([]);
  });

  it("prefers the env, else the panel", () => {
    expect(
      resolveFallbacks({ env: { CADUCEUS_ROUTER_FALLBACKS: "env/lane" }, prefs: { fallback: ["ui/lane"] } }),
    ).toEqual(["env/lane"]);
    expect(resolveFallbacks({ env: {}, prefs: { fallback: ["ui/lane"] } })).toEqual(["ui/lane"]);
    expect(resolveFallbacks({ env: {}, prefs: null })).toEqual([]);
  });
});
