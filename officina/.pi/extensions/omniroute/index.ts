// OmniRoute provider for Officina. Registers a dynamic OpenAI-compatible
// provider from OmniRoute's canonical model catalog. Strict passthrough is
// enforced per request with headers, so a global cache/compression toggle
// drifting on the gateway cannot silently rewrite prompts.

import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
import type { ExtensionAPI, ProviderConfig } from "@earendil-works/pi-coding-agent";
import {
  extractCatalogModels,
  filterCatalogByProviders,
  mapCatalogModels,
  type PiModelEntry,
} from "./catalog.ts";

const PROVIDER_ID = "omniroute";
const PROVIDER_NAME = "OmniRoute";
const DEFAULT_BASE_URL = "http://127.0.0.1:20128/v1";
const CACHE_FILE = join(homedir(), ".vitriol", "officina", "omniroute-models.json");
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
  return /^(0|false|off|no)$/i.test(String(process.env.OFFICINA_OMNIROUTE ?? "").trim());
}

function resolveBaseUrl(): string {
  return (process.env.OFFICINA_OMNIROUTE_BASE_URL || DEFAULT_BASE_URL).replace(/\/+$/, "");
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
  const apiKey = String(process.env.OFFICINA_OMNIROUTE_API_KEY ?? "").trim();
  if (!apiKey) {
    console.warn(
      "[omniroute] OFFICINA_OMNIROUTE_API_KEY missing; provider not registered. Run: vitriol omniroute setup",
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
        `[omniroute] catalog unavailable at ${baseUrl}: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  }

  if (models.length === 0) {
    console.warn("[omniroute] no models available; local engine remains the default");
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
}
