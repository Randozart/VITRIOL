import { describe, it, expect } from "vitest";
import {
  checkpointPeerGate,
  ckptConfig,
  decodeMarker,
  encodeMarker,
  markerMatchesSession,
  markerPath,
  parseSlotOpResponse,
  restoreIsCompatible,
  safeFilename,
  slotActionUrl,
  turnFilename,
} from "./client.ts";

describe("ckptConfig", () => {
  it("defaults: on, local engine, slot 0, every 10 turns", () => {
    const cfg = ckptConfig({});
    expect(cfg.enabled).toBe(true);
    expect(cfg.endpoint).toBe("http://127.0.0.1:8279");
    expect(cfg.slot).toBe(0);
    expect(cfg.everyTurns).toBe(10);
  });

  it("VITRIOL_BASE_URL /v1 suffix stripped (engine surface is /slots not /v1/slots)", () => {
    expect(ckptConfig({ VITRIOL_BASE_URL: "http://127.0.0.1:8279/v1/" }).endpoint).toBe("http://127.0.0.1:8279");
  });

  it("kill switch + cadence", () => {
    expect(ckptConfig({ TRIS_NO_VITRIOL_CKPT: "1" }).enabled).toBe(false);
    expect(ckptConfig({ TRIS_CKPT_EVERY_TURNS: "0" }).everyTurns).toBe(0); // manual only
    expect(ckptConfig({ TRIS_CKPT_EVERY_TURNS: "junk" }).everyTurns).toBe(10);
  });
});

describe("urls + filenames", () => {
  it("action URL shape matches the fork endpoints (server-context.cpp SLOT_SAVE/LOAD)", () => {
    const cfg = ckptConfig({});
    expect(slotActionUrl(cfg, "save")).toBe("http://127.0.0.1:8279/slots/0?action=save");
    expect(() => slotActionUrl(cfg, "delete" as never)).toThrow(/bad action/);
  });

  it("turn filenames pair with the snapshot ext refs turns/<n>", () => {
    expect(turnFilename(42, "abc")).toBe("abc-turn-42.bin");
  });

  it("safeFilename blocks traversal (no slashes, no dot-runs, .bin suffix)", () => {
    const f = safeFilename("../../etc/x");
    expect(f).not.toContain("/");
    expect(f).not.toContain("..");
    expect(f.endsWith(".bin")).toBe(true);
    expect(safeFilename("ok-1.bin")).toBe("ok-1.bin");
  });
});

describe("parseSlotOpResponse", () => {
  it("save shape — verified against live engine + server-task.cpp to_json", () => {
    const r = parseSlotOpResponse(200, { id_slot: 0, filename: "a.bin", n_saved: 53, n_written: 53181480, timings: { save_ms: 42.5 } });
    expect(r.ok).toBe(true);
    expect(r.nTokens).toBe(53);
    expect(r.nBytes).toBe(53181480);
    expect(r.tMs).toBeCloseTo(42.5);
    expect(r.filename).toBe("a.bin");
  });

  it("restore shape — n_restored/n_read/restore_ms", () => {
    const r = parseSlotOpResponse(200, { id_slot: 0, filename: "a.bin", n_restored: 53, n_read: 5e7, timings: { restore_ms: 88 } });
    expect(r.ok).toBe(true);
    expect(r.nTokens).toBe(53);
    expect(r.tMs).toBe(88);
  });

  it("4xx/5xx extracts engine error message", () => {
    const r = parseSlotOpResponse(400, { error: { code: 400, message: "server was started without --slot-save-path" } });
    expect(r.ok).toBe(false);
    expect(r.error).toContain("slot-save-path");
  });

  it("unknown 2xx ok without fabricated numbers", () => {
    const r = parseSlotOpResponse(200, { weird: true });
    expect(r.ok).toBe(true);
    expect(r.nTokens).toBeUndefined();
  });
});

describe("marker round-trip", () => {
  it("encode→decode preserves fields", () => {
    const m = { endpoint: "http://x", slot: 0, filename: "s-turn-10.bin", turn: 10, model: "M.gguf", at: 1 };
    expect(decodeMarker(encodeMarker(m))).toEqual(m);
  });

  it("corrupt marker decodes to null (never restores on garbage)", () => {
    expect(decodeMarker("{nope")).toBeNull();
    expect(decodeMarker('{"turn": 3}')).toBeNull(); // missing filename
  });

  it("markerPath is session-keyed and traversal-safe", () => {
    const p = markerPath(".pi/ckpt", "../../evil");
    expect(p.startsWith(".pi/ckpt/")).toBe(true);
    expect(p.split("/")).toHaveLength(3);
  });
});

describe("restoreIsCompatible", () => {
  it("same model restores; different model refuses (KV corruption guard)", () => {
    const m = { endpoint: "", slot: 0, filename: "f", turn: 1, model: "A.gguf", at: 0 };
    expect(restoreIsCompatible(m, "A.gguf")).toBe(true);
    expect(restoreIsCompatible(m, "B.gguf")).toBe(false);
    expect(restoreIsCompatible(m, "")).toBe(true); // engine validates when unknown
  });
});

// I5: a marker's KV blob must belong to THIS session — with parallel=1 the
// slot is reassigned by the engine (LCP/LRU), so a foreign-stem marker
// carries another conversation's state.
describe("markerMatchesSession", () => {
  it("accepts the session's own stem-encoded filename", () => {
    const m = { endpoint: "", slot: 0, filename: turnFilename(10, "sess-a"), turn: 10, model: "A", at: 0 };
    expect(markerMatchesSession(m, "sess-a")).toBe(true);
  });
  it("refuses a foreign session's blob", () => {
    const m = { endpoint: "", slot: 0, filename: turnFilename(10, "sess-b"), turn: 10, model: "A", at: 0 };
    expect(markerMatchesSession(m, "sess-a")).toBe(false);
  });
  it("refuses a same-stem turn mismatch", () => {
    const m = { endpoint: "", slot: 0, filename: turnFilename(10, "sess-a"), turn: 20, model: "A", at: 0 };
    expect(markerMatchesSession(m, "sess-a")).toBe(false);
  });
});

// I5: with live peers on the engine, the slot may hold (or be about to
// hold) a PEER's state — both autosave and crash-restore must stand down.
describe("checkpointPeerGate", () => {
  it("allows checkpointing when no peers are live", () => {
    expect(checkpointPeerGate(0).allowed).toBe(true);
    expect(checkpointPeerGate(0).reason).toBeUndefined();
  });
  it("blocks and explains when peers share the engine", () => {
    const v = checkpointPeerGate(2);
    expect(v.allowed).toBe(false);
    expect(v.reason).toContain("2 peer session");
  });
});

const LIVE = process.env.LC_LIVE === "1";
describe.skipIf(!LIVE)("live engine checkpoint (needs vitriol serve with slot_save_path)", () => {
  it("save then restore a round-trip on the running engine", async () => {
    const base = ckptConfig().endpoint;
    const save = await fetch(`${base}/slots/0?action=save`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ filename: "tris-test.bin" }),
    });
    const body = await save.json().catch(() => null);
    const parsed = parseSlotOpResponse(save.status, body);
    expect(parsed.ok, JSON.stringify(body)).toBe(true);
    expect(parsed.nTokens).toBeGreaterThan(0);
    const erase = await fetch(`${base}/slots/0?action=erase`, { method: "POST" });
    expect(erase.status).toBe(200); // cold before restore — round-trip must carry real state
    const restore = await fetch(`${base}/slots/0?action=restore`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ filename: "tris-test.bin" }),
    });
    const rb = await restore.json().catch(() => null);
    const rp = parseSlotOpResponse(restore.status, rb);
    expect(rp.ok, JSON.stringify(rb)).toBe(true);
    expect(rp.nTokens).toBe(parsed.nTokens); // same state back
  }, 150_000);
});
