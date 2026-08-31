import { describe, it, expect } from "vitest";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { emitHarnessEvent, eventsPath, harnessEvent } from "./events.ts";

describe("harnessEvent (pure)", () => {
  it("builds the frozen schema shape", () => {
    const rec = harnessEvent("lc-clearer", "cleared", { freed_tokens: 42.7, turn: 3.9, detail: "x".repeat(500) });
    expect(rec.src).toBe("lc-clearer");
    expect(rec.ev).toBe("cleared");
    expect(rec.freed_tokens).toBe(42); // floored
    expect(rec.turn).toBe(3);
    expect((rec.detail ?? "").length).toBe(200);
    expect(rec.ts).toBeGreaterThan(1e9);
  });

  it("sanitizes session stems (no traversal, no newlines)", () => {
    const rec = harnessEvent("lc-ckpt", "saved", { session: "../../a b\nx" });
    expect(rec.session).not.toContain("/");
    expect(rec.session).not.toContain("\n");
  });
});

describe("emitHarnessEvent (real fs)", () => {
  it("appends a parseable JSONL line to TRIS_STATE_DIR", () => {
    const dir = mkdtempSync(join("/tmp", "tris-ev-"));
    process.env.TRIS_STATE_DIR = dir;
    try {
      emitHarnessEvent(harnessEvent("lc-rtk", "reduced", { freed_tokens: 99 }));
      const f = join(dir, "events.jsonl");
      expect(existsSync(f)).toBe(true);
      expect(JSON.parse(readFileSync(f, "utf8").trim()).freed_tokens).toBe(99);
    } finally {
      delete process.env.TRIS_STATE_DIR;
      rmSync(dir, { recursive: true, force: true });
    }
  });

  it("unwritable dir is swallowed, never throws", () => {
    // /root: instant EACCES. (2026-08-29: mkdirSync recursive under /proc
    // HANGS on cachys procfs — a test-only trap, never a production path.)
    process.env.TRIS_STATE_DIR = "/root/tris-nope";
    try {
      expect(() => emitHarnessEvent(harnessEvent("lc-tasks", "updated"))).not.toThrow();
    } finally {
      delete process.env.TRIS_STATE_DIR;
    }
  });

  it("eventsPath respects TRIS_STATE_DIR and defaults to ~/.vitriol/officina/state (SS4)", () => {
    expect(eventsPath({ TRIS_STATE_DIR: "/x" })).toBe(join("/x", "events.jsonl"));
    expect(eventsPath({ HOME: "/h" })).toContain("/h/.vitriol/officina/state/events.jsonl");
  });
});
