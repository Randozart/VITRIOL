import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { existsSync, readFileSync, statSync } from "node:fs";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import { decide, parseSnapshot, pathOf, type PermSnapshot } from "./perms.ts";

// permissions-guard — the safety.permissions DSL, enforced (gap D1).
// Policy source: ~/.config/trismegistus/config.yaml, mirrored to
// permissions.json by `trismegistus perms-sync` (this runtime has no YAML
// parser; the mirror carries source_hash so the validator can catch drift).
//
// Enforcement at tool_call (pre-execution veto — the same hook write-guard
// uses). Semantics (docs/DAILY-DRIVER-GAP.md Decision 1):
//   deny          -> block with reason
//   ask + TUI     -> ui.confirm; declined = block
//   ask + headless-> BLOCK (fail-closed: unattended runs cannot self-approve)
//   no snapshot   -> default policy + one loud WARN event
//   read_file/... scope: edit|write|read only; bash stays with write-guard.
//
// Kill switch: TRIS_NO_PERMS=1 registers nothing.

/** Resolved PER CALL (import-time capture once made tests hit the live
 * mirror — caught by the wiring suite; env must win dynamically). */
function snapshotPath(env: NodeJS.ProcessEnv = process.env): string {
  return env.TRIS_PERMS_FILE || `${env.HOME || ""}/.config/trismegistus/permissions.json`;
}

export default function (pi: ExtensionAPI) {
  if (process.env.TRIS_NO_PERMS === "1") return;

  let cache: { mtime: number; snap: PermSnapshot } | null = null;
  let warned = false;

  function load(): PermSnapshot | null {
    try {
      const m = statSync(snapshotPath()).mtimeMs;
      if (cache && cache.mtime === m) return cache.snap;
      const snap = parseSnapshot(readFileSync(snapshotPath(), "utf8"));
      if (snap) cache = { mtime: m, snap };
      return snap;
    } catch {
      return null; // missing file: permissive default, warned once below
    }
  }

  pi.on("tool_call", async (event, ctx) => {
    const e = event as { toolName?: string; input?: Record<string, unknown> };
    const tool = String(e.toolName ?? "").toLowerCase();
    if (tool !== "edit" && tool !== "write" && tool !== "read") return undefined;
    const path = pathOf(e.input);
    if (!path) return undefined;

    const snap = load();
    if (!snap) {
      if (!warned) {
        warned = true;
        emitHarnessEvent(harnessEvent("lc-perms", "no-snapshot", { detail: "allow-all until perms-sync" }));
        ctx.ui.notify("permissions-guard: no permissions.json (run `trismegistus perms-sync`) — default policy in effect", "warning");
      }
      return undefined;
    }

    const v = decide(snap, tool, path, ctx.cwd);
    if (v.action === "allow") {
      if (v.ruleIndex >= 0) emitHarnessEvent(harnessEvent("lc-perms", "allow", { detail: `${tool} ${path}` }));
      return undefined;
    }
    if (v.action === "deny") {
      emitHarnessEvent(harnessEvent("lc-perms", "deny", { detail: `${tool} ${path}` }));
      return { block: true, reason: `permissions: ${v.pattern} denies ${tool} on this path (unified config safety.permissions)` };
    }
    // ask
    const headless = ctx.mode !== "tui" && ctx.mode !== "rpc";
    if (headless || !ctx.hasUI) {
      emitHarnessEvent(harnessEvent("lc-perms", "ask-denied-headless", { detail: `${tool} ${path}` }));
      return { block: true, reason: `permissions: ${v.pattern} requires approval for ${tool} — denied in headless mode (fail-closed)` };
    }
    const yes = await ctx.ui.confirm("permissions", `Allow ${tool} on ${path}?\n(rule: ${v.pattern})`);
    emitHarnessEvent(harnessEvent("lc-perms", yes ? "ask-approved" : "ask-declined", { detail: `${tool} ${path}` }));
    if (!yes) return { block: true, reason: `permissions: user declined ${tool} on ${path}` };
    return undefined;
  });
}
