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
  now?: number;
}

export interface CaduceusState extends Omit<CaduceusStateInput, "now"> {
  ts: number;
}

export function buildCaduceusState(input: CaduceusStateInput): CaduceusState {
  const { now, ...rest } = input;
  return { ts: now ?? Date.now(), ...rest };
}

/** Atomic write (tmp + rename) — the modal must never read a half file. */
export function writeCaduceusState(path: string, state: CaduceusState): void {
  mkdirSync(dirname(path), { recursive: true });
  const tmp = `${path}.tmp-${process.pid}`;
  writeFileSync(tmp, `${JSON.stringify(state, null, 2)}\n`, { mode: 0o600 });
  renameSync(tmp, path);
}
