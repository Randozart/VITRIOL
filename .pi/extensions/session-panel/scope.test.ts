// Session-panel scope (2026-11-16): a session renders ITS OWN provider's facts.
// Installs the real extension, then renders the REAL section registry.
import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const engine = vi.hoisted(() => ({
  started: 0,
  snap: {
    up: false,
    polled: true,
    delta: { tps: 0, tokens: 0 },
    ingest: { tps: 0, tokens: 0 },
    cumulativeIngest: 0,
    ejected: 0,
    loaded_model: "",
    loaded_path: "",
    rpc: [],
    stalled: false,
    total: 0,
    slots: [],
    slotsSeen: false,
    busy: 0,
    gpuLoad: 0,
  },
}));

vi.mock("../_shared/engine.ts", () => ({
  getEngineSnapshot: () => engine.snap,
  onEngineUpdate: () => () => {},
  startEnginePolling: () => {
    engine.started += 1;
  },
}));

import install from "./index.ts";
import { recordRoute } from "../_shared/route-ledger.ts";
import { renderAllSections } from "../_shared/sidebar.ts";

let home: string;
let ledgerDir: string;
let handlers: Map<string, (...args: unknown[]) => unknown>;

const plain = (lines: string[]) =>
  lines.join("\n").replace(/\x1b\[[0-9;]*m/g, "");
const sidebar = () => plain(renderAllSections());

const startSession = (provider: string, id: string, sessionId: string) =>
  handlers.get("session_start")?.(
    {},
    {
      cwd: home,
      hasUI: true,
      ui: { setSidebar: () => {}, setWidget: () => {}, notify: () => {} },
      model: { provider, id },
      sessionManager: {
        getSessionId: () => sessionId,
        getSessionFile: () => null,
      },
      getContextUsage: () => ({
        tokens: 12_000,
        contextWindow: 262_144,
        percent: 4.6,
      }),
    },
  );

beforeAll(() => {
  handlers = new Map();
  install({
    on: (event: string, handler: (...args: unknown[]) => unknown) =>
      handlers.set(event, handler),
    registerTool: () => {},
    registerCommand: () => {},
  } as unknown as ExtensionAPI);
});

beforeEach(() => {
  home = mkdtempSync(join(tmpdir(), "panel-scope-"));
  ledgerDir = mkdtempSync(join(tmpdir(), "panel-ledger-"));
  process.env.HOME = home;
  process.env.OFFICINA_ROUTE_LEDGER_DIR = ledgerDir;
  engine.started = 0;
  engine.snap.up = true;
  engine.snap.loaded_model = "Lapis Occultus";
  engine.snap.delta = { tps: 12.5, tokens: 10 };
  engine.snap.total = 1_000_000;
});

afterEach(() => {
  delete process.env.OFFICINA_ROUTE_LEDGER_DIR;
  rmSync(home, { recursive: true, force: true });
  rmSync(ledgerDir, { recursive: true, force: true });
});

describe("session-scoped sidebar telemetry", () => {
  it("shows a cloud session its own request, never the workshop engine", () => {
    recordRoute({
      ts: new Date().toISOString(),
      sessionId: "s-cloud",
      reason: "user",
      tier: "cloud",
      provider: "caduceus",
      model: "dahl/MiniMaxAI/MiniMax-M2.7",
      tokensIn: 69_481,
      tokensOut: 976,
      latencyMs: 44_706,
      costEstimate: 0,
      outcome: "ok",
    });
    startSession("caduceus", "dahl/MiniMaxAI/MiniMax-M2.7", "s-cloud");
    const text = sidebar();
    expect(text).toContain("api ");
    expect(text).toContain("dahl/MiniMaxAI/MiniMax-M2.7");
    expect(text).toContain("69.5k→976 tok");
    expect(text).toContain("44.7s");
    // No engine truth, no engine header, no local health (P2).
    expect(text).not.toContain("decoded this boot");
    expect(text).not.toContain("eng ");
    expect(text).not.toContain("local up");
    expect(text).not.toContain("Engine");
    // And it never started the shared /metrics poller (S6).
    expect(engine.started).toBe(0);
  });

  it("keeps a local session's engine rows exactly as before (P1)", () => {
    startSession("llamacpp", "Lapis Occultus", "s-local");
    const text = sidebar();
    expect(text).toContain("local up");
    expect(text).toContain("eng ");
    expect(text).toContain("decoded this boot");
    expect(engine.started).toBeGreaterThan(0);
  });

  it("names the free-lane pool for an officina/nimbus session", () => {
    mkdirSync(join(home, ".vitriol", "officina"), { recursive: true });
    writeFileSync(
      join(home, ".vitriol", "officina", "caduceus-state.json"),
      JSON.stringify({
        gateway_up: true,
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
        ],
        budget: { daily_tokens: 12_000, daily_cap: 2_000_000 },
      }),
    );
    require("node:fs").mkdirSync(join(home, ".vitriol", "officina"), {
      recursive: true,
    });
    startSession("officina", "nimbus", "s-lanes");
    const text = sidebar();
    expect(text).toContain("nimbus ");
    expect(text).toContain("2 lanes");
    // ctx 12k → the first lane already fits.
    expect(text).toContain("next dahl/MiniMaxAI/MiniMax-M2.7");
    expect(text).not.toContain("decoded this boot");
  });

  it("stays silent when no provider is known yet (P4)", () => {
    startSession("llamacpp", "Lapis Occultus", "s-warm");
    engine.snap.up = false; // engine down: no engine rows either
    startSession("", "", "s-none");
    const text = sidebar();
    expect(text).not.toContain("api ");
    expect(text).not.toContain("eng ");
  });
});
