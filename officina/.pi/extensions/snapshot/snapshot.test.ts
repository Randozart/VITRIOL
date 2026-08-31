import { describe, it, expect } from "vitest";
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  gitOpts,
  runSnapshotSequence,
  snapshotConfig,
  substituteStep,
  turnRef,
  worktreeDirty,
  type GitStep,
} from "./snap.ts";

function git(repo: string, ...argv: string[]): string {
  return execFileSync("git", argv, { cwd: repo, env: process.env }).toString().trim();
}

function makeRepo(): string {
  const repo = mkdtempSync(join(tmpdir(), "tris-snap-test-"));
  git(repo, "init", "-q", "-b", "main");
  git(repo, "config", "user.email", "test@trismegistus.local");
  git(repo, "config", "user.name", "tris test");
  writeFileSync(join(repo, "a.txt"), "one\n");
  writeFileSync(join(repo, ".gitignore"), "ignored.log\n");
  git(repo, "add", "-A");
  git(repo, "commit", "-q", "-m", "seed");
  return repo;
}

describe("snapshotConfig", () => {
  it("DEFAULTS OFF — opt-in via TRIS_SNAPSHOT=1 (no surprise git side effects)", () => {
    expect(snapshotConfig({}).enabled).toBe(false);
    expect(snapshotConfig({ TRIS_SNAPSHOT: "1" }).enabled).toBe(true);
  });
});

describe("pure sequence helpers", () => {
  it("turnRef builds per-turn names", () => {
    expect(turnRef("refs/trismegistus/turns", 7)).toBe("refs/trismegistus/turns/7");
  });

  it("substituteStep chains write-tree → commit-tree → update-ref", () => {
    const step: GitStep = { argv: ["commit-tree", "__tree__", "-m", "x"], useTmpIndex: false };
    expect(substituteStep(step, { tree: "abc123" })).toEqual(["commit-tree", "abc123", "-m", "x"]);
  });

  it("worktreeDirty: porcelain lines = dirty", () => {
    expect(worktreeDirty("")).toBe(false);
    expect(worktreeDirty("?? foo\n")).toBe(true);
  });

  it("gitOpts sets GIT_INDEX_FILE only for tmp-index steps", () => {
    const withIdx = gitOpts("/r", "/tmp/idx", { HOME: "/h" }) as { env?: Record<string, string> };
    expect(withIdx.env?.GIT_INDEX_FILE).toBe("/tmp/idx");
    const without = gitOpts("/r", undefined, { HOME: "/h" }) as { env?: Record<string, string> };
    expect(without.env?.GIT_INDEX_FILE).toBeUndefined();
  });
});

const hasGit = (() => {
  try {
    execFileSync("git", ["--version"]);
    return true;
  } catch {
    return false;
  }
})();
describe.skipIf(!hasGit)("snapshot plumbing against a real repo", () => {
  it("snapshots a dirty worktree WITHOUT touching user branch/index/worktree", () => {
    const repo = makeRepo();
    try {
      const headBefore = git(repo, "rev-parse", "HEAD");
      const userIndexBefore = git(repo, "status", "--porcelain");
      writeFileSync(join(repo, "b.txt"), "turn work\n");

      const tmpIdx = join(repo, ".git", "tris-tmp-index");
      const sha = runSnapshotSequence(repo, 3, snapshotConfig({}), tmpIdx, (argv, opts) =>
        execFileSync("git", argv, opts).toString().trim(),
      );
      rmSync(tmpIdx, { force: true });

      expect(sha).toBeTruthy();
      // ref exists and is NOT the branch head
      expect(git(repo, "rev-parse", "refs/trismegistus/turns/3")).toBe(sha);
      expect(git(repo, "rev-parse", "HEAD")).toBe(headBefore); // branch untouched
      expect(git(repo, "status", "--porcelain")).toBe(userIndexBefore || "?? b.txt"); // worktree untouched
      // snapshot content includes the turn work, and respects .gitignore
      const listed = git(repo, "ls-tree", "-r", "--name-only", sha as string);
      expect(listed).toContain("b.txt");
      writeFileSync(join(repo, "ignored.log"), "noise\n");
      const sha2 = runSnapshotSequence(repo, 4, snapshotConfig({}), tmpIdx, (argv, opts) =>
        execFileSync("git", argv, opts).toString().trim(),
      );
      expect(git(repo, "ls-tree", "-r", "--name-only", sha2 as string)).not.toContain("ignored.log");
      rmSync(tmpIdx, { force: true });
    } finally {
      rmSync(repo, { recursive: true, force: true });
    }
  }, 30_000);

  it("rewind path: lost work comes back from a turn ref", () => {
    const repo = makeRepo();
    try {
      writeFileSync(join(repo, "a.txt"), "modified by the model\n");
      const tmpIdx = join(repo, ".git", "tris-tmp-index2");
      const sha = runSnapshotSequence(repo, 5, snapshotConfig({}), tmpIdx, (argv, opts) =>
        execFileSync("git", argv, opts).toString().trim(),
      );
      rmSync(tmpIdx, { force: true });
      expect(sha).toBeTruthy();
      // model destroys its own work (checkout from HEAD = "undo everything"):
      git(repo, "checkout", "HEAD", "--", ".");
      expect(readFileSync(join(repo, "a.txt"), "utf8")).toBe("one\n");
      // rewind to turn 5 restores the snapshot of the work-in-progress:
      git(repo, "checkout", "refs/trismegistus/turns/5", "--", ".");
      expect(readFileSync(join(repo, "a.txt"), "utf8")).toBe("modified by the model\n");
    } finally {
      rmSync(repo, { recursive: true, force: true });
    }
  }, 30_000);

  it("returns null (never throws) outside a repo or on empty input", () => {
    const dir = mkdtempSync(join(tmpdir(), "tris-nogit-"));
    expect(existsSync(join(dir, ".git"))).toBe(false);
    const sha = runSnapshotSequence(dir, 1, snapshotConfig({}), join(dir, "idx"), (argv, opts) =>
      execFileSync("git", argv, opts).toString().trim(),
    );
    expect(sha).toBeNull();
    rmSync(dir, { recursive: true, force: true });
  }, 30_000);
});
