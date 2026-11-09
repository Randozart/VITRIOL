// Pure OmniRoute catalog -> pi model mapping. Kept separate from the extension
// wiring so the field heuristics stay unit-testable without a pi runtime.

export interface PiModelEntry {
  id: string;
  name: string;
  reasoning: boolean;
  input: ("text" | "image")[];
  contextWindow: number;
  maxTokens: number;
  cost: { input: number; output: number; cacheRead: number; cacheWrite: number };
}

type JsonRecord = Record<string, unknown>;

const DEFAULT_CONTEXT = 200_000;
const DEFAULT_MAX_TOKENS = 16_384;
const MIN_CONTEXT = 4_096;
function asRecord(value: unknown): JsonRecord {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as JsonRecord) : {};
}

function asNumber(value: unknown): number | undefined {
  if (typeof value === "number" && Number.isFinite(value) && value > 0) return value;
  if (typeof value === "string" && value.trim()) {
    const parsed = Number(value);
    if (Number.isFinite(parsed) && parsed > 0) return parsed;
  }
  return undefined;
}

function asString(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function asStringArray(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === "string" && item.trim().length > 0)
    : [];
}

function reasoningFlag(record: JsonRecord): boolean {
  const direct = record.reasoning;
  if (typeof direct === "boolean") return direct;

  const capabilities = asRecord(record.capabilities);
  const capability = capabilities.reasoning ?? record.reasoning;
  if (typeof capability === "boolean") return capability;
  if (capability && typeof capability === "object") {
    const flag = asRecord(capability);
    if (typeof flag.enabled === "boolean") return flag.enabled;
    const efforts = asStringArray(flag.effort ?? flag.efforts ?? flag.supportedThinkingEfforts);
    return efforts.length > 0 || Object.keys(flag).length > 0;
  }
  return false;
}

function visionFlag(record: JsonRecord): boolean {
  const capabilities = asRecord(record.capabilities);
  if (capabilities.vision === true || capabilities.attachment === true) return true;

  const modalities = asRecord(record.modalities);
  const architecture = asRecord(record.architecture);
  const inputs = [
    ...asStringArray(modalities.input),
    ...asStringArray(record.input_modalities),
    ...asStringArray(architecture.input_modalities),
  ];
  return inputs.some((item) => /image|vision/i.test(item));
}

function costOf(record: JsonRecord): PiModelEntry["cost"] {
  const pricing = asRecord(record.pricing);
  const cost = asRecord(record.cost);
  const source = Object.keys(pricing).length > 0 ? pricing : cost;
  return {
    input: asNumber(source.input) ?? asNumber(source.prompt) ?? 0,
    output: asNumber(source.output) ?? asNumber(source.completion) ?? 0,
    cacheRead: asNumber(source.cacheRead) ?? asNumber(source.cache_read) ?? 0,
    cacheWrite: asNumber(source.cacheWrite) ?? asNumber(source.cache_write) ?? 0,
  };
}

function isChatModel(record: JsonRecord): boolean {
  const type = asString(record.type);
  if (type && type !== "chat" && type !== "chat-completions") return false;

  const endpoints = asStringArray(record.supported_endpoints);
  if (endpoints.length === 0) return true;
  return endpoints.some((endpoint) => /chat|responses/i.test(endpoint));
}

export function extractCatalogModels(body: unknown): unknown[] {
  if (Array.isArray(body)) return body;
  const record = asRecord(body);
  for (const candidate of [record.data, record.models, asRecord(record.data).data]) {
    if (Array.isArray(candidate)) return candidate;
  }
  return [];
}

export function mapCatalogModels(rawModels: unknown[]): PiModelEntry[] {
  const byId = new Map<string, PiModelEntry>();

  for (const raw of rawModels) {
    const record = asRecord(raw);
    const id = asString(record.id);
    if (!id || !isChatModel(record)) continue;

    const contextWindow = Math.max(
      MIN_CONTEXT,
      asNumber(record.context_length) ??
        asNumber(record.max_context_window_tokens) ??
        asNumber(record.contextWindow) ??
        DEFAULT_CONTEXT,
    );
    const maxTokens = Math.min(
      contextWindow,
      asNumber(record.max_output_tokens) ??
        asNumber(record.maxOutputTokens) ??
        asNumber(record.maxTokens) ??
        DEFAULT_MAX_TOKENS,
    );
    const displayName =
      asString(record.display_name) ?? asString(record.name) ?? id.split("/").pop() ?? id;

    if (byId.has(id)) continue;
    byId.set(id, {
      id,
      name: displayName,
      reasoning: reasoningFlag(record),
      input: visionFlag(record) ? ["text", "image"] : ["text"],
      contextWindow,
      maxTokens,
      cost: costOf(record),
    });
  }

  return [...byId.values()].sort((a, b) => a.id.localeCompare(b.id));
}


const ALWAYS_KEEP_PROVIDERS = new Set(["auto"]);

export function filterCatalogByProviders(
  rawModels: unknown[],
  activeProviders: Set<string> | null | undefined,
): unknown[] {
  if (!activeProviders) return rawModels;
  if (activeProviders.size === 0) return [];
  return rawModels.filter((raw) => {
    const record = asRecord(raw);
    const owner = asString(record.owned_by);
    if (!owner) return true;
    const normalized = owner.toLowerCase();
    return activeProviders.has(normalized) || ALWAYS_KEEP_PROVIDERS.has(normalized);
  });
}
