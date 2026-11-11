// P1 routing registry — peer visibility for multi-session adaptive routing.
//
// Every TUI's adaptive-router writes one entry per turn decision; the other
// TUIs read it to know how many peers are on local and how senior they are.
//
// Pure core (upsert / read / expire / prune / serialize) is FS-free so the
// behaviour is unit-testable; the thin FS wrapper (mode 600, atomic write)
// lives at the bottom and is exercised by the jiti-isolation contract.
//
// Invariant I4: the entry shape carries NO prompt text — only ids, tier,
// model, token count, and (later) the pinned slot id.

import { chmodSync, existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";

export const DEFAULT_TTL_MS = 60_000;

export interface PeerEntry {
  ts: number;
  tuiPid: number;
  sessionId: string;
  tier: "local" | "cloud";
  model: string;
  ctxTokens: number;
  slotId?: number;
}

export interface RegistryCore {
  entries: Map<string, PeerEntry>;
}

function keyOf(tuiPid: number, sessionId: string): string {
  return `${tuiPid}:${sessionId}`;
}

/** Create an empty core (deterministic; "now" is passed to readers). */
export function createCore(_now: number): RegistryCore {
  return { entries: new Map() };
}

/**
 * Insert or replace the caller's entry. Replacement is by (tuiPid,
 * sessionId); latest ts wins (a re-write is always "newer").
 */
export function upsertEntry(core: RegistryCore, entry: PeerEntry): void {
  core.entries.set(keyOf(entry.tuiPid, entry.sessionId), entry);
}

/**
 * Live peers (not the caller, not expired), sorted oldest-first
 * (ts ascending, tuiPid tiebreak) so index 0 is the most senior.
 */
export function readPeers(
  core: RegistryCore,
  now: number,
  selfTuiPid: number,
  ttlMs: number = DEFAULT_TTL_MS,
): PeerEntry[] {
  const out: PeerEntry[] = [];
  for (const [key, e] of core.entries) {
    if (e.tuiPid === selfTuiPid) continue;
    if (now - e.ts > ttlMs) continue;
    out.push(e);
  }
  out.sort((a, b) => a.ts - b.ts || a.tuiPid - b.tuiPid);
  return out;
}

/** Remove expired entries; returns how many were dropped. */
export function pruneExpired(core: RegistryCore, now: number, ttlMs: number = DEFAULT_TTL_MS): number {
  let removed = 0;
  for (const [key, e] of core.entries) {
    if (now - e.ts > ttlMs) {
      core.entries.delete(key);
      removed += 1;
    }
  }
  return removed;
}

interface Serialized {
  /** Untrusted shape: every entry is validated field-by-field on load. */
  entries: unknown[];
}

/** Stable JSON (pid-ordered) so a diff of two serializations is meaningful. */
export function serialize(core: RegistryCore): string {
  const entries = [...core.entries.values()].sort((a, b) => a.tuiPid - b.tuiPid || a.sessionId.localeCompare(b.sessionId));
  const payload: Serialized = { entries };
  return JSON.stringify(payload);
}

const REQUIRED: Array<keyof PeerEntry> = ["ts", "tuiPid", "sessionId", "tier", "model", "ctxTokens"];

/** Load entries into the core; garbage input yields an empty core, never a throw. */
export function deserialize(core: RegistryCore, json: string): void {
  core.entries.clear();
  let parsed: unknown;
  try {
    parsed = JSON.parse(json);
  } catch {
    return;
  }
  if (typeof parsed !== "object" || parsed === null) return;
  const list = (parsed as Serialized).entries;
  if (!Array.isArray(list)) return;
  for (const raw of list) {
    if (typeof raw !== "object" || raw === null) continue;
    const e = raw as Record<string, unknown>;
    if (!REQUIRED.every((k) => e[k] !== undefined)) continue;
    if (typeof e.ts !== "number" || typeof e.tuiPid !== "number") continue;
    if (typeof e.sessionId !== "string" || typeof e.model !== "string") continue;
    if (typeof e.ctxTokens !== "number") continue;
    if (e.tier !== "local" && e.tier !== "cloud") continue;
    const entry: PeerEntry = {
      ts: e.ts,
      tuiPid: e.tuiPid,
      sessionId: e.sessionId,
      tier: e.tier,
      model: e.model,
      ctxTokens: e.ctxTokens,
    };
    if (typeof e.slotId === "number") entry.slotId = e.slotId;
    core.entries.set(keyOf(entry.tuiPid, entry.sessionId), entry);
  }
}

// ── FS wrapper: file-backed registry shared across TUI processes ──────────
//
// Each TUI is a separate process (own `pi --mode rpc`), so peer visibility
// must go through the filesystem, not module state. Reads are fresh (another
// process may have written since), writes are read-modify-write + atomic
// rename so a crash mid-write cannot truncate the registry.

/** Path of the shared registry (mode 600). Override for tests. */
export function registryPath(env: NodeJS.ProcessEnv = process.env): string {
  return (
    env.OFFICINA_ROUTING_REGISTRY ||
    join(homedir(), ".vitriol", "officina", "routing-registry.json")
  );
}

interface SharedRegistry {
  core: RegistryCore;
  /** Path last loaded from, so a test path change invalidates the cache. */
  path: string;
}

function shared(path: string): SharedRegistry {
  const g = globalThis as { __officinaRoutingRegistry?: SharedRegistry };
  // globalThis-backed: pi loads each extension through a separate jiti
  // instance, so a module-level cache would not be shared (AGENTS.md rule).
  if (!g.__officinaRoutingRegistry || g.__officinaRoutingRegistry.path !== path) {
    g.__officinaRoutingRegistry = { core: createCore(Date.now()), path };
  }
  return g.__officinaRoutingRegistry;
}

/** Read the registry from disk; missing/garbage input yields an empty core. */
export function loadRegistry(path: string = registryPath()): RegistryCore {
  const core = createCore(Date.now());
  if (!existsSync(path)) return core;
  try {
    deserialize(core, readFileSync(path, "utf8"));
  } catch {
    // Unreadable registry is not fatal: peer visibility degrades to "alone".
  }
  return core;
}

function writeAtomic(path: string, body: string): void {
  mkdirSync(dirname(path), { recursive: true });
  const tmp = `${path}.${process.pid}.tmp`;
  writeFileSync(tmp, body, { mode: 0o600 });
  chmodSync(tmp, 0o600);
  renameSync(tmp, path);
}

/**
 * Publish this session's decision. Read-modify-write so peers are preserved;
 * expired entries are pruned on the same pass.
 */
export function writeRegistryEntry(
  entry: PeerEntry,
  path: string = registryPath(),
  now: number = Date.now(),
): void {
  const core = loadRegistry(path);
  pruneExpired(core, now);
  upsertEntry(core, entry);
  writeAtomic(path, serialize(core));
  shared(path).core = core;
}

/** Live peers, read fresh from disk (excludes self by tuiPid). */
export function readLivePeers(
  selfTuiPid: number,
  opts: { now?: number; path?: string; ttlMs?: number } = {},
): PeerEntry[] {
  const path = opts.path ?? registryPath();
  const core = loadRegistry(path);
  return readPeers(core, opts.now ?? Date.now(), selfTuiPid, opts.ttlMs);
}

/** Test hook: drop the cached globalThis core. */
export function resetRegistryCache(): void {
  delete (globalThis as { __officinaRoutingRegistry?: SharedRegistry })
    .__officinaRoutingRegistry;
}
