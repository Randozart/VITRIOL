// Hook timing trace (2026-11-15).
//
// Why: a `bash` tool call was observed to emit `tool_execution_start` and then
// never spawn a shell. pi runs `tool_call` handlers, and drains previously
// emitted Agent events, *before* the tool executes — so one handler that never
// resolves hangs the turn with no output and nothing on disk to say which
// handler it was. This wrapper makes that handler visible.
//
// Opt-in: OFFICINA_HOOK_TRACE=1. Off by default: zero cost, no file writes.
// Trail: ~/.vitriol/officina/hooks/<extension>.jsonl (bounded).
//
// PROVENANCE: original Officina diagnostic.

import { appendFileSync, mkdirSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const CAP_BYTES = 2 * 1024 * 1024;
/** A hook slower than this is interesting even when it eventually returns. */
const SLOW_MS = 5_000;

export function hookTraceEnabled(): boolean {
  const value = (process.env.OFFICINA_HOOK_TRACE ?? "").toLowerCase();
  return value === "1" || value === "true" || value === "on";
}

function tracePath(extension: string): string {
  const safe =
    extension.replace(/[^A-Za-z0-9._-]/g, "_").slice(0, 60) || "extension";
  return join(homedir(), ".vitriol", "officina", "hooks", `${safe}.jsonl`);
}

function write(extension: string, record: Record<string, unknown>): void {
  try {
    const path = tracePath(extension);
    mkdirSync(join(homedir(), ".vitriol", "officina", "hooks"), {
      recursive: true,
    });
    try {
      if (statSync(path).size > CAP_BYTES) return;
    } catch {
      // missing file: first write
    }
    appendFileSync(
      path,
      JSON.stringify({ ts: Date.now(), extension, ...record }) + "\n",
      {
        mode: 0o600,
      },
    );
  } catch {
    // Diagnosis must never break the actor.
  }
}

/**
 * Wrap every `pi.on` registration so each hook logs enter/exit and duration.
 * Call once at the top of the extension's default export. Returns the same
 * `pi` so callers keep the existing shape.
 *
 * Generic over the caller's API type: nothing is imported from the pi package,
 * so this helper stays usable from any extension without coupling.
 */
export function traceHooks<T extends { on: unknown }>(
  extension: string,
  pi: T,
): T {
  if (!hookTraceEnabled()) return pi;
  const api = pi as unknown as {
    on: (event: string, handler: (...args: unknown[]) => unknown) => unknown;
  };
  const original = api.on.bind(api);
  api.on = (event: string, handler: (...args: unknown[]) => unknown) => {
    const wrapped = async (...args: unknown[]) => {
      const started = Date.now();
      write(extension, { hook: event, phase: "enter" });
      try {
        return await handler(...args);
      } finally {
        const ms = Date.now() - started;
        write(extension, {
          hook: event,
          phase: "exit",
          ms,
          slow: ms >= SLOW_MS,
        });
      }
    };
    return original(event, wrapped);
  };
  return pi;
}
