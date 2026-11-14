// GRIND wiring tests: the loop must continue, stop strictly, and always yield
// to the user / abort. A fake pi captures the registered handlers.
//
// The trigger is agent_settled (pi's "nothing else will run" point); the
// assistant text for the marker check arrives via message_end.

import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { beforeEach, describe, expect, it } from "vitest";

type Handler = (event: any, ctx: any) => unknown;

function fakePi() {
  const handlers = new Map<string, Handler[]>();
  return {
    on(event: string, fn: Handler) {
      const list = handlers.get(event) ?? [];
      list.push(fn);
      handlers.set(event, list);
    },
    async fire(event: string, payload: any, ctx: any) {
      for (const fn of handlers.get(event) ?? []) await fn(payload, ctx);
    },
    count(event: string) {
      return handlers.get(event)?.length ?? 0;
    },
  };
}

function makeCtx(cwd: string) {
  const sent: string[] = [];
  const notices: string[] = [];
  return {
    ctx: {
      cwd,
      sendUserMessage: async (text: string) => {
        sent.push(text);
      },
      ui: {
        notify: (text: string) => {
          notices.push(text);
        },
        setWidget: () => {},
      },
    },
    sent,
    notices,
  };
}

/** Fresh HOME so the mode facts file is isolated per test. */
function setMode(mode: string): string {
  const home = mkdtempSync(join(tmpdir(), "grind-home-"));
  mkdirSync(join(home, ".vitriol", "officina"), { recursive: true });
  writeFileSync(
    join(home, ".vitriol", "officina", "agent-mode.json"),
    JSON.stringify({ version: 1, ts: Date.now(), mode, label: mode.toUpperCase() }),
  );
  process.env.HOME = home;
  process.env.TRIS_TASKS_DIR = join(home, "tasks-none");
  process.env.OFFICINA_NO_GRIND = "0";
  delete process.env.OFFICINA_GRIND_MAX_TURNS;
  return home;
}

async function loadExtension(pi: ReturnType<typeof fakePi>) {
  const mod = await import("./index.ts");
  mod.default(pi as unknown as Parameters<typeof mod.default>[0]);
}

/** Simulate a completed turn: assistant message, then settle. */
async function settle(
  pi: ReturnType<typeof fakePi>,
  ctx: any,
  text: string,
  opts: { aborted?: boolean; stopReason?: string } = {},
) {
  await pi.fire(
    "message_end",
    {
      message: {
        role: "assistant",
        content: [{ type: "text", text }],
        stopReason: opts.stopReason,
      },
    },
    ctx,
  );
  await pi.fire("agent_settled", { aborted: opts.aborted ?? false }, ctx);
}

describe("grind loop wiring", () => {
  beforeEach(() => {
    setMode("grind");
  });

  it("registers its listeners and stays inert outside grind", async () => {
    setMode("build");
    const pi = fakePi();
    await loadExtension(pi);
    expect(pi.count("agent_settled")).toBe(1);
    const { ctx, sent } = makeCtx(mkdtempSync(join(tmpdir(), "grind-cwd-")));
    await settle(pi, ctx, "worked");
    expect(sent).toHaveLength(0);
  });

  it("sends one continuation when the turn ends without a marker", async () => {
    const pi = fakePi();
    await loadExtension(pi);
    const { ctx, sent } = makeCtx(mkdtempSync(join(tmpdir(), "grind-cwd-")));
    await settle(pi, ctx, "did half the work");
    expect(sent).toHaveLength(1);
    expect(sent[0]).toContain("GRIND round 1/24");
    expect(sent[0]).toContain("GRIND: DONE");
  });

  it("stops cleanly on GRIND: DONE with no open tasks", async () => {
    const pi = fakePi();
    await loadExtension(pi);
    const { ctx, sent, notices } = makeCtx(mkdtempSync(join(tmpdir(), "grind-cwd-")));
    await settle(pi, ctx, "everything green\nGRIND: DONE");
    expect(sent).toHaveLength(0);
    expect(notices.some((n) => n.includes("GRIND: DONE"))).toBe(true);
  });

  it("surfaces BLOCKED and stops", async () => {
    const pi = fakePi();
    await loadExtension(pi);
    const { ctx, sent, notices } = makeCtx(mkdtempSync(join(tmpdir(), "grind-cwd-")));
    await settle(pi, ctx, "GRIND: BLOCKED: needs the prod key");
    expect(sent).toHaveLength(0);
    expect(notices.some((n) => n.includes("needs the prod key"))).toBe(true);
  });

  it("never re-prompts after input from the user", async () => {
    const pi = fakePi();
    await loadExtension(pi);
    const { ctx, sent } = makeCtx(mkdtempSync(join(tmpdir(), "grind-cwd-")));
    await settle(pi, ctx, "worked");
    expect(sent).toHaveLength(1);
    // User types during the loop → stand down.
    await pi.fire("input", { source: "interactive", text: "stop" }, ctx);
    await settle(pi, ctx, "worked");
    expect(sent).toHaveLength(1);
    // A fresh user turn re-arms.
    await pi.fire("input", { source: "rpc", text: "new task" }, ctx);
    await settle(pi, ctx, "worked");
    expect(sent).toHaveLength(2);
  });

  it("ignores extension-sourced input", async () => {
    const pi = fakePi();
    await loadExtension(pi);
    const { ctx, sent } = makeCtx(mkdtempSync(join(tmpdir(), "grind-cwd-")));
    await settle(pi, ctx, "worked");
    await pi.fire("input", { source: "extension", text: "reminder" }, ctx);
    await settle(pi, ctx, "worked");
    expect(sent).toHaveLength(2);
  });

  it("never re-prompts after an abort (settled flag or stopReason)", async () => {
    const pi = fakePi();
    await loadExtension(pi);
    const { ctx, sent } = makeCtx(mkdtempSync(join(tmpdir(), "grind-cwd-")));
    await settle(pi, ctx, "worked");
    expect(sent).toHaveLength(1);
    await settle(pi, ctx, "partial", { aborted: true });
    expect(sent).toHaveLength(1);
    await pi.fire("input", { source: "rpc", text: "again" }, ctx);
    await settle(pi, ctx, "partial", { stopReason: "aborted" });
    expect(sent).toHaveLength(1);
  });

  it("stands down when the user queued a steering message", async () => {
    const pi = fakePi();
    await loadExtension(pi);
    const { ctx, sent } = makeCtx(mkdtempSync(join(tmpdir(), "grind-cwd-")));
    await pi.fire(
      "input",
      { source: "interactive", text: "do X instead", streamingBehavior: "steer" },
      ctx,
    );
    await settle(pi, ctx, "worked");
    expect(sent).toHaveLength(0);
  });

  it("enforces the turn cap", async () => {
    process.env.OFFICINA_GRIND_MAX_TURNS = "1";
    const pi = fakePi();
    await loadExtension(pi);
    const { ctx, sent, notices } = makeCtx(mkdtempSync(join(tmpdir(), "grind-cwd-")));
    await settle(pi, ctx, "worked once");
    expect(sent).toHaveLength(1);
    await settle(pi, ctx, "worked twice");
    expect(sent).toHaveLength(1);
    expect(notices.some((n) => n.includes("cap"))).toBe(true);
  });
});
