// github-tools unit tests (exec-infra stage 6, I6 read-only).
import { describe, expect, it } from "vitest";
import { GH_TOOLS, repoSlug } from "./gh.ts";

describe("repoSlug", () => {
  it("parses ssh and https remotes, with or without .git", () => {
    expect(repoSlug("git@github.com:Randozart/officina.git")).toBe("Randozart/officina");
    expect(repoSlug("git@github.com:Randozart/VITRIOL")).toBe("Randozart/VITRIOL");
    expect(repoSlug("https://github.com/Randozart/officina.git")).toBe("Randozart/officina");
  });

  it("rejects non-GitHub remotes", () => {
    expect(repoSlug("git@gitlab.com:a/b.git")).toBeNull();
    expect(repoSlug("https://git.sr.ht/~a/b")).toBeNull();
    expect(repoSlug("")).toBeNull();
  });
});

describe("tools (fake runner)", () => {
  const run = (scripted: Record<string, string>) => (args: string[]) => {
    const key = args.join(" ");
    if (key in scripted) return { ok: true, out: scripted[key] };
    return { ok: false, out: `unexpected: ${key}` };
  };

  it("pr list formats rows and respects the state flag", () => {
    const tool = GH_TOOLS.find((t) => t.name === "gh_pr_list")!;
    const res = tool.execute(
      { state: "all", limit: 5 },
      run({
        "pr list --state all --limit 5 --json number,title,author,headRefName,updatedAt":
          JSON.stringify([
            { number: 7, title: "Publish flow", state: "MERGED", author: { login: "Randozart" }, headRefName: "main", updatedAt: "2026-11-11T10:00:00Z" },
          ]),
      }),
      "/proj",
    );
    expect(res.text).toContain("#7 [MERGED] Publish flow");
    expect(res.text).toContain("@Randozart");
  });

  it("number tools refuse to run without a number (I6 guard rail)", () => {
    const view = GH_TOOLS.find((t) => t.name === "gh_pr_view")!;
    expect(view.execute({}, run({}), "/proj").text).toContain("required");
    const diff = GH_TOOLS.find((t) => t.name === "gh_pr_diff")!;
    expect(diff.execute({}, run({}), "/proj").text).toContain("required");
  });

  it("surfaces gh failures verbatim", () => {
    const tool = GH_TOOLS.find((t) => t.name === "gh_ci_status")!;
    const res = tool.execute({}, () => ({ ok: false, out: "auth required" }), "/proj");
    expect(res.text).toContain("gh_ci_status failed: auth required");
  });

  it("registers exactly read-only tools (I6)", () => {
    const names = GH_TOOLS.map((t) => t.name).sort();
    expect(names).toEqual(["gh_ci_status", "gh_issue_list", "gh_pr_diff", "gh_pr_list", "gh_pr_view"]);
    for (const t of GH_TOOLS) {
      expect(t.description.toLowerCase()).toContain("read-only");
    }
  });
});
