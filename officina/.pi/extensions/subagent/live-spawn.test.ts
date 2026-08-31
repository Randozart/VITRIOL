import { describe, it, expect } from "vitest";
import { resolve } from "node:path";
import { runSubCoder, runSubCodersConcurrent } from "./spawn.ts";

// Live end-to-end: spawns real child little-coder sessions against the local
// model. Skipped unless LC_LIVE=1 (needs a running model server). Run with:
//   LC_LIVE=1 LLAMACPP_API_KEY=noop npx vitest run .pi/extensions/subagent/live-spawn.test.ts
const LIVE = process.env.LC_LIVE === "1";
const MODEL = process.env.LC_LIVE_MODEL || "llamacpp/qwen3.6-35b-a3b";
const repoRoot = resolve(__dirname, "..", "..", "..");

describe.skipIf(!LIVE)("sub-coder live spawn", () => {
  it(
    "spawns a child that runs on the parent's model and returns a report",
    async () => {
      const r = await runSubCoder({
        id: "1",
        label: "ping",
        task: "Respond with exactly the single word ALIVE. Do not use any tools.",
        cwd: repoRoot,
        model: MODEL,
      });
      expect(r.exitCode).toBe(0);
      expect(r.report.toUpperCase()).toContain("ALIVE");
      expect(r.usage.turns).toBeGreaterThan(0);
    },
    240_000,
  );

  it(
    "runs two sub-coders in parallel and reports per-item",
    async () => {
      const results = await runSubCodersConcurrent(
        [
          { id: "1", label: "two", task: "What is 2+2? Reply with just the number.", cwd: repoRoot },
          { id: "2", label: "cap", task: "What is the capital of France? One word.", cwd: repoRoot },
        ],
        { model: MODEL },
      );
      expect(results).toHaveLength(2);
      expect(results.every((r) => r.exitCode === 0)).toBe(true);
      expect(results[0].report).toContain("4");
      expect(results[1].report.toLowerCase()).toContain("paris");
    },
    300_000,
  );
});
