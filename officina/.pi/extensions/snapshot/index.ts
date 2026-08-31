import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "@sinclair/typebox";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { gitOpts, runSnapshotSequence, snapshotConfig, turnRef, worktreeDirty } from "./snap.ts";

// snapshot — per-turn git snapshots (OpenCode §3.2 / REPORT-02 step 10).
// Plumbing-only: a throwaway GIT_INDEX_FILE + commit-tree + update-ref under
// refs/trismegistus/turns/<n>. The user's branch, index, and worktree are
// NEVER touched; .gitignore rules still apply (via git add -A honoring them).
// Rewind restores files from a ref EXPLICITLY (never automatic — Rule 12).
// Pairing with VITRIOL slot restore (§2.2) lands when the engine endpoint
// exists; until then rewind is code-only and says so.
//
// Default OFF: TRIS_SNAPSHOT=1 to arm. Kill switch is the same flag (Rule 15).

export default function (pi: ExtensionAPI) {
  const cfg = snapshotConfig();
  if (!cfg.enabled) return;

  function repoRoot(): string | null {
    try {
      return execFileSync("git", ["rev-parse", "--show-toplevel"], { cwd: process.cwd() }).toString().trim();
    } catch {
      return null; // not a git workspace — nothing to snapshot
    }
  }

  /** Commit current worktree state to turns/<turnIndex> via plumbing. Returns sha. */
  function snapshotTurn(repo: string, turn: number): string | null {
    const tmp = mkdtempSync(join(tmpdir(), "tris-snap-"));
    const idx = join(tmp, "index");
    try {
      return runSnapshotSequence(repo, turn, cfg, idx, (argv, opts) => execFileSync("git", argv, opts).toString().trim());
    } finally {
      rmSync(tmp, { recursive: true, force: true });
    }
  }

  pi.on("turn_end", async (event) => {
    const repo = repoRoot();
    if (!repo) return;
    const porcelain = safeStatus(repo);
    if (!worktreeDirty(porcelain)) return; // nothing changed this turn
    const turn = Number((event as { turnIndex?: number }).turnIndex ?? 0);
    snapshotTurn(repo, turn);
  });

  function safeStatus(repo: string): string {
    try {
      return execFileSync("git", ["status", "--porcelain"], gitOpts(repo)).toString();
    } catch {
      return "";
    }
  }

  pi.registerTool({
    name: "snapshot_log",
    label: "Snapshot Log",
    description: "List turn snapshots taken this session (refs under trismegistus/turns).",
    parameters: Type.Object({}),
    async execute() {
      const repo = repoRoot();
      if (!repo) return err("snapshot: not inside a git workspace");
      try {
        const out = execFileSync("git", ["for-each-ref", "--format=%(refname) %(objectname:short) %(subject)", cfg.refPrefix], gitOpts(repo)).toString();
        return { content: [{ type: "text" as const, text: out.trim() || "no snapshots yet" }], details: {} };
      } catch (e) {
        return err(`snapshot: ${(e as Error).message}`);
      }
    },
  });

  pi.registerTool({
    name: "snapshot_rewind",
    label: "Snapshot Rewind",
    description:
      "Restore workspace files from a turn snapshot (explicit only). Files land in the worktree+index; " +
      "conversation/KV rewind is NOT paired yet (needs VITRIOL slot restore, REPORT-02 §2.2).",
    parameters: Type.Object({ turn: Type.Number({ description: "Turn number to rewind to" }) }),
    async execute(_id, { turn }) {
      const repo = repoRoot();
      if (!repo) return err("snapshot: not inside a git workspace");
      const ref = turnRef(cfg.refPrefix, Number(turn));
      try {
        execFileSync("git", ["rev-parse", "--verify", ref], gitOpts(repo));
        execFileSync("git", ["checkout", ref, "--", "."], gitOpts(repo));
        return { content: [{ type: "text" as const, text: `restored worktree from ${ref} (KV not rewound — engine endpoint pending)` }], details: {} };
      } catch {
        return err(`snapshot: no snapshot for turn ${turn} (${ref})`);
      }
    },
  });
}

/** Error tool result. */
function err(text: string) {
  return { content: [{ type: "text" as const, text }], details: {}, isError: true };
}
