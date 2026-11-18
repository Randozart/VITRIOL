// Cross-session claims extension (2026-11-18).
//
// Publishes "this session has these files open" and raises a band warning when
// another live session claims the same path. Advisory by construction: nothing
// here can block or fail a tool call (invariant I1) — losing the warning must
// never cost the operator their edit.

import { setBandRow, setBandUi } from "../_shared/band.ts";
import {
  claimNote,
  claimsDir,
  foreignClaims,
  publishClaims,
} from "../_shared/claims.ts";

/** Tools whose argument names a file we are about to write. */
const FILE_TOOLS = new Set(["edit", "write"]);

/** Pull a path out of the tool arguments without assuming one shape. */
export function pathOf(args: unknown): string | null {
  if (!args || typeof args !== "object") return null;
  const record = args as Record<string, unknown>;
  for (const key of ["path", "file_path", "filename", "file"]) {
    const value = record[key];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return null;
}

export default function register(pi: any): void {
  if (process.env.OFFICINA_NO_CLAIMS === "1") return;
  const pid = process.pid;
  // Keep the claims of this session across turns: a file opened once stays
  // claimed until the TTL expires or the session exits.
  const mine = new Set<string>();
  let sessionId = `tui-${pid}`;

  const republish = (): void =>
    publishClaims({ pid, sessionId, paths: [...mine] });

  pi.on("session_start", (_event: any, ctx: any) => {
    const file = ctx?.sessionManager?.getSessionFile?.();
    const base = file ? file.split("/").filter(Boolean).pop() : undefined;
    sessionId = base && base.trim() ? base : `tui-${pid}`;
    republish();
  });

  pi.on("tool_call", (event: any, ctx: any) => {
    const toolName = (event as { toolName?: string }).toolName ?? "";
    if (!FILE_TOOLS.has(toolName)) return;
    const path = pathOf((event as { args?: unknown }).args);
    if (!path) return;
    mine.add(path);
    republish();
    const conflicts = foreignClaims([path], pid);
    try {
      setBandUi((ctx as { ui?: unknown })?.ui as never);
      setBandRow("claims", "state", claimNote(conflicts));
    } catch {
      // decoration contract
    }
    if (conflicts.length > 0 && ctx?.ui?.notify) {
      ctx.ui.notify(
        `${claimNote(conflicts)} — both sessions may overwrite this file`,
        "warning",
      );
    }
    return undefined;
  });

  pi.on("session_shutdown", () => {
    // Release claims so a stale shard never warns the next session.
    mine.clear();
    publishClaims({ pid, sessionId, paths: [] });
  });

  pi.registerCommand?.("claims", {
    description: "show which files other sessions have open",
    handler: async (
      _args: string,
      ctx: { ui?: { notify?: (m: string, l?: string) => void } },
    ) => {
      const { readClaims } = await import("../_shared/claims.ts");
      const others = readClaims().filter((c) => c.pid !== pid);
      const lines = others.flatMap((c) => c.paths.map((p) => `${c.pid}  ${p}`));
      ctx?.ui?.notify?.(
        lines.length ? lines.join("\n") : "no other session claims a file",
        "info",
      );
    },
  });
}

/** Where claims are stored (exported for the test and for /claims). */
export const CLAIMS_DIR = (): string => claimsDir();
