// Compact when a GRIND (or PLAN) run finishes (2026-11-18).
//
// Why this boundary and not a fill threshold: a finished loop is the one moment
// where nothing is in flight (no tool mid-run, no stream to interrupt, no partial
// output worth keeping) and the context is at its fattest, because the run just
// produced thousands of tool results. A numeric threshold fires repeatedly as the
// fill wobbles; the end of a piece of work happens exactly once.
//
// Guards, each of which exists because of a failure seen in this repo:
//   - `compact_on_finish` must be on (env override wins);
//   - the context must be worth summarising (`min_tokens`, default 80k — a 20k
//     session gains nothing and costs a summary);
//   - a cooldown, so two loops back to back cannot compact twice in a minute
//     (double compaction is the loop we spent 2026-11-17 diagnosing);
//   - once per finished run: the request is stamped, so a re-entered settled
//     handler cannot fire again for the same run.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { createHash } from "node:crypto";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { emitHarnessEvent, harnessEvent } from "./events.ts";

export interface CompactOnFinishPolicy {
  enabled: boolean;
  minTokens: number;
  cooldownMs: number;
}

const DEFAULTS: CompactOnFinishPolicy = {
  enabled: true,
  minTokens: 80_000,
  cooldownMs: 10 * 60_000,
};

/** Resolve the policy: env overrides the defaults (no config file yet — this is
 *  a policy of one number, and an env var is enough to switch it off). */
export function resolveCompactPolicy(
  env: NodeJS.ProcessEnv = process.env,
): CompactOnFinishPolicy {
  const off = /^(1|true|yes|on)$/i.test(
    String(env.OFFICINA_NO_COMPACT_ON_FINISH ?? "").trim(),
  );
  const min = Number(env.OFFICINA_COMPACT_ON_FINISH_MIN_TOKENS);
  const cooldown = Number(env.OFFICINA_COMPACT_ON_FINISH_COOLDOWN_MS);
  return {
    enabled: !off,
    minTokens: Number.isFinite(min) && min >= 0 ? min : DEFAULTS.minTokens,
    cooldownMs:
      Number.isFinite(cooldown) && cooldown >= 0
        ? cooldown
        : DEFAULTS.cooldownMs,
  };
}

/**
 * Path of the request stamp (survives restarts, so a crash mid-compaction does
 * not turn the next loop end into a second request).
 *
 * Session-keyed (2026-11-18): the stamp used to be ONE global file, so with
 * several sessions in one project the first session's stamp silenced every
 * other session's compaction for the whole cooldown. Each session now owns its
 * stamp; the legacy single file is still read when no session id is known
 * (older callers, tests) so an in-flight run keeps working.
 */
export function stampPath(home = homedir(), sessionId?: string): string {
  const base = join(home, ".vitriol", "officina");
  if (!sessionId) return join(base, "compact-on-finish.json");
  return join(base, "compact-on-finish", `${safeStampKey(sessionId)}.json`);
}

/**
 * Filename-safe key for a session id. Session file basenames can exceed the
 * filesystem's 255-byte limit (they carry `.patched-<n>` chains), so hash them.
 */
export function safeStampKey(sessionId: string): string {
  return createHash("sha1").update(sessionId).digest("hex").slice(0, 16);
}

/**
 * This session's stamp identity: the session file basename when pi exposes one,
 * else the process — the same fallback the routing registry uses.
 */
export function sessionStampId(ctx: {
  sessionManager?: { getSessionFile?: () => string | undefined };
}): string {
  const file = ctx.sessionManager?.getSessionFile?.();
  const base = file ? file.split("/").filter(Boolean).pop() : undefined;
  return base && base.trim() ? base : `tui-${process.pid}`;
}

/** Pure decision: may we request a compaction right now? */
export function shouldCompactOnFinish(input: {
  policy: CompactOnFinishPolicy;
  fillTokens: number | null;
  lastRequestAt: number | null;
  now: number;
}): { ok: boolean; reason: string } {
  const { policy, fillTokens, lastRequestAt, now } = input;
  if (!policy.enabled) return { ok: false, reason: "disabled" };
  if (fillTokens === null || !Number.isFinite(fillTokens)) {
    return { ok: false, reason: "context size unknown" };
  }
  if (fillTokens < policy.minTokens) {
    return {
      ok: false,
      reason: `only ${Math.round(fillTokens)} tokens (min ${policy.minTokens})`,
    };
  }
  if (lastRequestAt !== null && now - lastRequestAt < policy.cooldownMs) {
    return { ok: false, reason: "cooldown" };
  }
  return { ok: true, reason: "finished run" };
}

/** Read the last request stamp, or null. */
export function lastCompactRequest(
  home = homedir(),
  sessionId?: string,
): number | null {
  const read = (id?: string): number | null => {
    try {
      const doc = JSON.parse(readFileSync(stampPath(home, id), "utf8")) as {
        at?: unknown;
      };
      return typeof doc.at === "number" && Number.isFinite(doc.at)
        ? doc.at
        : null;
    } catch {
      return null;
    }
  };
  const own = read(sessionId);
  // Sessions start fresh: only fall back to the legacy global stamp when this
  // session has never stamped, so an upgrade mid-run does not double-request.
  return own ?? (sessionId ? read(undefined) : null);
}

/** Record a request stamp. Best effort: a failed write must not break the turn. */
export function stampCompactRequest(
  now: number,
  reason: string,
  home = homedir(),
  sessionId?: string,
): void {
  try {
    const path = stampPath(home, sessionId);
    mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
    writeFileSync(path, `${JSON.stringify({ at: now, reason }, null, 2)}\n`, {
      mode: 0o600,
    });
  } catch {
    /* best effort */
  }
}

/**
 * Ask for a compaction at the end of a run, if the policy allows it.
 *
 * Returns the decision so callers can paint it; the compaction itself is
 * asynchronous and reports through `onComplete` / `onError`.
 */
export function compactOnFinish(
  ctx: {
    compact?: (options?: {
      customInstructions?: string;
      onComplete?: (result: unknown) => void;
      onError?: (error: Error) => void;
    }) => void;
    getContextUsage?: () =>
      { tokens: number | null; contextWindow: number } | undefined;
    ui?: { notify?: (message: string, level?: string) => void };
    sessionManager?: { getSessionFile?: () => string | undefined };
  },
  reason: string,
  options: {
    home?: string;
    now?: number;
    policy?: CompactOnFinishPolicy;
    sessionId?: string;
  } = {},
): { requested: boolean; reason: string } {
  const now = options.now ?? Date.now();
  const policy = options.policy ?? resolveCompactPolicy();
  const sessionId = options.sessionId ?? sessionStampId(ctx);
  const usage = ctx.getContextUsage?.();
  const decision = shouldCompactOnFinish({
    policy,
    fillTokens: usage?.tokens ?? null,
    lastRequestAt: lastCompactRequest(options.home, sessionId),
    now,
  });

  emitHarnessEvent(
    harnessEvent("lc-compact", "finish", {
      detail: `${decision.ok ? "request" : "skip"} (${reason}): ${decision.reason}`,
    }),
  );

  if (!decision.ok) return { requested: false, reason: decision.reason };

  try {
    stampCompactRequest(now, reason, options.home, sessionId);
    ctx.compact?.({
      onError: (error) => {
        emitHarnessEvent(
          harnessEvent("lc-compact", "finish-error", {
            detail: `${reason}: ${error instanceof Error ? error.message : String(error)}`,
          }),
        );
        ctx.ui?.notify?.(
          `compaction after ${reason} failed: ${error.message}`,
          "warning",
        );
      },
    });
  } catch (error) {
    return {
      requested: false,
      reason: `request failed: ${error instanceof Error ? error.message : String(error)}`,
    };
  }
  return { requested: true, reason: decision.reason };
}
