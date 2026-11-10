// Fiat Lux — the free-tier merry-go-round coupling.
//
// Pure helpers so the CLI (fiat.mjs) and the tests can share them without
// importing the gateway. The Caduceus catalog is the source of truth for
// which models are free; the coupling file is the thing pi actually loads.

import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export const FIAT_LUX_ID = "fiat-lux";
export const FIAT_LUX_NAME = "Fiat Lux – free merry-go-round";
export const DEFAULT_CONTEXT = 65_536;
export const DEFAULT_MAX_TOKENS = 8_192;

// A catalog row is a Fiat Lux member when its id or name marks it free.
// The gateway catalog is the source of truth for zero-cost tiers; we do not
// hardcode model lists (AGENTS rule: no hardcoded model constants).
const FREE_RE = /\bfree\b|-(free|freemium)\b|\$0\b|zero.cost/i;

// A catalog row is a Fiat Lux member when its MODEL id or NAME marks it
// free (not the provider slug — a provider named "freebuff" is not itself
// evidence that a model is free). The gateway catalog is the source of
// truth for zero-cost tiers; we do not hardcode model lists.
export function isFreeModel(row) {
  if (!row || typeof row !== "object") return false;
  for (const key of ["id", "name", "root", "display_name"]) {
    if (FREE_RE.test(String(row[key] ?? ""))) return true;
  }
  return false;
}

// Derive the Fiat Lux CouplingDef from a catalog of model rows.
// Fallback chain order: stable by (provider, id) — the TUI adds round-robin
// advancement by re-sorting at write time when a provider fails; the pure
// function stays deterministic for tests.
export function buildFiatLuxCoupling(catalogRows, baseUrl) {
  const models = [];
  const seen = new Set();
  for (const row of Array.isArray(catalogRows) ? catalogRows : []) {
    if (!isFreeModel(row)) continue;
    const id = typeof row.id === "string" ? row.id.trim() : "";
    if (!id || seen.has(id)) continue;
    seen.add(id);
    const name =
      typeof row.name === "string" && row.name.trim()
        ? row.name.trim()
        : id;
    const contextWindow =
      Number.isFinite(Number(row.context_length)) && Number(row.context_length) > 0
        ? Math.floor(Number(row.context_length))
        : DEFAULT_CONTEXT;
    const maxTokens =
      Number.isFinite(Number(row.max_output_tokens)) && Number(row.max_output_tokens) > 0
        ? Math.floor(Number(row.max_output_tokens))
        : DEFAULT_MAX_TOKENS;
    models.push({ id, name, contextWindow, maxTokens });
  }
  models.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  if (models.length === 0) return null;
  return {
    id: FIAT_LUX_ID,
    name: FIAT_LUX_NAME,
    baseUrl: String(baseUrl || "").replace(/\/+$/, ""),
    api: "openai-completions",
    apiKeyEnv: "OFFICINA_CADUCEUS_API_KEY",
    models,
  };
}

// Merge the Fiat Lux coupling into an existing couplings file, replacing any
// prior fiat-lux entry and preserving all other couplings + unknown keys.
// Returns { text, changed }.
export function mergeFiatLuxIntoCouplings(existingText, coupling) {
  let parsed = { couplings: [] };
  if (existingText && existingText.trim()) {
    try {
      parsed = JSON.parse(existingText);
    } catch {
      parsed = { couplings: [] };
    }
  }
  if (!Array.isArray(parsed.couplings)) parsed.couplings = [];
  const kept = parsed.couplings.filter((c) => !(c && typeof c === "object" && c.id === FIAT_LUX_ID));
  const next = { ...parsed, couplings: [...kept, coupling] };
  const text = `${JSON.stringify(next, null, 2)}\n`;
  return { text, changed: text !== existingText };
}

// Atomic, 0600 write of the couplings file (never clobbers sibling couplings
// because mergeFiatLuxIntoCouplings re-serializes the whole document).
export function writeFiatLuxCouplings(filePath, coupling) {
  const existing = existsSync(filePath) ? readFileSync(filePath, "utf8") : "";
  const { text, changed } = mergeFiatLuxIntoCouplings(existing, coupling);
  if (!changed) return false;
  mkdirSync(join(filePath, ".."), { recursive: true });
  const tmp = `${filePath}.tmp-${process.pid}`;
  writeFileSync(tmp, text, { mode: 0o600 });
  renameSync(tmp, filePath);
  return true;
}

// Classify a Fiat Lux membership row for the TUI: which effort ladder rung
// does this provider sit on? (zero-key, reused-auth, oauth, api-key)
export function membershipState(provider, { connectedProviders, authProviders, oauthProviders }) {
  const p = String(provider || "").trim().toLowerCase();
  if (!p) return "api-key";
  if (connectedProviders?.has(p)) return "active";
  if (authProviders?.has(p)) return "auth-reused";
  if (oauthProviders?.has(p)) return "oauth";
  return "api-key";
}
