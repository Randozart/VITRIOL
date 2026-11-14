// Pure Caduceus catalog -> pi model mapping. Kept separate from the extension
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

// ── Free/tool-capable discovery (2026-11-10) ──────────────────────────────
//
// Caduceus aggregates several free/zero-cost providers. The overflow router
// needs candidates that can actually drive an agent loop: chat + tool calling.
// Owner priority reflects observed viability:
//   opencode-go       — subscription lane, tool calls work, no per-token bill
//   dahl              — free token allowance, tool-capable
//   freeinference     — free research key; manual models carry no price block
//   agentrouter/agnes/api-airforce/aion/cohere/bytez — explicit zero-priced lanes
//   glm/zai           — free FLASH tier when quota is available
//   cloudflare-playground — free, but requires a Playwright runtime
// duckduckgo-web / aihorde have zero-cost models but advertise no tool
// calling, so they are excluded from agent routing.

export const FREE_OWNER_RANK: Record<string, number> = {
  "opencode-go": 0,
  dahl: 1,
  freeinference: 2,
  agentrouter: 3,
  agnes: 4,
  "api-airforce": 5,
  aion: 6,
  cohere: 7,
  bytez: 8,
  friendliai: 9,
  glm: 10,
  zai: 11,
  "cloudflare-playground": 12,
  freetheai: 13,
  "freemodel-dev": 14,
};

/** Owners whose free grant has no per-token price in the gateway catalog.
 *  Manual FreeInference models are added without a pricing block; they are
 *  still free research lanes, never auto-billed. */
/** Owners whose "zero price" comes from a PAID plan the operator pays for.
 *  They stay usable but rank AFTER every free-tier lane — when a free lane
 *  is exhausted (429/quota/cooling) the router walks down to them
 *  (2026-11-13, operator: "opencode go is paid, I have a subscription"). */
export const PAID_SUBSCRIPTION_OWNERS = new Set(["opencode-go"]);

const COSTLESS_OWNERS = new Set([
  "opencode-go",
  "dahl",
  "cloudflare-playground",
  "freeinference",
]);

export interface FreeCandidate {
  id: string;
  owner: string;
  contextWindow: number;
  maxTokens: number;
  /** Explicit zero pricing, or a known-costless subscription/playground lane. */
  free: boolean;
  /** No paid plan required (free tier). Paid-subscription lanes are false
   *  even though their marginal token price is zero. */
  freeTier: boolean;
}

function toolCallingFlag(record: JsonRecord): boolean {
  const capabilities = asRecord(record.capabilities);
  if (capabilities.tool_calling === true || capabilities.toolCalling === true) return true;
  const tools = asRecord(capabilities.tools);
  return tools.enabled === true;
}

function ownerOf(record: JsonRecord, id: string): string {
  return (asString(record.owned_by) ?? id.split("/")[0] ?? "").toLowerCase();
}

function pricingKnown(record: JsonRecord): boolean {
  const raw = record.pricing ?? record.cost;
  return !!raw && typeof raw === "object" && Object.keys(asRecord(raw)).length > 0;
}

export interface ToolCandidateOptions {
  /** Only admit verified-free models (default true, today's behaviour). */
  freeOnly?: boolean;
  /** Owners whose paid models may pass while freeOnly is on (caduceus-prefs
   *  `allow_paid`). Owners outside FREE_OWNER_RANK are rank 99. */
  allowPaidOwners?: ReadonlySet<string>;
}

/** All tool-capable chat models in preferred routing order.
 *  freeOnly=true reproduces the historical free lane; allowPaidOwners widens
 *  it to explicitly opted-in owners without changing the default. */
export function toolCandidates(
  rawModels: unknown[],
  opts: ToolCandidateOptions = {},
): FreeCandidate[] {
  const freeOnly = opts.freeOnly ?? true;
  const allowPaid = opts.allowPaidOwners ?? new Set<string>();
  const out: FreeCandidate[] = [];
  const seen = new Set<string>();
  for (const raw of rawModels) {
    const record = asRecord(raw);
    const id = asString(record.id);
    if (!id || seen.has(id) || !isChatModel(record)) continue;
    if (!toolCallingFlag(record)) continue;

    const owner = ownerOf(record, id);
    const rank = FREE_OWNER_RANK[owner];
    const paidAllowed = allowPaid.has(owner);
    // freeOnly admits ranked owners plus explicit allow_paid owners; with
    // freeOnly off every tool-capable chat model is fair game.
    if (freeOnly && rank === undefined && !paidAllowed) continue;

    const pricing = pricingKnown(record);
    const cost = costOf(record);
    const free = pricing
      ? cost.input === 0 && cost.output === 0
      : COSTLESS_OWNERS.has(owner);
    const freeTier = free && !PAID_SUBSCRIPTION_OWNERS.has(owner);
    if (freeOnly && !free && !paidAllowed) continue;

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

    seen.add(id);
    out.push({ id, owner, contextWindow, maxTokens, free, freeTier });
  }

  // Drop effort-tier duplicates (`foo-high`, `foo-low`, …) when the base
  // `foo` is also present; the agent only needs one lane per family.
  const ids = new Set(out.map((c) => c.id));
  const effort = /-(high|low|max|medium|minimal|none|xhigh)$/i;
  const deduped = out.filter((c) => {
    const m = effort.exec(c.id);
    if (!m) return true;
    return !ids.has(c.id.slice(0, -m[0].length));
  });

  // Model-family preference: DeepSeek V4.1 Flash first, then other native
  // DeepSeek Flash lanes, then MiMo Flash, then GLM Flash, then everything.
  // Owner health/priority sorts first so a broken free owner does not outrank
  // a working one.
  const familyRank = (id: string): number => {
    const s = id.toLowerCase();
    if (/deepseek.*v4\.1.*flash/.test(s)) return 0;
    if (/deepseek.*flash/.test(s)) return 1;
    if (/mimo.*flash/.test(s)) return 2;
    if (/glm.*flash/.test(s)) return 3;
    if (/flash/.test(s)) return 4;
    return 5;
  };
  deduped.sort((a, b) => {
    // Free-tier lanes always rank before paid-subscription lanes; the
    // router walks down the list as lanes exhaust (cooling/429).
    const tier = (a.freeTier ? 0 : 1) - (b.freeTier ? 0 : 1);
    if (tier !== 0) return tier;
    const owner = (FREE_OWNER_RANK[a.owner] ?? 99) - (FREE_OWNER_RANK[b.owner] ?? 99);
    if (owner !== 0) return owner;
    const family = familyRank(a.id) - familyRank(b.id);
    if (family !== 0) return family;
    if (a.contextWindow !== b.contextWindow) return b.contextWindow - a.contextWindow;
    return a.id.localeCompare(b.id);
  });
  return deduped;
}

/** Historical entry point: verified-free tool lanes only (I1 parity). */
export function freeToolCandidates(rawModels: unknown[]): FreeCandidate[] {
  return toolCandidates(rawModels, { freeOnly: true });
}
