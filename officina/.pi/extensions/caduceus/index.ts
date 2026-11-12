// Caduceus provider for Officina. Registers a dynamic OpenAI-compatible
// provider from the gateway's model catalog (OmniRoute vendor, MIT). Strict passthrough is
// enforced per request with headers, so a global cache/compression toggle
// drifting on the gateway cannot silently rewrite prompts.

import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
import type { ExtensionAPI, ProviderConfig } from "@earendil-works/pi-coding-agent";
import { budgetStatus } from "../_shared/budget.ts";
import { classifyConnections, type GatewayConnection } from "../_shared/lane-health.ts";
import {
  extractCatalogModels,
  filterCatalogByProviders,
  mapCatalogModels,
  type PiModelEntry,
} from "./catalog.ts";
import { buildCaduceusState, writeCaduceusState } from "./state.ts";

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

  // State file for the Rust TUI's /caduceus modal (read-only view, I4).
  // Written at startup, on every session switch, and every 60 s; failures
  // are silent (the modal shows its own staleness instead).
  const writeState = async (): Promise<void> => {
    try {
      const buckets = await fetchConnectionBuckets(baseUrl);
      const cache = readCache(baseUrl);
      const budget = budgetStatus();
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
        }),
      );
    } catch {
      // decoration contract: never break the workshop
    }
  };
  void writeState();
  const stateTimer = setInterval(() => void writeState(), 60_000);
  stateTimer.unref?.();
  pi.on("session_start", () => void writeState());

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

}
