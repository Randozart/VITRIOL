import { afterAll, describe, it, expect } from "vitest";
import { decide, globToRegExp, matches, parseSnapshot, pathOf, type PermSnapshot } from "./perms.ts";
import { commandVerdict, parseMode, pathVerdict, protectedPath } from "./policy.ts";

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
    expect(matches("**/.env", "/proj", "/proj/x.env")).toBe(false);
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

describe("permission modes + protected roots", () => {
  const home = "/home/u";
  const cwd = "/proj";

  it("parses modes and defaults to ask", () => {
    expect(parseMode("yolo")).toBe("yolo");
    expect(parseMode("AUTO")).toBe("auto");
    expect(parseMode("nonsense")).toBe("ask");
  });

  it("protected roots win in every mode", () => {
    for (const mode of ["ask", "auto", "yolo"] as const) {
      expect(protectedPath("/etc/passwd", cwd, home)).toBe(true);
      expect(protectedPath(`${home}/.ssh/id_ed25519`, cwd, home)).toBe(true);
      expect(protectedPath(".env", cwd, home)).toBe(true);
      expect(pathVerdict("write", "/etc/passwd", cwd, home, mode, null).action).toBe("ask");
    }
  });

  it("ask mode allows in-cwd writes but asks outside", () => {
    expect(pathVerdict("write", "src/a.ts", cwd, home, "ask", null).action).toBe("allow");
    expect(pathVerdict("write", "/tmp/out.txt", cwd, home, "ask", null).action).toBe("ask");
  });

  it("yolo allows outside writes but not protected roots", () => {
    expect(pathVerdict("write", "/tmp/out.txt", cwd, home, "yolo", null).action).toBe("allow");
    expect(pathVerdict("write", "/etc/hosts", cwd, home, "yolo", null).action).toBe("ask");
  });

  it("catastrophic shell commands deny; risky shell asks only in ask mode", () => {
    expect(commandVerdict("rm -rf /", cwd, home, "yolo").action).toBe("deny");
    expect(commandVerdict("mkfs.ext4 /dev/sda", cwd, home, "auto").action).toBe("deny");
    expect(commandVerdict("sudo apt install x", cwd, home, "ask").action).toBe("ask");
    expect(commandVerdict("sudo apt install x", cwd, home, "yolo").action).toBe("allow");
    expect(commandVerdict("echo ok > /etc/motd", cwd, home, "yolo").action).toBe("ask");
    expect(commandVerdict("cargo test", cwd, home, "ask").action).toBe("allow");
    // Benign redirects must not trip the protected-root ask (2026-11-10).
    expect(
      commandVerdict("ls -t SESSION_LOG_* 2>/dev/null | head -5", cwd, home, "yolo").action,
    ).toBe("allow");
    expect(commandVerdict("cat /dev/sda", cwd, home, "yolo").action).toBe("ask");
  });
});

// ── wiring: drive the real tool_call handler with stub pi/ctx ────────────
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import register from "./index.ts";

function harness(snapshotText: string | null, env: Record<string, string>, mode = "ask") {
  const prev = { ...process.env };
  Object.assign(process.env, env);
  for (const k of ["TRIS_NO_PERMS", "TRIS_PERMS_FILE"]) if (!(k in env)) delete process.env[k];
  const dir = mkdtempSync("/tmp/tris-perms-mode-");
  const modeFile = join(dir, "mode");
  writeFileSync(modeFile, mode);
  process.env.OFFICINA_PERMISSIONS_MODE_FILE = modeFile;
  let handler: ((e: unknown, c: unknown) => Promise<unknown>) | null = null;
  const notices: string[] = [];
  const pi = { on: (ev: string, h: never) => { if (ev === "tool_call") handler = h; }, registerTool: () => undefined };
  register(pi as never);
  const ctxFor = (mode: string, hasUI: boolean, confirmAnswer = false) => ({
    cwd: "/proj", mode, hasUI,
    ui: { notify: (m: string) => notices.push(m), confirm: async () => confirmAnswer },
  });
  const call = async (tool: string, input: Record<string, unknown>, modeCtx = "print", hasUI = false, confirmAnswer = false) =>
    handler ? await handler({ toolName: tool, input }, ctxFor(modeCtx, hasUI, confirmAnswer)) : undefined;
  const restore = () => { rmSync(dir, { recursive: true, force: true }); for (const k of Object.keys(process.env)) if (!(k in prev)) delete process.env[k]; Object.assign(process.env, prev); };
  return { call, notices, handlerWasRegistered: () => handler !== null, restore };
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

  it("sandboxed mode wraps bash; other modes never do (I1/I6)", async () => {
    const input = { command: "echo hi" };
    const h = harness(null, {
      TRIS_PERMS_FILE: join(dir, "absent.json"),
      HOME: "/home/u",
      OFFICINA_SANDBOX_BWRAP: "/bin/sh",
    }, "sandboxed");
    expect(await h.call("bash", input)).toBeUndefined();
    // OFFICINA_SANDBOX_BWRAP=/bin/sh override → the binary path is /bin/sh
    expect(String(input.command)).toContain("/bin/sh");
    expect(String(input.command)).toContain("--ro-bind");
    expect(String(input.command)).toContain("--unshare-net");
    expect(String(input.command)).toContain("echo hi");

    // yolo: byte-identical command
    const input2 = { command: "echo hi" };
    const h2 = harness(null, {
      TRIS_PERMS_FILE: join(dir, "absent.json"),
      HOME: "/home/u",
      OFFICINA_SANDBOX_BWRAP: "/bin/sh",
    }, "yolo");
    await h2.call("bash", input2);
    expect(input2.command).toBe("echo hi");
    h2.restore();

    // missing bwrap: one warning, command untouched
    const input3 = { command: "echo hi" };
    const h3 = harness(null, {
      TRIS_PERMS_FILE: join(dir, "absent.json"),
      HOME: "/home/u",
      OFFICINA_SANDBOX_BWRAP: "/definitely/not/here",
    }, "sandboxed");
    await h3.call("bash", input3);
    expect(input3.command).toBe("echo hi");
    expect(h3.notices.filter((n) => n.includes("bwrap")).length).toBe(1);
    await h3.call("bash", { command: "echo again" });
    expect(h3.notices.filter((n) => n.includes("bwrap")).length).toBe(1);
    h3.restore();

    // network opt-in drops --unshare-net
    const input4 = { command: "echo hi" };
    const h4 = harness(null, {
      TRIS_PERMS_FILE: join(dir, "absent.json"),
      HOME: "/home/u",
      OFFICINA_SANDBOX_BWRAP: "/bin/sh",
      OFFICINA_SANDBOX_NET: "1",
    }, "sandboxed");
    await h4.call("bash", input4);
    expect(String(input4.command)).not.toContain("--unshare-net");
    h4.restore();
    h.restore();
  });

  it("auto-dispatch: long-runners are rewritten; #fg/kill-switch/short/sandboxed are not (I4/I6)", async () => {
    const base = { TRIS_PERMS_FILE: join(dir, "absent.json"), HOME: "/home/u" };

    // auto mode: cargo build → dispatch rewrite + pointer notice
    const input = { command: "cargo build --release" };
    const h = harness(null, { ...base, OFFICINA_SANDBOX_BWRAP: "/bin/sh" }, "auto");
    expect(await h.call("bash", input)).toBeUndefined();
    expect(String(input.command)).toContain("setsid bash -c");
    expect(String(input.command)).toContain("longjob");
    expect(String(input.command)).toContain("cargo build --release");
    expect(h.notices.some((n) => n.includes("dispatched"))).toBe(true);
    h.restore();

    // I4: #fg opt-out — byte-identical
    const input2 = { command: "cargo build --release #fg" };
    const h2 = harness(null, { ...base }, "auto");
    await h2.call("bash", input2);
    expect(input2.command).toBe("cargo build --release #fg");
    h2.restore();

    // I6: kill switch — byte-identical
    const input3 = { command: "cargo build --release" };
    const h3 = harness(null, { ...base, OFFICINA_NO_AUTOBG: "1" }, "auto");
    await h3.call("bash", input3);
    expect(input3.command).toBe("cargo build --release");
    h3.restore();

    // I4 precision: short commands untouched
    const input4 = { command: "ls -la" };
    const h4 = harness(null, { ...base }, "auto");
    await h4.call("bash", input4);
    expect(input4.command).toBe("ls -la");
    h4.restore();

    // sandboxed mode skips dispatch, wraps instead (bwrap + detached deferred)
    const input5 = { command: "cargo build --release" };
    const h5 = harness(null, { ...base, OFFICINA_SANDBOX_BWRAP: "/bin/sh" }, "sandboxed");
    await h5.call("bash", input5);
    expect(String(input5.command)).toContain("/bin/sh");
    expect(String(input5.command)).not.toContain("setsid");
    h5.restore();
  });

  it("missing snapshot: mode policy still applies, warned once", async () => {
    const h = harness(null, { TRIS_PERMS_FILE: join(dir, "absent.json"), HOME: "/home/u" });
    expect(await h.call("write", { path: "x.txt" })).toBeUndefined();
    expect(await h.call("write", { path: "y.txt" })).toBeUndefined();
    expect(h.notices.filter((n) => n.includes("permissions")).length).toBe(1);
    h.restore();
  });

  it("deny blocks headless and TUI alike", async () => {
    const h = harness(null, { TRIS_PERMS_FILE: file, HOME: "/home/u" });
    const r = await h.call("write", { path: "apps/web/.env" });
    expect((r as { block?: boolean }).block).toBe(true);
    expect(String((r as { reason?: string }).reason)).toContain("denies");
    h.restore();
  });

  it("protected/ask fails closed in print mode", async () => {
    const h = harness(null, { TRIS_PERMS_FILE: file, HOME: "/home/u" });
    const r = await h.call("write", { path: "/home/u/.ssh/id" }, "print", false);
    expect((r as { block?: boolean }).block).toBe(true);
    expect(String((r as { reason?: string }).reason)).toContain("headless");
    h.restore();
  });

  it("ask prompts in TUI: declined blocks, approved passes", async () => {
    const h = harness(null, { TRIS_PERMS_FILE: file, HOME: "/home/u" });
    const no = await h.call("write", { path: "/home/u/.ssh/id" }, "rpc", true, false);
    expect((no as { block?: boolean }).block).toBe(true);
    const yes = await h.call("write", { path: "/home/u/.ssh/id" }, "rpc", true, true);
    expect(yes).toBeUndefined();
    h.restore();
  });

  it("bash: safe passes, sudo asks, yolo allows non-protected sudo", async () => {
    const h = harness(null, { TRIS_PERMS_FILE: file, HOME: "/home/u" });
    expect(await h.call("bash", { command: "ls -la" })).toBeUndefined();
    const ask = await h.call("bash", { command: "sudo -n true" }, "rpc", true, false);
    expect((ask as { block?: boolean }).block).toBe(true);
    h.restore();

    const y = harness(null, { TRIS_PERMS_FILE: file, HOME: "/home/u" }, "yolo");
    expect(await y.call("bash", { command: "sudo -n true" }, "rpc", true, false)).toBeUndefined();
    const root = await y.call("bash", { command: "echo x > /etc/motd" }, "rpc", true, false);
    expect((root as { block?: boolean }).block).toBe(true);
    y.restore();
  });

  afterAll(() => rmSync(dir, { recursive: true, force: true }));
});
