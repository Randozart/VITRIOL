import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "@sinclair/typebox";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { gitOpts, runSnapshotSequence, snapshotConfig, turnRef, worktreeDirty } from "./snap.ts";
import { sessionTurnRef } from "../_shared/turnkeys.ts";

// snapshot — per-turn git snapshots (OpenCode §3.2 / REPORT-02 step 10).
// Plumbing-only: a throwaway GIT_INDEX_FILE + commit-tree + update-ref under
// refs/trismegistus/turns/<session-stem>/<n> (session-namespaced since
// 2026-11-11 — I6: per-session turn indexes collided across concurrent
// sessions under the flat layout, so a rewind could restore a foreign tree).
// The user's branch, index, and worktree are NEVER touched; .gitignore rules
// still apply (via git add -A honoring them). Rewind restores files from a
// ref EXPLICITLY (never automatic — Rule 12) and falls back to the flat
// pre-namespacing refs for old sessions.
//
// Default OFF: TRIS_SNAPSHOT=1 to arm. Kill switch is the same flag (Rule 15).

export default function (pi: ExtensionAPI) {
  const cfg = snapshotConfig();
  if (!cfg.enabled) return;

  let sessionStem = "session";

  pi.on("session_start", async (_event, ctx) => {
    const sm = (ctx as { sessionManager?: { getSessionFile?: () => string | null } }).sessionManager;
    sessionStem = (sm?.getSessionFile?.() ?? "").split("/").pop()?.replace(/\.jsonl$/, "") || "session";
  });

  function repoRoot(): string | null {
    try {
      return execFileSync("git", ["rev-parse", "--show-toplevel"], { cwd: process.cwd() }).toString().trim();
    } catch {
      return null; // not a git workspace — nothing to snapshot
    }
  }

  /** Commit current worktree state to turns/<stem>/<turnIndex> via plumbing. Returns sha. */
  function snapshotTurn(repo: string, turn: number): string | null {
    const tmp = mkdtempSync(join(tmpdir(), "tris-snap-"));
    const idx = join(tmp, "index");
    try {
      return runSnapshotSequence(repo, turn, cfg, idx, (argv, opts) => execFileSync("git", argv, opts).toString().trim(), process.env, sessionStem);
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
      // Session-namespaced ref first (all new snapshots); flat ref falls
      // back for sessions snapshotted before the 2026-11-11 namespacing.
      const refs = [sessionTurnRef(Number(turn), sessionStem, cfg.refPrefix), turnRef(cfg.refPrefix, Number(turn))];
      for (const ref of refs) {
        try {
          execFileSync("git", ["rev-parse", "--verify", ref], gitOpts(repo));
          execFileSync("git", ["checkout", ref, "--", "."], gitOpts(repo));
          return { content: [{ type: "text" as const, text: `restored worktree from ${ref} (KV not rewound — engine endpoint pending)` }], details: {} };
        } catch {
          // try the next ref shape
        }
      }
      return err(`snapshot: no snapshot for turn ${turn} (${refs.join(" or ")})`);
    },
  });
}

/** Error tool result. */
function err(text: string) {
  return { content: [{ type: "text" as const, text }], details: {}, isError: true };
}
