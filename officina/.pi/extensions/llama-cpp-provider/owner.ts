import { randomUUID } from "node:crypto";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";

interface Owner { session: string; branch: string }
const ENTRY = "officina.kv-owner";
type OwnerContext = Pick<ExtensionContext, "sessionManager">;

/** Pi loads extensions through separate jiti instances; ownership must survive that boundary. */
function owners(): Map<string, Owner> {
  const root = globalThis as { __officinaCacheOwners?: Map<string, Owner> };
  return root.__officinaCacheOwners ??= new Map();
}

/** Return only this process's current local session/branch identity. */
export function localCacheOwner(session: string | undefined): Owner | undefined {
  if (!session || process.env.OFFICINA_NO_CACHE_OWNERS === "1") return undefined;
  return owners().get(session);
}

/** Restore only a versioned branch marker for this exact session. */
function savedBranch(ctx: OwnerContext, session: string): string | undefined {
  const entries = ctx.sessionManager.getBranch?.() ?? [];
  const previous = entries.slice().reverse().find((entry: any) => entry?.type === "custom" && entry?.customType === ENTRY) as
    { data?: { version?: number; session?: string; branch?: string } } | undefined;
  const saved = previous?.data;
  if (saved?.version !== 1 || saved.session !== session || typeof saved.branch !== "string" || saved.branch.length === 0 || saved.branch.length > 256) return undefined;
  return saved.branch;
}

/** Navigation records a new frontier; ordinary reload keeps the persisted identity. */
function bindOwner(pi: ExtensionAPI, ctx: OwnerContext, fork: boolean): Owner | undefined {
  const session = ctx.sessionManager?.getSessionId?.();
  if (!session || typeof pi.appendEntry !== "function") return undefined;
  const previous = savedBranch(ctx, session);
  const branch = fork ? randomUUID() : previous ?? randomUUID();
  if (fork || branch !== previous) pi.appendEntry(ENTRY, { version: 1, session, branch });
  return { session, branch };
}

/** Persist branch identity as non-message session metadata; navigation keeps the old frontier alive. */
export function registerCacheOwners(pi: ExtensionAPI): void {
  if (process.env.OFFICINA_NO_CACHE_OWNERS === "1") return;
  let current: string | undefined;
  const safeBind = (ctx: OwnerContext, fork: boolean) => {
    if (current) owners().delete(current);
    try {
      const owner = bindOwner(pi, ctx, fork);
      current = owner?.session;
      if (owner) owners().set(owner.session, owner);
    }
    catch {
      current = undefined;
      emitHarnessEvent(harnessEvent("lc-route", "cache-owner-unavailable"));
    }
  };
  pi.on("session_start", (_event, ctx) => safeBind(ctx, false));
  pi.on("session_tree", (_event, ctx) => safeBind(ctx, true));
  pi.on("session_shutdown", () => { if (current) owners().delete(current); });
}
