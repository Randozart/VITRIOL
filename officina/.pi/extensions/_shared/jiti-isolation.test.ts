import { afterEach, beforeAll, afterAll, describe, expect, it } from "vitest";
import { createRequire } from "node:module";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

// Cross-instance regression guard (2026-09-06).
//
// pi loads every -e extension through a SEPARATE jiti instance with
// moduleCache: false (pi loader.js:325-332) — each extension gets its own
// module scope, so module-level state does NOT cross extension boundaries.
// Every cross-extension contract must therefore be globalThis-backed:
//   - _shared/sidebar.ts registry + listeners   (2026-09-04)
//   - task-state / scratchpad session file paths (2026-09-06)
//   - _shared/engine.ts poller + snapshot        (2026-09-06)
// These tests recreate pi's loader EXACTLY: two independent jiti instances
// over the same files, one Node process. Before the fixes they failed
// silently: the sidebar stayed stale, task rows vanished, and three engine
// pollers hammered /metrics + /slots at once.

const req = createRequire(
  fileURLToPath(new URL("../../../node_modules/@earendil-works/pi-coding-agent/package.json", import.meta.url)),
);
const { createJiti } = req("jiti") as typeof import("jiti");

const HERE = dirname(fileURLToPath(import.meta.url)); // .../extensions/_shared
const EXT = dirname(HERE); // .../extensions
const TASK_STATE = join(EXT, "task-state", "index.ts");
const SCRATCHPAD = join(EXT, "scratchpad", "index.ts");
const SIDEBAR = join(EXT, "_shared", "sidebar.ts");
const ENGINE = join(EXT, "_shared", "engine.ts");

/** Two independent jiti instances — the exact loader shape pi uses. */
function jitiPair() {
  const a = createJiti(import.meta.url, { moduleCache: false });
  const b = createJiti(import.meta.url, { moduleCache: false });
  return { a, b } as const;
}

interface ToolLike {
  execute(id: string, input: unknown): Promise<{ content?: Array<{ text?: string }>; isError?: boolean }>;
}

/** Capture pi's registration surface so tests can fire handlers/tools. */
function fakePi() {
  const tools: Record<string, ToolLike> = {};
  const handlers: Record<string, Array<(_event: unknown, ctx: unknown) => unknown>> = {};
  const pi = {
    registerTool: (t: { name: string; execute: ToolLike["execute"] }) => { tools[t.name] = { execute: t.execute }; },
    on: (ev: string, fn: (_event: unknown, ctx: unknown) => unknown) => { (handlers[ev] ??= []).push(fn); },
    registerCommand: () => {},
  };
  return { pi, tools, handlers } as const;
}

function sessionCtxFor(sessionFile: string) {
  return { hasUI: false, sessionManager: { getSessionFile: () => sessionFile } };
}

describe("jiti isolation: cross-extension contracts survive separate module instances", () => {
  let dir: string;
  let unsubs: Array<() => void> = [];

  beforeAll(() => {
    dir = mkdtempSync(join(tmpdir(), "jiti-isolation-"));
    // Set BEFORE the first jiti.import — module init reads these.
    process.env.TRIS_TASKS_DIR = dir;
    process.env.OFFICINA_SCRATCHPAD_DIR = dir;
    // Engine test points polling at a dead port: fetches fail fast, poll()
    // never throws (observability contract), no real HTTP noise.
    process.env.VITRIOL_BASE_URL = "http://127.0.0.1:1";
  });
  afterAll(() => {
    for (const k of ["TRIS_TASKS_DIR", "OFFICINA_SCRATCHPAD_DIR", "VITRIOL_BASE_URL"]) delete process.env[k];
    rmSync(dir, { recursive: true, force: true });
  });
  afterEach(() => {
    for (const u of unsubs.splice(0)) u();
  });

  it("task-state: write on instance A is visible to instance B's sidebar getter, and the update signal crosses", async () => {
    const { a, b } = jitiPair();
    const tsA = (await a.import(TASK_STATE, { default: true })) as (pi: unknown) => void;
    const { pi, tools, handlers } = fakePi();
    tsA(pi);
    // session_start on A repoints the shared session-file state
    for (const h of handlers["session_start"] ?? []) {
      await h({}, sessionCtxFor(join(dir, "2026-09-06T00-00-00-000Z_task-a.jsonl")));
    }

    // namespace import (NO default:true) — we need the getter exports
    const sbB = (await b.import(SIDEBAR)) as typeof import("./sidebar.ts");
    const tsB = (await b.import(TASK_STATE)) as typeof import("../task-state/index.ts");
    let fired = 0;
    unsubs.push(sbB.onSidebarUpdate(() => { fired++; }));

    await tools["update_tasks"].execute("1", {
      tasks: [{ id: 1, description: "cross-instance guard", status: "in_progress" }],
    });

    expect(fired).toBe(1); // listener sharing (sidebar.ts globalThis fix)
    const summary = tsB.getTaskSummary();
    expect(summary).not.toBeNull(); // file-state sharing (globalThis fix)
    expect(summary?.total).toBe(1);
    expect(summary?.inProgress).toBe(1);
  });

  it("scratchpad: write on instance A is visible to instance B's sidebar getter", async () => {
    const { a, b } = jitiPair();
    const spA = (await a.import(SCRATCHPAD, { default: true })) as (pi: unknown) => void;
    const { pi, tools, handlers } = fakePi();
    spA(pi);
    for (const h of handlers["session_start"] ?? []) {
      await h({}, sessionCtxFor(join(dir, "2026-09-06T00-00-01-000Z_scratch-a.jsonl")));
    }

    const spB = (await b.import(SCRATCHPAD)) as typeof import("../scratchpad/index.ts");
    const summary = await (async () => {
      await tools["scratchpad_write"].execute("1", { facts: ["jiti guard fact"] });
      return spB.getScratchpadSummary();
    })();
    expect(summary).not.toBeNull();
    expect(summary?.facts).toBe(1);
    expect(summary?.lines).toBe(1);
  });

  it("engine: one startEnginePolling call is visible across instances (single shared poller)", async () => {
    const { a, b } = jitiPair();
    const engA = (await a.import(ENGINE)) as typeof import("./engine.ts");
    const engB = (await b.import(ENGINE)) as typeof import("./engine.ts");

    expect(engB.enginePollerActive()).toBe(false);
    engA.startEnginePolling();
    // Before the globalThis fix, B had its OWN timer variable — this was false.
    expect(engB.enginePollerActive()).toBe(true);

    let notified = 0;
    const unsub = engB.onEngineUpdate(() => { notified++; });
    // Poll ticks are async; give the dead-port fetch a moment to fail through.
    await new Promise((r) => setTimeout(r, 50));
    unsub();
    engA.stopEnginePolling();
    expect(engB.enginePollerActive()).toBe(false); // stop propagates too
    // The dead-port poll still ran and notified the shared listener set
    // (down-snapshot is a valid notification).
    expect(notified).toBeGreaterThanOrEqual(1);
  });
});
