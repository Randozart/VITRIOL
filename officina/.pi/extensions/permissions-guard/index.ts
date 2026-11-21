import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { existsSync, mkdirSync, readFileSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
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
  addProjectRule,
  projectStorePath,
  clearProjectRules,
  leadingProgram,
  loadProjectStore,
  matchesProjectRule,
  normalizeCommand,
} from "./perms-project.ts";
import { protectedGlobs } from "./policy.ts";
import { renderReport, loadGlobalRules, resolveMode } from "./perms-report.ts";
import { traceHooks } from "../_shared/hook-trace.ts";
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
  return (
    env.TRIS_PERMS_FILE ||
    `${env.HOME || ""}/.config/trismegistus/permissions.json`
  );
}

function modePath(env: NodeJS.ProcessEnv = process.env): string {
  if (env.OFFICINA_PERMISSIONS_MODE_FILE)
    return env.OFFICINA_PERMISSIONS_MODE_FILE;
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

/** Content preview bounds (I3): lines first, chars as the hard stop. */
export const PREVIEW_LINES = 40;
export const PREVIEW_CHARS = 2000;

/** `+ `-prefixed bounded preview of the content a write/edit will put on
 *  disk. Pure; exported for tests. Pure + deterministic by contract. */
export function contentPreviewOf(
  content: string | undefined,
): string | undefined {
  if (!content || !content.trim()) return undefined;
  const all = content.split("\n");
  const shown = all.slice(0, PREVIEW_LINES);
  const out: string[] = [];
  let chars = 0;
  for (const line of shown) {
    const capped = line.length > 200 ? `${line.slice(0, 200)}…` : line;
    chars += capped.length + 2;
    out.push(`+ ${capped}`);
    if (chars > PREVIEW_CHARS) {
      out.pop();
      out.push("+ … (preview truncated at char budget)");
      break;
    }
  }
  const rest = all.length - out.length;
  if (rest > 0) out.push(`… (${rest} more lines)`);
  return out.join("\n");
}

export default function (pi: ExtensionAPI) {
  if (process.env.TRIS_NO_PERMS === "1") return;
  // Opt-in hook timing (OFFICINA_HOOK_TRACE=1): a tool_call handler that never
  // resolves blocks the tool with no output (2026-11-15).
  traceHooks("permissions-guard", pi);

  let cache: { mtime: number; snap: PermSnapshot } | null = null;
  let warned = false;
  let sandboxWarned = false;

  /** cwd of the tool call being decided (project scoping). */
  let currentCwd = "";

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
    program = "",
    contentPreview?: string,
  ): Promise<{ block: true; reason: string } | undefined> {
    const mode = currentMode();
    if (verdict.action === "allow") {
      emitHarnessEvent(
        harnessEvent("lc-perms", "allow", { detail: `${mode} ${detail}` }),
      );
      return undefined;
    }
    if (verdict.action === "deny") {
      emitHarnessEvent(
        harnessEvent("lc-perms", "deny", { detail: `${mode} ${detail}` }),
      );
      return { block: true, reason: `permissions: ${verdict.reason}` };
    }
    const headless = ctx.mode !== "tui" && ctx.mode !== "rpc";
    if (headless || !ctx.hasUI) {
      emitHarnessEvent(
        harnessEvent("lc-perms", "ask-denied-headless", {
          detail: `${mode} ${detail}`,
        }),
      );
      return {
        block: true,
        reason: `permissions: ${verdict.reason} — denied in headless mode (fail-closed)`,
      };
    }
    // Project-scoped allowlist (2026-11-17): consulted BEFORE asking, after
    // protected roots and explicit denies have already decided. A stored rule
    // can never override those (invariant I2 — protected roots win).
    // I2 (protected roots win): the project allowlist covers COMMANDS only.
    // A path ask happens because the path is protected (secrets, ~/.ssh,
    // /etc/**) or because the policy wants a human — a standing rule must never
    // silence that, so path tools always prompt.
    const cwd = projectCwd();
    const commandTool = tool === "bash" || tool === "powershell";
    const rules = commandTool ? loadProjectStore(cwd).rules : [];
    const stored = commandTool
      ? matchesProjectRule(rules, tool, commandOf(detail, tool))
      : null;
    if (stored) {
      emitHarnessEvent(
        harnessEvent("lc-perms", "project-allow", {
          detail: `${mode} ${stored.kind}:${stored.pattern} → ${detail}`,
        }),
      );
      return undefined;
    }

    // pi's `select` RPC cannot carry a message (rpc-types: select has only
    // title+options), so the command has to ride on the CONFIRM dialog — which
    // is the safety-critical question anyway. Remembering is a follow-up
    // question, offered only after an approval (2026-11-17).
    const yes = await ctx.ui?.confirm(
      `permissions · ${tool}`,
      buildAskMessage(tool, verdict.reason, detail, mode, contentPreview),
    );
    emitHarnessEvent(
      harnessEvent("lc-perms", yes ? "ask-approved" : "ask-declined", {
        detail: `${mode} ${yes ? "yes" : "no"} ${detail}`,
      }),
    );
    if (!yes)
      return { block: true, reason: `permissions: user declined ${tool}` };

    // Offer to remember, default = No (Enter never grants a standing rule).
    const canRemember =
      process.env.OFFICINA_PERMS_NO_REMEMBER !== "1" &&
      typeof ctx.ui?.select === "function";
    if (canRemember) {
      const target = program ? `all ${program}` : "this command";
      const pick = await ctx.ui?.select("Remember for this project?", [
        "No, just this once",
        `Allow ${target} from now on`,
      ]);
      if (typeof pick === "string" && pick.startsWith("Allow ")) {
        const kind = program ? "program" : "command";
        const pattern = program || normalizeCommand(commandOf(detail, tool));
        const saved = addProjectRule(cwd, { tool, kind, pattern });
        emitHarnessEvent(
          harnessEvent("lc-perms", "project-rule", {
            detail: `${saved ? "stored" : "STORE FAILED"} ${kind}:${pattern} (${cwd})`,
          }),
        );
        if (!saved) {
          ctx.ui.notify?.(
            "permissions: could not store the project rule (see lc-perms/project-rule)",
            "warning",
          );
        }
      }
    }
    return undefined;
  }

  /** The project the operator is working in (the tool call's cwd). */
  function projectCwd(): string {
    return (currentCwd || process.cwd()).replace(/\/+$/, "") || "/";
  }

  /** The command (or path) a rule would be filed against. */
  function commandOf(detail: string, tool: string): string {
    return tool === "bash" || tool === "powershell"
      ? detail
      : detail.replace(new RegExp(`^${tool}\\s+`), "");
  }

  /**
   * The dialog body: the reason, then the FULL command (capped so a pathological
   * paste cannot hang a frame), then the mode. The command used to be collapsed
   * to one line and cut at 120 chars, which made heredocs and && chains
   * unreadable at exactly the wrong moment (2026-11-17).
   *
   * 2026-11-21 (I2/I3): write/edit dialogs gain a bounded CONTENT preview —
   * the operator was approving a path blind. Preview is `+ `-prefixed, capped
   * at CONTENT_LINES/CONTENT_CHARS with a tail summary (I3: a 1 MB write
   * must not hang a frame).
   */
  function buildAskMessage(
    tool: string,
    reason: string,
    detail: string,
    mode: string,
    contentPreview?: string,
  ): string {
    const MAX = 2000;
    const full = detail.length > MAX ? `${detail.slice(0, MAX)}\n… (${detail.length - MAX} more chars)` : detail;
    const shown = tool === "bash" || tool === "powershell" ? `$ ${full.replace(/\n/g, "\n  ")}` : full;
    const preview = contentPreview ? `\n\n${contentPreview}` : "";
    return `${reason}\n\n${shown}${preview}\n\n(mode: ${mode})`;
  }



  // /permissions — read-only report of every rule surface (2026-11-21, I4/I5).
  pi.registerCommand("permissions-report", {
    description:
      "show active mode, global path rules, project allowlist, protected roots (read-only; /permissions switches mode)",
    handler: async (_args: string, ctx: unknown) => {
      const c = ctx as { cwd?: string; ui?: any };
      const cwd = (c.cwd || process.cwd()).replace(/\/+$/, "");
      const { mode, source } = resolveMode(process.env);
      const globalPath = join(
        homedir(),
        ".config",
        "trismegistus",
        "permissions.json",
      );
      const lines = renderReport({
        mode,
        modeSource: source,
        globalRules: loadGlobalRules(globalPath),
        globalPath,
        projectRules: (() => {
          const store = loadProjectStore(cwd);
          return store ? store.rules.map((r) => ({ tool: r.tool, kind: r.kind, pattern: r.pattern })) : null;
        })(),
        projectPath: projectStorePath(cwd),
        protectedRoots: protectedGlobs(process.env.HOME || homedir()),
      });
      await c.ui?.select?.(
        `permissions · ${lines.length} lines (esc to close)`,
        lines,
      );
    },
  });

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
      emitHarnessEvent(
        harnessEvent("lc-perms", "no-snapshot", { detail: "mode policy only" }),
      );
      ctx.ui.notify(
        "permissions-guard: no permissions.json (run `trismegistus perms-sync`) — mode policy in effect",
        "warning",
      );
    }

    currentCwd = cwd;
    if (tool === "bash" || tool === "powershell") {
      const command = String(input.command ?? "");
      if (!command.trim()) return undefined;
      const verdict = commandVerdict(command, cwd, home, mode);
      const enforced = await decideAndEnforce(
        tool,
        verdict,
        command, // FULL command: the dialog must show what is being approved
        ctx as any,
        leadingProgram(command),
      );
      if (enforced) return enforced;
      // Codex-qol stage 10 (I6): only sandboxed mode wraps; bwrap missing
      // degrades to unwrapped with exactly one warning per session.
      if (tool === "bash" && shouldWrap(mode)) {
        const bwrap = findBwrap();
        if (!bwrap) {
          if (!sandboxWarned) {
            sandboxWarned = true;
            ctx.ui.notify?.(
              "sandbox: bwrap not found — commands run unwrapped",
              "warning",
            );
          }
        } else {
          const net = process.env.OFFICINA_SANDBOX_NET === "1";
          (e.input as Record<string, unknown>).command = wrapBwrap(
            command,
            cwd,
            bwrap,
            { net },
          );
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
          (e.input as Record<string, unknown>).command = rewriteToDispatch(
            command,
            paths,
            job,
          );
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

    if (tool !== "edit" && tool !== "write" && tool !== "read")
      return undefined;
    const path = pathOf(input);
    if (!path) return undefined;
    const verdict = pathVerdict(tool, path, cwd, home, mode, snap);
    // I2: the operator approves content, not a path. (read keeps path-only.)
    const preview =
      tool === "read" ? undefined : contentPreviewOf(input.content as string);
    return decideAndEnforce(
      tool,
      verdict,
      `${tool} ${path}`,
      ctx as any,
      undefined,
      preview,
    );
  });
}

function shortCommand(command: string): string {
  const one = command.replace(/\s+/g, " ").trim();
  return one.length > 120 ? `${one.slice(0, 119)}…` : one;
}
