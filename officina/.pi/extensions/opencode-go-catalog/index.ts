// OpenCode Go catalog completion (2026-11-15, operator request).
//
// pi ships OpenCode Go models built in, but its generated catalog can lag the
// live endpoint by a few ids. This extension re-registers the same provider
// with the missing chat models appended. Authentication and stream handlers
// stay untouched (the built-in provider object is reused); the TUI injects
// OPENCODE_API_KEY from the operator's OpenCode auth store at spawn.
//
// Kill switch: OFFICINA_NO_OPENCODE_GO_CATALOG=1.
import { opencodeGoProvider } from "@earendil-works/pi-ai/providers/opencode-go";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const BASE_URL = "https://opencode.ai/inference/go/openai/v1";
const COMPAT = {
  supportsStore: false,
  supportsDeveloperRole: false,
  supportsStrictMode: true,
  maxTokensField: "max_tokens",
};

const MISSING_MODELS = [
  {
    id: "hy4",
    name: "Hy4",
    api: "openai-completions",
    provider: "opencode-go",
    baseUrl: BASE_URL,
    reasoning: true,
    input: ["text", "image"],
    cost: { input: 0.834, output: 2.501, cacheRead: 0.042, cacheWrite: 0 },
    contextWindow: 1048576,
    maxTokens: 65536,
    compat: COMPAT,
    thinkingLevelMap: { off: "none", high: "high" },
    inputLimits: {
      images: { resize: { maxWidth: 2000, maxHeight: 2000, maxBytes: 4718592, jpegQuality: 80 } },
    },
    type: "chat",
  },
  {
    id: "longcat-2.5-free",
    name: "LongCat 2.5 Free",
    api: "openai-completions",
    provider: "opencode-go",
    baseUrl: BASE_URL,
    reasoning: true,
    input: ["text", "image"],
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    contextWindow: 1000000,
    maxTokens: 262144,
    compat: { ...COMPAT, thinkingFormat: "deepseek" },
    inputLimits: {
      images: { resize: { maxWidth: 2000, maxHeight: 2000, maxBytes: 4718592, jpegQuality: 80 } },
    },
    type: "chat",
  },
] as const;

/** Only the models the built-in catalog does not already expose. */
export function missingModels(existingIds: Iterable<string>) {
  const have = new Set(existingIds);
  return MISSING_MODELS.filter((m) => !have.has(m.id));
}

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_NO_OPENCODE_GO_CATALOG === "1") return;
  try {
    const base = opencodeGoProvider() as unknown as {
      getModels: (...args: unknown[]) => unknown[];
      getAllModels: (...args: unknown[]) => unknown[];
    };
    const getModels = base.getModels.bind(base);
    const getAllModels = base.getAllModels.bind(base);
    const extras = missingModels(
      getModels().map((m) => String((m as { id?: unknown })?.id ?? "")),
    );
    if (extras.length === 0) return;
    base.getModels = (...args: unknown[]) => [...getModels(...args), ...extras];
    base.getAllModels = (...args: unknown[]) => [...getAllModels(...args), ...extras];
    pi.registerProvider(base as never);
  } catch (error) {
    console.warn(
      `[opencode-go-catalog] catalog extension skipped: ${
        error instanceof Error ? error.message : String(error)
      }`,
    );
  }
}
