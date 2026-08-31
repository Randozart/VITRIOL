import { afterAll, describe, it, expect } from "vitest";
import { decide, globToRegExp, matches, parseSnapshot, pathOf, type PermSnapshot } from "./perms.ts";

const snap = (over: Partial<PermSnapshot> = {}): PermSnapshot => ({
  default_action: "allow",
  rules: [
    { tool: "edit", pattern: "src/**", action: "allow" },
    { tool: "write", pattern: "**/.env", action: "deny" },
    { tool: "write", pattern: "**/.ssh/**", action: "ask" },
  ],
  source_hash: "h",
  ...over,
});

describe("glob semantics (the trap class)", () => {
  it("**/ means zero-or-more dirs, NOT wildcard-suffix", () => {
    expect(matches("**/.env", "/proj", "/proj/.env")).toBe(true);
    expect(matches("**/.env", "/proj", "/proj/a/b/.env")).toBe(true);
    expect(matches("**/.env", "/proj", "/proj/x.env")).toBe(false); // the over-match bug
  });

  it("src/** matches below src, not srcX", () => {
    expect(matches("src/**", "/proj", "src/a/b.ts")).toBe(true);
    expect(matches("src/**", "/proj", "src")).toBe(false);
    expect(matches("src/**", "/proj", "srcx/a.ts")).toBe(false);
  });

  it("single star does not cross directories", () => {
    expect(matches("*.ts", "/p", "a.ts")).toBe(true);
    expect(matches("*.ts", "/p", "a/b.ts")).toBe(false);
  });

  it("absolute paths under cwd match relative patterns; outside match abs form", () => {
    expect(matches("**/.env", "/proj", "/proj/.env")).toBe(true);
    expect(matches("/etc/**", "/proj", "/etc/passwd")).toBe(true);
    expect(matches("src/**", "/proj", "/other/src/a.ts")).toBe(false);
  });
});

describe("decide — first match wins", () => {
  it("returns the earliest matching rule even if later rules differ", () => {
    const s = snap({ rules: [{ tool: "write", pattern: "**", action: "deny" }, { tool: "write", pattern: "src/**", action: "allow" }] });
    expect(decide(s, "write", "src/a.ts", "/proj").action).toBe("deny");
  });

  it("tool mismatch skips; default when nothing matches", () => {
    const v = decide(snap(), "read", "docs/x.md", "/proj");
    expect(v.action).toBe("allow");
    expect(v.ruleIndex).toBe(-1);
  });

  it("deny/ask classification on the real policy", () => {
    const s = snap();
    expect(decide(s, "write", "apps/web/.env", "/repo").action).toBe("deny");
    expect(decide(s, "write", "/home/u/.ssh/id_ed25519", "/repo").action).toBe("ask");
  });
});

describe("parseSnapshot / pathOf", () => {
  it("rejects corrupt or unknown-action snapshots", () => {
    expect(parseSnapshot("{")).toBeNull();
    expect(parseSnapshot('{"default_action":"yolo","rules":[]}')).toBeNull();
    expect(parseSnapshot('{"default_action":"allow","rules":[{"tool":"x","pattern":"*","action":"explode"}]}')).toBeNull();
  });

  it("accepts the real mirror shape", () => {
    const s = parseSnapshot(JSON.stringify(snap()));
    expect(s?.rules[1].pattern).toBe("**/.env");
  });

  it("pathOf covers pi + legacy input keys", () => {
    expect(pathOf({ path: "a" })).toBe("a");
    expect(pathOf({ file_path: "b" })).toBe("b");
    expect(pathOf({ goal: "x" })).toBeNull();
    expect(pathOf(undefined)).toBeNull();
  });
});


// ── wiring: drive the real tool_call handler with stub pi/ctx ────────────
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import register from "./index.ts";

interface Stub { calls: Array<{ mode: string; hasUI: boolean; tool: string; path: string; confirmAnswer?: boolean }> }

function harness(snapshotText: string | null, env: Record<string, string>) {
  const prev = { ...process.env };
  Object.assign(process.env, env);
  for (const k of ["TRIS_NO_PERMS", "TRIS_PERMS_FILE"]) if (!(k in env)) delete process.env[k];
  let handler: ((e: unknown, c: unknown) => Promise<unknown>) | null = null;
  const notices: string[] = [];
  const pi = { on: (ev: string, h: never) => { if (ev === "tool_call") handler = h; }, registerTool: () => undefined };
  register(pi as never);
  const ctxFor = (mode: string, hasUI: boolean, confirmAnswer = false) => ({
    cwd: "/proj", mode, hasUI,
    ui: { notify: (m: string) => notices.push(m), confirm: async () => confirmAnswer },
  });
  const call = async (tool: string, path: string, mode = "print", hasUI = false, confirmAnswer = false) =>
    handler ? await handler({ toolName: tool, input: { path } }, ctxFor(mode, hasUI, confirmAnswer)) : undefined;
  return { call, notices, handlerWasRegistered: () => handler !== null, restore: () => { for (const k of Object.keys(process.env)) if (!(k in prev)) delete process.env[k]; Object.assign(process.env, prev); } };
}

describe("permissions-guard wiring", () => {
  const dir = mkdtempSync("/tmp/tris-perms-");
  const file = join(dir, "perms.json");
  writeFileSync(file, JSON.stringify({
    default_action: "allow",
    rules: [
      { tool: "write", pattern: "**/.env", action: "deny" },
      { tool: "write", pattern: "**/.ssh/**", action: "ask" },
    ],
    source_hash: "x",
  }));

  it("kill switch registers nothing", () => {
    const h = harness(null, { TRIS_NO_PERMS: "1" });
    expect(h.handlerWasRegistered()).toBe(false);
    h.restore();
  });

  it("missing snapshot: allows with exactly one warning", async () => {
    const h = harness(null, { TRIS_PERMS_FILE: join(dir, "absent.json") });
    expect(await h.call("write", "x.txt")).toBeUndefined();
    expect(await h.call("write", "y.txt")).toBeUndefined();
    expect(h.notices.filter((n) => n.includes("permissions")).length).toBe(1); // warned once
    h.restore();
  });

  it("deny blocks headless and TUI alike", async () => {
    const h = harness(null, { TRIS_PERMS_FILE: file });
    const r = await h.call("write", "apps/web/.env");
    expect((r as { block?: boolean }).block).toBe(true);
    expect(String((r as { reason?: string }).reason)).toContain("denies");
    h.restore();
  });

  it("ask fails closed in print mode", async () => {
    const h = harness(null, { TRIS_PERMS_FILE: file });
    const r = await h.call("write", "/home/u/.ssh/id", "print", false);
    expect((r as { block?: boolean }).block).toBe(true);
    expect(String((r as { reason?: string }).reason)).toContain("headless");
    h.restore();
  });

  it("ask prompts in TUI: declined blocks, approved passes", async () => {
    const h = harness(null, { TRIS_PERMS_FILE: file });
    const no = await h.call("write", "/home/u/.ssh/id", "tui", true, false);
    expect((no as { block?: boolean }).block).toBe(true);
    const yes = await h.call("write", "/home/u/.ssh/id", "tui", true, true);
    expect(yes).toBeUndefined();
    h.restore();
  });

  it("unrelated tools and non-path inputs pass through", async () => {
    const h = harness(null, { TRIS_PERMS_FILE: file });
    expect(await h.call("bash", "", "tui", true)).toBeUndefined();
    expect(await h.call("write", "src/clean.ts", "tui", true)).toBeUndefined();
    h.restore();
  });

  afterAll(() => rmSync(dir, { recursive: true, force: true }));
});
