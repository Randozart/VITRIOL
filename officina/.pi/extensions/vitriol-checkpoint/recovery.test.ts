import { afterEach, describe, expect, it, vi } from "vitest";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import checkpointExtension from "./index.ts";
import { encodeMarker, markerPath, turnFilename } from "./client.ts";
import { quarantineRecoveryMarker, shouldQuarantineRestore } from "./recovery.ts";
import { readLivePeers } from "../_shared/routing-registry.ts";

vi.mock("../_shared/routing-registry.ts", () => ({ readLivePeers: vi.fn(() => []) }));
vi.mock("../_shared/events.ts", () => ({ emitHarnessEvent: vi.fn(), harnessEvent: vi.fn() }));

const directories: string[] = [];
const rejected = "Unable to restore slot: No available space in KV cache or invalid slot save file";

/** Keep every test marker and backup outside the operator's real stores. */
function directory(): string {
  const dir = mkdtempSync(join(tmpdir(), "officina-restore-"));
  directories.push(dir);
  return dir;
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
  vi.mocked(readLivePeers).mockReturnValue([]);
  for (const dir of directories.splice(0)) rmSync(dir, { recursive: true, force: true });
});

/** Load the actual extension with mocked HTTP and real temporary marker files. */
function recoveryFixture(status = 400, error = rejected) {
  const dir = directory();
  vi.stubEnv("TRIS_CKPT_DIR", dir);
  vi.stubEnv("TRIS_NO_VITRIOL_CKPT", "0");
  const marker = markerPath(dir, "session");
  const raw = encodeMarker({ endpoint: "http://127.0.0.1:8279", slot: 0,
    filename: turnFilename(50, "session"), turn: 50, model: "nimbus", at: 1 });
  writeFileSync(marker, raw);
  const fetch = vi.fn(async () => new Response(JSON.stringify({ error: { message: error } }), { status }));
  vi.stubGlobal("fetch", fetch);
  const handlers = new Map<string, (event: unknown, ctx: unknown) => Promise<unknown>>();
  const pi = { on: (name: string, handler: (event: unknown, ctx: unknown) => Promise<unknown>) => handlers.set(name, handler), registerTool: () => {} };
  checkpointExtension(pi as unknown as ExtensionAPI);
  const notify = vi.fn();
  const ctx = { sessionManager: { getSessionFile: () => "/tmp/session.jsonl" }, model: { id: "nimbus" }, ui: { notify } };
  const start = () => handlers.get("session_start")!({}, ctx);
  return { dir, marker, raw, fetch, notify, start, ctx, handlers };
}

describe("restore failure disposition", () => {
  it.each([400, 422])("quarantines the observed checkpoint rejection (%i)", (status) => {
    expect(shouldQuarantineRestore({ ok: false, status, error: rejected })).toBe(true);
  });
  it("recognizes explicit missing files without confusing an absent endpoint", () => {
    expect(shouldQuarantineRestore({ ok: false, status: 404, error: "checkpoint file not found" })).toBe(true);
    expect(shouldQuarantineRestore({ ok: false, status: 404, error: "Not Found" })).toBe(false);
    expect(shouldQuarantineRestore({ ok: false, status: 400, error: "Invalid slot ID" })).toBe(false);
  });
  it.each([0, 401, 403, 408, 429, 500, 503])("keeps recovery available for temporary/unclassified status %i", (status) => {
    expect(shouldQuarantineRestore({ ok: false, status, error: rejected })).toBe(false);
  });
});

describe("checkpoint recovery integration", () => {
  it("tries a rejected checkpoint once across reloads and preserves every marker byte", async () => {
    const h = recoveryFixture();
    await h.start();
    expect(existsSync(h.marker)).toBe(false);
    const backups = readdirSync(join(h.dir, "failed-restores"));
    const backup = join(h.dir, "failed-restores", backups[0], "session.json");
    expect(readFileSync(backup, "utf8")).toBe(h.raw);
    expect(h.notify.mock.calls[0][0]).toContain(backup);
    await h.start();
    expect(h.fetch).toHaveBeenCalledTimes(1);
  });
  it.each([401, 429, 503])("retains the active marker and retries after HTTP %i", async (status) => {
    const h = recoveryFixture(status);
    await h.start();
    await h.start();
    expect(readFileSync(h.marker, "utf8")).toBe(h.raw);
    expect(h.fetch).toHaveBeenCalledTimes(2);
  });
  it("preserves recovery after network failure and timeout", async () => {
    const h = recoveryFixture();
    h.fetch.mockRejectedValue(new Error("fetch failed"));
    await h.start();
    expect(readFileSync(h.marker, "utf8")).toBe(h.raw);
    h.fetch.mockRejectedValue(new DOMException("Timed out", "TimeoutError"));
    await h.start();
    expect(readFileSync(h.marker, "utf8")).toBe(h.raw);
  });
  it("keeps successful restore behavior and does not quarantine", async () => {
    const h = recoveryFixture(200);
    await h.start();
    expect(existsSync(h.marker)).toBe(false);
    expect(existsSync(join(h.dir, "failed-restores"))).toBe(false);
    expect(h.notify.mock.calls[0][0]).toContain("restored warm KV");
  });
  it("does not touch recovery or call the engine when peers are active", async () => {
    const h = recoveryFixture();
    vi.mocked(readLivePeers).mockReturnValue([{} as never]);
    await h.start();
    expect(h.fetch).not.toHaveBeenCalled();
    expect(readFileSync(h.marker, "utf8")).toBe(h.raw);
  });
  it("fails safely if the backup directory cannot be created", async () => {
    const h = recoveryFixture();
    writeFileSync(join(h.dir, "failed-restores"), "occupied");
    await h.start();
    expect(readFileSync(h.marker, "utf8")).toBe(h.raw);
    expect(h.notify.mock.calls[0][0]).toContain("marker kept");
  });
  it("allows a later autosave to establish a fresh recovery marker", async () => {
    const h = recoveryFixture();
    await h.start();
    h.fetch.mockImplementation(async () => new Response("{}", { status: 200 }));
    await h.handlers.get("turn_end")!({ turnIndex: 60 }, h.ctx);
    expect(JSON.parse(readFileSync(h.marker, "utf8")).turn).toBe(60);
  });
});

describe("marker preservation", () => {
  it("handles long repaired-session marker basenames without exceeding NAME_MAX", () => {
    const path = join(directory(), `${"s".repeat(237)}.json`);
    writeFileSync(path, "original marker");
    const backup = quarantineRecoveryMarker(path, "original marker");
    expect(backup).toBeDefined();
    expect(readFileSync(backup!, "utf8")).toBe("original marker");
  });
  it("does not quarantine a marker that changed during the request", () => {
    const path = join(directory(), "session.json");
    writeFileSync(path, "new successful save");
    expect(quarantineRecoveryMarker(path, "old rejected save")).toBeUndefined();
    expect(readFileSync(path, "utf8")).toBe("new successful save");
  });
});
