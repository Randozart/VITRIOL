// Caduceus provider for Officina. Registers a dynamic OpenAI-compatible
// provider from the gateway's model catalog (OmniRoute vendor, MIT). Strict passthrough is
// enforced per request with headers, so a global cache/compression toggle
// drifting on the gateway cannot silently rewrite prompts.

import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import type { ExtensionAPI, ProviderConfig } from "@earendil-works/pi-coding-agent";
import { budgetStatus } from "../_shared/budget.ts";
import { allowPaidOwners, readPrefs } from "../_shared/caduceus-prefs.ts";
import { classifyConnections, type GatewayConnection } from "../_shared/lane-health.ts";
import {
  extractCatalogModels,
  filterCatalogByProviders,
  mapCatalogModels,
  toolCandidates,
  type PiModelEntry,
} from "./catalog.ts";
import {
  buildCaduceusState,
  buildProviderRows,
  writeCaduceusState,
  type AccountFact,
  type CandidateRow,
  type CatalogProviderFact,
  type OAuthProviderFact,
  type ProviderRow,
} from "./state.ts";

const PROVIDER_ID = "caduceus";
const PROVIDER_NAME = "Caduceus";
const DEFAULT_BASE_URL = "http://127.0.0.1:20128/v1";
const CACHE_FILE = join(homedir(), ".vitriol", "officina", "caduceus-models.json");
const STATE_FILE = join(homedir(), ".vitriol", "officina", "caduceus-state.json");
const VENDOR_API_MODULE = join(
  homedir(),
  ".vitriol",
  "vendor",
  "omniroute",
  "node_modules",
  "omniroute",
  "bin",
  "cli",
  "api.mjs",
);
const PASSTHROUGH_HEADERS = Object.freeze({
  "X-OmniRoute-No-Cache": "true",
  "X-OmniRoute-Compression": "off",
  "x-omniroute-no-memory": "true",
});

// Provider catalog (2026-11-12, panel stage 2). The authoritative list with
// hasFree/freeNote/website/authHint lives behind the vendor CLI
// (`providers available --json`). Cached on disk for 10 minutes; without it
// the page degrades to accounts.json + the OAuth registry (never lies).
const OFFICINA_DIR = join(homedir(), ".vitriol", "officina");
const PROVIDER_CACHE_FILE = join(OFFICINA_DIR, "provider-catalog.json");
const PROVIDER_CACHE_TTL_MS = 10 * 60_000;
const VENDOR_CLI = join(
  homedir(),
  ".vitriol",
  "vendor",
  "omniroute",
  "node_modules",
  ".bin",
  "omniroute",
);

export function parseJsonLoose(text: string): unknown {
  const cleaned = text.replace(/\x1b\[[0-9;]*m/g, "");
  const obj = cleaned.indexOf("{");
  const arr = cleaned.indexOf("[");
  const start = obj < 0 ? arr : arr < 0 ? obj : Math.min(obj, arr);
  if (start < 0) return null;
  const close = cleaned[start] === "{" ? "}" : "]";
  const end = cleaned.lastIndexOf(close);
  if (end <= start) return null;
  try {
    return JSON.parse(cleaned.slice(start, end + 1));
  } catch {
    return null;
  }
}

function readProviderCache(): { fetchedAt: number; rows: CatalogProviderFact[] } | null {
  try {
    const parsed = JSON.parse(readFileSync(PROVIDER_CACHE_FILE, "utf8")) as {
      fetchedAt?: number;
      rows?: unknown;
    };
    if (!Array.isArray(parsed.rows)) return null;
    return { fetchedAt: Number(parsed.fetchedAt) || 0, rows: parsed.rows as CatalogProviderFact[] };
  } catch {
    return null;
  }
}

function writeProviderCache(rows: CatalogProviderFact[]): void {
  try {
    mkdirSync(dirname(PROVIDER_CACHE_FILE), { recursive: true });
    const tmp = `${PROVIDER_CACHE_FILE}.tmp-${process.pid}`;
    writeFileSync(tmp, `${JSON.stringify({ fetchedAt: Date.now(), rows }, null, 2)}\n`, {
      mode: 0o600,
    });
    renameSync(tmp, PROVIDER_CACHE_FILE);
  } catch {
    // cache is an optimization
  }
}

/** Provider catalog rows (id/name/website/category/hasFree/freeNote/authHint). */
function readProviderCatalog(baseUrl: string): CatalogProviderFact[] {
  const cached = readProviderCache();
  if (cached && Date.now() - cached.fetchedAt < PROVIDER_CACHE_TTL_MS) return cached.rows;
  if (!existsSync(VENDOR_CLI)) return cached?.rows ?? [];
  try {
    const res = spawnSync(
      VENDOR_CLI,
      ["--base-url", new URL(baseUrl).origin, "providers", "available", "--json"],
      { encoding: "utf8", timeout: 8_000, env: process.env },
    );
    const parsed = parseJsonLoose(String(res.stdout ?? ""));
    const list = Array.isArray(parsed)
      ? parsed
      : Array.isArray((parsed as { providers?: unknown[] })?.providers)
        ? ((parsed as { providers: unknown[] }).providers as unknown[])
        : [];
    const rows: CatalogProviderFact[] = list
      .filter((item): item is Record<string, unknown> => !!item && typeof item === "object")
      .map((item) => ({
        id: String(item.id ?? ""),
        name: String(item.name ?? item.id ?? ""),
        website: String(item.website ?? ""),
        category: String(item.category ?? ""),
        hasFree: item.hasFree === true,
        freeNote: String(item.freeNote ?? ""),
        authHint: String(item.authHint ?? ""),
      }))
      .filter((row) => !!row.id);
    if (rows.length > 0) writeProviderCache(rows);
    return rows.length > 0 ? rows : cached?.rows ?? [];
  } catch {
    return cached?.rows ?? [];
  }
}

function readAccountsDoc(dir: string): { accounts: AccountFact[]; oauth: string[] } {
  try {
    const doc = JSON.parse(readFileSync(join(dir, "accounts.json"), "utf8")) as {
      accounts?: AccountFact[];
      oauth?: unknown[];
    };
    return {
      accounts: Array.isArray(doc.accounts) ? doc.accounts : [],
      oauth: Array.isArray(doc.oauth)
        ? doc.oauth.map((id) => String(id ?? "").trim()).filter(Boolean)
        : [],
    };
  } catch {
    return { accounts: [], oauth: [] };
  }
}

function readSignupProgress(dir: string): Record<string, { status?: string } | undefined> {
  try {
    const doc = JSON.parse(readFileSync(join(dir, "signup-progress.json"), "utf8")) as {
      providers?: Record<string, { status?: string } | undefined>;
    };
    return doc.providers && typeof doc.providers === "object" ? doc.providers : {};
  } catch {
    return {};
  }
}

/** Env var names present in the env file or the process environment. */
function readEnvNames(dir: string): Set<string> {
  const names = new Set<string>(Object.keys(process.env));
  try {
    const path = join(dir, "env");
    if (existsSync(path)) {
      for (const line of readFileSync(path, "utf8").split(/\r?\n/)) {
        const m = line.match(/^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=/);
        if (m) names.add(m[1]);
      }
    }
  } catch {
    // cosmetic
  }
  return names;
}

/** Upsert KEY=VALUE in the officina env file (atomic, 0600). */
export function writeEnvKey(dir: string, name: string, value: string): void {
  const path = join(dir, "env");
  const old = existsSync(path) ? readFileSync(path, "utf8") : "";
  const lines = old.split(/\r?\n/);
  const re = new RegExp(`^\\s*${name}\\s*=`);
  let found = false;
  const next = lines.map((line) => {
    if (re.test(line)) {
      found = true;
      return `${name}=${value}`;
    }
    return line;
  });
  if (!found) {
    if (next.length > 0 && next[next.length - 1] === "") next.splice(next.length - 1, 0, `${name}=${value}`);
    else next.push(`${name}=${value}`);
  }
  mkdirSync(dir, { recursive: true });
  const tmp = `${path}.tmp-${process.pid}`;
  writeFileSync(tmp, next.join("\n"), { mode: 0o600 });
  renameSync(tmp, path);
}

/** Flip accounts.json enabled=true for a provider (atomic). */
export function enableAccount(dir: string, provider: string): boolean {
  const path = join(dir, "accounts.json");
  try {
    const doc = JSON.parse(readFileSync(path, "utf8")) as { accounts?: Array<Record<string, unknown>> };
    const list = Array.isArray(doc.accounts) ? doc.accounts : [];
    const entry = list.find(
      (a) => String(a?.provider ?? "").toLowerCase() === provider.toLowerCase(),
    );
    if (!entry) return false;
    entry.enabled = true;
    const tmp = `${path}.tmp-${process.pid}`;
    writeFileSync(tmp, `${JSON.stringify(doc, null, 2)}\n`, { mode: 0o600 });
    renameSync(tmp, path);
    return true;
  } catch {
    return false;
  }
}

/** Push the accounts file to the gateway (same CLI the operator would run). */
function runAccountsSync(): string {
  try {
    const script = fileURLToPath(new URL("../../../scripts/caduceus.mjs", import.meta.url));
    const res = spawnSync(process.execPath, [script, "accounts", "sync", "--json"], {
      encoding: "utf8",
      timeout: 60_000,
      env: process.env,
    });
    if (res.error) return `sync failed (${res.error.message})`;
    if (res.status !== 0) {
      const tail = String(res.stderr ?? "").trim().split("\n").slice(-1)[0] ?? "";
      return `sync exited ${res.status ?? "?"}${tail ? `: ${tail}` : ""}`;
    }
    return "synced";
  } catch (error) {
    return `sync failed (${error instanceof Error ? error.message : String(error)})`;
  }
}

/** OAuth registry from the pure vendor-independent lib that the CLI uses. */
async function loadOAuthProviders(): Promise<OAuthProviderFact[]> {
  try {
    const url = new URL("../../../scripts/caduceus-oauth-lib.mjs", import.meta.url);
    const mod = (await import(url.href)) as { OAUTH_PROVIDERS?: OAuthProviderFact[] };
    return Array.isArray(mod.OAUTH_PROVIDERS) ? mod.OAUTH_PROVIDERS : [];
  } catch {
    return [];
  }
}

type RawCatalog = { fetchedAt: string; baseUrl: string; data: unknown[] };

function envKillSwitch(): boolean {
  return /^(0|false|off|no)$/i.test(String(process.env.OFFICINA_CADUCEUS ?? "").trim());
}

function resolveBaseUrl(): string {
  return (process.env.OFFICINA_CADUCEUS_BASE_URL || DEFAULT_BASE_URL).replace(/\/+$/, "");
}

function readCache(baseUrl: string): RawCatalog | undefined {
  if (!existsSync(CACHE_FILE)) return undefined;
  try {
    const parsed = JSON.parse(readFileSync(CACHE_FILE, "utf8")) as Partial<RawCatalog>;
    if (parsed.baseUrl !== baseUrl || !Array.isArray(parsed.data)) return undefined;
    return {
      fetchedAt: typeof parsed.fetchedAt === "string" ? parsed.fetchedAt : "",
      baseUrl,
      data: parsed.data,
    };
  } catch {
    return undefined;
  }
}

function writeCache(baseUrl: string, data: unknown[]): void {
  try {
    mkdirSync(dirname(CACHE_FILE), { recursive: true });
    const tmp = `${CACHE_FILE}.tmp-${process.pid}`;
    const payload: RawCatalog = { fetchedAt: new Date().toISOString(), baseUrl, data };
    writeFileSync(tmp, `${JSON.stringify(payload, null, 2)}\n`, { mode: 0o600 });
    renameSync(tmp, CACHE_FILE);
  } catch {
    // Cache is an optimization; never block the workshop on it.
  }
}

async function fetchActiveProviders(baseUrl: string): Promise<Set<string> | null> {
  if (!existsSync(VENDOR_API_MODULE)) return null;
  try {
    const mod = await import(pathToFileURL(VENDOR_API_MODULE).href);
    const res = await mod.apiFetch("/api/providers?limit=5000", {
      baseUrl: new URL(baseUrl).origin,
      timeout: 5_000,
    });
    if (!res.ok) return null;
    const body = await res.json();
    const list = Array.isArray(body) ? body : body?.providers ?? body?.connections ?? [];
    if (!Array.isArray(list)) return null;
    return new Set(
      list
        .filter((connection) => connection?.isActive !== false)
        .map((connection) => String(connection?.provider ?? "").trim().toLowerCase())
        .filter(Boolean),
    );
  } catch {
    return null;
  }
}

/** Connection health buckets for the state file; null = gateway unreachable. */
async function fetchConnectionBuckets(
  baseUrl: string,
): Promise<{ active: string[]; backoff: string[]; hard_down: string[] } | null> {
  if (!existsSync(VENDOR_API_MODULE)) return null;
  try {
    const mod = await import(pathToFileURL(VENDOR_API_MODULE).href);
    const res = await mod.apiFetch("/api/providers?limit=5000", {
      baseUrl: new URL(baseUrl).origin,
      timeout: 5_000,
    });
    if (!res.ok) return null;
    const body = await res.json();
    const list = Array.isArray(body) ? body : body?.providers ?? body?.connections ?? [];
    if (!Array.isArray(list)) return null;
    const h = classifyConnections(list as GatewayConnection[]);
    return {
      active: [...h.active].sort(),
      backoff: [...h.backoff].sort(),
      hard_down: [...h.hardDown].sort(),
    };
  } catch {
    return null;
  }
}

function mask(value: string): string {
  return value.length <= 4 ? "***" : `...${value.slice(-4)}`;
}

/** Models cooling down right now (provider-health.json cooldown marks). */
function coolingCount(officinaDir: string): number {
  try {
    const health = JSON.parse(readFileSync(join(officinaDir, "provider-health.json"), "utf8")) as {
      models?: Record<string, { cooldownUntil?: number }>;
    };
    const now = Date.now();
    return Object.values(health.models ?? {}).filter(
      (r) => Number(r?.cooldownUntil) > now,
    ).length;
  } catch {
    return 0;
  }
}

async function fetchCatalog(
  baseUrl: string,
  apiKey: string,
  knownActiveProviders?: Set<string> | null,
): Promise<RawCatalog> {
  const [res, activeProviders] = await Promise.all([
    fetch(`${baseUrl}/models?prefix=alias`, {
      headers: apiKey ? { authorization: `Bearer ${apiKey}` } : undefined,
      signal: AbortSignal.timeout(10_000),
    }),
    knownActiveProviders === undefined ? fetchActiveProviders(baseUrl) : knownActiveProviders,
  ]);
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  const body = await res.json();
  const data = filterCatalogByProviders(extractCatalogModels(body), activeProviders);
  if (data.length === 0) throw new Error("empty catalog");
  return { fetchedAt: new Date().toISOString(), baseUrl, data };
}

function providerConfig(
  baseUrl: string,
  apiKey: string,
  models: PiModelEntry[],
  refresh: ProviderConfig["refreshModels"],
): ProviderConfig {
  return {
    name: PROVIDER_NAME,
    baseUrl,
    apiKey,
    api: "openai-completions",
    headers: { ...PASSTHROUGH_HEADERS },
    models,
    refreshModels: refresh,
  };
}

export default async function (pi: ExtensionAPI) {
  if (envKillSwitch()) return;

  const baseUrl = resolveBaseUrl();
  const apiKey = String(process.env.OFFICINA_CADUCEUS_API_KEY ?? "").trim();

  const officinaDir = join(homedir(), ".vitriol", "officina");
  const readKeyRows = () => {
    const env = new Map<string, string>();
    try {
      const path = join(officinaDir, "env");
      if (existsSync(path)) {
        for (const line of readFileSync(path, "utf8").split(/\r?\n/)) {
          const m = line.match(/^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$/);
          if (m) env.set(m[1], m[2]);
        }
      }
    } catch {
      // cosmetic
    }
    const rows: Array<{ provider: string; env: string; value: string; source: string }> = [];
    try {
      const accounts = JSON.parse(readFileSync(join(officinaDir, "accounts.json"), "utf8"));
      for (const account of Array.isArray(accounts?.accounts) ? accounts.accounts : []) {
        const key = String(account?.credential_env ?? "").trim();
        const fileValue = key ? env.get(key) ?? "" : "";
        const shellValue = key ? process.env[key] ?? "" : "";
        const value = shellValue || fileValue;
        if (!value) continue;
        rows.push({
          provider: String(account.provider ?? "?"),
          env: key,
          value: mask(value),
          source: shellValue ? "shell" : "env-file",
        });
      }
    } catch {
      // no accounts file
    }
    return rows;
  };

  // State file for the Rust TUI's /caduceus page (facts only, I4).
  // Written at startup, on every session switch, and every 60 s; failures
  // are silent (the page shows its own staleness instead). The provider
  // catalog spawn is skipped on the startup write so session start never
  // waits on the vendor CLI (it lands on the first tick / session switch).
  const oauthProviders = await loadOAuthProviders();
  const writeState = async (allowCatalogFetch = true): Promise<void> => {
    try {
      const buckets = await fetchConnectionBuckets(baseUrl);
      const cache = readCache(baseUrl);
      const budget = budgetStatus();
      const prefs = readPrefs();
      const accountsDoc = readAccountsDoc(officinaDir);
      const oauthRows: OAuthProviderFact[] =
        oauthProviders.length > 0
          ? oauthProviders
          : accountsDoc.oauth.map((id) => ({ id, name: id, flow: "oauth" }));

      const freeCounts = new Map<string, number>();
      const candidates: CandidateRow[] = [];
      if (cache) {
        for (const c of toolCandidates(extractCatalogModels(cache.data), {
          freeOnly: prefs?.free_only !== false,
          allowPaidOwners: allowPaidOwners(prefs),
        })) {
          if (c.free) freeCounts.set(c.owner, (freeCounts.get(c.owner) ?? 0) + 1);
          candidates.push({
            id: c.id,
            owner: c.owner,
            context_window: c.contextWindow,
            max_tokens: c.maxTokens,
            free: c.free,
          });
        }
      }
      const providers: ProviderRow[] = buildProviderRows({
        catalog: allowCatalogFetch ? readProviderCatalog(baseUrl) : readProviderCache()?.rows ?? [],
        accounts: accountsDoc.accounts,
        oauth: oauthRows,
        envNames: readEnvNames(officinaDir),
        buckets,
        freeCounts,
        signup: readSignupProgress(officinaDir),
      });

      writeCaduceusState(
        STATE_FILE,
        buildCaduceusState({
          base_url: baseUrl,
          gateway_up: buckets !== null,
          catalog_count: cache?.data.length ?? 0,
          catalog_fetched_at: cache?.fetchedAt ?? "",
          connections: buckets ?? { active: [], backoff: [], hard_down: [] },
          cooling: coolingCount(officinaDir),
          inference_key: apiKey ? mask(apiKey) : "",
          keys: readKeyRows(),
          budget: {
            daily_tokens: budget.dailyTokens,
            daily_cap: budget.dailyCap,
            monthly_tokens: budget.monthlyTokens,
            monthly_cap: budget.monthlyCap,
            allowed: budget.allowed,
            paid_allowed: budget.paidAllowed,
          },
          providers,
          candidates,
        }),
      );
    } catch {
      // decoration contract: never break the workshop
    }
  };
  void writeState(false);
  const stateTimer = setInterval(() => void writeState(true), 60_000);
  stateTimer.unref?.();
  pi.on("session_start", () => void writeState(true));

  if (!apiKey) {
    console.warn(
      "[caduceus] OFFICINA_CADUCEUS_API_KEY missing; provider not registered. Run: vitriol caduceus setup",
    );
    return;
  }

  const activeProviders = await fetchActiveProviders(baseUrl);
  const cached = readCache(baseUrl);
  let models = cached
    ? mapCatalogModels(filterCatalogByProviders(cached.data, activeProviders))
    : [];
  let needsNetwork = models.length === 0;

  if (needsNetwork) {
    try {
      const fresh = await fetchCatalog(baseUrl, apiKey, activeProviders);
      writeCache(baseUrl, fresh.data);
      models = mapCatalogModels(fresh.data);
    } catch (error) {
      console.warn(
        `[caduceus] catalog unavailable at ${baseUrl}: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  }

  if (models.length === 0) {
    console.warn("[caduceus] no models available; local engine remains the default");
    return;
  }

  const refreshModels: ProviderConfig["refreshModels"] = async (context) => {
    if (!context.allowNetwork || context.signal?.aborted) return models;
    const fresh = await fetchCatalog(baseUrl, apiKey);
    const mapped = mapCatalogModels(fresh.data);
    if (mapped.length === 0) return models;
    models = mapped;
    writeCache(baseUrl, fresh.data);
    return mapped;
  };

  pi.registerProvider(PROVIDER_ID, providerConfig(baseUrl, apiKey, models, refreshModels));

  if (!needsNetwork) {
    void fetchCatalog(baseUrl, apiKey)
      .then((fresh) => {
        const mapped = mapCatalogModels(fresh.data);
        if (mapped.length === 0) return;
        models = mapped;
        writeCache(baseUrl, fresh.data);
        pi.registerProvider(PROVIDER_ID, providerConfig(baseUrl, apiKey, models, refreshModels));
      })
      .catch(() => {
        // Keep the cached catalog.
      });
  }

  pi.registerCommand("keys", {
    description: "Show configured provider keys (masked)",
    handler: async (_args: string, ctx: { ui: any }) => {
      const rows = readKeyRows();
      const inference = String(process.env.OFFICINA_CADUCEUS_API_KEY ?? "").trim();
      const lines = [
        `Caduceus inference: ${inference ? `set (${mask(inference)})` : "missing"}`,
        ...rows.map((r) => `${r.provider} · ${r.env} · ${r.value} · ${r.source}`),
      ];
      await ctx.ui.notify?.(`keys (${rows.length}):\n${lines.join("\n")}`, "info");
    },
  });

  pi.registerCommand("providers", {
    description: "Show provider health and cooling lanes",
    handler: async (_args: string, ctx: { ui: any }) => {
      const rows = readKeyRows();
      const cooling = coolingCount(officinaDir);
      await ctx.ui.notify?.(
        `providers: ${rows.length} keyed · ${cooling} cooling — ${rows.map((r) => r.provider).join(", ")}`,
        "info",
      );
    },
  });

  // Quick key add (2026-11-12, panel stage 4): invoked by the TUI's `a` key
  // as `/caduceus-add <provider>`. Writes the env key, enables the account,
  // pushes it to the gateway, and refreshes the state file. The key is
  // visible while typing (pi has no password mode) — the wizard
  // (`vitriol caduceus signup --only <id>`) is the hidden alternative.
  pi.registerCommand("caduceus-add", {
    description: "Add a provider API key (env + accounts + sync)",
    handler: async (args: string, ctx: { ui: any }) => {
      const provider = String(args ?? "").trim().toLowerCase();
      if (!provider) {
        await ctx.ui.notify?.("usage: /caduceus-add <provider>", "warning");
        return;
      }
      const account = readAccountsDoc(officinaDir).accounts.find(
        (a) => String(a?.provider ?? "").toLowerCase() === provider,
      );
      const envName = String(account?.credential_env ?? "").trim();
      if (!account || !envName) {
        await ctx.ui.notify?.(
          `caduceus-add: unknown provider '${provider}' (no accounts.json entry with credential_env)`,
          "error",
        );
        return;
      }
      const typed = await ctx.ui.input?.(
        `API key for ${provider} (${envName}) — visible while typing`,
        "paste key",
      );
      const key = String(typed ?? "").trim();
      if (!key) {
        await ctx.ui.notify?.(`caduceus-add ${provider}: cancelled`, "info");
        return;
      }
      writeEnvKey(officinaDir, envName, key);
      process.env[envName] = key;
      const enabled = enableAccount(officinaDir, provider);
      const sync = runAccountsSync();
      await writeState(true);
      await ctx.ui.notify?.(
        `caduceus-add ${provider}: key saved (${envName}) · ${
          enabled ? "account enabled" : "account row missing"
        } · ${sync}`,
        enabled && sync === "synced" ? "info" : "warning",
      );
    },
  });

}
