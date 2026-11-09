// Pure OAuth metadata/parsing for Officina's OmniRoute wrapper. The server-side
// provider registry is the source of truth; this mirrors the stable ids/flows
// needed to drive browser and device grants from the CLI.

export const OAUTH_PROVIDERS = [
  { id: "gemini", name: "Google Gemini", flow: "browser", native: true, backend: "gemini" },
  { id: "antigravity", name: "Antigravity", flow: "browser", native: true, backend: "antigravity" },
  { id: "cursor", name: "Cursor", flow: "import", native: true, backend: "cursor" },
  { id: "zed", name: "Zed", flow: "import", native: true, backend: "zed" },
  { id: "kiro", name: "Amazon Kiro", flow: "social", native: true, backend: "kiro" },
  { id: "claude-code", name: "Claude Code (OAuth)", flow: "browser", native: true, backend: "claude" },
  { id: "codex", name: "OpenAI Codex (OAuth)", flow: "callback", native: true, backend: "codex" },
  { id: "copilot", name: "GitHub Copilot", flow: "device", native: true, backend: "github" },
  {
    id: "amazon-q",
    name: "Amazon Q",
    flow: "social",
    native: true,
    backend: "kiro",
    note: "alias of kiro",
  },
  {
    id: "openference",
    name: "Openference",
    flow: "browser",
    native: false,
    backend: "openference",
    note: "loopback callback; paste URL/code",
  },
  {
    id: "agy",
    name: "Antigravity CLI (agy)",
    flow: "browser",
    native: false,
    backend: "agy",
    note: "Antigravity CLI client identity",
  },
  {
    id: "qoder",
    name: "Qoder",
    flow: "browser",
    native: false,
    backend: "qoder",
    requiresEnv: [
      "QODER_OAUTH_AUTHORIZE_URL",
      "QODER_OAUTH_TOKEN_URL",
      "QODER_OAUTH_USERINFO_URL",
      "QODER_OAUTH_CLIENT_ID",
      "QODER_OAUTH_CLIENT_SECRET",
    ],
    note: "disabled unless QODER_OAUTH_* is configured",
  },
  {
    id: "freebuff",
    name: "Freebuff",
    flow: "device",
    native: false,
    backend: "freebuff",
    note: "device code + browser poll",
  },
];

export function resolveBackendKey(id) {
  return OAUTH_PROVIDERS.find((provider) => provider.id === id)?.backend ?? id;
}

export function findOAuthProvider(id) {
  return OAUTH_PROVIDERS.find((provider) => provider.id === id);
}

export function parseOAuthCallback(input, expectedState) {
  const text = String(input ?? "").trim();
  if (!text) throw new Error("empty authorization callback");

  let code = "";
  let state = "";
  try {
    const url = new URL(text);
    const error = url.searchParams.get("error");
    if (error) throw new Error(`authorization failed: ${error}`);
    code = url.searchParams.get("code") ?? "";
    state = url.searchParams.get("state") ?? url.hash.replace(/^#/, "");
  } catch (error) {
    if (error instanceof Error && error.message.startsWith("authorization failed:")) throw error;
    const [rawCode, rawState] = text.split("#", 2);
    code = rawCode ?? "";
    state = rawState ?? "";
  }

  if (!code) throw new Error("no authorization code found");
  if (expectedState && state && state !== expectedState) {
    throw new Error("OAuth state mismatch");
  }

  return { code, state: state || expectedState || "" };
}
