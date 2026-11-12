// github-tools — read-only GitHub tools over the authenticated gh CLI
// (exec-infra stage 6, 2026-11-12; plan
// .opencode/plans/officina-exec-infra-2026-11-12.md).
//
// I6: LIST/VIEW ONLY — no mutation tools (no PR create, no comments), even
// if the model asks. Repo comes from `git remote get-url origin`; anything
// non-GitHub means the extension registers nothing (honest absence).
// gh itself is the auth boundary (keyring) — no tokens pass through here.

export interface RunResult {
  ok: boolean;
  out: string;
}

export type GhRunner = (args: string[], cwd: string) => RunResult;

const GH_TIMEOUT_MS = 20_000;
const OUTPUT_CAP = 8_000;

/** git@github.com:owner/repo(.git) | https://github.com/owner/repo(.git) */
export function repoSlug(remoteUrl: string): string | null {
  const url = String(remoteUrl ?? "").trim();
  const ssh = /^git@github\.com:([^/]+)\/(.+?)(\.git)?$/i.exec(url);
  if (ssh) return `${ssh[1]}/${ssh[2]}`;
  const https = /^https:\/\/github\.com\/([^/]+)\/(.+?)(\.git)?$/i.exec(url);
  if (https) return `${https[1]}/${https[2]}`;
  return null;
}

export function makeRunner(): GhRunner {
  return (args, cwd) => {
    try {
      const res = spawnSync("gh", args, { cwd, encoding: "utf8", timeout: GH_TIMEOUT_MS });
      if (res.error) return { ok: false, out: res.error.message };
      if (res.status !== 0) return { ok: false, out: String(res.stderr ?? "").trim() };
      return { ok: true, out: String(res.stdout ?? "") };
    } catch (error) {
      return { ok: false, out: error instanceof Error ? error.message : String(error) };
    }
  };
}

import { spawnSync } from "node:child_process";

function cap(text: string): string {
  const t = text.trim();
  return t.length > OUTPUT_CAP ? `${t.slice(0, OUTPUT_CAP)}\n… (truncated at ${OUTPUT_CAP} chars)` : t;
}

function fmtPr(p: Record<string, unknown>): string {
  return `#${p.number} [${p.state}] ${p.title} — @${(p.author as { login?: string })?.login ?? "?"} · ${(p.headRefName as string) ?? "?"} · ${p.updatedAt}`;
}

export interface GhTool {
  name: string;
  label: string;
  description: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  parameters: any;
  execute(input: Record<string, unknown>, run: GhRunner, cwd: string): { text: string };
}

const stateParam = {
  state: { type: "string", enum: ["open", "closed", "all"], description: "PR state filter (default open)" },
  limit: { type: "number", description: "1-30 (default 10)" },
};

export const GH_TOOLS: GhTool[] = [
  {
    name: "gh_pr_list",
    label: "GitHub PR List",
    description: "List pull requests for the origin GitHub repo (read-only).",
    parameters: { type: "object", properties: stateParam, required: [] },
    execute(input, run, cwd) {
      const state = String(input.state ?? "open");
      const limit = Math.max(1, Math.min(30, Number(input.limit ?? 10)));
      const r = run(["pr", "list", "--state", state, "--limit", String(limit),
        "--json", "number,title,author,headRefName,updatedAt"], cwd);
      if (!r.ok) return { text: `gh_pr_list failed: ${r.out}` };
      const rows: Array<Record<string, unknown>> = JSON.parse(r.out || "[]");
      return { text: cap(rows.map(fmtPr).join("\n") || "no pull requests") };
    },
  },
  {
    name: "gh_pr_view",
    label: "GitHub PR View",
    description: "Show one pull request: title, author, state, branch, body (read-only).",
    parameters: { type: "object", properties: { number: { type: "number" } }, required: ["number"] },
    execute(input, run, cwd) {
      if (!Number(input.number)) return { text: "gh_pr_view: 'number' is required" };
      const r = run(["pr", "view", String(input.number),
        "--json", "number,title,author,state,body,headRefName,baseRefName,additions,deletions"], cwd);
      if (!r.ok) return { text: `gh_pr_view failed: ${r.out}` };
      const p = JSON.parse(r.out) as Record<string, unknown>;
      const body = String(p.body ?? "").slice(0, 1_500);
      return {
        text: cap(
          `#${p.number} [${p.state}] ${p.title}\n@${(p.author as { login?: string })?.login} · ${p.headRefName} → ${p.baseRefName} · +${p.additions}/-${p.deletions}\n\n${body}`,
        ),
      };
    },
  },
  {
    name: "gh_pr_diff",
    label: "GitHub PR Diff",
    description: "The unified diff of one pull request (read-only, capped).",
    parameters: { type: "object", properties: { number: { type: "number" } }, required: ["number"] },
    execute(input, run, cwd) {
      if (!Number(input.number)) return { text: "gh_pr_diff: 'number' is required" };
      const r = run(["pr", "diff", String(input.number)], cwd);
      if (!r.ok) return { text: `gh_pr_diff failed: ${r.out}` };
      return { text: cap(r.out) };
    },
  },
  {
    name: "gh_issue_list",
    label: "GitHub Issue List",
    description: "List issues for the origin GitHub repo (read-only).",
    parameters: { type: "object", properties: stateParam, required: [] },
    execute(input, run, cwd) {
      const state = String(input.state ?? "open");
      const limit = Math.max(1, Math.min(30, Number(input.limit ?? 10)));
      const r = run(["issue", "list", "--state", state, "--limit", String(limit),
        "--json", "number,title,author,updatedAt"], cwd);
      if (!r.ok) return { text: `gh_issue_list failed: ${r.out}` };
      const rows: Array<Record<string, unknown>> = JSON.parse(r.out || "[]");
      return {
        text: cap(
          rows
            .map((i) => `#${i.number} ${i.title} — @${(i.author as { login?: string })?.login ?? "?"} · ${i.updatedAt}`)
            .join("\n") || "no issues",
        ),
      };
    },
  },
  {
    name: "gh_ci_status",
    label: "GitHub CI Status",
    description: "Recent workflow runs (status + conclusion) for the origin repo (read-only).",
    parameters: {
      type: "object",
      properties: { branch: { type: "string", description: "filter by branch" } },
      required: [],
    },
    execute(input, run, cwd) {
      const args = ["run", "list", "--limit", "8", "--json", "displayTitle,status,conclusion,headBranch,createdAt"];
      if (input.branch) args.push("--branch", String(input.branch));
      const r = run(args, cwd);
      if (!r.ok) return { text: `gh_ci_status failed: ${r.out}` };
      const rows: Array<Record<string, unknown>> = JSON.parse(r.out || "[]");
      return {
        text: cap(
          rows
            .map((w) => `[${w.status}${w.conclusion ? `/${w.conclusion}` : ""}] ${w.displayTitle} · ${w.headBranch} · ${w.createdAt}`)
            .join("\n") || "no workflow runs",
        ),
      };
    },
  },
];
