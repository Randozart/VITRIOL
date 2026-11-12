// End-to-end wiring test for the verify extension (fake exec, mock pi).
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import type { ExecFn } from "./runner.ts";
import verifyExt from "./index.ts";

let root: string;
beforeAll(() => {
  root = mkdtempSync(join(tmpdir(), "verify-idx-"));
  writeFileSync(join(root, "package.json"), JSON.stringify({
    scripts: { typecheck: "tsc --noEmit", test: "vitest --run" },
    devDependencies: { vitest: "^3" },
  }));
  writeFileSync(join(root, "tsconfig.json"), "{}");
});
afterAll(() => rmSync(root, { recursive: true, force: true }));

type Handler = (event: unknown, ctx: unknown) => unknown;
interface MockPi {
  handlers: Map<string, Handler>;
  tools: Map<string, any>;
  commands: Map<string, any>;
  on: (ev: string, h: Handler) => void;
  registerTool: (t: any) => void;
  registerCommand: (n: string, c: any) => void;
}
function mockPi(): MockPi {
  const handlers = new Map<string, Handler>();
  const tools = new Map<string, any>();
  const commands = new Map<string, any>();
  return {
    handlers, tools, commands,
    on: (ev, h) => { handlers.set(ev, h); },
    registerTool: (t) => { tools.set(t.name, t); },
    registerCommand: (n, c) => { commands.set(n, c); },
  };
}

const tick = (ms = 40) => new Promise((r) => setTimeout(r, ms));

function sessionCtx() {
  return { hasUI: false, ui: null, cwd: root };
}
function editEvent(absPath: string) {
  return { toolName: "edit", isError: false, input: { path: absPath } };
}

const report = (r: { content: Array<{ text: string }> }) => r.content[0]!.text;

afterEach(() => {
  delete process.env.OFFICINA_NO_VERIFY;
});

describe("verify extension e2e", () => {
  it("registers tool + command; auto-baseline → pre-existing → in-progress → regression ladder", async () => {
    const pi = mockPi();
    const path = join(root, "old.ts");
    const touched = join(root, "touched.ts");
    const bystander = join(root, "bystander.ts");
    let typecheckOut = `${path}(1,1): error TS1: preexisting`;

    const exec: ExecFn = async (argv) => {
      const cmd = argv.join(" ");
      if (cmd.includes("typecheck")) {
        return typecheckOut === "" ? { code: 0, out: "" } : { code: 1, out: typecheckOut };
      }
      return { code: 0, out: "" }; // npm test green
    };

    verifyExt(pi as never, { exec });
    expect(pi.tools.has("verify")).toBe(true);
    expect(pi.commands.has("verify")).toBe(true);

    // session start → cheap auto pass captures baseline (fire-and-forget)
    pi.handlers.get("session_start")!({}, sessionCtx());
    await tick();
    // observable side effect: harness event appended for the auto run
    // (baseline itself is internal — prove it through the next tool run)

    // 1) same reality → pre-existing, no red
    let out = report(await pi.tools.get("verify")!.execute("1", { tier: "full" }));
    expect(out).toMatch(/PRE-EXISTING \(1\)/);
    expect(out).not.toMatch(/REGRESSIONS \(/);
    expect(out).not.toMatch(/IN-PROGRESS \(/);

    // 2) new diagnostic in a file YOU edited → in-progress (I1: not red)
    pi.handlers.get("tool_result")!(editEvent(touched), {});
    typecheckOut += `\n${touched}(2,2): error TS2: mine`;
    out = report(await pi.tools.get("verify")!.execute("2", { tier: "full" }));
    expect(out).toMatch(/IN-PROGRESS \(1\)/);
    expect(out).not.toMatch(/REGRESSIONS \(/);

    // 3) new diagnostic in an UNTOUCHED file → regression (red)
    typecheckOut += `\n${bystander}(3,3): error TS3: broke`;
    out = report(await pi.tools.get("verify")!.execute("3", { tier: "full" }));
    expect(out).toMatch(/REGRESSIONS \(1\)/);
    expect(out).toContain("bystander.ts");
    // both old + mine remain pre-existing/in-progress, not regressions
    expect(out).toMatch(/PRE-EXISTING \(1\)/);
    expect(out).toMatch(/IN-PROGRESS \(1\)/);

    // 4) strict upgrades in-progress to regression
    out = report(await pi.tools.get("verify")!.execute("4", { tier: "full", strict: true }));
    expect(out).toMatch(/REGRESSIONS \(2\)/);
    expect(out).not.toMatch(/IN-PROGRESS \(/);
  });

  it("/verify command args reach the run (cheap skips test gate)", async () => {
    const pi = mockPi();
    verifyExt(pi as never, { exec: async () => ({ code: 0, out: "" }) });
    const handler = pi.commands.get("verify")!.handler;
    const notified: string[] = [];
    const selected: string[] = [];
    const ctx = {
      hasUI: false,
      cwd: root,
      ui: {
        notify: async (m: string) => { notified.push(m); },
        select: async (m: string) => { selected.push(m); },
      },
    };
    pi.handlers.get("session_start")!({}, ctx); // auto pass with green fake exec
    await tick();
    await handler("cheap", ctx);
    expect(selected.join()).toMatch(/VERIFY · cheap/);
    expect(notified.join()).not.toMatch(/already running/);
  });
});

describe("kill switch", () => {
  it("OFFICINA_NO_VERIFY=1 registers nothing", () => {
    process.env.OFFICINA_NO_VERIFY = "1";
    const pi = mockPi();
    verifyExt(pi as never);
    expect(pi.tools.size).toBe(0);
    expect(pi.commands.size).toBe(0);
    expect(pi.handlers.size).toBe(0);
  });
});
