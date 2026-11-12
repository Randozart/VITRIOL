import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { existsSync, mkdirSync, readFileSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import { parseSnapshot, pathOf, type PermSnapshot } from "./perms.ts";
import {
  commandVerdict,
  parseMode,
  pathVerdict,
  type PermMode,
  type Verdict,
} from "./policy.ts";
import { findBwrap, shouldWrap, wrapBwrap } from "./sandbox.ts";
import {
  getJobsRoot,
  isLongRunner,
  jobDir,
  rewriteToDispatch,
} from "../longjob/longjob.ts";

// permissions-guard — path + shell policy, enforced (gap D1).
//
// Policy sources, in precedence order:
//   1. protected roots (hardcoded in policy.ts; YOLO cannot bypass)
//   2. ~/.config/trismegistus/permissions.json path rules (perms-sync mirror)
//   3. permission MODE from ~/.vitriol/officina/permissions-mode, then
//      OFFICINA_PERMISSIONS_MODE, default "ask"
//
// Modes (2026-11-10 operator decision):
//   ask  — in-cwd writes allowed; outside edits + risky shell ask.
//   auto — risky shell allowed; protected roots still ask.
//   yolo — everything auto-approved EXCEPT protected roots and catastrophic
//          commands.
//
// Kill switch: TRIS_NO_PERMS=1 registers nothing.

/** Resolved PER CALL (import-time capture once made tests hit the live
 * mirror — caught by the wiring suite; env must win dynamically). */
function snapshotPath(env: NodeJS.ProcessEnv = process.env): string {
  return env.TRIS_PERMS_FILE || `${env.HOME || ""}/.config/trismegistus/permissions.json`;
}

function modePath(env: NodeJS.ProcessEnv = process.env): string {
  if (env.OFFICINA_PERMISSIONS_MODE_FILE) return env.OFFICINA_PERMISSIONS_MODE_FILE;
  return `${env.HOME || homedir()}/.vitriol/officina/permissions-mode`;
}

function currentMode(env: NodeJS.ProcessEnv = process.env): PermMode {
  try {
    const p = modePath(env);
    if (existsSync(p)) return parseMode(readFileSync(p, "utf8"));
  } catch {
    // fall through to env/default
  }
  return parseMode(env.OFFICINA_PERMISSIONS_MODE);
}

export default function (pi: ExtensionAPI) {
  if (process.env.TRIS_NO_PERMS === "1") return;

  let cache: { mtime: number; snap: PermSnapshot } | null = null;
  let warned = false;
  let sandboxWarned = false;

  function load(): PermSnapshot | null {
    try {
      const m = statSync(snapshotPath()).mtimeMs;
      if (cache && cache.mtime === m) return cache.snap;
      const snap = parseSnapshot(readFileSync(snapshotPath(), "utf8"));
      if (snap) cache = { mtime: m, snap };
      return snap;
    } catch {
      return null; // missing file: policy defaults still apply
    }
  }

  async function decideAndEnforce(
    tool: string,
    verdict: Verdict,
    detail: string,
    ctx: { mode?: string; hasUI?: boolean; ui?: any },
  ): Promise<{ block: true; reason: string } | undefined> {
    const mode = currentMode();
    if (verdict.action === "allow") {
      emitHarnessEvent(harnessEvent("lc-perms", "allow", { detail: `${mode} ${detail}` }));
      return undefined;
    }
    if (verdict.action === "deny") {
      emitHarnessEvent(harnessEvent("lc-perms", "deny", { detail: `${mode} ${detail}` }));
      return { block: true, reason: `permissions: ${verdict.reason}` };
    }
    const headless = ctx.mode !== "tui" && ctx.mode !== "rpc";
    if (headless || !ctx.hasUI) {
      emitHarnessEvent(harnessEvent("lc-perms", "ask-denied-headless", { detail: `${mode} ${detail}` }));
      return {
        block: true,
        reason: `permissions: ${verdict.reason} — denied in headless mode (fail-closed)`,
      };
    }
    const yes = await ctx.ui?.confirm(
      "permissions",
      `Allow ${tool}?\n${verdict.reason}\n${detail}\n(mode: ${mode})`,
    );
    emitHarnessEvent(
      harnessEvent("lc-perms", yes ? "ask-approved" : "ask-declined", {
        detail: `${mode} ${detail}`,
      }),
    );
    if (!yes) return { block: true, reason: `permissions: user declined ${tool}` };
    return undefined;
  }

  pi.on("tool_call", async (event, ctx) => {
    const e = event as { toolName?: string; input?: Record<string, unknown> };
    const tool = String(e.toolName ?? "").toLowerCase();
    const input = e.input ?? {};
    const cwd = (ctx as { cwd?: string }).cwd || process.cwd();
    const home = process.env.HOME || homedir();
    const mode = currentMode();

    const snap = load();
    if (!snap && !warned) {
      warned = true;
      emitHarnessEvent(harnessEvent("lc-perms", "no-snapshot", { detail: "mode policy only" }));
      ctx.ui.notify(
        "permissions-guard: no permissions.json (run `trismegistus perms-sync`) — mode policy in effect",
        "warning",
      );
    }

    if (tool === "bash" || tool === "powershell") {
      const command = String(input.command ?? "");
      if (!command.trim()) return undefined;
      const verdict = commandVerdict(command, cwd, home, mode);
      const enforced = await decideAndEnforce(tool, verdict, shortCommand(command), ctx as any);
      if (enforced) return enforced;
      // Codex-qol stage 10 (I6): only sandboxed mode wraps; bwrap missing
      // degrades to unwrapped with exactly one warning per session.
      if (tool === "bash" && shouldWrap(mode)) {
        const bwrap = findBwrap();
        if (!bwrap) {
          if (!sandboxWarned) {
            sandboxWarned = true;
            ctx.ui.notify?.("sandbox: bwrap not found — commands run unwrapped", "warning");
          }
        } else {
          const net = process.env.OFFICINA_SANDBOX_NET === "1";
          (e.input as Record<string, unknown>).command = wrapBwrap(command, cwd, bwrap, { net });
        }
        return undefined;
      }
      // longjob auto-dispatch (2026-11-12, I4/I6): known long-runners are
      // dispatched instead of blocking the agent. #fg wins; the sandboxed
      // mode above already returned (bwrap + detached jobs deferred);
      // OFFICINA_NO_AUTOBG=1 restores today's foreground behaviour.
      // I1 fail-open: any registry error falls through to foreground.
      if (
        tool === "bash" &&
        process.env.OFFICINA_NO_AUTOBG !== "1" &&
        isLongRunner(command)
      ) {
        try {
          const job = `j-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
          const paths = jobDir(getJobsRoot(), job);
          mkdirSync(paths.dir, { recursive: true, mode: 0o700 });
          (e.input as Record<string, unknown>).command = rewriteToDispatch(command, paths, job);
          ctx.ui.notify?.(
            `longjob ${job}: dispatched (log ${paths.log}) — poll bg_status ${job}, bg_wait ${job} <max_s>, or end your turn; you'll be woken with the result`,
            "info",
          );
        } catch {
          // I1: registry failure runs the command foreground
        }
        return undefined;
      }
      return undefined;
    }

    if (tool !== "edit" && tool !== "write" && tool !== "read") return undefined;
    const path = pathOf(input);
    if (!path) return undefined;
    const verdict = pathVerdict(tool, path, cwd, home, mode, snap);
    return decideAndEnforce(tool, verdict, `${tool} ${path}`, ctx as any);
  });
}

function shortCommand(command: string): string {
  const one = command.replace(/\s+/g, " ").trim();
  return one.length > 120 ? `${one.slice(0, 119)}…` : one;
}
