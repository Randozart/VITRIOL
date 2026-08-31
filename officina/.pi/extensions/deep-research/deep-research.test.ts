import { describe, it, expect } from "vitest";
import { allocateBudget, clampAgents } from "./budget.ts";
import {
  extractJsonArray,
  extractJsonObject,
  slugify,
  reportFilename,
  digestFindings,
  resolveMaxDefault,
} from "./helpers.ts";
import { computeHeuristics, hasTable, hasSelfReference, distinctCitations } from "./metrics.ts";
import { parseJudge } from "./judge.ts";
import { writeInstructionBlock } from "./prompts.ts";
import { REASONING_ALLOWED_TOOLS, RESEARCH_ALLOWED_TOOLS, type PipelineMeta } from "./pipeline.ts";
import { SUBCODER_ALLOWED_TOOLS } from "../subagent/spawn.ts";
import { renderProgressLines } from "./progress-bar.ts";
import { visibleWidth } from "../_shared/width.ts";
import type { SubCoderResult } from "../subagent/spawn.ts";
import registerDeepResearch from "./index.ts";

describe("allocateBudget", () => {
  // The reference-derived table: lead(1) + wave1 + reserve = cap.
  const table: Array<[number, number, number]> = [
    // [M, wave1, reserve]
    [10, 6, 3],
    [9, 6, 2],
    [8, 5, 2],
    [7, 4, 2],
    [6, 3, 2],
    [5, 3, 1],
    [4, 2, 1],
    [3, 1, 1],
    [2, 1, 0],
  ];
  for (const [m, wave1, reserve] of table) {
    it(`M=${m} → lead 1 + wave1 ${wave1} + reserve ${reserve}`, () => {
      const b = allocateBudget(m);
      expect(b.single).toBe(false);
      expect(b.wave1).toBe(wave1);
      expect(b.reserve).toBe(reserve);
      // Invariant: lead + wave1 + reserve exactly spends the cap.
      expect(1 + b.wave1 + b.reserve).toBe(m);
    });
  }

  it("M=1 is the single-agent path (no lead, no fan-out)", () => {
    const b = allocateBudget(1);
    expect(b.single).toBe(true);
    expect(b.wave1).toBe(0);
    expect(b.reserve).toBe(0);
  });

  it("clamps out-of-range and non-finite caps into [1,10]", () => {
    expect(allocateBudget(0).single).toBe(true); // → 1
    expect(allocateBudget(-5).single).toBe(true); // → 1
    expect(allocateBudget(99).cap).toBe(10);
    expect(clampAgents(Number.NaN)).toBe(10);
    expect(clampAgents(3.9)).toBe(3);
  });
});

describe("extractJsonArray", () => {
  it("parses a bare JSON array", () => {
    expect(extractJsonArray('[{"label":"a","task":"t"}]')).toEqual([{ label: "a", task: "t" }]);
  });
  it("pulls the array out of prose / code fences", () => {
    const text = 'Sure:\n```json\n[{"q":"why?","options":["a","b"]}]\n```\ndone';
    expect(extractJsonArray(text)).toEqual([{ q: "why?", options: ["a", "b"] }]);
  });
  it("returns [] when there is no array or it is malformed", () => {
    expect(extractJsonArray("no json here")).toEqual([]);
    expect(extractJsonArray("")).toEqual([]);
    expect(extractJsonArray("[ not, valid ]")).toEqual([]);
  });
});

describe("slugify / reportFilename", () => {
  it("slugifies a messy topic into a bounded kebab slug", () => {
    expect(slugify("Best LOCAL coding models (2026)!")).toBe("best-local-coding-models-2026");
    expect(slugify("   ")).toBe("research");
    expect(slugify("a".repeat(80)).length).toBeLessThanOrEqual(48);
  });
  it("builds a timestamped filename from an injected date", () => {
    const d = new Date(2026, 6, 5, 9, 3, 7); // 2026-07-05 09:03:07 (month is 0-based)
    expect(reportFilename("Local models", d)).toBe("deep-research-local-models-20260705-090307.md");
  });
});

describe("resolveMaxDefault", () => {
  it("prefers the env override, clamped", () => {
    expect(resolveMaxDefault({ LITTLE_CODER_DEEP_RESEARCH_MAX: "4" }, null)).toBe(4);
    expect(resolveMaxDefault({ LITTLE_CODER_DEEP_RESEARCH_MAX: "999" }, null)).toBe(10);
  });
  it("falls back to settings, then to 10", () => {
    expect(resolveMaxDefault({}, { deep_research: { default_max_subagents: 3 } })).toBe(3);
    expect(resolveMaxDefault({}, null)).toBe(10);
    expect(resolveMaxDefault({ LITTLE_CODER_DEEP_RESEARCH_MAX: "" }, { deep_research: { default_max_subagents: 7 } })).toBe(7);
  });
});

describe("digestFindings", () => {
  const mk = (over: Partial<SubCoderResult>): SubCoderResult => ({
    id: "1",
    label: "x",
    task: "t",
    exitCode: 0,
    report: "",
    messages: [],
    stderr: "",
    usage: { input: 0, output: 0, cost: 0, turns: 0, contextTokens: 0 },
    ...over,
  });

  it("labels each finding and marks failures instead of dropping them", () => {
    const out = digestFindings([
      mk({ label: "models", report: "qwen leads" }),
      mk({ label: "prices", exitCode: 1, errorMessage: "timeout" }),
    ]);
    expect(out).toContain("Finding 1: models");
    expect(out).toContain("qwen leads");
    expect(out).toContain("Finding 2: prices");
    expect(out).toContain("research failed: timeout");
  });
});

describe("writeInstructionBlock (coverage disclosure)", () => {
  const base = ["topic", "brief", "digest"] as const;

  it("stays silent about coverage when no agents failed", () => {
    const out = writeInstructionBlock(...base, 0);
    expect(out).not.toContain("Research coverage");
    expect(out).not.toContain("partial coverage");
    // Still a normal write instruction.
    expect(out).toContain("executive summary");
  });

  it("instructs a coverage note (singular) when one agent failed", () => {
    const out = writeInstructionBlock(...base, 1);
    expect(out).toContain("partial coverage");
    expect(out).toContain("1 research agent");
    expect(out).toContain("1 subtopic was");
    expect(out).toContain("Do NOT invent content");
  });

  it("pluralizes for multiple failures", () => {
    const out = writeInstructionBlock(...base, 3);
    expect(out).toContain("3 research agents");
    expect(out).toContain("3 subtopics were");
  });

  it("defaults to no coverage note when the count is omitted", () => {
    expect(writeInstructionBlock(...base)).not.toContain("partial coverage");
  });
});

describe("child toolsets (project-safety)", () => {
  it("research agents keep browsing but never get bash (no scaffolding/compiling in the tree)", () => {
    const tools = RESEARCH_ALLOWED_TOOLS.split(",");
    expect(tools).not.toContain("bash");
    expect(tools).toContain("webfetch");
    expect(tools).toContain("websearch");
    expect(tools).toContain("read");
    // Exactly SUBCODER_ALLOWED_TOOLS minus bash.
    expect(RESEARCH_ALLOWED_TOOLS).toBe(SUBCODER_ALLOWED_TOOLS.split(",").filter((t) => t !== "bash").join(","));
  });
  it("reasoning agents get read-only local tools only — no web, no bash", () => {
    const tools = REASONING_ALLOWED_TOOLS.split(",");
    for (const banned of ["bash", "webfetch", "websearch", "BrowserNavigate"]) expect(tools).not.toContain(banned);
    expect(tools).toContain("read");
  });
});

describe("extractJsonObject", () => {
  it("parses an object out of prose / fences", () => {
    expect(extractJsonObject('grade:\n```json\n{"coverage":4,"notes":"ok"}\n```')).toEqual({ coverage: 4, notes: "ok" });
  });
  it("returns null for arrays, malformed, or empty", () => {
    expect(extractJsonObject("[1,2,3]")).toBeNull();
    expect(extractJsonObject("{ nope }")).toBeNull();
    expect(extractJsonObject("")).toBeNull();
  });
});

describe("parseJudge", () => {
  it("clamps dimensions to 1-5 and computes the mean", () => {
    const j = parseJudge('{"coverage":5,"grounding":4,"coherence":5,"usefulness":4,"correctness":3,"notes":"solid"}');
    expect(j).not.toBeNull();
    expect(j!.coverage).toBe(5);
    expect(j!.overall).toBeCloseTo(4.2, 1);
  });
  it("clamps out-of-range numbers and rejects all-zero garbage", () => {
    expect(parseJudge('{"coverage":9,"grounding":0,"coherence":-2,"usefulness":3,"correctness":3}')!.coverage).toBe(5);
    expect(parseJudge("no json")).toBeNull();
  });
});

describe("metrics", () => {
  const meta: PipelineMeta = {
    questionCount: 3,
    wave1Count: 3,
    gapCount: 1,
    failedSubagents: 0,
    jsonFallbacks: 0,
    phaseMs: {},
    totalMs: 0,
  };
  const goodReport = [
    "# Rust vs Go for CLI Tools",
    "",
    "Both are strong choices [1]. Rust favors small binaries [2]; Go compiles faster [3].",
    "",
    "| Lang | Startup |",
    "| --- | --- |",
    "| Rust | 2ms |",
    "| Go | 3ms |",
    "",
    "### Sources",
    "[1] https://example.com/a",
    "[2] https://example.com/b",
    "[3] https://go.dev/blog/c",
  ].join("\n");

  it("detects tables, citations, sources, and distinct references", () => {
    expect(hasTable(goodReport)).toBe(true);
    expect(distinctCitations(goodReport)).toBe(3);
    const h = computeHeuristics(goodReport, meta);
    expect(h.hasSourcesSection).toBe(true);
    expect(h.uniqueSources).toBe(3);
    expect(h.hasH1).toBe(true);
    expect(h.structureScore).toBeGreaterThan(50);
  });

  it("flags self-referential phrasing the report rules forbid", () => {
    expect(hasSelfReference("I found that Rust is fast.")).toBe(true);
    expect(hasSelfReference("The agents discovered X.")).toBe(true);
    expect(hasSelfReference("Rust is fast and safe.")).toBe(false);
  });

  it("penalizes failed subagents in the structure score", () => {
    const withFails = computeHeuristics(goodReport, { ...meta, failedSubagents: 2 });
    const clean = computeHeuristics(goodReport, meta);
    expect(withFails.structureScore).toBeLessThan(clean.structureScore);
  });
});

describe("extension registration (load smoke test)", () => {
  it("registers the f2 toggle, /deep-research command, and lifecycle hooks without throwing", () => {
    const shortcuts: string[] = [];
    const commands: string[] = [];
    const events: string[] = [];
    const pi: any = {
      registerShortcut: (id: string) => shortcuts.push(id),
      registerCommand: (name: string) => commands.push(name),
      on: (evt: string) => events.push(evt),
    };
    expect(() => registerDeepResearch(pi)).not.toThrow();
    expect(shortcuts).toContain("f2");
    expect(commands).toContain("deep-research");
    // The write handoff + guards + reset all depend on these hooks being wired.
    for (const e of ["input", "before_agent_start", "tool_call", "agent_end", "session_start"]) {
      expect(events).toContain(e);
    }
  });
});

describe("renderProgressLines", () => {
  it("fills the bar proportionally to done/total", () => {
    const empty = renderProgressLines({ done: 0, total: 7, phase: "researching", elapsedMs: 0, width: 80 });
    const half = renderProgressLines({ done: 4, total: 8, phase: "researching", elapsedMs: 0, width: 80 });
    const full = renderProgressLines({ done: 7, total: 7, phase: "researching", elapsedMs: 0, width: 80 });
    // header shows the count; bar line carries the fill glyphs.
    expect(empty[0]).toContain("0/7 agents");
    expect(full[0]).toContain("7/7 agents");
    const fillCount = (s: string) => [...s].filter((c) => c === "▓").length;
    expect(fillCount(empty[1])).toBe(0);
    expect(fillCount(half[1])).toBeGreaterThan(0);
    expect(fillCount(half[1])).toBeLessThan(fillCount(full[1]));
  });

  it("clamps done into [0,total] and never divides by zero", () => {
    const over = renderProgressLines({ done: 99, total: 3, phase: "x", elapsedMs: 0, width: 80 });
    expect(over[0]).toContain("3/3 agents");
    const zero = renderProgressLines({ done: 0, total: 0, phase: "x", elapsedMs: 0, width: 80 });
    expect(zero[0]).toContain("1 agents"); // total floored to 1
  });

  // issue #48: any widget line wider than the terminal crashes pi-tui. Every
  // emitted line MUST fit the width at any terminal size.
  it("never emits a line wider than the terminal width", () => {
    for (const width of [20, 24, 40, 80, 120]) {
      const lines = renderProgressLines({
        done: 3,
        total: 7,
        phase: "filling gaps in the research coverage",
        elapsedMs: 754_000,
        width,
      });
      for (const line of lines) {
        expect(visibleWidth(line)).toBeLessThanOrEqual(width);
      }
    }
  });
});
