// Caduceus gateway state file (2026-11-12, QoL batch stage 2).
//
// The caduceus extension periodically serializes the gateway's observable
// reality to ~/.vitriol/officina/caduceus-state.json; the Rust TUI's
// /caduceus modal reads that file (read-only, no HTTP from Rust — plan
// invariant I4). Keys are snake_case so the serde side needs no renames.
// Pure builder + atomic writer, unit-tested.

import { mkdirSync, renameSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

export interface ConnectionBuckets {
  active: string[];
  backoff: string[];
  hard_down: string[];
}

export interface KeyRow {
  provider: string;
  env: string;
  value: string;
  source: string;
}

/** One row of the Providers tab (2026-11-12, panel stage 2). Facts only:
 *  enablement, key presence, gateway bucket, free model count, signup method. */
export interface ProviderRow {
  id: string;
  name: string;
  website: string;
  /** "api-key" | "oauth" | "" when unknown. */
  category: string;
  has_free: boolean;
  free_note: string;
  auth_hint: string;
  credential_env: string;
  key_present: boolean;
  enabled: boolean;
  /** signup-progress.json status: keyed|skipped|failed|pending|"" */
  signup_status: string;
  /** active | backoff | down | unknown (gateway does not list it). */
  gateway: string;
  free_models: number;
  /** Signup flow: api-key | browser | device | import | social | callback | "". */
  method: string;
}

/** One row of the Models tab. `free` is derived from the catalog — never
 *  user-writable (plan invariant I3). */
export interface CandidateRow {
  id: string;
  owner: string;
  context_window: number;
  max_tokens: number;
  free: boolean;
}

export interface CatalogProviderFact {
  id?: string;
  name?: string;
  website?: string;
  category?: string;
  hasFree?: boolean;
  freeNote?: string;
  authHint?: string;
}

export interface AccountFact {
  provider?: string;
  name?: string;
  credential_env?: string;
  website?: string;
  enabled?: boolean;
}

export interface OAuthProviderFact {
  id?: string;
  name?: string;
  flow?: string;
  note?: string;
}

export interface ProviderFactsInput {
  catalog: CatalogProviderFact[];
  accounts: AccountFact[];
  oauth: OAuthProviderFact[];
  /** Env var names present in the env file or the process environment. */
  envNames: ReadonlySet<string>;
  /** Gateway connection buckets; null = gateway unreachable. */
  buckets: ConnectionBuckets | null;
  /** Free tool-candidate count per owner id. */
  freeCounts: ReadonlyMap<string, number>;
  /** signup-progress.json providers map. */
  signup: Record<string, { status?: string } | undefined>;
}

/** Merge accounts + gateway catalog + OAuth registry into stable rows.
 *  Union by lowercase id, alphabetical, never invents providers. */
export function buildProviderRows(input: ProviderFactsInput): ProviderRow[] {
  const rows = new Map<string, ProviderRow>();
  const ensure = (id: string): ProviderRow => {
    const key = id.trim().toLowerCase();
    let row = rows.get(key);
    if (!row) {
      row = {
        id: key,
        name: key,
        website: "",
        category: "",
        has_free: false,
        free_note: "",
        auth_hint: "",
        credential_env: "",
        key_present: false,
        enabled: false,
        signup_status: "",
        gateway: "unknown",
        free_models: 0,
        method: "",
      };
      rows.set(key, row);
    }
    return row;
  };

  for (const account of input.accounts) {
    const id = String(account?.provider ?? "").trim();
    if (!id) continue;
    const row = ensure(id);
    if (account.name) row.name = String(account.name);
    if (account.website) row.website = String(account.website);
    if (account.credential_env) row.credential_env = String(account.credential_env);
    row.enabled = account.enabled === true;
  }

  for (const provider of input.catalog) {
    const id = String(provider?.id ?? "").trim();
    if (!id) continue;
    const row = ensure(id);
    if (provider.name) row.name = String(provider.name);
    if (provider.website) row.website = String(provider.website);
    if (provider.category) row.category = String(provider.category);
    row.has_free = provider.hasFree === true;
    if (provider.freeNote) row.free_note = String(provider.freeNote);
    if (provider.authHint) row.auth_hint = String(provider.authHint);
  }

  for (const provider of input.oauth) {
    const id = String(provider?.id ?? "").trim();
    if (!id) continue;
    const row = ensure(id);
    if (provider.name) row.name = String(provider.name);
    row.category = "oauth";
    row.method = String(provider.flow ?? "");
    if (provider.note && !row.free_note) row.free_note = String(provider.note);
  }

  for (const row of rows.values()) {
    if (!row.category) row.category = row.credential_env ? "api-key" : "";
    if (!row.method) row.method = row.category === "api-key" ? "api-key" : "";
    row.key_present = !!row.credential_env && input.envNames.has(row.credential_env);
    row.free_models = input.freeCounts.get(row.id) ?? 0;
    row.signup_status = String(input.signup[row.id]?.status ?? "");
    if (input.buckets) {
      if (input.buckets.active.includes(row.id)) row.gateway = "active";
      else if (input.buckets.backoff.includes(row.id)) row.gateway = "backoff";
      else if (input.buckets.hard_down.includes(row.id)) row.gateway = "down";
    }
  }

  return [...rows.values()].sort((a, b) => a.id.localeCompare(b.id));
}

export interface BudgetSlice {
  daily_tokens: number;
  daily_cap: number;
  monthly_tokens: number;
  monthly_cap: number;
  allowed: boolean;
  paid_allowed: boolean;
}

export interface CaduceusStateInput {
  base_url: string;
  gateway_up: boolean;
  catalog_count: number;
  catalog_fetched_at: string;
  connections: ConnectionBuckets;
  cooling: number;
  inference_key: string;
  keys: KeyRow[];
  budget: BudgetSlice;
  providers?: ProviderRow[];
  candidates?: CandidateRow[];
  now?: number;
}

export interface CaduceusState extends Omit<CaduceusStateInput, "now" | "providers" | "candidates"> {
  version: 2;
  ts: number;
  providers: ProviderRow[];
  candidates: CandidateRow[];
}

export function buildCaduceusState(input: CaduceusStateInput): CaduceusState {
  const { now, providers, candidates, ...rest } = input;
  return {
    version: 2,
    ts: now ?? Date.now(),
    ...rest,
    providers: providers ?? [],
    candidates: candidates ?? [],
  };
}

/** Atomic write (tmp + rename) — the modal must never read a half file. */
export function writeCaduceusState(path: string, state: CaduceusState): void {
  mkdirSync(dirname(path), { recursive: true });
  const tmp = `${path}.tmp-${process.pid}`;
  writeFileSync(tmp, `${JSON.stringify(state, null, 2)}\n`, { mode: 0o600 });
  renameSync(tmp, path);
}
