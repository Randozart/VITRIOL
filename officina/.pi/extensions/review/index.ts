// /review — Codex-style read-only code review (codex-qol stage 6).
//
// Presets gather a diff via git subprocess, a routed agent turn reviews it
// (lapis/nimbus lanes as usual), and the model's ```officina-review JSON
// block lands in .officina/review.json for the TUI findings modal. While the
// review turn is in flight, edit/write/bash are BLOCKED — the review can
// never mutate the tree (plan invariant I2).

import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import {
  parseFindingsBlock,
  summarizeFindings,
  writeFindings,
  type ReviewFindings,
} from "./findings.ts";

const MAX_DIFF_CHARS = 60_000;
const BLOCKED_DURING_REVIEW = new Set(["edit", "write", "bash"]);

interface ReviewTurn {
  preset: string;
  chunks: string[];
}

export default function (pi: ExtensionAPI) {
  let review: ReviewTurn | null = null;

  const runGit = (cwd: string, args: string[]): { ok: boolean; out: string } => {
    try {
      const res = spawnSync("git", args, { cwd, encoding: "utf8", timeout: 15_000 });
      if (res.error) return { ok: false, out: res.error.message };
      if (res.status !== 0) return { ok: false, out: String(res.stderr ?? "").trim() };
      return { ok: true, out: String(res.stdout ?? "") };
    } catch (error) {
      return { ok: false, out: error instanceof Error ? error.message : String(error) };
    }
  };

  const baseBranch = (cwd: string): string | null => {
    for (const candidate of ["main", "master", "origin/main", "origin/master"]) {
      if (runGit(cwd, ["rev-parse", "--verify", candidate]).ok) return candidate;
    }
    return null;
  };

  const gatherDiff = (
    cwd: string,
    preset: string,
    arg: string,
  ): { ok: boolean; out: string } => {
    switch (preset) {
      case "uncommitted":
        return runGit(cwd, ["diff", "HEAD"]);
      case "commit": {
        const ref = arg || "HEAD";
        return runGit(cwd, ["show", "--format=medium", ref]);
      }
      case "base": {
        const base = arg || baseBranch(cwd);
        if (!base) return { ok: false, out: "no main/master/origin base branch found" };
        const mb = runGit(cwd, ["merge-base", base, "HEAD"]);
        if (!mb.ok) return { ok: false, out: `merge-base failed: ${mb.out}` };
        return runGit(cwd, ["diff", `${mb.out.trim()}..HEAD`]);
      }
      default:
        return { ok: false, out: `unknown preset ${preset}` };
    }
  };

  const finish = (ctx: { ui: any; cwd?: string }) => {
    const active = review;
    review = null;
    if (!active) return;
    const text = active.chunks.join("");
    const cwd = ctx.cwd ?? process.cwd();
    const findings = parseFindingsBlock(text, active.preset);
    if (!findings) {
      ctx.ui.notify?.(
        "review: model returned no officina-review block — see the transcript",
        "warning",
      );
      return;
    }
    const dir = join(cwd, ".officina");
    writeFindings(dir, findings);
    ctx.ui.notify?.(
      `review: ${summarizeFindings(findings)} — /review-findings for the modal`,
      findings.findings.some((f) => f.severity === "high") ? "warning" : "info",
    );
  };

  pi.registerCommand("review", {
    description:
      "Read-only review: [uncommitted|commit [ref]|base [branch]|custom <focus>] — findings land in .officina/review.json",
    handler: async (args: string, ctx: any) => {
      const cwd = String(ctx.cwd ?? process.cwd());
      const gitTop = runGit(cwd, ["rev-parse", "--show-toplevel"]);
      if (!gitTop.ok) {
        await ctx.ui.notify?.("review: not a git repository", "error");
        return;
      }
      const trimmed = String(args ?? "").trim();
      let preset = "";
      let arg = "";
      if (trimmed) {
        const [head, ...rest] = trimmed.split(/\s+/);
        preset = head.toLowerCase();
        arg = rest.join(" ");
      } else {
        const picked = await ctx.ui.select?.(
          "review what?",
          [
            "uncommitted — working tree vs HEAD",
            "commit — last commit (HEAD)",
            "base branch — merge-base vs main",
            "custom — describe the focus",
          ],
        );
        if (!picked) return;
        preset = picked.split(" ")[0];
      }
      let focus = "";
      if (preset === "custom") {
        focus = String(
          (await ctx.ui.input?.("what should the review focus on?", "e.g. the new cache layer")) ??
            "",
        ).trim();
        if (!focus) return;
      }

      const diff = gatherDiff(cwd, preset, arg);
      if (!diff.ok) {
        await ctx.ui.notify?.(`review: git failed — ${diff.out}`, "error");
        return;
      }
      const body = diff.out.trim();
      if (!body) {
        await ctx.ui.notify?.(`review: nothing to review (${preset})`, "info");
        return;
      }
      const clipped =
        body.length > MAX_DIFF_CHARS
          ? `${body.slice(0, MAX_DIFF_CHARS)}\n… (diff truncated at ${MAX_DIFF_CHARS} chars)`
          : body;

      const prompt = [
        "You are performing a READ-ONLY code review.",
        `Preset: ${preset}${arg ? ` (${arg})` : ""}${focus ? `\nFocus: ${focus}` : ""}`,
        "Judge correctness bugs, security issues, broken invariants, and test gaps.",
        "You cannot run tools during this turn — do not try.",
        "Respond with EXACTLY ONE fenced block tagged officina-review containing JSON:",
        "```",
        '{"version":1,"summary":"<one paragraph>","findings":[{"severity":"high|medium|low|info","file":"<path>","line":"<line or range>","title":"<short>","detail":"<why + fix suggestion>"}]}',
        "```",
        "Severity guide: high = ships broken/unsafe; medium = likely bug or missing test; low = smell; info = note.",
        "No text before or after the block.",
        "",
        "DIFF:",
        "```diff",
        clipped,
        "```",
      ].join("\n");

      review = { preset, chunks: [] };
      await ctx.sendUserMessage(prompt);
    },
  });

  // I2: hard read-only while a review turn is in flight.
  pi.on("tool_call", (event) => {
    if (!review) return undefined;
    const name = String((event as { toolName?: string }).toolName ?? "").toLowerCase();
    if (BLOCKED_DURING_REVIEW.has(name)) {
      return { block: true, reason: `review turn is read-only — ${name} is blocked` };
    }
    return undefined;
  });

  // Capture assistant text for the findings block.
  pi.on("message_end", (event) => {
    if (!review) return;
    const message = (event as { message?: { role?: string; content?: unknown } }).message;
    if (!message || message.role !== "assistant") return;
    const content = message.content;
    if (typeof content === "string") {
      review.chunks.push(content);
    } else if (Array.isArray(content)) {
      for (const block of content) {
        const b = block as { type?: string; text?: unknown };
        if (b?.type === "text" && typeof b.text === "string") review.chunks.push(b.text);
      }
    }
  });

  // A review is one turn; settle on the first turn end (or run end on abort).
  pi.on("turn_end", (_event, ctx) => finish(ctx as { ui: any; cwd?: string }));
  pi.on("agent_end", (_event, ctx) => {
    if (review) finish(ctx as { ui: any; cwd?: string });
  });
}
