// Pure account-config helpers for `vitriol omniroute accounts`. Kept separate
// from the CLI entrypoint so validation, redaction, and import-payload building
// are unit-testable without touching the gateway or filesystem.

const ENV_NAME_RE = /^[A-Za-z_][A-Za-z0-9_]*$/;

export function stripAnsi(value) {
  return String(value ?? "").replace(/\x1b\[[0-9;]*m/g, "");
}

export function parseCliJson(value) {
  const text = stripAnsi(value);
  const objectStart = text.indexOf("{");
  const arrayStart = text.indexOf("[");
  const start =
    objectStart < 0 ? arrayStart : arrayStart < 0 ? objectStart : Math.min(objectStart, arrayStart);
  if (start < 0) throw new Error("no JSON object found in command output");
  const close = text[start] === "{" ? "}" : "]";
  const end = text.lastIndexOf(close);
  if (end <= start) throw new Error("unterminated JSON in command output");
  return JSON.parse(text.slice(start, end + 1));
}

export function catalogMap(payload) {
  const list = Array.isArray(payload)
    ? payload
    : Array.isArray(payload?.providers)
      ? payload.providers
      : Array.isArray(payload?.data)
        ? payload.data
        : [];
  const map = new Map();
  for (const item of list) {
    if (!item || typeof item !== "object") continue;
    const id = typeof item.id === "string" ? item.id.trim() : "";
    if (!id) continue;
    map.set(id.toLowerCase(), item);
  }
  return map;
}

export function normalizeAccountsConfig(parsed) {
  const errors = [];
  let accounts = [];
  let oauth = [];

  if (Array.isArray(parsed)) {
    accounts = parsed;
  } else if (parsed && typeof parsed === "object") {
    if (Array.isArray(parsed.accounts)) accounts = parsed.accounts;
    else if (Array.isArray(parsed.providers)) accounts = parsed.providers;
    else errors.push("config must contain an accounts[] array");
    oauth = Array.isArray(parsed.oauth)
      ? parsed.oauth.filter((item) => typeof item === "string" && item.trim()).map((item) => item.trim())
      : [];
  } else {
    errors.push("config must be a JSON object or array");
  }

  return { accounts, oauth, errors };
}

function normalizeOnly(only) {
  if (!only) return null;
  const values = Array.isArray(only) ? only : String(only).split(",");
  const set = new Set(
    values
      .flatMap((value) => String(value).split(","))
      .map((value) => value.trim().toLowerCase())
      .filter(Boolean),
  );
  return set.size > 0 ? set : null;
}

function accountIdentity(provider, name) {
  return `${provider.toLowerCase()}\0${name.toLowerCase()}`;
}

export function buildImportPayload(config, catalog, env = {}, opts = {}) {
  const entries = [];
  const skipped = [];
  const errors = [];
  const seen = new Map();
  const only = normalizeOnly(opts.only);
  const strict = opts.strict === true;

  for (const raw of config.accounts ?? []) {
    if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
      errors.push({ provider: "", name: "", reason: "invalid_entry" });
      continue;
    }

    const provider = typeof raw.provider === "string" ? raw.provider.trim() : "";
    const name = typeof raw.name === "string" && raw.name.trim() ? raw.name.trim() : provider;
    if (!provider) {
      errors.push({ provider: "", name, reason: "provider_required" });
      continue;
    }

    if (only && !only.has(provider.toLowerCase()) && !only.has(name.toLowerCase())) {
      skipped.push({ provider, name, reason: "filtered" });
      continue;
    }

    const identity = accountIdentity(provider, name);
    if (seen.has(identity)) {
      errors.push({ provider, name, reason: "duplicate_identity" });
      continue;
    }
    seen.set(identity, true);

    if (raw.enabled === false) {
      skipped.push({ provider, name, reason: "disabled" });
      continue;
    }

    if (!catalog.has(provider.toLowerCase())) {
      errors.push({ provider, name, reason: "unknown_provider" });
      continue;
    }

    if (
      raw.apiKey !== undefined ||
      raw.api_key !== undefined ||
      raw.credential !== undefined ||
      raw.key !== undefined
    ) {
      errors.push({ provider, name, reason: "literal_credentials_forbidden" });
      continue;
    }

    const credentialEnv =
      raw.credential_env ?? raw.credentialEnv ?? raw.api_key_env ?? raw.apiKeyEnv;
    const allowNoCredential = raw.allow_no_credential === true || raw.allowNoCredential === true;
    const entry = { provider, name };
    let credential = "";

    if (credentialEnv !== undefined) {
      if (typeof credentialEnv !== "string" || !ENV_NAME_RE.test(credentialEnv.trim())) {
        errors.push({ provider, name, reason: "invalid_credential_env" });
        continue;
      }
      credential = String(env[credentialEnv.trim()] ?? "").trim();
      if (!credential) {
        if (strict) {
          errors.push({ provider, name, reason: `missing_env:${credentialEnv.trim()}` });
        } else {
          skipped.push({ provider, name, reason: `missing_env:${credentialEnv.trim()}` });
        }
        continue;
      }
      entry.apiKey = credential;
    } else if (!allowNoCredential) {
      errors.push({ provider, name, reason: "missing_credential_env" });
      continue;
    }

    if (allowNoCredential) entry.allowNoCredential = true;

    const defaultModel = raw.default_model ?? raw.defaultModel;
    if (defaultModel !== undefined) {
      if (typeof defaultModel !== "string" || !defaultModel.trim()) {
        errors.push({ provider, name, reason: "invalid_default_model" });
        continue;
      }
      entry.defaultModel = defaultModel.trim();
    }

    if (raw.priority !== undefined) {
      const priority = Number(raw.priority);
      if (!Number.isInteger(priority) || priority < 1) {
        errors.push({ provider, name, reason: "invalid_priority" });
        continue;
      }
      entry.priority = priority;
    }

    const providerSpecificData = raw.provider_specific_data ?? raw.providerSpecificData;
    if (providerSpecificData !== undefined) {
      if (
        !providerSpecificData ||
        typeof providerSpecificData !== "object" ||
        Array.isArray(providerSpecificData)
      ) {
        errors.push({ provider, name, reason: "invalid_provider_specific_data" });
        continue;
      }
      entry.providerSpecificData = providerSpecificData;
    }

    entries.push(entry);
  }

  return { entries, skipped, errors };
}

export function summarizeImportResults(results) {
  const counts = { created: 0, skipped_existing: 0, failed: 0, other: 0 };
  const failed = [];
  const list = Array.isArray(results) ? results : [];

  for (const result of list) {
    if (!result || typeof result !== "object") {
      counts.other += 1;
      continue;
    }
    if (result.status === "created") counts.created += 1;
    else if (result.status === "skipped_existing") counts.skipped_existing += 1;
    else if (result.ok === false || result.status === "error") {
      counts.failed += 1;
      failed.push({
        provider: typeof result.provider === "string" ? result.provider : "",
        name: typeof result.name === "string" ? result.name : "",
        error: typeof result.error === "string" ? result.error : "import failed",
      });
    } else {
      counts.other += 1;
    }
  }

  return { counts, failed, total: list.length };
}

export function maskSecret(value) {
  const text = String(value ?? "");
  return text.length <= 4 ? "***" : `...${text.slice(-4)}`;
}


export const OPENCODE_PROVIDER_MAP = [
  { from: "opencode-go", provider: "opencode-go", name: "opencode-go" },
  { from: "zai", provider: "zai", name: "zai" },
  { from: "zai-coding-plan", provider: "glm", name: "glm-coding" },
];

export const FREE_OPTIONAL_PROVIDERS = [];

function identity(provider, name) {
  return `${String(provider).toLowerCase()}\0${String(name).toLowerCase()}`;
}

export function mapOpenCodeAuth(auth, catalog, existingIdentities = new Set()) {
  const entries = [];
  const skipped = [];
  const errors = [];
  const source = auth && typeof auth === "object" ? auth : {};
  const existing = existingIdentities instanceof Set ? existingIdentities : new Set(existingIdentities);

  for (const mapping of OPENCODE_PROVIDER_MAP) {
    const record = source[mapping.from];
    if (!record || typeof record !== "object") {
      skipped.push({ provider: mapping.provider, name: mapping.name, reason: "not_present" });
      continue;
    }
    if (record.type !== "api") {
      skipped.push({ provider: mapping.provider, name: mapping.name, reason: "unsupported_type" });
      continue;
    }
    const key = typeof record.key === "string" ? record.key.trim() : "";
    if (!key) {
      skipped.push({ provider: mapping.provider, name: mapping.name, reason: "empty_key" });
      continue;
    }
    if (!catalog.has(mapping.provider.toLowerCase())) {
      errors.push({ provider: mapping.provider, name: mapping.name, reason: "unknown_provider" });
      continue;
    }
    if (existing.has(identity(mapping.provider, mapping.name))) {
      skipped.push({ provider: mapping.provider, name: mapping.name, reason: "already_configured" });
      continue;
    }
    entries.push({ provider: mapping.provider, name: mapping.name, apiKey: key });
  }

  return { entries, skipped, errors };
}

export function selectOptionalFreeProviders(catalog, existingProviders = new Set()) {
  const entries = [];
  const skipped = [];
  const errors = [];
  const existing = existingProviders instanceof Set ? existingProviders : new Set(existingProviders);

  for (const candidate of FREE_OPTIONAL_PROVIDERS) {
    if (!catalog.has(candidate.provider.toLowerCase())) {
      errors.push({ provider: candidate.provider, name: candidate.name, reason: "unknown_provider" });
      continue;
    }
    if (existing.has(candidate.provider.toLowerCase())) {
      skipped.push({ provider: candidate.provider, name: candidate.name, reason: "already_configured" });
      continue;
    }
    entries.push({ provider: candidate.provider, name: candidate.name, allowNoCredential: true });
  }

  return { entries, skipped, errors };
}


export const FREE_OAUTH_PROVIDERS = [
  "openference",
  "qoder",
  "agy",
  "kiro",
  "amazon-q",
  "freebuff",
];

export function buildFreeAccountTemplate(plans, oauth = FREE_OAUTH_PROVIDERS) {
  return {
    accounts: (Array.isArray(plans) ? plans : []).map((plan) => ({
      provider: plan.provider,
      name: plan.name,
      credential_env: plan.credential_env,
      website: plan.website,
      enabled: false,
    })),
    oauth: [...oauth],
  };
}
