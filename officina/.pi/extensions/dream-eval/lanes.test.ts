// dream-eval --lanes tests (S4/I6): read-only lane scoring.
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

let dir: string;
let base: string;

// The extension module is jiti-loaded with side effects; test the exported
// pure helper by importing it directly (scoreLanes is exported).
beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "dream-eval-lanes-"));
  base = dir;
  process.env.OFFICINA_GATEWAY_HEALTH_FILE = join(dir, "gateway-health.json");
});

afterEach(() => {
  rmSync(dir, { recursive: true, force: true });
  vi.unstubAllGlobals();
});

const EVAL = {
  hash: "a".repeat(64),
  items: [
    {
      id: "q1",
      question: "What split does the engine use?",
      mustContain: ["27,21"],
      class: "number",
      provenance: "ledger:1",
      split: "eval",
    },
    {
      id: "q2",
      question: "Which GPUs are resident?",
      mustContain: ["3060", "1070"],
      class: "convention",
      provenance: "memory:2",
      split: "eval",
    },
  ],
};

describe("scoreLanes (S4)", () => {
  it("scores lanes against the frozen set, ranking honest results", async () => {
    const evalPath = join(dir, "eval-set.json");
    writeFileSync(evalPath, JSON.stringify(EVAL));
    const mod = await import("./index.ts");
    const calls: string[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (_url: string | URL, init?: { body?: string }) => {
        const body = JSON.parse(init?.body ?? "{}") as { model?: string };
        calls.push(body.model ?? "?");
        // "good" answers q1 exactly and 1/2 of q2's musts; "bad" misses both.
        const text =
          body.model === "good/lane"
            ? "ts 27,21 with memory"
            : "nothing relevant";
        return {
          json: async () => ({ content: text }),
        } as never;
      }),
    );
    const results = await mod.scoreLanes(
      "http://x",
      ["good/lane", "bad/lane"],
      evalPath,
    );
    expect(results).toEqual([
      { lane: "good/lane", passRate: 0.5 },
      { lane: "bad/lane", passRate: 0 },
    ]);
    // Every call carried the lane as the model — lanes mode walks lanes.
    expect(calls.every((m) => m === "good/lane" || m === "bad/lane")).toBe(
      true,
    );
  });

  it("reports a request failure without writing prefs (I6)", async () => {
    const evalPath = join(dir, "eval-set.json");
    writeFileSync(evalPath, JSON.stringify(EVAL));
    const mod = await import("./index.ts");
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw new Error("conn refused");
      }),
    );
    const results = await mod.scoreLanes("http://x", ["dead/lane"], evalPath);
    expect(results).toEqual([{ lane: "dead/lane", error: "request failed" }]);
    // I6: no prefs file written anywhere.
    expect(
      require("node:fs").existsSync(join(dir, "caduceus-prefs.json")),
    ).toBe(false);
  });
});
