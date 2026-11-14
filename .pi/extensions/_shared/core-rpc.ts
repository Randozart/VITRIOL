// Core RPC client (2026-11-14) — Phase 2 sidecar for vitriol-core.
//
// Line-delimited JSON over stdio to `vitriol-core-rpc`. FAIL-OPEN by design:
// a missing binary, spawn error, timeout or malformed reply resolves
// `undefined`, and callers fall back to the TypeScript implementation. The
// sidecar must never be able to block a turn (invariant I1/P6).
//
// Enable: OFFICINA_ROUTER_CORE=rust.
// Binary: OFFICINA_CORE_BIN, else vitriol-core/target/{release,debug}/…, else
// ~/.local/bin/vitriol-core-rpc.

import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export function coreEnabled(): boolean {
  return String(process.env.OFFICINA_ROUTER_CORE ?? "").toLowerCase() === "rust";
}

function candidateBinaries(): string[] {
  // An explicit override is authoritative: no silent fallback to another
  // binary (tests and operators rely on that).
  const explicit = process.env.OFFICINA_CORE_BIN;
  if (explicit) return [explicit];
  const here = dirname(fileURLToPath(import.meta.url)); // .../officina/.pi/extensions/_shared
  const repoRoot = resolve(here, "..", "..", "..", "..");
  return [
    join(homedir(), ".local", "bin", "vitriol-core-rpc"),
    join(repoRoot, "vitriol-core", "target", "release", "vitriol-core-rpc"),
    join(repoRoot, "vitriol-core", "target", "debug", "vitriol-core-rpc"),
  ];
}

export function coreBinaryPath(): string | undefined {
  return candidateBinaries().find((path) => {
    try {
      return existsSync(path);
    } catch {
      return false;
    }
  });
}

interface CoreClientState {
  child?: ChildProcessWithoutNullStreams;
  buffer: string;
  nextId: number;
  pending: Map<number, (value: unknown) => void>;
  spawnFailedAt?: number;
}

const DEFAULT_TIMEOUT_MS = 250;

function state(): CoreClientState {
  const g = globalThis as { __vitriolCoreClient?: CoreClientState };
  // globalThis-backed: pi loads every extension through a separate jiti
  // instance (AGENTS.md rule) — the sidecar must be shared, not duplicated.
  if (!g.__vitriolCoreClient) {
    g.__vitriolCoreClient = { buffer: "", nextId: 1, pending: new Map() };
  }
  return g.__vitriolCoreClient;
}

function resolvePending(client: CoreClientState, value: unknown): void {
  for (const [id, done] of client.pending) {
    client.pending.delete(id);
    done(value);
  }
}

function ensureChild(client: CoreClientState): ChildProcessWithoutNullStreams | undefined {
  if (client.child && !client.child.killed) return client.child;
  const bin = coreBinaryPath();
  if (!bin) return undefined;
  try {
    const child = spawn(bin, [], { stdio: ["pipe", "pipe", "pipe"] });
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => {
      client.buffer += chunk;
      let idx = client.buffer.indexOf("\n");
      while (idx >= 0) {
        const line = client.buffer.slice(0, idx);
        client.buffer = client.buffer.slice(idx + 1);
        if (line.trim()) {
          try {
            const msg = JSON.parse(line) as { id?: number; result?: unknown; error?: string };
            if (typeof msg.id === "number") {
              const done = client.pending.get(msg.id);
              client.pending.delete(msg.id);
              done?.(msg.error ? undefined : msg.result);
            }
          } catch {
            // malformed line — ignore
          }
        }
        idx = client.buffer.indexOf("\n");
      }
    });
    const fail = (): void => {
      resolvePending(client, undefined);
      client.child = undefined;
      client.buffer = "";
    };
    child.on("error", fail);
    child.on("exit", fail);
    client.child = child;
    return child;
  } catch {
    client.spawnFailedAt = Date.now();
    return undefined;
  }
}

/** One JSON-RPC call. Resolves undefined on ANY problem (fail-open). */
export function coreCall(
  method: string,
  params: unknown,
  timeoutMs = DEFAULT_TIMEOUT_MS,
): Promise<unknown> {
  if (!coreEnabled()) return Promise.resolve(undefined);
  const client = state();
  const child = ensureChild(client);
  if (!child) return Promise.resolve(undefined);
  const id = client.nextId++;
  return new Promise((resolvePromise) => {
    const timer = setTimeout(() => {
      client.pending.delete(id);
      resolvePromise(undefined);
    }, timeoutMs);
    client.pending.set(id, (value) => {
      clearTimeout(timer);
      resolvePromise(value);
    });
    try {
      child.stdin.write(`${JSON.stringify({ id, method, params })}\n`);
    } catch {
      clearTimeout(timer);
      client.pending.delete(id);
      resolvePromise(undefined);
    }
  });
}

export interface CoreAdmissionLane {
  id: string;
  invalid: boolean;
  cooling: boolean;
  ownerPaused: boolean;
  declared: number;
  learned?: number | null;
  safeBytes?: number | null;
}

/** Batched admission (one IPC per routing decision). */
export async function coreAdmissions(
  promptTokens: number,
  reserveTokens: number,
  lanes: CoreAdmissionLane[],
): Promise<Map<string, string> | undefined> {
  const result = (await coreCall("admit", { promptTokens, reserveTokens, lanes })) as
    | { decisions?: Record<string, string> }
    | undefined;
  const decisions = result?.decisions;
  if (!decisions) return undefined;
  return new Map(Object.entries(decisions));
}

export interface CoreProbePlan {
  action: "probe" | "wait" | "exhausted";
  lane?: { id: string; owner: string };
  why?: string;
  backoffMs?: number;
  bucket?: unknown;
}

/** Probe scheduler decision (pure Rust policy). */
export async function coreProbePlan(params: unknown): Promise<CoreProbePlan | undefined> {
  const result = (await coreCall("probe_plan", params)) as CoreProbePlan | undefined;
  return result && typeof result.action === "string" ? result : undefined;
}

/** Test helper: kill any spawned sidecar. */
export function resetCoreClient(): void {
  const client = state();
  try {
    client.child?.kill();
  } catch {
    // ignore
  }
  resolvePending(client, undefined);
  client.child = undefined;
  client.buffer = "";
  (globalThis as { __vitriolCoreClient?: CoreClientState }).__vitriolCoreClient = undefined;
}
