import { describe, it, expect } from "vitest";
import { formatPlan, parseRefs, pairTurns, turnFilename } from "./rewind.ts";
import register from "./index.ts";

describe("parseRefs/pairTurns (pure)", () => {
  it("parses newest-first, ignores junk", () => {
    const out = "refs/trismegistus/turns/3 abc\nrefs/trismegistus/turns/10 def\nnoise\nrefs/trismegistus/turns/x";
    expect(parseRefs(out)).toEqual([10, 3]);
  });
  it("honors custom prefix", () => {
    expect(parseRefs("a/turns/7", "a/turns/")).toEqual([7]);
  });
  it("filters future turns and caps at 10", () => {
    const many = Array.from({ length: 15 }, (_, i) => i + 1);
    const pairs = pairTurns(many, 12);
    expect(pairs[0].turn).toBe(12);
    expect(pairs.length).toBe(10);
  });
  it("plan names both halves + the danger", () => {
    const t = formatPlan(9, "s-turn-9.bin", [{ turn: 9, code: true, kv: "attempt" }]);
    expect(t).toContain("refs/trismegistus/turns/9");
    expect(t).toContain("s-turn-9.bin");
    expect(t).toContain("OVERWRITES UNCOMMITTED");
  });
  it("turnFilename is the SHARED convention (matches checkpoint)", () => {
    expect(turnFilename(4, "abc")).toBe("abc-turn-4.bin");
  });
});

interface Sent { confirmAnswer: boolean }

function stub(opts: { mode?: string; hasUI?: boolean; refsOut?: string; restoreOk?: boolean; confirm?: boolean; gitFails?: boolean }) {
  const notices: Array<[string, string]> = [];
  const calls: string[][] = [];
  let handler: ((args: string, ctx: unknown) => Promise<void>) | null = null;
  const pi = {
    registerCommand: (name: string, o: { handler: typeof handler }) => { if (name === "rewind") handler = o.handler; },
    on: () => undefined,
    registerTool: () => undefined,
  };
  const deps = {
    git: async (argv: string[]) => {
      calls.push(argv);
      if (opts.gitFails && argv[0] !== "for-each-ref") throw new Error("no ref");
      return argv[0] === "for-each-ref" ? (opts.refsOut ?? "refs/trismegistus/turns/3\n") : "";
    },
    restore: async () => ({ ok: opts.restoreOk ?? true, note: opts.restoreOk === false ? "engine refused" : "KV slot restored" }),
  };
  const ctx = {
    cwd: "/proj", mode: opts.mode ?? "tui", hasUI: opts.hasUI ?? true,
    ui: { notify: (m: string, t?: string) => notices.push([m, t ?? "info"]), confirm: async () => opts.confirm ?? true },
    sessionManager: { getSessionFile: () => "/x/.pi/sessions/sessA.jsonl" },
  };
  register(pi as never, deps);
  return { run: (a: string) => handler!(a, ctx), notices, calls, registered: () => handler !== null };
}

describe("rewind command wiring", () => {
  it("kill switch registers nothing", () => {
    process.env.TRIS_NO_REWIND = "1";
    expect(stub({}).registered()).toBe(false);
    delete process.env.TRIS_NO_REWIND;
  });

  it("no-arg lists turns", async () => {
    const h = stub({ refsOut: "refs/trismegistus/turns/5\nrefs/trismegistus/turns/2\n" });
    await h.run("");
    expect(h.notices.some((n) => n[0].includes("5, 2"))).toBe(true);
  });

  it("headless refuses destructive action", async () => {
    const h = stub({ mode: "print", hasUI: false });
    await h.run("3");
    expect(h.calls).toHaveLength(0);
    expect(h.notices[0][0]).toContain("headless");
  });

  it("confirm gates both halves; shared session stem in filename", async () => {
    const h = stub({ confirm: true });
    await h.run("3");
    const checkout = h.calls.find((c) => c[0] === "checkout");
    expect(checkout).toEqual(["checkout", "refs/trismegistus/turns/3", "--", "."]);
    expect(h.notices.at(-1)![0]).toContain("sessA-turn-3.bin"); // from sessionManager, not env
    expect(h.notices.at(-1)![0]).toContain("KV slot restored");
  });

  it("declined confirm touches nothing", async () => {
    const h = stub({ confirm: false });
    await h.run("3");
    expect(h.calls.filter((c) => c[0] === "checkout" || c[0] === "rev-parse")).toHaveLength(0);
    expect(h.notices.at(-1)![0]).toContain("declined");
  });

  it("missing code ref still attempts KV (halves independent)", async () => {
    const h = stub({ gitFails: true });
    await h.run("3");
    expect(h.notices.at(-1)![0]).toContain("code: no snapshot ref");
    expect(h.notices.at(-1)![1]).toBe("warning");
  });
});
