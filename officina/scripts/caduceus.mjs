#!/usr/bin/env node
// Caduceus bootstrap for Officina. Keeps the local VITRIOL engine as the default
// provider and registers the Caduceus gateway (backed by the OmniRoute vendor
// package) as an explicit OpenAI-compatible provider.
//
// PROVENANCE: integration glue only; OmniRoute itself is MIT and installed as a
// pinned vendor package. No OmniRoute source is copied into this repo.

import { spawn, spawnSync } from "node:child_process";
import { createInterface } from "node:readline/promises";
import {
  appendFileSync,
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { parseEnvFile, systemdUnit, upsertEnvFile } from "./caduceus-lib.mjs";
import {
  findOAuthProvider,
  OAUTH_PROVIDERS,
  parseOAuthCallback,
} from "./caduceus-oauth-lib.mjs";
import {
  buildFreeAccountTemplate,
  buildImportPayload,
  catalogMap,
  enableAccountsForKeys,
  mapOpenCodeAuth,
  maskSecret,
  normalizeAccountsConfig,
  parseCliJson,
  selectOptionalFreeProviders,
  stripAnsi,
  summarizeImportResults,
} from "./caduceus-accounts-lib.mjs";

const HOME = process.env.HOME || homedir();
const PIN = "3.8.53";
const PLAYWRIGHT_PIN = "1.62.1";
const PORT = 20128;
const ORIGIN = `http://127.0.0.1:${PORT}`;
const VENDOR_DIR = join(HOME, ".vitriol", "vendor", "omniroute");
const BIN = join(VENDOR_DIR, "node_modules", ".bin", "omniroute");
const API_MODULE = join(VENDOR_DIR, "node_modules", "omniroute", "bin", "cli", "api.mjs");
const ENV_FILE = join(HOME, ".vitriol", "officina", "env");
const CACHE_FILE = join(HOME, ".vitriol", "officina", "caduceus-models.json");
const ACCOUNTS_FILE = join(HOME, ".vitriol", "officina", "accounts.json");
const EGRESS_FILE = join(HOME, ".vitriol", "officina", "egress.json");
const EGRESS_ENV_FILE = join(HOME, ".vitriol", "officina", "egress.env");
const SERVICE_UNIT = join(HOME, ".config", "systemd", "user", "vitriol-caduceus.service");
const OLD_SERVICE_UNIT = join(HOME, ".config", "systemd", "user", "vitriol-omniroute.service");
const OLD_CACHE_FILE = join(HOME, ".vitriol", "officina", "omniroute-models.json");
const SESSIONS_DIR = join(HOME, ".pi", "agent", "sessions");
const SIGNUP_PROGRESS = join(HOME, ".vitriol", "officina", "signup-progress.json");
const DAHL_BASE_URL = (process.env.DAHL_BASE_URL || "https://inference.dahl.global/v1").replace(/\/+$/, "");
const DAHL_KEEPALIVE_FILE = join(HOME, ".vitriol", "officina", "dahl-keepalive.json");
const DAHL_KEEPALIVE_SERVICE = join(HOME, ".config", "systemd", "user", "vitriol-dahl-keepalive.service");
const DAHL_KEEPALIVE_TIMER = join(HOME, ".config", "systemd", "user", "vitriol-dahl-keepalive.timer");
const DAHL_KEEPALIVE_DAYS = Math.max(1, Math.min(6, Number(process.env.DAHL_KEEPALIVE_DAYS) || 3));
const ENGINE_URL = (process.env.VITRIOL_ENGINE_URL || "http://127.0.0.1:8279").replace(/\/+$/, "");
const ROUTE_LEDGER_DIR = process.env.OFFICINA_ROUTE_LEDGER_DIR || join(HOME, ".vitriol", "officina");
const PROVIDER_HEALTH_FILE = process.env.OFFICINA_PROVIDER_HEALTH_FILE || join(HOME, ".vitriol", "officina", "provider-health.json");
const ROUTING_PROFILE_FILE = process.env.OFFICINA_ROUTING_PROFILE_FILE || join(HOME, ".vitriol", "officina", "routing-profile");
const TUI_LOCK_FILE = process.env.OFFICINA_TUI_LOCK || join(HOME, ".vitriol", "officina", "tui.lock");
const PI_SETTINGS = join(HOME, ".pi", "agent", "settings.json");
const OPENCODE_AUTH_FILE =
  process.env.OPENCODE_AUTH_FILE || join(HOME, ".local", "share", "opencode", "auth.json");
const DATA_DIR = process.env.DATA_DIR || join(HOME, ".omniroute");
const DB_FILE = join(DATA_DIR, "storage.sqlite");
const PROVIDER_ID = "opencode-go";
const CACHE_TARGETS = ["opencode-go", "glm", "zai"];
const CONFIG_KEYS = {
  cache: { env: "OFFICINA_CACHE_MODE", values: ["auto", "local", "cloud", "off"] },
  praetor: { env: "OFFICINA_PRAETOR", values: ["on", "off"], map: { on: "1", off: "0" } },
};
// Free-onboarding entries that are currently unusable upstream.
const FREE_ONBOARDING_EXCLUDE = new Set(["uncloseai"]);

for (const stream of [process.stdout, process.stderr]) {
  stream.on("error", (error) => {
    if (error?.code === "EPIPE") process.exit(0);
  });
}

function log(message) {
  process.stdout.write(`${message}\n`);
}

function warn(message) {
  process.stderr.write(`[caduceus] ${message}\n`);
}

function fail(message, code = 1) {
  const error = new Error(message);
  error.exitCode = code;
  throw error;
}

function hasCommand(name) {
  const res = spawnSync("bash", ["-c", `command -v ${name}`], {
    encoding: "utf8",
  });
  return res.status === 0 && res.stdout.trim().length > 0;
}

function run(command, args, opts = {}) {
  const res = spawnSync(command, args, {
    encoding: "utf8",
    timeout: opts.timeout ?? 120_000,
    input: opts.input,
    env: { ...process.env, ...(opts.env ?? {}) },
    stdio: opts.inherit ? "inherit" : ["pipe", "pipe", "pipe"],
  });
  if (res.error) fail(`${command} failed: ${res.error.message}`);
  if (res.status !== 0) {
    const detail = (res.stderr || res.stdout || "").trim();
    fail(
      `${command} ${args.join(" ")} failed (exit ${res.status})${detail ? `: ${detail}` : ""}`,
    );
  }
  return res.stdout ?? "";
}

function runCli(args, opts = {}) {
  return run(BIN, args, { ...opts, env: { ...egressEnv(), ...(opts.env ?? {}) } });
}

async function gatewayApi(path, opts = {}) {
  const mod = await import(pathToFileURL(API_MODULE).href);
  return mod.apiFetch(path, { baseUrl: ORIGIN, ...opts });
}

function readStdin() {
  return readFileSync(0, "utf8");
}

function parseFlags(argv) {
  const flags = { _: [] };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (!arg.startsWith("--")) {
      flags._.push(arg);
      continue;
    }
    const eq = arg.indexOf("=");
    const key = eq > 0 ? arg.slice(2, eq) : arg.slice(2);
    if (eq > 0) {
      flags[key] = arg.slice(eq + 1);
      continue;
    }
    const next = argv[i + 1];
    if (next && !next.startsWith("--")) {
      flags[key] = next;
      i += 1;
    } else {
      flags[key] = true;
    }
  }
  return flags;
}

function readEnv() {
  if (!existsSync(ENV_FILE)) return new Map();
  try {
    return parseEnvFile(readFileSync(ENV_FILE, "utf8"));
  } catch {
    return new Map();
  }
}

function mergedEnv() {
  return { ...Object.fromEntries(readEnv()), ...process.env };
}

function writeEnv(updates) {
  const dir = dirname(ENV_FILE);
  mkdirSync(dir, { recursive: true, mode: 0o700 });
  const current = existsSync(ENV_FILE) ? readFileSync(ENV_FILE, "utf8") : "";
  const next = upsertEnvFile(current, updates);
  const tmp = `${ENV_FILE}.tmp-${process.pid}`;
  writeFileSync(tmp, next, { mode: 0o600 });
  chmodSync(tmp, 0o600);
  renameSync(tmp, ENV_FILE);
  chmodSync(ENV_FILE, 0o600);
}

function writeJsonAtomic(path, value) {
  mkdirSync(dirname(path), { recursive: true });
  const tmp = `${path}.tmp-${process.pid}`;
  writeFileSync(tmp, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
  renameSync(tmp, path);
}

async function serverUp() {
  try {
    const res = await fetch(`${ORIGIN}/api/health`, { signal: AbortSignal.timeout(1500) });
    return res.status < 500;
  } catch {
    return false;
  }
}

async function waitForServer(timeoutMs = 60_000) {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    if (await serverUp()) return true;
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  return false;
}

async function waitForDown(timeoutMs = 15_000) {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    if (!(await serverUp())) return true;
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  return false;
}

async function startServer() {
  if (await serverUp()) return false;
  if (existsSync(SERVICE_UNIT)) {
    const res = systemctlUser(["start", "vitriol-caduceus.service"]);
    if (!res.ok) fail(`could not start vitriol-caduceus.service: ${res.out}`);
  } else {
    runCli(["serve", "--headless", "--daemon", "--port", String(PORT)], {
      env: { PORT: String(PORT), OMNIROUTE_HEADLESS: "1", ...egressEnv() },
      timeout: 120_000,
    });
  }
  if (!(await waitForServer())) fail(`gateway did not become healthy at ${ORIGIN}/api/health`);
  return true;
}

async function stopServer() {
  if (!(await serverUp())) return true;
  if (existsSync(SERVICE_UNIT)) {
    const res = systemctlUser(["stop", "vitriol-caduceus.service"]);
    if (!res.ok) {
      warn(`could not stop vitriol-caduceus.service: ${res.out}`);
      return false;
    }
    return waitForDown();
  }
  try {
    runCli(["stop", "--port", String(PORT)], { timeout: 30_000 });
  } catch (error) {
    warn(error instanceof Error ? error.message : String(error));
    return false;
  }
  return waitForDown();
}

function installedVersion() {
  if (!existsSync(BIN)) return null;
  const res = spawnSync(BIN, ["--version"], { encoding: "utf8", timeout: 15_000 });
  if (res.status !== 0) return null;
  return String(res.stdout || "").trim();
}

function playwrightInstalled() {
  return existsSync(join(VENDOR_DIR, "node_modules", "playwright", "package.json"));
}

function installOmniRoute(force = false) {
  const version = installedVersion();
  if (!force && version === PIN && playwrightInstalled()) return;
  if (!hasCommand("npm")) fail("npm is required to install the OmniRoute vendor package");
  log(`[caduceus] installing OmniRoute vendor package@${PIN} + playwright@${PLAYWRIGHT_PIN} under ${VENDOR_DIR}`);
  mkdirSync(VENDOR_DIR, { recursive: true });
  run(
    "npm",
    [
      "install",
      "--prefix",
      VENDOR_DIR,
      "--no-save",
      "--no-audit",
      "--no-fund",
      `omniroute@${PIN}`,
      `playwright@${PLAYWRIGHT_PIN}`,
    ],
    { inherit: true, timeout: 900_000 },
  );
  const after = installedVersion();
  if (after !== PIN) fail(`OmniRoute vendor install finished but version is ${after ?? "missing"}`);
  if (!playwrightInstalled()) fail("playwright package was not installed");
}

function browserRuntimePresent() {
  if (!playwrightInstalled()) return false;
  const cli = join(VENDOR_DIR, "node_modules", "playwright", "cli.js");
  if (!existsSync(cli)) return false;
  const script =
    'import("playwright").then(({chromium}) => process.stdout.write(chromium.executablePath())).catch(() => {})';
  const res = spawnSync(process.execPath, ["--input-type=module", "-e", script], {
    cwd: VENDOR_DIR,
    encoding: "utf8",
    timeout: 30_000,
  });
  const browserPath = String(res.stdout || "").trim();
  return Boolean(browserPath) && existsSync(browserPath);
}

function ensureBrowserRuntime() {
  if (browserRuntimePresent()) return true;
  if (!playwrightInstalled()) installOmniRoute(true);
  const cli = join(VENDOR_DIR, "node_modules", "playwright", "cli.js");
  if (!existsSync(cli)) {
    warn("playwright CLI missing; browser-backed free providers will not work");
    return false;
  }
  log("[caduceus] installing Chromium for browser-backed free providers");
  try {
    run(process.execPath, [cli, "install", "chromium"], {
      inherit: true,
      timeout: 900_000,
    });
  } catch (error) {
    warn(`Chromium install failed: ${error instanceof Error ? error.message : String(error)}`);
    return false;
  }
  return browserRuntimePresent();
}

function providerConnectionCount(provider) {
  if (!existsSync(DB_FILE)) return 0;
  if (!hasCommand("sqlite3")) return null;
  const escaped = provider.replaceAll("'", "''");
  const res = spawnSync(
    "sqlite3",
    [DB_FILE, `SELECT COUNT(*) FROM provider_connections WHERE provider = '${escaped}';`],
    { encoding: "utf8", timeout: 15_000 },
  );
  if (res.status !== 0) return null;
  const count = Number.parseInt(String(res.stdout || "").trim(), 10);
  return Number.isFinite(count) ? count : null;
}

function addOpenCodeProvider(flags = {}) {
  const existing = providerConnectionCount(PROVIDER_ID);
  if (existing && existing > 0) {
    log(`[caduceus] provider ${PROVIDER_ID} already configured`);
    return false;
  }

  let credential = process.env.OPENCODE_API_KEY || "";
  if (!credential) {
    const useStdin = flags["opencode-key-stdin"] || !process.stdin.isTTY;
    if (!useStdin) {
      fail(
        "OpenCode credential required. Pipe it with --opencode-key-stdin or export OPENCODE_API_KEY.",
      );
    }
    credential = readStdin().trim();
  }
  if (!credential) fail("OpenCode credential was empty");

  log(`[caduceus] adding provider ${PROVIDER_ID}`);
  runCli(
    [
      "--base-url",
      ORIGIN,
      "providers",
      "add",
      PROVIDER_ID,
      "--credential-stdin",
      "--name",
      PROVIDER_ID,
      "--yes",
    ],
    { input: `${credential}\n`, timeout: 120_000 },
  );
  return true;
}

async function createInferenceKey() {
  log("[caduceus] creating local inference key for Officina");
  const res = await gatewayApi("/api/keys", {
    method: "POST",
    body: { name: "officina" },
  });
  if (!res.ok) {
    const detail = await res.text().catch(() => "");
    fail(`could not create inference key (HTTP ${res.status})${detail ? `: ${detail}` : ""}`);
  }
  const parsed = await res.json();
  const key = typeof parsed?.key === "string" ? parsed.key : "";
  if (!key) fail("could not parse the inference key from the gateway");
  return key;
}

async function fetchActiveProviders() {
  try {
    const res = await gatewayApi("/api/providers?limit=5000");
    if (!res.ok) return null;
    const body = await res.json();
    const list = Array.isArray(body) ? body : body?.connections ?? body?.providers ?? [];
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

function filterModelsByActiveProviders(list, activeProviders) {
  if (!activeProviders) return list;
  if (activeProviders.size === 0) return [];
  return list.filter((model) => {
    const owner = typeof model?.owned_by === "string" ? model.owned_by.toLowerCase() : "";
    return !owner || owner === "auto" || activeProviders.has(owner);
  });
}

async function fetchModels(apiKey) {
  const url = `${ORIGIN}/v1/models?prefix=alias`;
  try {
    const [res, activeProviders] = await Promise.all([
      fetch(url, {
        headers: apiKey ? { authorization: `Bearer ${apiKey}` } : undefined,
        signal: AbortSignal.timeout(10_000),
      }),
      fetchActiveProviders(),
    ]);
    const text = await res.text();
    let body = null;
    try {
      body = JSON.parse(text);
    } catch {
      body = null;
    }
    const list = Array.isArray(body)
      ? body
      : body?.data?.data ?? body?.data ?? body?.models ?? [];
    const data = Array.isArray(list)
      ? filterModelsByActiveProviders(list, activeProviders)
      : [];
    return { ok: res.ok, status: res.status, data, body };
  } catch (error) {
    return { ok: false, status: 0, data: [], error: error instanceof Error ? error.message : String(error) };
  }
}

function cacheCatalog(baseUrl, data) {
  writeJsonAtomic(CACHE_FILE, {
    fetchedAt: new Date().toISOString(),
    baseUrl,
    data,
  });
}

function writePassthroughSettings() {
  if (!existsSync(DB_FILE)) {
    warn(`gateway DB not found at ${DB_FILE}; relying on per-request headers`);
    return false;
  }
  if (!hasCommand("sqlite3")) {
    warn("sqlite3 not found; relying on per-request headers");
    return false;
  }
  const sql = [
    "BEGIN IMMEDIATE;",
    "INSERT OR REPLACE INTO key_value(namespace, key, value) VALUES",
    "('databaseSettings','cache.semanticCacheEnabled','false'),",
    "('databaseSettings','cache.semanticCacheVectorEnabled','false'),",
    "('databaseSettings','cache.semanticCacheRequireZeroTemp','true'),",
    "('settings','memoryEnabled','false'),",
    "('compression','defaultMode','\"off\"'),",
    "('compression','enabled','false'),",
    "('compression','activeComboId','null');",
    "COMMIT;",
  ].join(" ");
  const res = spawnSync("sqlite3", [DB_FILE, sql], { encoding: "utf8", timeout: 30_000 });
  if (res.status !== 0) {
    warn(`could not write passthrough settings: ${(res.stderr || "").trim()}`);
    return false;
  }
  return true;
}

function clearCachedResponses() {
  if (!existsSync(DB_FILE) || !hasCommand("sqlite3")) return;
  const res = spawnSync("sqlite3", [DB_FILE, "DELETE FROM semantic_cache;"], {
    encoding: "utf8",
    timeout: 30_000,
  });
  if (res.status !== 0) warn(`could not clear the semantic cache table: ${(res.stderr || "").trim()}`);
}

function resolveInferenceKey(env) {
  return env.get("OFFICINA_CADUCEUS_API_KEY") || "";
}

async function setup(flags) {
  if (!flags["no-install"]) installOmniRoute(Boolean(flags.force));
  else if (!existsSync(BIN)) fail(`Caduceus gateway not installed at ${BIN} (remove --no-install)`);

  let startedByUs = false;
  try {
    log(`[caduceus] ensuring gateway on ${ORIGIN}`);
    startedByUs = await startServer();

    addOpenCodeProvider(flags);

    const env = readEnv();
    let apiKey = resolveInferenceKey(env);
    if (apiKey) {
      const probe = await fetchModels(apiKey);
      if (!probe.ok && probe.status !== 404) apiKey = "";
    }
    if (!apiKey) apiKey = await createInferenceKey();

    const stopped = await stopServer();
    if (stopped) {
      writePassthroughSettings();
      clearCachedResponses();
    } else {
      warn("gateway did not stop; skipping DB passthrough write (headers still enforce)");
    }
    await startServer();

    const catalog = await fetchModels(apiKey);
    if (!catalog.ok) {
      warn(`catalog probe failed with HTTP ${catalog.status}; Officina will retry at runtime`);
    } else {
      cacheCatalog(`${ORIGIN}/v1`, catalog.data);
    }

    writeEnv({
      OFFICINA_CADUCEUS: "1",
      OFFICINA_CADUCEUS_BASE_URL: `${ORIGIN}/v1`,
      OFFICINA_CADUCEUS_API_KEY: apiKey,
    });

    log("");
    log("Caduceus ready (powered by OmniRoute).");
    log(`  base URL: ${ORIGIN}/v1`);
    log(`  models:   ${catalog.ok ? catalog.data.length : "unavailable"}`);
    log(`  env:      ${ENV_FILE}`);
    log("");
    log("Next: vitriol officina, then /model -> caduceus/<model>.");

    if (flags.free) {
      log("");
      log("[caduceus] --free: adding zero-key providers and reusing local auth");
      await cmdAccountsFree({ ...flags, apply: true, _: [] });
      await cmdCache({ _: ["enable"], providers: "opencode-go,glm" });
    }
  } catch (error) {
    if (startedByUs) {
      warn("setup failed; stopping the gateway started by this run");
      await stopServer().catch(() => {});
    }
    throw error;
  }
}

async function cmdUp() {
  if (!existsSync(BIN)) fail(`Caduceus gateway not installed at ${BIN}; run: vitriol caduceus setup`);
  const started = await startServer();
  log(started ? "Caduceus gateway started." : "Caduceus gateway already running.");
}

async function cmdDown() {
  if (!existsSync(BIN)) fail(`Caduceus gateway not installed at ${BIN}`);
  const stopped = await stopServer();
  if (!stopped) fail("Caduceus gateway did not stop");
  log("Caduceus gateway stopped.");
}

async function cmdStatus() {
  const env = readEnv();
  const apiKey = resolveInferenceKey(env) || process.env.OFFICINA_CADUCEUS_API_KEY || "";
  const up = await serverUp();
  const version = installedVersion();
  const catalog = up ? await fetchModels(apiKey) : { ok: false, status: 0, data: [] };
  const connections = providerConnectionCount(PROVIDER_ID);

  log(`Caduceus ${version ?? "not installed"} (OmniRoute vendor)`);
  log(`  gateway:    ${up ? `up (${ORIGIN})` : "down"}`);
  const connectionText =
    connections === null
      ? "unknown"
      : `${connections} connection${connections === 1 ? "" : "s"}`;
  log(`  provider:   ${PROVIDER_ID} (${connectionText})`);
  const catalogText = catalog.ok
    ? `${catalog.data.length} models`
    : `unavailable (HTTP ${catalog.status})`;
  log(`  catalog:    ${catalogText}`);
  log(`  inference:  ${apiKey ? `set (...${apiKey.slice(-4)})` : "missing"}`);
  log(`  env file:   ${ENV_FILE}`);
  log(`  passthrough: compression off, semantic cache off, memory off, prompt cache on`);
}

async function cmdSync() {
  const env = readEnv();
  const apiKey = resolveInferenceKey(env) || process.env.OFFICINA_CADUCEUS_API_KEY || "";
  if (!apiKey) fail("missing OFFICINA_CADUCEUS_API_KEY; run: vitriol caduceus setup");
  if (!(await serverUp())) fail(`gateway is down at ${ORIGIN}; run: vitriol caduceus up`);
  const catalog = await fetchModels(apiKey);
  if (!catalog.ok) fail(`catalog fetch failed with HTTP ${catalog.status}`);
  cacheCatalog(`${ORIGIN}/v1`, catalog.data);
  log(`Cached ${catalog.data.length} models from ${ORIGIN}/v1/models -> ${CACHE_FILE}`);
}




function readEgress() {
  if (!existsSync(EGRESS_FILE)) return { mode: "off", proxy: null, no_proxy: [] };
  try {
    const parsed = JSON.parse(readFileSync(EGRESS_FILE, "utf8"));
    return {
      mode: parsed?.mode === "fixed" ? "fixed" : "off",
      proxy: typeof parsed?.proxy === "string" ? parsed.proxy : null,
      no_proxy: Array.isArray(parsed?.no_proxy) ? parsed.no_proxy.filter((x) => typeof x === "string") : [],
    };
  } catch {
    return { mode: "off", proxy: null, no_proxy: [] };
  }
}

function buildEgressEnv(egress) {
  if (process.env.OFFICINA_EGRESS === "0") return {};
  if (egress.mode !== "fixed" || !egress.proxy) return {};
  const noProxy = ["127.0.0.1", "localhost", "::1", ...egress.no_proxy].join(",");
  return {
    HTTPS_PROXY: egress.proxy,
    HTTP_PROXY: egress.proxy,
    ALL_PROXY: egress.proxy,
    https_proxy: egress.proxy,
    http_proxy: egress.proxy,
    all_proxy: egress.proxy,
    NO_PROXY: noProxy,
    no_proxy: noProxy,
  };
}

function egressEnv() {
  return buildEgressEnv(readEgress());
}

function writeEgressEnv(value) {
  const env = buildEgressEnv(value);
  const text = Object.entries(env)
    .map(([key, val]) => `${key}=${String(val).replace(/\n/g, "")}`)
    .join("\n");
  writeFileSync(EGRESS_ENV_FILE, text ? `${text}\n` : "", { mode: 0o600 });
  chmodSync(EGRESS_ENV_FILE, 0o600);
}

function maskProxy(proxy) {
  const text = String(proxy ?? "");
  if (!text) return "(none)";
  try {
    const url = new URL(text);
    if (url.username) url.username = "***";
    if (url.password) url.password = "***";
    return url.toString();
  } catch {
    return text.replace(/\/\/[^/@]+@/, "//***@");
  }
}

function writeEgress(value) {
  writeJsonAtomic(EGRESS_FILE, value);
  chmodSync(EGRESS_FILE, 0o600);
  writeEgressEnv(value);
}

async function restartIfRunning() {
  if (!(await serverUp())) return false;
  const stopped = await stopServer();
  if (!stopped) return false;
  await startServer();
  return true;
}

async function cmdEgress(flags) {
  const sub = flags._[0] || "status";
  if (sub === "status" || flags.json) {
    const egress = readEgress();
    const up = await serverUp();
    const report = {
      mode: egress.mode,
      proxy: egress.mode === "fixed" ? maskProxy(egress.proxy) : null,
      no_proxy: egress.no_proxy,
      gateway: up ? "up" : "down",
      disabled: process.env.OFFICINA_EGRESS === "0",
    };
    if (flags.json || sub === "status") {
      if (flags.json) log(JSON.stringify(report, null, 2));
      else {
        log(`Egress mode: ${report.mode}`);
        log(`  proxy:   ${report.proxy ?? "(none)"}`);
        log(`  no_proxy: ${report.no_proxy.join(",") || "(defaults)"}`);
        log(`  gateway: ${report.gateway}`);
        if (report.disabled) log("  disabled: OFFICINA_EGRESS=0");
        log("  policy: fixed egress only; no quota-based IP rotation");
      }
      if (sub === "status") return;
    }
  }

  if (sub === "set") {
    const proxy = typeof flags.proxy === "string" ? flags.proxy.trim() : "";
    if (!proxy) fail("--proxy is required");
    let parsed;
    try {
      parsed = new URL(proxy);
    } catch {
      fail("--proxy must be a valid http(s) or socks5 URL");
    }
    if (!["http:", "https:", "socks5:"].includes(parsed.protocol)) {
      fail("--proxy must use http, https, or socks5");
    }
    const rawNoProxy = typeof flags["no-proxy"] === "string" ? flags["no-proxy"] : "";
    const noProxy = rawNoProxy
      .split(",")
      .map((value) => value.trim())
      .filter(Boolean);
    writeEgress({ mode: "fixed", proxy, no_proxy: noProxy });
    const restarted = await restartIfRunning();
    log(`Fixed egress configured: ${maskProxy(proxy)}${restarted ? " (gateway restarted)" : ""}`);
    log("Policy: fixed egress only; no quota-based IP rotation.");
    return;
  }

  if (sub === "clear") {
    writeEgress({ mode: "off", proxy: null, no_proxy: [] });
    const restarted = await restartIfRunning();
    log(`Fixed egress cleared${restarted ? " (gateway restarted)" : ""}.`);
    return;
  }

  if (sub === "test") {
    const egress = readEgress();
    if (egress.mode !== "fixed" || !egress.proxy) fail("no fixed egress configured");
    if (!hasCommand("curl")) fail("curl is required for egress test");
    const res = spawnSync(
      "curl",
      ["--proxy", egress.proxy, "-sS", "-m", "15", "https://api.ipify.org?format=json"],
      { encoding: "utf8", timeout: 20_000 },
    );
    if (res.status !== 0) fail(`proxy test failed: ${(res.stderr || "").trim()}`);
    let ip = "";
    try {
      ip = String(JSON.parse(res.stdout).ip ?? "");
    } catch {
      ip = String(res.stdout || "").trim();
    }
    log(`Proxy reachable. Egress IP: ${ip || "(unknown)"}`);
    log(`Gateway: ${(await serverUp()) ? "up" : "down"}`);
    return;
  }

  fail(`unknown egress command: ${sub}`);
}


function praetorArmed() {
  return process.env.OFFICINA_PRAETOR === "1" || process.env.TRIS_DIAG_PRAETOR === "1";
}

function systemctlUser(args) {
  if (!hasCommand("systemctl")) return { ok: false, out: "systemctl not found" };
  const res = spawnSync("systemctl", ["--user", ...args], {
    encoding: "utf8",
    timeout: 20_000,
  });
  return { ok: res.status === 0, out: String(res.stdout || res.stderr || "").trim() };
}

function migrateString(value) {
  if (value === "omniroute") return "caduceus";
  if (value.startsWith("omniroute/")) return `caduceus/${value.slice("omniroute/".length)}`;
  return value;
}

function migrateJson(value) {
  if (typeof value === "string") return migrateString(value);
  if (Array.isArray(value)) return value.map(migrateJson);
  if (value && typeof value === "object") {
    const out = {};
    for (const [k, v] of Object.entries(value)) {
      out[migrateString(k)] = migrateJson(v);
    }
    return out;
  }
  return value;
}

function migrateEnvText(text) {
  const lines = String(text).split(/\r?\n/);
  const newKeys = new Set();
  for (const line of lines) {
    const m = line.match(/^\s*(OFFICINA_CADUCEUS[A-Z0-9_]*)\s*=/);
    if (m) newKeys.add(m[1]);
  }
  const out = [];
  let changed = 0;
  for (const line of lines) {
    const old = line.match(/^(\s*)OFFICINA_OMNIROUTE([A-Z0-9_]*)\s*=/);
    if (!old) {
      out.push(line);
      continue;
    }
    const key = `OFFICINA_CADUCEUS${old[2]}`;
    changed += 1;
    if (newKeys.has(key)) continue;
    newKeys.add(key);
    out.push(line.replace(/^\s*OFFICINA_OMNIROUTE[A-Z0-9_]*\s*=/, `${key}=`));
  }
  return { text: out.join("\n"), changed };
}

function walkJsonl(dir, out = []) {
  if (!existsSync(dir)) return out;
  for (const ent of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, ent.name);
    if (ent.isDirectory()) walkJsonl(path, out);
    else if (ent.isFile() && ent.name.endsWith(".jsonl")) out.push(path);
  }
  return out;
}

function migrateSessionFile(file, write = true) {
  const text = readFileSync(file, "utf8");
  if (!text.includes("omniroute")) return { changed: false, lines: 0 };
  let changedLines = 0;
  const out = text.split("\n").map((line) => {
    if (!line.includes("omniroute")) return line;
    try {
      const parsed = JSON.parse(line);
      const migrated = migrateJson(parsed);
      const next = JSON.stringify(migrated);
      if (next !== line) changedLines += 1;
      return next;
    } catch {
      // Never corrupt a line that is not valid JSON; leave it for manual review.
      return line;
    }
  });
  if (changedLines === 0) return { changed: false, lines: 0 };
  if (write) {
    const tmp = `${file}.migrate-${process.pid}`;
    writeFileSync(tmp, out.join("\n"));
    chmodSync(tmp, statSync(file).mode);
    renameSync(tmp, file);
  }
  return { changed: true, lines: changedLines };
}

async function cmdMigrate(flags) {
  const apply = Boolean(flags.apply) && !flags["dry-run"];
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  const backupDir = join(HOME, ".vitriol", "officina", `migrate-backup-${stamp}`);
  const summary = { env: 0, settings: false, sessions: [], cache: false, unit: false, backupDir };
  const backup = (src, rel) => {
    if (!apply) return;
    const dst = join(backupDir, rel);
    mkdirSync(dirname(dst), { recursive: true, mode: 0o700 });
    copyFileSync(src, dst);
    chmodSync(dst, statSync(src).mode);
  };
  if (apply) mkdirSync(backupDir, { recursive: true, mode: 0o700 });

  // env keys
  if (existsSync(ENV_FILE)) {
    const raw = readFileSync(ENV_FILE, "utf8");
    const migrated = migrateEnvText(raw);
    summary.env = migrated.changed;
    if (migrated.changed > 0) {
      backup(ENV_FILE, "env");
      if (apply) {
        const tmp = `${ENV_FILE}.migrate-${process.pid}`;
        writeFileSync(tmp, migrated.text, { mode: 0o600 });
        chmodSync(tmp, 0o600);
        renameSync(tmp, ENV_FILE);
      }
    }
  }

  // cache
  if (existsSync(OLD_CACHE_FILE) && !existsSync(CACHE_FILE)) {
    summary.cache = true;
    if (apply) {
      backup(OLD_CACHE_FILE, "omniroute-models.json");
      copyFileSync(OLD_CACHE_FILE, CACHE_FILE);
      chmodSync(CACHE_FILE, 0o600);
      rmSync(OLD_CACHE_FILE, { force: true });
    }
  }

  // pi settings
  for (const file of [PI_SETTINGS, join(HOME, ".pi", "agent", "models-store.json")]) {
    if (!existsSync(file)) continue;
    const raw = readFileSync(file, "utf8");
    if (!raw.includes("omniroute")) continue;
    try {
      const parsed = JSON.parse(raw);
      const migrated = migrateJson(parsed);
      summary.settings = true;
      backup(file, file.endsWith("settings.json") ? "settings.json" : "models-store.json");
      if (apply) {
        const tmp = `${file}.migrate-${process.pid}`;
        writeFileSync(tmp, `${JSON.stringify(migrated, null, 2)}\n`);
        chmodSync(tmp, statSync(file).mode);
        renameSync(tmp, file);
      }
    } catch (error) {
      warn(`could not migrate ${file}: ${error instanceof Error ? error.message : String(error)}`);
    }
  }

  // session JSONL provider/model references
  for (const file of walkJsonl(SESSIONS_DIR)) {
    const raw = readFileSync(file, "utf8");
    if (!raw.includes("omniroute")) continue;
    if (apply) backup(file, join("sessions", file.slice(SESSIONS_DIR.length + 1)));
    const result = migrateSessionFile(file, apply);
    if (result.changed) summary.sessions.push({ file, lines: result.lines });
  }

  // service unit
  if (existsSync(OLD_SERVICE_UNIT) || systemctlUser(["is-active", "vitriol-omniroute.service"]).ok) {
    summary.unit = true;
    if (apply) {
      if (existsSync(OLD_SERVICE_UNIT)) backup(OLD_SERVICE_UNIT, "vitriol-omniroute.service");
      systemctlUser(["disable", "--now", "vitriol-omniroute.service"]);
      if (existsSync(OLD_SERVICE_UNIT)) rmSync(OLD_SERVICE_UNIT, { force: true });
      systemctlUser(["daemon-reload"]);
      if (!flags["no-service"]) await cmdService({ _: ["install"] });
    }
  }
  if (existsSync(OLD_CACHE_FILE) && apply) rmSync(OLD_CACHE_FILE, { force: true });

  if (flags.json) {
    log(JSON.stringify({ applied: apply, ...summary }, null, 2));
    return;
  }
  log(`${apply ? "Migrated" : "Dry run:"} OmniRoute -> Caduceus`);
  log(`  env keys: ${summary.env}`);
  log(`  settings: ${summary.settings ? "changed" : "no change"}`);
  log(`  cache: ${summary.cache ? "renamed" : "no change"}`);
  log(`  sessions: ${summary.sessions.length} file(s)`);
  log(`  service unit: ${summary.unit ? "migrated" : "no change"}`);
  if (apply) log(`  backup: ${backupDir}`);
  else log("  re-run with --apply to write changes");
}

async function cmdService(flags) {
  const sub = flags._[0] || "status";
  if (sub === "status") {
    const enabled = systemctlUser(["is-enabled", "vitriol-caduceus.service"]);
    const active = systemctlUser(["is-active", "vitriol-caduceus.service"]);
    const up = await serverUp();
    const report = {
      installed: existsSync(SERVICE_UNIT),
      enabled: enabled.ok,
      active: active.ok,
      gateway: up ? "up" : "down",
      unit: SERVICE_UNIT,
    };
    if (flags.json) {
      log(JSON.stringify(report, null, 2));
      return;
    }
    log(`Service: ${report.installed ? "installed" : "not installed"}`);
    log(`  enabled: ${report.enabled ? "yes" : "no"}`);
    log(`  active:  ${report.active ? "yes" : "no"}`);
    log(`  gateway: ${report.gateway}`);
    log(`  unit:    ${report.unit}`);
    return;
  }

  if (sub === "install") {
    if (!existsSync(BIN)) fail(`Caduceus gateway not installed at ${BIN}; run: vitriol caduceus setup`);
    mkdirSync(join(HOME, ".config", "systemd", "user"), { recursive: true });
    writeEgressEnv(readEgress());
    const unit = systemdUnit({
      bin: "%h/.vitriol/vendor/omniroute/node_modules/.bin/omniroute",
      envFile: "%h/.vitriol/officina/egress.env",
      port: PORT,
    });
    writeFileSync(SERVICE_UNIT, unit, { mode: 0o644 });
    if (await serverUp()) {
      log("[caduceus] stopping the running gateway to free the port for the user service");
      await stopServer();
    }
    const reload = systemctlUser(["daemon-reload"]);
    if (!reload.ok) fail(`systemctl --user daemon-reload failed: ${reload.out}`);
    const enable = systemctlUser(["enable", "--now", "vitriol-caduceus.service"]);
    if (!enable.ok) fail(`could not enable vitriol-caduceus.service: ${enable.out}`);
    const linger = spawnSync("loginctl", ["enable-linger", process.env.USER || ""], {
      encoding: "utf8",
      timeout: 15_000,
    });
    if (linger.status !== 0) {
      warn("loginctl enable-linger failed; the service may not start until you log in");
    }
    if (!(await waitForServer())) fail("service started but gateway is not healthy");
    log(`Installed and started ${SERVICE_UNIT}`);
    log("Next: vitriol caduceus service status");
    return;
  }

  if (sub === "remove") {
    systemctlUser(["disable", "--now", "vitriol-caduceus.service"]);
    if (existsSync(SERVICE_UNIT)) {
      rmSync(SERVICE_UNIT, { force: true });
      log(`Removed ${SERVICE_UNIT}`);
    } else {
      log("Service unit was not installed");
    }
    systemctlUser(["daemon-reload"]);
    return;
  }

  fail(`unknown service command: ${sub}`);
}

async function cmdDoctor(flags) {
  const checks = [];
  const add = (name, status, detail) => checks.push({ name, status, detail });

  const version = installedVersion();
  if (!version) add("gateway binary", "fail", `missing at ${BIN}`);
  else if (version === PIN) add("gateway binary", "ok", version);
  else add("gateway binary", "warn", `${version} (pinned ${PIN})`);

  const up = await serverUp();
  add("gateway", up ? "ok" : "fail", up ? ORIGIN : `down at ${ORIGIN}`);

  const env = readEnv();
  const apiKey = resolveInferenceKey(env) || process.env.OFFICINA_CADUCEUS_API_KEY || "";
  add("inference key", apiKey ? "ok" : "fail", apiKey ? `set (...${apiKey.slice(-4)})` : "missing");

  if (existsSync(ENV_FILE)) {
    const mode = statSync(ENV_FILE).mode & 0o777;
    add("env permissions", mode === 0o600 ? "ok" : "warn", `${mode.toString(8)} ${ENV_FILE}`);
  } else {
    add("env permissions", "warn", `missing ${ENV_FILE}`);
  }

  const ka = keepaliveStatus();
  if (!ka.keyPresent) {
    add("dahl keepalive", "warn", "DAHL_API_KEY missing");
  } else if (!ka.lastAt) {
    add("dahl keepalive", "warn", "never requested; run vitriol caduceus keepalive");
  } else if (ka.ageDays != null && ka.ageDays >= 7) {
    add("dahl keepalive", "fail", `${ka.ageDays.toFixed(1)}d stale — free tokens may have burned`);
  } else if (ka.ageDays != null && ka.ageDays >= 5) {
    add("dahl keepalive", "warn", `${ka.ageDays.toFixed(1)}d ago — 7d inactivity burns free tokens`);
  } else {
    add("dahl keepalive", "ok", `${ka.ageDays?.toFixed(1) ?? "?"}d ago`);
  }
  if (ka.keyPresent && !ka.timerInstalled) {
    add("dahl keepalive timer", "warn", "not installed; run vitriol caduceus keepalive --install");
  }

  let connections = [];
  if (up) {
    try {
      connections = await fetchConnectionsApi();
    } catch (error) {
      add("connections", "fail", error instanceof Error ? error.message : String(error));
    }
  }
  if (up && connections.length >= 0) {
    add(
      "connections",
      connections.length > 0 ? "ok" : "fail",
      connections.length > 0 ? connections.map((c) => c.provider).join(", ") : "none",
    );
  }

  if (up && apiKey) {
    const catalog = await fetchModels(apiKey);
    add("catalog", catalog.ok && catalog.data.length > 0 ? "ok" : "warn", catalog.ok ? `${catalog.data.length} models` : `HTTP ${catalog.status}`);
  }

  const cacheTargets = new Set(["opencode-go", "glm"]);
  const cacheRows = connections.filter((c) => cacheTargets.has(String(c.provider)));
  if (cacheRows.length === 0) add("cache overrides", "warn", "targets not configured");
  else {
    const disabled = cacheRows.filter((c) => !c.providerSpecificData?.cache?.supportsPromptCaching);
    add(
      "cache overrides",
      disabled.length === 0 ? "ok" : "warn",
      disabled.length === 0 ? "opencode-go/glm enabled" : disabled.map((c) => c.provider).join(", ") + " disabled",
    );
  }

  add("browser runtime", browserRuntimePresent() ? "ok" : "warn", browserRuntimePresent() ? "Playwright + Chromium" : "missing Chromium");

  if (hasCommand("praetor")) {
    const armed = praetorArmed();
    add("praetor", "ok", armed ? "binary armed" : "binary available (OFFICINA_PRAETOR not set)");
  } else {
    add("praetor", "warn", "binary not found");
  }

  const egress = readEgress();
  add("egress", egress.mode === "fixed" ? "ok" : "warn", egress.mode === "fixed" ? maskProxy(egress.proxy) : "off");

  const enabled = systemctlUser(["is-enabled", "vitriol-caduceus.service"]);
  const active = systemctlUser(["is-active", "vitriol-caduceus.service"]);
  add(
    "boot service",
    enabled.ok && active.ok ? "ok" : "warn",
    enabled.ok && active.ok ? "enabled + active" : existsSync(SERVICE_UNIT) ? "installed but inactive" : "not installed",
  );

  add("accounts config", existsSync(ACCOUNTS_FILE) ? "ok" : "warn", existsSync(ACCOUNTS_FILE) ? ACCOUNTS_FILE : "missing; run accounts init or accounts free --template");

  const failures = checks.filter((c) => c.status === "fail");
  const warnings = checks.filter((c) => c.status === "warn");
  if (flags.json) {
    log(JSON.stringify({ ok: failures.length === 0, failures: failures.length, warnings: warnings.length, checks }, null, 2));
  } else {
    for (const check of checks) {
      log(`${check.status.toUpperCase().padEnd(4)} ${check.name}: ${check.detail}`);
    }
    log("");
    log(`${failures.length} failure(s), ${warnings.length} warning(s)`);
  }
  if (failures.length > 0) process.exitCode = 1;
}

async function fetchConnectionsApi() {
  const res = await gatewayApi("/api/providers?limit=5000");
  if (!res.ok) {
    const detail = await res.text().catch(() => "");
    fail(`provider list failed (HTTP ${res.status})${detail ? `: ${detail}` : ""}`);
  }
  const body = await res.json();
  return Array.isArray(body) ? body : body?.connections ?? body?.providers ?? [];
}


async function cmdUse(flags) {
  const localValue = typeof flags.local === "string" ? flags.local.trim() : "";
  const requested =
    typeof flags._[0] === "string" ? flags._[0].trim() : typeof flags.model === "string" ? flags.model.trim() : localValue;
  const settings = existsSync(PI_SETTINGS)
    ? JSON.parse(readFileSync(PI_SETTINGS, "utf8"))
    : {};

  if (flags.local) {
    settings.defaultProvider = "llamacpp";
    if (requested) settings.defaultModel = requested;
    writeJsonAtomic(PI_SETTINGS, settings);
    chmodSync(PI_SETTINGS, 0o600);
    log(`Default provider set to llamacpp${requested ? ` (${requested})` : ""}.`);
    return;
  }

  if (!requested) fail("usage: vitriol caduceus use <model-id> [--local]");
  if (!existsSync(BIN)) fail(`Caduceus gateway not installed at ${BIN}; run: vitriol caduceus setup`);
  await startServer();

  const env = readEnv();
  const apiKey = resolveInferenceKey(env) || process.env.OFFICINA_CADUCEUS_API_KEY || "";
  if (!apiKey) fail("missing inference key; run: vitriol caduceus setup");
  const catalog = await fetchModels(apiKey);
  if (!catalog.ok) fail(`catalog fetch failed with HTTP ${catalog.status}`);

  const model = catalog.data.find((entry) => entry?.id === requested);
  if (!model) {
    const suggestions = catalog.data
      .map((entry) => String(entry?.id ?? ""))
      .filter((id) => id.toLowerCase().includes(requested.toLowerCase()))
      .slice(0, 10);
    fail(
      `unknown model: ${requested}${suggestions.length > 0 ? `\nDid you mean:\n  ${suggestions.join("\n  ")}` : ""}`,
    );
  }

  settings.defaultProvider = "caduceus";
  settings.defaultModel = model.id;
  writeJsonAtomic(PI_SETTINGS, settings);
  chmodSync(PI_SETTINGS, 0o600);

  if (flags.json) {
    log(JSON.stringify({ provider: "caduceus", model: model.id, settings: PI_SETTINGS }, null, 2));
  } else {
    log(`Default model set to caduceus/${model.id}`);
    log(`Settings: ${PI_SETTINGS}`);
    log("Next: vitriol officina");
  }
}


function configValue(key) {
  const env = readEnv();
  const fromFile = env.get(CONFIG_KEYS[key].env);
  const fromProcess = process.env[CONFIG_KEYS[key].env];
  return fromProcess ?? fromFile ?? "";
}

function cmdConfig(flags) {
  const sub = flags._[0] || "get";
  if (sub === "get") {
    const report = {
      cache: configValue("cache") || "auto",
      praetor: configValue("praetor") === "1" || configValue("praetor") === "on" ? "on" : "off",
      egress: readEgress().mode,
      caduceus: readEnv().get("OFFICINA_CADUCEUS") ?? "unset",
      envFile: ENV_FILE,
    };
    if (flags.json) {
      log(JSON.stringify(report, null, 2));
    } else {
      log(`cache:     ${report.cache}`);
      log(`praetor:   ${report.praetor}`);
      log(`egress:    ${report.egress}`);
      log(`caduceus: ${report.caduceus}`);
      log(`env file:  ${report.envFile}`);
    }
    return;
  }

  if (sub !== "set") fail(`unknown config command: ${sub}`);
  const key = flags._[1];
  const raw = flags._[2];
  if (!key || !raw) fail("usage: vitriol caduceus config set <cache|praetor> <value>");
  const spec = CONFIG_KEYS[key];
  if (!spec) fail(`unknown config key: ${key}`);

  const value = String(raw).toLowerCase();
  if (!spec.values.includes(value)) {
    fail(`${key} must be one of: ${spec.values.join(", ")}`);
  }
  const stored = spec.map ? spec.map[value] : value;
  writeEnv({ [spec.env]: stored });
  log(`${key} = ${value} (${ENV_FILE})`);
}

async function cmdCache(flags) {
  if (!existsSync(BIN)) fail(`Caduceus gateway not installed at ${BIN}; run: vitriol caduceus setup`);
  const sub = flags._[0] || "status";
  if (sub !== "status" && sub !== "enable" && sub !== "disable") {
    fail(`unknown cache command: ${sub}`);
  }
  await startServer();
  const connections = await fetchConnectionsApi();
  const targets = flags.providers
    ? String(flags.providers).split(",").map((s) => s.trim()).filter(Boolean)
    : CACHE_TARGETS;
  const targetSet = new Set(targets);
  const rows = [];
  const errors = [];

  for (const connection of connections) {
    const provider = String(connection?.provider ?? "");
    if (!targetSet.has(provider)) continue;
    const psd =
      connection.providerSpecificData && typeof connection.providerSpecificData === "object"
        ? connection.providerSpecificData
        : {};
    const cache =
      psd.cache && typeof psd.cache === "object" && !Array.isArray(psd.cache) ? psd.cache : {};
    const current = cache.supportsPromptCaching === true;
    const desired = sub === "enable" ? true : sub === "disable" ? false : current;

    if (sub !== "status" && desired !== current) {
      const res = await gatewayApi(`/api/providers/${encodeURIComponent(String(connection.id))}`, {
        method: "PUT",
        body: { providerSpecificData: { ...psd, cache: { ...cache, supportsPromptCaching: desired } } },
      });
      if (!res.ok) {
        const detail = await res.text().catch(() => "");
        errors.push({ provider, name: connection.name, status: res.status, detail });
        continue;
      }
    }
    rows.push({ provider, name: connection.name, supportsPromptCaching: desired });
  }

  if (flags.json) {
    log(JSON.stringify({ action: sub, targets, rows, errors }, null, 2));
    return;
  }
  log(`Prompt-cache override (${sub}) for ${rows.length} connection(s)`);
  for (const row of rows) {
    log(`  ${row.provider}/${row.name}: ${row.supportsPromptCaching ? "enabled" : "disabled"}`);
  }
  for (const error of errors) {
    log(`  ERROR ${error.provider}/${error.name}: HTTP ${error.status} ${String(error.detail).trim()}`);
  }
  if (rows.length === 0) log("  no matching connections");
  if (errors.length > 0) fail(`cache override failed for ${errors.length} connection(s)`);
}

function resolveAccountsPath(flags = {}) {
  const raw = typeof flags.file === "string" && flags.file.trim() ? flags.file.trim() : ACCOUNTS_FILE;
  return raw.startsWith("~/") ? join(HOME, raw.slice(2)) : raw;
}

function loadAccountConfig(flags = {}) {
  const path = resolveAccountsPath(flags);
  if (!existsSync(path)) {
    fail(`accounts config not found at ${path}; run: vitriol caduceus accounts init`);
  }
  let parsed;
  try {
    parsed = JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    fail(`cannot read accounts config ${path}: ${error instanceof Error ? error.message : String(error)}`);
  }
  const normalized = normalizeAccountsConfig(parsed);
  if (normalized.errors.length > 0) fail(`invalid accounts config: ${normalized.errors.join("; ")}`);
  return { ...normalized, path };
}

function loadProviderCatalog() {
  const output = runCli(["--base-url", ORIGIN, "providers", "available", "--json"], {
    timeout: 60_000,
  });
  return catalogMap(parseCliJson(output));
}

function printAccountIssues(payload) {
  for (const error of payload.errors) {
    log(`  ERROR ${error.provider || "?"}/${error.name || "?"}: ${error.reason}`);
  }
  const quiet = new Set(["disabled", "filtered"]);
  for (const skip of payload.skipped) {
    if (!quiet.has(skip.reason)) log(`  SKIP  ${skip.provider}/${skip.name}: ${skip.reason}`);
  }
  const disabled = payload.skipped.filter((skip) => skip.reason === "disabled").length;
  if (disabled > 0) log(`  SKIP  ${disabled} disabled entry(s)`);
}

function printImportSummary(summary) {
  log(
    `Import: created=${summary.counts.created} skipped_existing=${summary.counts.skipped_existing} failed=${summary.counts.failed} other=${summary.counts.other}`,
  );
  for (const failure of summary.failed) {
    log(`  FAILED ${failure.provider}/${failure.name}: ${failure.error}`);
  }
}

function accountsTemplate() {
  return {
    accounts: [
      {
        provider: "opencode-go",
        name: "opencode-go",
        credential_env: "OPENCODE_API_KEY",
      },
    ],
    oauth: ["claude-code", "codex", "copilot"],
  };
}

function cmdAccountsInit(flags) {
  const path = resolveAccountsPath(flags);
  if (existsSync(path) && !flags.force) {
    log(`Accounts config already exists: ${path} (use --force to overwrite)`);
    return;
  }
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  const text = `${JSON.stringify(accountsTemplate(), null, 2)}\n`;
  const tmp = `${path}.tmp-${process.pid}`;
  writeFileSync(tmp, text, { mode: 0o600 });
  chmodSync(tmp, 0o600);
  renameSync(tmp, path);
  chmodSync(path, 0o600);
  log(`Wrote accounts template: ${path}`);
  log("Add provider entries with credential_env names, then run: vitriol caduceus accounts sync");
}


function readOpenCodeAuth() {
  if (!existsSync(OPENCODE_AUTH_FILE)) return null;
  try {
    return JSON.parse(readFileSync(OPENCODE_AUTH_FILE, "utf8"));
  } catch {
    return null;
  }
}

async function loadConfiguredConnections() {
  if (!(await serverUp())) return [];
  const parsed = parseCliJson(
    runCli(["--base-url", ORIGIN, "providers", "list", "--json"], { timeout: 30_000 }),
  );
  return Array.isArray(parsed) ? parsed : parsed?.providers ?? parsed?.connections ?? [];
}

function identitiesFromConnections(connections) {
  return new Set(
    connections.map((connection) => {
      const provider = String(connection?.provider ?? "").toLowerCase();
      const name = String(connection?.name || connection?.provider || "").toLowerCase();
      return `${provider}\0${name}`;
    }),
  );
}

function providersFromConnections(connections) {
  return new Set(
    connections
      .map((connection) => String(connection?.provider ?? "").trim().toLowerCase())
      .filter(Boolean),
  );
}

async function importProviderEntries(entries, opts = {}) {
  const dir = mkdtempSync(join(tmpdir(), "vitriol-caduceus-"));
  const importFile = join(dir, "providers.json");
  try {
    chmodSync(dir, 0o700);
    writeFileSync(importFile, `${JSON.stringify(entries, null, 2)}\n`, { mode: 0o600 });
    chmodSync(importFile, 0o600);

    const args = [
      "--base-url",
      ORIGIN,
      "providers",
      "import",
      importFile,
      "--continue-on-error",
      "--json",
    ];
    if (opts.dryRun) args.push("--dry-run");

    const result = spawnSync(BIN, args, { encoding: "utf8", timeout: 180_000 });
    let parsed;
    try {
      parsed = parseCliJson(result.stdout);
    } catch {
      parsed = null;
    }
    if (!parsed) {
      const detail = stripAnsi(result.stderr || result.stdout || "").trim();
      fail(`providers import failed (exit ${result.status})${detail ? `: ${detail}` : ""}`);
    }
    const results = Array.isArray(parsed) ? parsed : parsed?.results ?? [];
    return summarizeImportResults(results);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

function summarizeFreeOnboarding(results) {
  const counts = { created: 0, skipped: 0, failed: 0 };
  const failed = [];
  for (const result of Array.isArray(results) ? results : []) {
    if (result?.status === "created") counts.created += 1;
    else if (result?.status === "skipped") counts.skipped += 1;
    else {
      counts.failed += 1;
      failed.push({
        provider: typeof result?.providerId === "string" ? result.providerId : "",
        error: typeof result?.reason === "string" ? result.reason : "free onboarding failed",
      });
    }
  }
  return { counts, failed };
}

async function fetchFreeOnboardingCandidates() {
  const res = await gatewayApi("/api/providers/free-onboarding");
  if (!res.ok) {
    const detail = await res.text().catch(() => "");
    fail(`free onboarding list failed (HTTP ${res.status})${detail ? `: ${detail}` : ""}`);
  }
  const body = await res.json();
  return Array.isArray(body?.providers) ? body.providers : [];
}

async function applyFreeOnboarding(ids) {
  const res = await gatewayApi("/api/providers/free-onboarding", {
    method: "POST",
    body: { providerIds: ids, confirmed: true },
  });
  if (!res.ok) {
    const detail = await res.text().catch(() => "");
    fail(`free onboarding apply failed (HTTP ${res.status})${detail ? `: ${detail}` : ""}`);
  }
  const body = await res.json();
  return Array.isArray(body?.results) ? body.results : [];
}

function freePlanRows(catalog) {
  return [...catalog.values()]
    .filter((provider) => provider?.hasFree === true && provider?.category === "api-key")
    .map((provider) => {
      let env = `${String(provider.id).toUpperCase().replace(/[^A-Z0-9]+/g, "_")}_API_KEY`;
      if (!/^[A-Za-z_]/.test(env)) env = `CADUCEUS_${env}`;
      return {
        provider: provider.id,
        name: provider.name ?? provider.id,
        website: provider.website ?? "",
        credential_env: env,
        freeNote: provider.freeNote ?? "",
        authHint: provider.authHint ?? "",
      };
    })
    .sort((a, b) => a.provider.localeCompare(b.provider));
}

function readSignupProgress() {
  try {
    const parsed = JSON.parse(readFileSync(SIGNUP_PROGRESS, "utf8"));
    return parsed && typeof parsed === "object" && parsed.providers ? parsed : { providers: {} };
  } catch {
    return { providers: {} };
  }
}

function saveSignupProgress(progress) {
  progress.updatedAt = new Date().toISOString();
  writeJsonAtomic(SIGNUP_PROGRESS, progress);
  chmodSync(SIGNUP_PROGRESS, 0o600);
}

function writeEnvKey(key, value) {
  const old = existsSync(ENV_FILE) ? readFileSync(ENV_FILE, "utf8") : "";
  const next = upsertEnvFile(old, { [key]: value });
  mkdirSync(dirname(ENV_FILE), { recursive: true, mode: 0o700 });
  const tmp = `${ENV_FILE}.signup-${process.pid}`;
  writeFileSync(tmp, next, { mode: 0o600 });
  chmodSync(tmp, 0o600);
  renameSync(tmp, ENV_FILE);
  chmodSync(ENV_FILE, 0o600);
}

function ensureSignupAccounts(plans, path = ACCOUNTS_FILE) {
  let doc = existsSync(path)
    ? JSON.parse(readFileSync(path, "utf8"))
    : buildFreeAccountTemplate(plans);
  if (!Array.isArray(doc.accounts)) doc.accounts = [];
  const seen = new Set(doc.accounts.map((a) => String(a?.provider ?? "").toLowerCase()));
  for (const plan of plans) {
    if (seen.has(plan.provider.toLowerCase())) continue;
    doc.accounts.push({
      provider: plan.provider,
      name: plan.name,
      credential_env: plan.credential_env,
      website: plan.website,
      enabled: false,
    });
    seen.add(plan.provider.toLowerCase());
  }
  writeJsonAtomic(path, doc);
  chmodSync(path, 0o600);
  return doc;
}

function enableSignupAccount(plan, path = ACCOUNTS_FILE) {
  const doc = JSON.parse(readFileSync(path, "utf8"));
  if (!Array.isArray(doc.accounts)) doc.accounts = [];
  let entry = doc.accounts.find(
    (a) => String(a?.provider ?? "").toLowerCase() === plan.provider.toLowerCase(),
  );
  if (!entry) {
    entry = {
      provider: plan.provider,
      name: plan.name,
      credential_env: plan.credential_env,
      website: plan.website,
    };
    doc.accounts.push(entry);
  }
  entry.credential_env = plan.credential_env;
  entry.enabled = true;
  writeJsonAtomic(path, doc);
  chmodSync(path, 0o600);
}

async function promptHidden(label) {
  if (!process.stdin.isTTY) return promptLine(label);
  process.stdout.write(label);
  return new Promise((resolve, reject) => {
    const stdin = process.stdin;
    const wasRaw = stdin.isRaw;
    let buffer = "";
    const cleanup = () => {
      stdin.off("data", onData);
      try {
        stdin.setRawMode(Boolean(wasRaw));
      } catch {
        // terminal already gone
      }
      if (!wasRaw) stdin.pause();
      process.stdout.write("\n");
    };
    const onData = (chunk) => {
      for (const ch of String(chunk)) {
        if (ch === "\r" || ch === "\n") {
          cleanup();
          resolve(buffer.trim());
          return;
        }
        if (ch === "\u0003") {
          cleanup();
          reject(new Error("cancelled"));
          return;
        }
        if (ch === "\u007f" || ch === "\b") {
          buffer = buffer.slice(0, -1);
          continue;
        }
        if (ch >= " ") buffer += ch;
      }
    };
    stdin.setRawMode(true);
    stdin.resume();
    stdin.setEncoding("utf8");
    stdin.on("data", onData);
  });
}

async function syncSignupProvider(plan, catalog) {
  const loaded = loadAccountConfig({ file: ACCOUNTS_FILE });
  const payload = buildImportPayload(loaded, catalog, mergedEnv(), { only: plan.provider });
  if (payload.errors.length > 0) {
    return { ok: false, detail: payload.errors.map((e) => e.reason).join(", ") };
  }
  if (payload.entries.length === 0) {
    return { ok: false, detail: "no importable account (missing env?)" };
  }
  const summary = await importProviderEntries(payload.entries);
  return {
    ok: summary.counts.failed === 0,
    detail: `created=${summary.counts.created} existing=${summary.counts.skipped_existing} failed=${summary.counts.failed}`,
  };
}

async function cmdSignup(flags) {
  if (flags.reset) {
    rmSync(SIGNUP_PROGRESS, { force: true });
    log("Signup progress reset.");
    return;
  }
  if (!existsSync(BIN)) fail(`Caduceus gateway not installed at ${BIN}; run: vitriol caduceus setup`);
  await startServer();

  const catalog = loadProviderCatalog();
  const allPlans = freePlanRows(catalog);
  const only = typeof flags.only === "string"
    ? new Set(flags.only.split(",").map((s) => s.trim().toLowerCase()).filter(Boolean))
    : null;
  let plans = only ? allPlans.filter((p) => only.has(p.provider.toLowerCase())) : allPlans;
  if (typeof flags["start-at"] === "string") {
    const start = flags["start-at"].trim().toLowerCase();
    const idx = plans.findIndex((p) => p.provider.toLowerCase() === start);
    if (idx > 0) plans = plans.slice(idx);
  }
  const progress = readSignupProgress();
  progress.providers ??= {};

  if (flags.status || flags.json) {
    const counts = { keyed: 0, skipped: 0, failed: 0, pending: 0 };
    for (const plan of allPlans) {
      const status = progress.providers[plan.provider]?.status ?? "pending";
      counts[status] = (counts[status] ?? 0) + 1;
    }
    if (flags.json) {
      log(JSON.stringify({ counts, progress }, null, 2));
    } else {
      log(`Signup progress: keyed ${counts.keyed} · skipped ${counts.skipped} · failed ${counts.failed} · pending ${counts.pending}`);
      for (const [provider, entry] of Object.entries(progress.providers)) {
        log(`  ${provider}\t${entry.status}${entry.detail ? `\t${entry.detail}` : ""}`);
      }
    }
    return;
  }

  ensureSignupAccounts(allPlans);
  log("Caduceus signup wizard — manual signup, one account per provider.");
  log("Nothing is submitted automatically; keys are written only after you type them.");
  log(`Providers: ${plans.length}. Enter=next, k=paste key, s=skip, o=reopen, q=quit.`);
  if (!process.stdin.isTTY) log("warning: stdin is not a TTY; key input may echo.");

  let createdSince = 0;
  for (let i = 0; i < plans.length; i++) {
    const plan = plans[i];
    const prior = progress.providers[plan.provider]?.status;
    if (!flags.redo && (prior === "keyed" || prior === "skipped")) continue;
    const url = plan.website || "";
    log("");
    log(`[${i + 1}/${plans.length}] ${plan.provider} — ${plan.name}`);
    if (plan.freeNote) log(`  ${plan.freeNote}`);
    if (url) log(`  signup: ${url}`);
    log(`  env:    ${plan.credential_env}`);
    if (url && !flags["no-open"]) openOAuthBrowser(url);

    let answered = false;
    while (!answered) {
      const action = await promptLine("[Enter] next · k key · s skip · o reopen · q quit › ");
      const cmd = action.toLowerCase();
      if (cmd === "q" || cmd === "quit") {
        saveSignupProgress(progress);
        log(`Stopped. ${createdSince} key(s) added this run.`);
        return;
      }
      if (cmd === "" || cmd === "n" || cmd === "next") {
        answered = true;
        continue;
      }
      if (cmd === "o" || cmd === "open") {
        if (url) openOAuthBrowser(url);
        continue;
      }
      if (cmd === "s" || cmd === "skip") {
        progress.providers[plan.provider] = { status: "skipped", updatedAt: new Date().toISOString() };
        saveSignupProgress(progress);
        answered = true;
        continue;
      }
      let key = "";
      if (cmd === "k" || cmd === "key") {
        try {
          key = await promptHidden("API key (hidden): ");
        } catch {
          log("  cancelled.");
          answered = true;
          continue;
        }
      } else if (action.length >= 8 && !/^[a-z]+$/.test(action)) {
        // Low-friction: pasting the key directly at the action prompt works.
        key = action;
      } else {
        log("  unknown action; use Enter, k, s, o, or q.");
        continue;
      }
      if (!key) {
        log("  empty key; try again.");
        continue;
      }
      writeEnvKey(plan.credential_env, key);
      process.env[plan.credential_env] = key;
      enableSignupAccount(plan);
      if (flags["no-sync"]) {
        progress.providers[plan.provider] = { status: "keyed", detail: "saved (sync skipped)", updatedAt: new Date().toISOString() };
        saveSignupProgress(progress);
        createdSince += 1;
        log(`  saved ${plan.credential_env} (sync skipped)`);
        answered = true;
        continue;
      }
      let result;
      try {
        result = await syncSignupProvider(plan, catalog);
      } catch (error) {
        result = { ok: false, detail: error instanceof Error ? error.message : String(error) };
      }
      if (result.ok) {
        progress.providers[plan.provider] = { status: "keyed", detail: result.detail, updatedAt: new Date().toISOString() };
        createdSince += 1;
        log(`  ✓ synced (${result.detail})`);
      } else {
        progress.providers[plan.provider] = { status: "failed", detail: result.detail, updatedAt: new Date().toISOString() };
        log(`  ✗ sync failed: ${result.detail}`);
        log("  key is saved in the env file; re-run signup to retry, or check the key.");
      }
      saveSignupProgress(progress);
      answered = true;
    }
  }
  saveSignupProgress(progress);
  log(`\nDone. ${createdSince} key(s) added this run.`);
  log(`Progress: ${SIGNUP_PROGRESS}`);
  log("Next: restart Officina so the Caduceus catalog/router sees the new providers.");
}

async function cmdAccountsFree(flags) {
  if (!existsSync(BIN)) fail(`Caduceus gateway not installed at ${BIN}; run: vitriol caduceus setup`);

  const catalog = loadProviderCatalog();
  if (flags.template) {
    const plans = freePlanRows(catalog);
    const path = resolveAccountsPath(flags);
    if (existsSync(path) && !flags.force) {
      log(`Accounts config already exists: ${path} (use --force to overwrite)`);
      return;
    }
    const template = buildFreeAccountTemplate(plans);
    writeJsonAtomic(path, template);
    chmodSync(path, 0o600);
    log(`Wrote ${plans.length} disabled free-tier entries to ${path}`);
    log("Set enabled=true for chosen providers, add their env vars, then run accounts sync.");
    return;
  }

  if (flags.plans) {
    const plans = freePlanRows(catalog);
    if (flags.json) {
      log(JSON.stringify({ plans }, null, 2));
      return;
    }
    log(`Free-tier API-key providers: ${plans.length} (signup required)`);
    for (const plan of plans) {
      log(`  ${plan.provider}\t${plan.credential_env}\t${plan.website}`);
    }
    log("Add chosen entries to ~/.vitriol/officina/accounts.json, then run accounts sync.");
    return;
  }
  await startServer();
  const connections = await loadConfiguredConnections();
  const configuredIdentities = identitiesFromConnections(connections);
  const configuredProviders = providersFromConnections(connections);

  const freeProviders = (await fetchFreeOnboardingCandidates()).filter(
    (provider) => !FREE_ONBOARDING_EXCLUDE.has(provider.id),
  );
  const optional = flags["no-optional"]
    ? { entries: [], skipped: [], errors: [] }
    : selectOptionalFreeProviders(catalog, configuredProviders);

  let discovered = { entries: [], skipped: [], errors: [] };
  if (!flags["no-existing-auth"]) {
    const auth = readOpenCodeAuth();
    if (auth) discovered = mapOpenCodeAuth(auth, catalog, configuredIdentities);
  }

  const plannedEntries = [...optional.entries, ...discovered.entries];
  const human = !flags.json;
  const apply = Boolean(flags.apply) && !flags["dry-run"];

  if (human) {
    log(`Free onboarding candidates: ${freeProviders.length}`);
    for (const provider of freeProviders) {
      log(`  ${provider.id}${provider.defaultModel ? ` (${provider.defaultModel})` : ""}`);
    }
    printAccountIssues(optional);
    printAccountIssues(discovered);
    if (discovered.entries.length > 0) {
      log(
        `Existing OpenCode credentials: ${discovered.entries.map((entry) => entry.provider).join(", ")}`,
      );
    }
    if (!apply) log("Dry run. Re-run with --apply to add these providers.");
  }

  if (!apply) {
    if (flags.json) {
      log(
        JSON.stringify(
          {
            freeOnboarding: freeProviders.map((provider) => ({
              provider: provider.id,
              name: provider.name,
              defaultModel: provider.defaultModel,
              website: provider.website,
            })),
            optional: {
              planned: optional.entries.map((entry) => entry.provider),
              skipped: optional.skipped,
              errors: optional.errors,
            },
            existingAuth: {
              planned: discovered.entries.map((entry) => ({
                provider: entry.provider,
                name: entry.name,
              })),
              skipped: discovered.skipped,
              errors: discovered.errors,
            },
          },
          null,
          2,
        ),
      );
    }
    return;
  }

  if (!flags["no-browser"]) {
    ensureBrowserRuntime();
  }

  let onboarding = { counts: { created: 0, skipped: 0, failed: 0 }, failed: [] };
  if (freeProviders.length > 0) {
    onboarding = summarizeFreeOnboarding(
      await applyFreeOnboarding(freeProviders.map((provider) => provider.id)),
    );
  }

  let imported = summarizeImportResults([]);
  if (plannedEntries.length > 0) {
    imported = await importProviderEntries(plannedEntries);
  }

  if (human) {
    log(
      `Free onboarding: created=${onboarding.counts.created} skipped=${onboarding.counts.skipped} failed=${onboarding.counts.failed}`,
    );
    for (const failure of onboarding.failed) log(`  FAILED ${failure.provider}: ${failure.error}`);
    if (plannedEntries.length > 0) {
      log(`Existing/optional imports: ${plannedEntries.length} planned`);
      printImportSummary(imported);
    }
  }

  if (flags.json) {
    log(
      JSON.stringify(
        {
          applied: true,
          freeOnboarding: onboarding,
          optional: {
            planned: optional.entries.map((entry) => entry.provider),
            skipped: optional.skipped,
            errors: optional.errors,
          },
          existingAuth: {
            planned: discovered.entries.map((entry) => ({
              provider: entry.provider,
              name: entry.name,
            })),
            skipped: discovered.skipped,
            errors: discovered.errors,
          },
          import: imported,
        },
        null,
        2,
      ),
    );
  }

  if (flags.strict && (onboarding.counts.failed > 0 || imported.counts.failed > 0)) {
    fail("strict mode: one or more free providers failed");
  }
}

async function cmdAccountsDiscover(flags) {
  if (!existsSync(BIN)) fail(`Caduceus gateway not installed at ${BIN}; run: vitriol caduceus setup`);
  const auth = readOpenCodeAuth();
  if (!auth) fail(`OpenCode auth file not found or unreadable: ${OPENCODE_AUTH_FILE}`);

  const catalog = loadProviderCatalog();
  await startServer();
  const connections = await loadConfiguredConnections();
  const result = mapOpenCodeAuth(auth, catalog, identitiesFromConnections(connections));
  const human = !flags.json;
  const apply = Boolean(flags.apply) && !flags["dry-run"];

  if (human) {
    log(`OpenCode auth: ${OPENCODE_AUTH_FILE}`);
    printAccountIssues(result);
    if (!apply) log("Dry run. Re-run with --apply to import these credentials.");
  }

  if (result.entries.length === 0 || !apply) {
    if (flags.json) {
      log(
        JSON.stringify(
          {
            source: OPENCODE_AUTH_FILE,
            applied: false,
            planned: result.entries.map((entry) => ({
              provider: entry.provider,
              name: entry.name,
            })),
            skipped: result.skipped,
            errors: result.errors,
          },
          null,
          2,
        ),
      );
    }
    return;
  }

  const summary = await importProviderEntries(result.entries);
  if (human) printImportSummary(summary);
  if (flags.json) {
    log(
      JSON.stringify(
        {
          source: OPENCODE_AUTH_FILE,
          applied: true,
          planned: result.entries.map((entry) => ({
            provider: entry.provider,
            name: entry.name,
          })),
          skipped: result.skipped,
          errors: result.errors,
          import: summary,
        },
        null,
        2,
      ),
    );
  }
  if (flags.strict && summary.counts.failed > 0) fail("strict mode: import failures");
}

async function cmdAccountsSync(flags) {
  if (flags.only === true) fail("--only needs a value");
  if (!existsSync(BIN)) fail(`Caduceus gateway not installed at ${BIN}; run: vitriol caduceus setup`);

  const loaded = loadAccountConfig(flags);
  const catalog = loadProviderCatalog();
  const payload = buildImportPayload(loaded, catalog, mergedEnv(), {
    only: flags.only,
    strict: Boolean(flags.strict),
  });
  const human = !flags.json;

  if (human) {
    log(`Accounts config: ${loaded.path}`);
    printAccountIssues(payload);
  }

  if (payload.errors.length > 0 && flags.strict) {
    fail(`strict mode: ${payload.errors.length} account error(s)`);
  }
  if (payload.entries.length === 0) {
    if (human) {
      log("No importable accounts. Set the referenced env vars or fix the config.");
    } else {
      log(
        JSON.stringify(
          {
            config: loaded.path,
            dryRun: Boolean(flags["dry-run"]),
            planned: [],
            skipped: payload.skipped,
            errors: payload.errors,
            import: summarizeImportResults([]),
          },
          null,
          2,
        ),
      );
    }
    return;
  }

  await startServer();
  const dir = mkdtempSync(join(tmpdir(), "vitriol-caduceus-"));
  const importFile = join(dir, "providers.json");
  try {
    chmodSync(dir, 0o700);
    writeFileSync(importFile, `${JSON.stringify(payload.entries, null, 2)}\n`, { mode: 0o600 });
    chmodSync(importFile, 0o600);

    const args = [
      "--base-url",
      ORIGIN,
      "providers",
      "import",
      importFile,
      "--continue-on-error",
      "--json",
    ];
    if (flags["dry-run"]) args.push("--dry-run");

    const result = spawnSync(BIN, args, { encoding: "utf8", timeout: 180_000 });
    let parsed;
    try {
      parsed = parseCliJson(result.stdout);
    } catch {
      parsed = null;
    }
    if (!parsed) {
      const detail = stripAnsi(result.stderr || result.stdout || "").trim();
      fail(`providers import failed (exit ${result.status})${detail ? `: ${detail}` : ""}`);
    }

    const results = Array.isArray(parsed) ? parsed : parsed?.results ?? [];
    const summary = summarizeImportResults(results);
    if (human) {
      log(`Planned: ${payload.entries.length} account(s)${flags["dry-run"] ? " (dry run)" : ""}`);
      printImportSummary(summary);
    }

    if (flags.json) {
      log(
        JSON.stringify(
          {
            config: loaded.path,
            dryRun: Boolean(flags["dry-run"]),
            planned: payload.entries.map((entry) => ({
              provider: entry.provider,
              name: entry.name,
              defaultModel: entry.defaultModel,
              priority: entry.priority,
            })),
            skipped: payload.skipped,
            errors: payload.errors,
            import: summary,
          },
          null,
          2,
        ),
      );
    }

    if (flags.strict && summary.counts.failed > 0) {
      fail(`strict mode: ${summary.counts.failed} import failure(s)`);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

function accountEnvNames(config) {
  return new Set(
    (Array.isArray(config?.accounts) ? config.accounts : [])
      .map((account) =>
        String(
          account?.credential_env ?? account?.credentialEnv ?? account?.api_key_env ?? account?.apiKeyEnv ?? "",
        ).trim(),
      )
      .filter(Boolean),
  );
}

async function syncEnabledAccounts(accountsPath) {
  const loaded = loadAccountConfig({ file: accountsPath });
  const catalog = loadProviderCatalog();
  const payload = buildImportPayload(loaded, catalog, mergedEnv(), {});
  if (payload.entries.length === 0) {
    return { summary: summarizeImportResults([]), payload };
  }
  await startServer();
  const summary = await importProviderEntries(payload.entries);
  return { summary, payload };
}

async function cmdAccountsEnable(flags) {
  const path = resolveAccountsPath(flags);
  if (!existsSync(path)) fail(`accounts config not found at ${path}; run: vitriol caduceus accounts init`);
  const config = JSON.parse(readFileSync(path, "utf8"));
  const env = mergedEnv();
  const present = new Set(
    [...accountEnvNames(config)].filter((key) => String(env[key] ?? "").trim()),
  );
  const result = enableAccountsForKeys(config, present, {
    disableMissing: Boolean(flags["disable-missing"]),
  });
  if (!flags["dry-run"]) {
    writeJsonAtomic(path, config);
    chmodSync(path, 0o600);
  }
  let synced = null;
  if (flags.sync && !flags["dry-run"]) {
    synced = await syncEnabledAccounts(path);
  }
  const report = {
    config: path,
    dryRun: Boolean(flags["dry-run"]),
    enabled: result.enabled,
    disabled: result.disabled,
    present: result.matched,
    missing: result.missing,
    sync: synced ? synced.summary : undefined,
  };
  if (flags.json) {
    log(JSON.stringify(report, null, 2));
    return;
  }
  log(`${report.dryRun ? "Would enable" : "Enabled"} ${result.enabled} account(s); ${result.matched} key(s) present; ${result.disabled} disabled.`);
  if (synced) printImportSummary(synced.summary);
  if (!flags["dry-run"] && !flags.sync) log("Next: vitriol caduceus accounts sync");
}

async function cmdImportKeys(flags) {
  const positional = (flags._ ?? []).filter((v) => v !== "import-keys");
  const file =
    typeof positional[0] === "string"
      ? positional[0]
      : typeof flags["keys-file"] === "string"
        ? flags["keys-file"]
        : "";
  if (!file) fail("usage: vitriol caduceus accounts import-keys <keys-file> [--dry-run] [--no-sync] [--json]");
  if (!existsSync(file)) fail(`keys file not found: ${file}`);
  try {
    const mode = statSync(file).mode & 0o777;
    if (mode & 0o077) log(`warning: ${file} is mode ${mode.toString(8)}; recommend chmod 600.`);
  } catch {
    // best-effort warning only
  }
  const updates = Object.fromEntries(parseEnvFile(readFileSync(file, "utf8")));
  const names = Object.keys(updates).filter((key) => /^[A-Za-z_][A-Za-z0-9_]*$/.test(key));
  if (names.length === 0) fail(`no KEY=VALUE entries found in ${file}`);
  const path = resolveAccountsPath(flags);
  if (!existsSync(path)) fail(`accounts config not found at ${path}; run: vitriol caduceus accounts init`);
  const config = JSON.parse(readFileSync(path, "utf8"));
  const result = enableAccountsForKeys(config, new Set(names), {});
  if (!flags["dry-run"]) {
    if (existsSync(ENV_FILE)) {
      const stamp = new Date().toISOString().replace(/[:.]/g, "-");
      const backup = `${ENV_FILE}.bak-${stamp}`;
      copyFileSync(ENV_FILE, backup);
      chmodSync(backup, 0o600);
    }
    writeEnv(updates);
    writeJsonAtomic(path, config);
    chmodSync(path, 0o600);
  }
  let synced = null;
  if (!flags["no-sync"] && !flags["dry-run"]) {
    synced = await syncEnabledAccounts(path);
  }
  const report = {
    keysFile: file,
    dryRun: Boolean(flags["dry-run"]),
    imported: names.length,
    matchedAccounts: result.matched,
    unmatchedKeys: result.unmatched,
    synced: synced ? synced.summary : undefined,
    config: path,
  };
  if (flags.json) {
    log(JSON.stringify(report, null, 2));
    return;
  }
  log(`${report.dryRun ? "Would import" : "Imported"} ${names.length} key(s) from ${file}; ${result.matched} matched account(s).`);
  if (result.unmatched.length > 0) log(`  unmatched env names: ${result.unmatched.join(", ")}`);
  if (synced) printImportSummary(synced.summary);
  if (!flags["dry-run"] && flags["no-sync"]) log("Next: vitriol caduceus accounts sync");
}

function readKeepalive() {
  try {
    return JSON.parse(readFileSync(DAHL_KEEPALIVE_FILE, "utf8"));
  } catch {
    return null;
  }
}

function keepaliveStatus() {
  const record = readKeepalive();
  const env = mergedEnv();
  const key = String(env.DAHL_API_KEY ?? "").trim();
  const last = record?.lastAt ? Date.parse(record.lastAt) : 0;
  const ageMs = last ? Date.now() - last : null;
  return {
    keyPresent: Boolean(key),
    keyMasked: key ? maskSecret(key) : null,
    lastAt: record?.lastAt ?? null,
    ageDays: ageMs == null ? null : ageMs / 86_400_000,
    model: record?.model ?? null,
    endpoint: DAHL_BASE_URL,
    timerInstalled: existsSync(DAHL_KEEPALIVE_TIMER),
  };
}

async function cmdKeepalive(flags) {
  if (flags.status) {
    const status = keepaliveStatus();
    if (flags.json) {
      log(JSON.stringify(status, null, 2));
    } else {
      log(`Dahl keepalive: ${status.keyPresent ? `key set (${status.keyMasked})` : "DAHL_API_KEY missing"}`);
      log(`  endpoint: ${status.endpoint}`);
      log(`  last request: ${status.lastAt ?? "never"}${status.ageDays != null ? ` (${status.ageDays.toFixed(1)}d ago)` : ""}`);
      log(`  timer: ${status.timerInstalled ? "installed" : "not installed"}`);
    }
    return;
  }

  if (flags.remove) {
    systemctlUser(["disable", "--now", "vitriol-dahl-keepalive.timer"]);
    rmSync(DAHL_KEEPALIVE_TIMER, { force: true });
    rmSync(DAHL_KEEPALIVE_SERVICE, { force: true });
    systemctlUser(["daemon-reload"]);
    log("Dahl keepalive timer removed.");
    return;
  }

  if (flags.install) {
    const node = process.execPath;
    const script = process.argv[1] ? resolve(process.argv[1]) : "";
    if (!script) fail("cannot resolve the caduceus script path for the timer");
    mkdirSync(join(HOME, ".config", "systemd", "user"), { recursive: true });
    const service = `[Unit]
Description=Caduceus Dahl keepalive
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
ExecStart="${node}" "${script}" keepalive --quiet
`;
    const timer = `[Unit]
Description=Caduceus Dahl keepalive timer

[Timer]
OnBootSec=2min
OnUnitActiveSec=${DAHL_KEEPALIVE_DAYS}d
Persistent=true

[Install]
WantedBy=timers.target
`;
    writeFileSync(DAHL_KEEPALIVE_SERVICE, service, { mode: 0o644 });
    writeFileSync(DAHL_KEEPALIVE_TIMER, timer, { mode: 0o644 });
    systemctlUser(["daemon-reload"]);
    const enable = systemctlUser(["enable", "--now", "vitriol-dahl-keepalive.timer"]);
    if (!enable.ok) fail(`could not enable vitriol-dahl-keepalive.timer: ${enable.out}`);
    log(`Installed Dahl keepalive timer (every ${DAHL_KEEPALIVE_DAYS}d).`);
    log(`  ${DAHL_KEEPALIVE_TIMER}`);
    return;
  }

  const env = mergedEnv();
  const key = String(env.DAHL_API_KEY ?? "").trim();
  if (!key) fail("DAHL_API_KEY is missing (add it to ~/.vitriol/officina/env)");
  const model = typeof flags.model === "string" ? flags.model : process.env.DAHL_KEEPALIVE_MODEL || "MiniMaxAI/MiniMax-M2.7";
  if (flags["dry-run"]) {
    log(`Would POST ${DAHL_BASE_URL}/chat/completions · model ${model} · key ${maskSecret(key)}`);
    return;
  }
  const body = {
    model,
    messages: [{ role: "user", content: "ping" }],
    max_tokens: 1,
    temperature: 0,
    stream: false,
  };
  let status = 0;
  let detail = "";
  try {
    const res = await fetch(`${DAHL_BASE_URL}/chat/completions`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${key}` },
      body: JSON.stringify(body),
      signal: AbortSignal.timeout(30_000),
    });
    status = res.status;
    detail = (await res.text()).slice(0, 300);
    if (!res.ok) {
      throw new Error(`HTTP ${res.status}${detail ? `: ${detail}` : ""}`);
    }
    let usage = null;
    try {
      usage = JSON.parse(detail)?.usage ?? null;
    } catch {
      // keepalive still succeeded
    }
    const record = {
      lastAt: new Date().toISOString(),
      status,
      model,
      endpoint: DAHL_BASE_URL,
      usage,
    };
    writeJsonAtomic(DAHL_KEEPALIVE_FILE, record);
    chmodSync(DAHL_KEEPALIVE_FILE, 0o600);
    if (!flags.quiet) {
      log(`Dahl keepalive ok · ${model} · HTTP ${status}`);
      if (usage) log(`  usage: ${JSON.stringify(usage)}`);
    }
    if (flags.json) log(JSON.stringify(record, null, 2));
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    if (flags.json) log(JSON.stringify({ ok: false, status, error: message }, null, 2));
    else process.stderr.write(`[caduceus] Dahl keepalive failed: ${message}\n`);
    process.exitCode = 1;
  }
}

function readJsonSafe(path) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch {
    return null;
  }
}

function readRouteLedger(days = 31) {
  const rows = [];
  try {
    const names = readdirSync(ROUTE_LEDGER_DIR)
      .filter((name) => /^route-ledger-\d{8}\.jsonl$/.test(name))
      .sort()
      .slice(-days);
    for (const name of names) {
      for (const line of readFileSync(join(ROUTE_LEDGER_DIR, name), "utf8").split("\n")) {
        if (!line.trim()) continue;
        try {
          rows.push(JSON.parse(line));
        } catch {
          // torn line
        }
      }
    }
  } catch {
    // no ledger
  }
  return rows;
}

function summarizeRoutes(rows) {
  const out = {
    requests: 0,
    local: 0,
    cloud: 0,
    failures: 0,
    tokensIn: 0,
    tokensOut: 0,
    cacheRead: 0,
    cacheWrite: 0,
    byModel: {},
  };
  for (const e of rows) {
    out.requests += 1;
    if (e.tier === "local") out.local += 1;
    else out.cloud += 1;
    if (e.outcome === "error") out.failures += 1;
    out.tokensIn += Number(e.tokensIn) || 0;
    out.tokensOut += Number(e.tokensOut) || 0;
    out.cacheRead += Number(e.cacheRead) || 0;
    out.cacheWrite += Number(e.cacheWrite) || 0;
    const key = `${e.provider ?? "?"}/${e.model ?? "?"}`;
    const row = (out.byModel[key] ??= { requests: 0, tokensIn: 0, tokensOut: 0, failures: 0 });
    row.requests += 1;
    row.tokensIn += Number(e.tokensIn) || 0;
    row.tokensOut += Number(e.tokensOut) || 0;
    if (e.outcome === "error") row.failures += 1;
  }
  return out;
}

async function cmdStats(flags) {
  const rows = readRouteLedger(31);
  const today = new Date().toISOString().slice(0, 10);
  const month = today.slice(0, 7);
  const filtered = rows.filter((e) => {
    if (flags.today) return String(e.ts ?? "").slice(0, 10) === today;
    if (flags.month) return String(e.ts ?? "").slice(0, 7) === month;
    return true;
  });
  const summary = summarizeRoutes(filtered);
  if (flags.json) {
    log(JSON.stringify({ scope: flags.today ? "today" : flags.month ? "month" : "30d", ...summary }, null, 2));
    return;
  }
  log(`Route stats (${flags.today ? "today" : flags.month ? "month" : "30d"}): ${summary.requests} requests · local ${summary.local} · cloud ${summary.cloud} · ${summary.failures} failures`);
  log(`  tokens in ${summary.tokensIn} · out ${summary.tokensOut} · cache read ${summary.cacheRead} · write ${summary.cacheWrite}`);
  const top = Object.entries(summary.byModel).sort((a, b) => (b[1].tokensIn + b[1].tokensOut) - (a[1].tokensIn + a[1].tokensOut)).slice(0, 10);
  for (const [model, row] of top) {
    log(`  ${model}\t${row.requests} req\t${row.tokensIn + row.tokensOut} tok\t${row.failures} fail`);
  }
}

async function cmdPreflight(flags) {
  const checks = [];
  if (flags["start-local"]) {
    const start = spawnSync("systemctl", ["start", "vitriol-server"], { encoding: "utf8", timeout: 30_000 });
    if (start.status !== 0) log(`warning: could not start vitriol-server: ${(start.stderr || start.stdout || "").trim()}`);
  }
  const add = (name, status, detail) => checks.push({ name, status, detail });
  let local = false;
  try {
    const res = await fetch(`${ENGINE_URL}/health`, { signal: AbortSignal.timeout(1500) });
    local = res.ok;
  } catch {
    local = false;
  }
  add("local engine", local ? "ok" : "warn", local ? ENGINE_URL : `down at ${ENGINE_URL}`);
  const gateway = await serverUp();
  add("caduceus gateway", gateway ? "ok" : "fail", gateway ? ORIGIN : `down at ${ORIGIN}`);

  const env = mergedEnv();
  const accounts = readJsonSafe(ACCOUNTS_FILE);
  const accountRows = Array.isArray(accounts?.accounts) ? accounts.accounts : [];
  const present = accountRows.filter((a) => {
    const key = String(a?.credential_env ?? "").trim();
    return key && String(env[key] ?? "").trim();
  }).length;
  add("provider keys", present > 0 ? "ok" : "warn", `${present}/${accountRows.length} present`);

  let catalogAge = null;
  try {
    catalogAge = (Date.now() - statSync(CACHE_FILE).mtimeMs) / 3_600_000;
  } catch {
    // missing
  }
  add("catalog", catalogAge == null ? "warn" : catalogAge < 48 ? "ok" : "warn", catalogAge == null ? "missing" : `${catalogAge.toFixed(1)}h old`);

  const ka = keepaliveStatus();
  add("dahl keepalive", ka.lastAt && ka.ageDays < 7 ? "ok" : "warn", ka.lastAt ? `${ka.ageDays.toFixed(1)}d ago` : "never");

  let profile = "auto";
  try {
    profile = readFileSync(ROUTING_PROFILE_FILE, "utf8").trim() || "auto";
  } catch {
    // auto
  }
  add("profile", "ok", profile);

  const health = readJsonSafe(PROVIDER_HEALTH_FILE);
  const now = Date.now();
  const cooling = Object.values(health?.models ?? {}).filter((r) => Number(r?.cooldownUntil) > now).length;
  add("cooling lanes", "ok", `${cooling} cooling`);

  const failures = checks.filter((c) => c.status === "fail").length;
  const warnings = checks.filter((c) => c.status === "warn").length;
  if (flags.json) {
    log(JSON.stringify({ ok: failures === 0, failures, warnings, checks }, null, 2));
  } else {
    for (const c of checks) log(`${c.status.toUpperCase().padEnd(4)} ${c.name}: ${c.detail}`);
    log(`\n${failures} failure(s), ${warnings} warning(s)`);
  }
  if (failures > 0) process.exitCode = 1;
}

function sessionDirFor(cwd) {
  const encoded = `--${String(cwd).replace(/^\//, "").replace(/\//g, "-")}--`;
  return join(HOME, ".pi", "agent", "sessions", encoded);
}

function newestSession(dir) {
  try {
    return readdirSync(dir)
      .filter((name) => name.endsWith(".jsonl"))
      .filter((name) => !name.includes(".bak-") && !name.includes(".corrupt-") && !name.includes(".recovered-") && !name.includes(".tmp-"))
      .map((name) => ({ name, mtime: statSync(join(dir, name)).mtimeMs }))
      .sort((a, b) => b.mtime - a.mtime)[0]?.name;
  } catch {
    return undefined;
  }
}

function tuiRunning() {
  try {
    const pid = Number(readFileSync(TUI_LOCK_FILE, "utf8").trim());
    if (!Number.isInteger(pid) || pid <= 0 || pid === process.pid) return null;
    try {
      process.kill(pid, 0);
      return pid;
    } catch (error) {
      return error?.code === "EPERM" ? pid : null;
    }
  } catch {
    return null;
  }
}

function assertTuiStopped(action) {
  const pid = tuiRunning();
  if (pid) fail(`${action}: Officina TUI is running (PID ${pid}); exit it first`);
}

function inspectSessionFile(file) {
  const health = {
    file,
    valid: false,
    entries: 0,
    dangling: 0,
    duplicateIds: 0,
    missingParents: 0,
    badCompactions: 0,
    reason: "",
  };
  let entries;
  try {
    entries = readFileSync(file, "utf8").split("\n").filter(Boolean).map((line) => JSON.parse(line));
  } catch (error) {
    health.reason = `invalid JSONL: ${error instanceof Error ? error.message : String(error)}`;
    return health;
  }
  health.entries = entries.length;
  if (entries[0]?.type !== "session") {
    health.reason = "missing session header";
    return health;
  }
  const byId = new Map();
  for (const entry of entries) {
    if (!entry?.id) continue;
    if (byId.has(entry.id)) health.duplicateIds += 1;
    else byId.set(entry.id, entry);
  }
  if (health.duplicateIds > 0) {
    health.reason = `${health.duplicateIds} duplicate entry id(s)`;
    return health;
  }
  let cursor = entries[entries.length - 1]?.id;
  const path = [];
  const seen = new Set();
  while (cursor) {
    if (seen.has(cursor)) {
      health.reason = "cycle in parent chain";
      return health;
    }
    seen.add(cursor);
    const entry = byId.get(cursor);
    if (!entry) {
      health.missingParents += 1;
      health.reason = `missing parent entry ${cursor}`;
      return health;
    }
    path.push(entry);
    cursor = entry.parentId;
  }
  path.reverse();
  const results = new Set(
    path
      .map((e) => e?.message)
      .filter((m) => m?.role === "toolResult")
      .map((m) => m.toolCallId),
  );
  for (const entry of path) {
    const msg = entry?.message;
    if (msg?.role !== "assistant" || !Array.isArray(msg.content)) continue;
    for (const part of msg.content) {
      if (part?.type === "toolCall" && part.id && !results.has(part.id)) health.dangling += 1;
    }
  }
  for (const entry of path) {
    if (entry?.firstKeptEntryId && !byId.has(entry.firstKeptEntryId)) health.badCompactions += 1;
  }
  if (path.some((entry) => String(entry?.id ?? "").startsWith("recover-"))) {
    health.reason = "synthetic recovery entries; start a fresh session";
    return health;
  }
  if (health.dangling > 0) {
    health.reason = `${health.dangling} dangling tool call(s) on the active path`;
    return health;
  }
  if (health.badCompactions > 0) {
    health.reason = `${health.badCompactions} broken compaction reference(s)`;
    return health;
  }
  health.valid = true;
  return health;
}

async function cmdSessionDoctor(flags) {
  const cwd = typeof flags.cwd === "string" ? flags.cwd : process.cwd();
  let file = typeof flags.file === "string" ? flags.file : "";
  if (!file) {
    const dir = sessionDirFor(cwd);
    const name = newestSession(dir);
    if (!name) fail(`no session found for ${cwd}`);
    file = join(dir, name);
  }
  if (!existsSync(file)) fail(`session file not found: ${file}`);
  const health = inspectSessionFile(file);
  if (flags.json) {
    log(JSON.stringify(health, null, 2));
  } else {
    log(`Session: ${health.file}`);
    log(`  entries: ${health.entries}`);
    log(`  dangling tool calls: ${health.dangling}`);
    log(`  duplicate ids: ${health.duplicateIds}`);
    log(`  broken compactions: ${health.badCompactions}`);
    log(`  status: ${health.valid ? "OK" : "CORRUPT"}${health.reason ? ` — ${health.reason}` : ""}`);
  }
  if (!health.valid) process.exitCode = 1;
}

async function cmdRecoverSession(flags) {
  const cwd = typeof flags.cwd === "string" ? flags.cwd : process.cwd();
  let file = typeof flags.file === "string" ? flags.file : "";
  if (!file) {
    const dir = sessionDirFor(cwd);
    const name = newestSession(dir);
    if (!name) fail(`no session found for ${cwd}`);
    file = join(dir, name);
  }
  if (!existsSync(file)) fail(`session file not found: ${file}`);
  assertTuiStopped("recover-session");
  const lines = readFileSync(file, "utf8").split("\n").filter(Boolean);
  const entries = [];
  for (const line of lines) {
    try {
      entries.push(JSON.parse(line));
    } catch {
      fail(`session is not valid JSONL: ${file}`);
      return;
    }
  }
  const header = entries[0]?.type === "session" ? entries[0] : null;
  const byId = new Map(entries.filter((e) => e?.id).map((e) => [e.id, e]));
  const leafId = entries[entries.length - 1]?.id;
  const path = [];
  let cursor = leafId;
  while (cursor && byId.has(cursor)) {
    path.push(byId.get(cursor));
    cursor = byId.get(cursor).parentId;
  }
  path.reverse();
  const resultIds = new Set(
    path
      .map((e) => e?.message)
      .filter((m) => m?.role === "toolResult")
      .map((m) => m.toolCallId),
  );
  const missing = [];
  for (const entry of path) {
    const msg = entry?.message;
    for (const call of Array.isArray(msg?.content) ? msg.content : []) {
      if (call?.type === "toolCall" && call.id && !resultIds.has(call.id)) {
        missing.push({ entry, call });
      }
    }
  }
  if (missing.length === 0) {
    log("No dangling tool calls on the active path; session is already resumable.");
    return;
  }
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  const out = flags.deep ? `${file}.recovered-${stamp}.jsonl` : file;
  const synthetic = [];
  if (flags.deep) {
    let tail = null;
    for (const entry of path) {
      const cloned = { ...entry, parentId: tail };
      synthetic.push(cloned);
      tail = cloned.id;
      const msg = entry?.message;
      for (const call of Array.isArray(msg?.content) ? msg.content : []) {
        if (call?.type !== "toolCall" || !call.id || resultIds.has(call.id)) continue;
        const id = `recover-${stamp}-${call.id.slice(0, 8)}`;
        synthetic.push({
          type: "message",
          id,
          parentId: tail,
          timestamp: new Date().toISOString(),
          message: {
            role: "toolResult",
            toolCallId: call.id,
            toolName: call.name ?? "tool",
            content: [{
              type: "text",
              text: "[interrupted] This tool call was interrupted before execution. It was not executed; continue without its result unless you need to re-run it.",
            }],
            isError: true,
            timestamp: Date.now(),
          },
        });
        tail = id;
      }
    }
    if (header) synthetic.unshift(header);
  } else {
    let tail = path[path.length - 1]?.id;
    for (const { call } of missing) {
      const id = `recover-${stamp}-${call.id.slice(0, 8)}`;
      synthetic.push({
        type: "message",
        id,
        parentId: tail,
        timestamp: new Date().toISOString(),
        message: {
          role: "toolResult",
          toolCallId: call.id,
          toolName: call.name ?? "tool",
          content: [{
            type: "text",
            text: "[interrupted] Tool call was interrupted before it completed. It was not executed. Re-run it if the result is still needed.",
          }],
          isError: true,
          timestamp: Date.now(),
        },
      });
      tail = id;
    }
  }
  if (flags["dry-run"]) {
    log(`${flags.deep ? "Would linearize" : "Would append"} ${missing.length} synthetic tool result(s).`);
    if (flags.deep) log(`Output would be: ${out}`);
    return;
  }
  if (flags.deep) {
    const tmp = `${out}.tmp-${process.pid}`;
    writeFileSync(tmp, `${synthetic.map((e) => JSON.stringify(e)).join("\n")}\n`, { mode: 0o600 });
    chmodSync(tmp, 0o600);
    renameSync(tmp, out);
    log(`Recovered ${missing.length} dangling call(s) into ${out}`);
    log(`Launch with: officina --session ${out}`);
  } else {
    const backup = `${file}.bak-${stamp}`;
    copyFileSync(file, backup);
    chmodSync(backup, 0o600);
    appendFileSync(file, `${synthetic.map((e) => JSON.stringify(e)).join("\n")}\n`, { mode: 0o600 });
    chmodSync(file, 0o600);
    log(`Recovered ${missing.length} interrupted tool call(s) in ${file}`);
    log(`Backup: ${backup}`);
    log("Restart Officina to reload the session.");
  }
}

async function cmdKeys(flags) {
  const loaded = loadAccountConfig(flags);
  const fileEnv = readEnv();
  const env = mergedEnv();
  let connections = [];
  if (await serverUp()) {
    try {
      const parsed = parseCliJson(
        runCli(["--base-url", ORIGIN, "providers", "list", "--json"], { timeout: 30_000 }),
      );
      connections = Array.isArray(parsed) ? parsed : parsed?.providers ?? parsed?.connections ?? [];
    } catch {
      // key presence is still useful without link status
    }
  }
  const linked = new Set(
    connections.map((connection) => {
      const provider = String(connection?.provider ?? "").toLowerCase();
      const name = String(connection?.name || connection?.provider || "").toLowerCase();
      return `${provider}\0${name}`;
    }),
  );
  const rows = loaded.accounts.map((account) => {
    if (!account || typeof account !== "object") return null;
    const provider = String(account.provider ?? "").trim();
    const name = String(account.name ?? provider).trim();
    const key = String(
      account.credential_env ?? account.credentialEnv ?? account.api_key_env ?? account.apiKeyEnv ?? "",
    ).trim();
    if (!key) return null;
    const shellValue = process.env[key]?.trim() ?? "";
    const fileValue = String(fileEnv.get(key) ?? "").trim();
    const value = shellValue || fileValue;
    return {
      provider,
      name,
      env: key,
      present: Boolean(value),
      source: shellValue ? "shell" : fileValue ? "env-file" : "missing",
      masked: value ? maskSecret(value) : null,
      gateway: linked.has(`${provider.toLowerCase()}\0${name.toLowerCase()}`),
    };
  }).filter(Boolean);
  const object = flags.all ? rows : rows.filter((row) => row.present);
  const inference = env.OFFICINA_CADUCEUS_API_KEY?.trim() ?? "";
  const report = {
    envFile: ENV_FILE,
    inferenceKey: inference ? maskSecret(inference) : null,
    keys: object,
    summary: {
      present: rows.filter((row) => row.present).length,
      missing: rows.filter((row) => !row.present).length,
      linked: rows.filter((row) => row.gateway).length,
    },
  };
  if (flags.json) {
    log(JSON.stringify(report, null, 2));
    return;
  }
  log(`Env file: ${ENV_FILE}`);
  log(`Caduceus inference key: ${inference ? `set (${maskSecret(inference)})` : "missing"}`);
  if (object.length === 0) {
    log("No provider keys found (use --all to list missing entries).");
    return;
  }
  for (const row of object) {
    log(
      `  ${row.present ? "✓" : "·"} ${row.provider}/${row.name}  ${row.env}  ${row.masked ?? "missing"}  ${row.source}${row.gateway ? "  gateway" : ""}`,
    );
  }
  log(`Summary: ${report.summary.present} present · ${report.summary.missing} missing · ${report.summary.linked} gateway-linked`);
}

async function cmdAccountsStatus(flags) {
  const loaded = loadAccountConfig(flags);
  const catalog = loadProviderCatalog();
  const catalogCategories = new Map();
  for (const provider of catalog.values()) {
    const category = typeof provider.category === "string" ? provider.category : "unknown";
    catalogCategories.set(category, (catalogCategories.get(category) ?? 0) + 1);
  }

  const up = await serverUp();
  let connections = [];
  let connectionError = "";
  if (up) {
    try {
      const parsed = parseCliJson(
        runCli(["--base-url", ORIGIN, "providers", "list", "--json"], { timeout: 30_000 }),
      );
      connections = Array.isArray(parsed) ? parsed : parsed?.providers ?? parsed?.connections ?? [];
    } catch (error) {
      connectionError = error instanceof Error ? error.message : String(error);
    }
  }

  const fileEnv = readEnv();
  const env = mergedEnv();
  const configured = new Set(
    connections.map((connection) => {
      const provider = String(connection?.provider ?? "").toLowerCase();
      const name = String(connection?.name || connection?.provider || "").toLowerCase();
      return `${provider}\0${name}`;
    }),
  );

  const rows = loaded.accounts.map((account) => {
    if (!account || typeof account !== "object") {
      return { provider: "?", name: "?", configured: false, credential: "invalid" };
    }
    const provider = typeof account.provider === "string" ? account.provider.trim() : "";
    const name =
      typeof account.name === "string" && account.name.trim()
        ? account.name.trim()
        : provider;
    const credentialEnv =
      account.credential_env ?? account.credentialEnv ?? account.api_key_env ?? account.apiKeyEnv;
    const key = credentialEnv ? String(credentialEnv) : "";
    const shellValue = key ? process.env[key]?.trim() : "";
    const fileValue = key ? String(fileEnv.get(key) ?? "").trim() : "";
    const source = shellValue ? "shell" : fileValue ? "env-file" : "missing";
    const value = shellValue || fileValue;
    const credential = key
      ? `env:${key} (${value ? `${maskSecret(value)}, ${source}` : "missing"})`
      : account.allow_no_credential || account.allowNoCredential
        ? "no credential"
        : "missing credential_env";
    return {
      provider,
      name,
      configured: configured.has(`${provider.toLowerCase()}\0${name.toLowerCase()}`),
      credential,
      credential_env: key || null,
      credential_source: source,
      credential_masked: value ? maskSecret(value) : null,
    };
  });

  const report = {
    config: loaded.path,
    gateway: up ? "up" : "down",
    accounts: rows,
    catalog: Object.fromEntries([...catalogCategories.entries()].sort()),
    oauth: loaded.oauth,
    connectionError: connectionError || undefined,
  };

  if (flags.json) {
    log(JSON.stringify(report, null, 2));
    return;
  }

  const present = rows.filter((row) => row.credential_source !== "missing").length;
  const linked = rows.filter((row) => row.configured).length;
  log(`Accounts config: ${loaded.path}`);
  log(`Gateway: ${up ? `up (${ORIGIN})` : "down"}`);
  if (connectionError) log(`Connection read failed: ${connectionError}`);
  log(`Keys present: ${present}/${rows.length} · gateway connections matched: ${linked}`);
  for (const row of rows) {
    log(
      `  ${row.configured ? "configured" : "missing   "}  ${row.provider}/${row.name}  [${row.credential}]`,
    );
  }
  if (rows.length === 0) log("  no accounts configured in the file");
  const catalogText = [...catalogCategories.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([category, count]) => `${category}=${count}`)
    .join(" ");
  log(`Catalog: ${catalogText}`);
  if (loaded.oauth.length > 0) log(`OAuth requested: ${loaded.oauth.join(", ")}`);
}

function openOAuthBrowser(url) {
  if (process.env.OFFICINA_NO_BROWSER === "1") return;
  try {
    const child = spawn("xdg-open", [url], { detached: true, stdio: "ignore" });
    child.unref();
  } catch {
    // URL is still printed; opening is best-effort.
  }
}

async function promptLine(label) {
  const rl = createInterface({ input: process.stdin, output: process.stdout });
  try {
    return String(await rl.question(label)).trim();
  } finally {
    rl.close();
  }
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function oauthFailure(res, label) {
  const detail = await res.text().catch(() => "");
  fail(`${label} (HTTP ${res.status})${detail ? `: ${detail}` : ""}`);
}

async function oauthBrowserLogin(provider, flags) {
  const redirect = typeof flags["redirect-uri"] === "string" ? flags["redirect-uri"] : "";
  const query = redirect ? `?redirect_uri=${encodeURIComponent(redirect)}` : "";
  const res = await gatewayApi(`/api/oauth/${provider.backend}/authorize${query}`);
  if (!res.ok) await oauthFailure(res, `OAuth authorize failed for ${provider.id}`);

  const start = await res.json();
  const url = start.authUrl ?? start.authorizeUrl ?? start.url;
  if (!url) fail(`OAuth unavailable for ${provider.id}: ${start.error ?? "no authUrl returned"}`);

  if (provider.requiresEnv?.some((key) => !process.env[key])) {
    warn(`${provider.id} may need server env: ${provider.requiresEnv.join(", ")}`);
  }

  log(`\nOpen this URL to authorize:\n  ${url}\n`);
  if (!flags["no-browser"]) openOAuthBrowser(url);

  const input = await promptLine("Callback URL or code: ");
  let callback;
  try {
    callback = parseOAuthCallback(input, start.state);
  } catch (error) {
    fail(error instanceof Error ? error.message : String(error));
  }

  const body = {
    code: callback.code,
    redirectUri: start.redirectUri ?? redirect,
    state: callback.state || start.state,
  };
  if (start.codeVerifier) body.codeVerifier = start.codeVerifier;

  const exchange = await gatewayApi(`/api/oauth/${provider.backend}/exchange`, {
    method: "POST",
    body,
  });
  if (!exchange.ok) await oauthFailure(exchange, `OAuth exchange failed for ${provider.id}`);

  const result = await exchange.json().catch(() => ({}));
  const connection = result.connection ?? {};
  log(`Authorized: ${connection.email ?? connection.displayName ?? connection.id ?? provider.id}`);
}

async function oauthDeviceLogin(provider, flags) {
  const res = await gatewayApi(`/api/oauth/${provider.backend}/device-code`);
  if (!res.ok) await oauthFailure(res, `device-code failed for ${provider.id}`);

  const start = await res.json();
  const deviceCode = start.deviceCode ?? start.device_code ?? "";
  if (!deviceCode) fail(`server did not return a device code for ${provider.id}`);

  const userCode = start.userCode ?? start.user_code ?? "";
  const uri =
    start.verificationUriComplete ??
    start.verification_uri_complete ??
    start.verificationUri ??
    start.verification_uri ??
    start.authUrl ??
    start.url ??
    "";
  if (userCode) log(`\nDevice code: ${userCode}`);
  if (uri) log(`Visit: ${uri}\n`);
  if (!flags["no-browser"] && uri) openOAuthBrowser(uri);

  const timeout = Number(flags.timeout) || 300_000;
  const deadline = Date.now() + timeout;
  let interval = Math.max(1, Number(start.intervalMs ?? start.interval ?? 5)) * 1000;

  while (Date.now() < deadline) {
    await sleep(interval);
    const pollRes = await gatewayApi(`/api/oauth/${provider.backend}/poll`, {
      method: "POST",
      body: {
        deviceCode,
        ...(start.codeVerifier ? { codeVerifier: start.codeVerifier } : {}),
      },
    });
    if (!pollRes.ok) {
      const body = await pollRes.json().catch(() => null);
      if (body && body.pending !== false) continue;
      await oauthFailure(pollRes, `device authorization failed for ${provider.id}`);
    }
    const poll = await pollRes.json().catch(() => null);
    if (!poll) continue;
    if (poll.success) {
      const connection = poll.connection ?? {};
      log(`Authorized: ${connection.email ?? connection.displayName ?? connection.id ?? provider.id}`);
      return;
    }
    if (poll.error === "slow_down") {
      interval += 5_000;
      continue;
    }
    if (poll.error && !poll.pending) {
      fail(`device auth failed: ${poll.errorDescription ?? poll.error}`);
    }
  }
  fail(`device authorization timed out after ${Math.round(timeout / 1000)}s`);
}

async function oauthNativeLogin(provider, flags) {
  const nativeId = provider.id === "amazon-q" ? "kiro" : provider.id;
  if (provider.flow === "social" && !flags.social) {
    fail(`${provider.id} requires --social google|github`);
  }
  await startServer();
  const args = ["--base-url", ORIGIN, "providers", "auth", nativeId];
  if (provider.flow === "import" && !flags["no-import-system"]) args.push("--import-from-system");
  if (flags["import-from-system"] && provider.flow !== "import") args.push("--import-from-system");
  if (flags["no-browser"]) args.push("--no-browser");
  if (flags.social) args.push("--social", String(flags.social));
  if (flags.timeout) args.push("--timeout", String(flags.timeout));
  run(BIN, args, { inherit: true, timeout: 600_000 });
}

async function cmdAccountsOauth(flags) {
  const sub = flags._[1] || "list";
  const id = flags._[2];

  if (sub === "list" || sub === "help") {
    log("OAuth providers:");
    for (const provider of OAUTH_PROVIDERS) {
      const kind = provider.native ? "native" : "free  ";
      const note = provider.note ? `  (${provider.note})` : "";
      log(`  ${provider.id.padEnd(14)} ${provider.flow.padEnd(9)} ${kind}${note}`);
    }
    return;
  }

  if (!existsSync(BIN)) fail(`Caduceus gateway not installed at ${BIN}`);
  if (sub === "status") {
    if (!(await serverUp())) fail(`gateway is down at ${ORIGIN}; run: vitriol caduceus up`);
    run(BIN, ["--base-url", ORIGIN, "oauth", "status"], { inherit: true, timeout: 60_000 });
    return;
  }

  if (sub !== "login") fail(`unknown accounts oauth command: ${sub}`);
  const provider = findOAuthProvider(String(id ?? ""));
  if (!provider) {
    fail(`unsupported OAuth provider: ${id ?? "(missing)"}; try: vitriol caduceus accounts oauth list`);
  }

  if (provider.native) {
    await oauthNativeLogin(provider, flags);
    return;
  }

  await startServer();
  if (provider.flow === "browser") {
    await oauthBrowserLogin(provider, flags);
    return;
  }
  if (provider.flow === "device") {
    await oauthDeviceLogin(provider, flags);
    return;
  }
  fail(`unsupported flow ${provider.flow} for ${provider.id}`);
}

async function cmdAccounts(flags) {
  const sub = flags._[0] || "status";
  if (sub === "init") return cmdAccountsInit(flags);
  if (sub === "sync") return cmdAccountsSync(flags);
  if (sub === "status") return cmdAccountsStatus(flags);
  if (sub === "keys") return cmdKeys(flags);
  if (sub === "enable") return cmdAccountsEnable(flags);
  if (sub === "import-keys") return cmdImportKeys(flags);
  if (sub === "free") return cmdAccountsFree(flags);
  if (sub === "signup") return cmdSignup(flags);
  if (sub === "discover") return cmdAccountsDiscover(flags);
  if (sub === "oauth") return cmdAccountsOauth(flags);
  if (sub === "help") {
    printAccountsHelp();
    return;
  }
  printAccountsHelp();
  fail(`unknown accounts command: ${sub}`, 2);
}

function printAccountsHelp() {
  log(`vitriol caduceus accounts — bulk API-key import + assisted OAuth

Usage:
  vitriol caduceus accounts init [--file PATH] [--force]
  vitriol caduceus accounts sync [--file PATH] [--dry-run] [--json]
                                  [--only ID] [--strict]
  vitriol caduceus accounts status [--file PATH] [--json]
  vitriol caduceus accounts keys [--all] [--json]
  vitriol caduceus accounts enable [--sync] [--disable-missing] [--dry-run] [--json]
  vitriol caduceus accounts import-keys <keys-file> [--dry-run] [--no-sync] [--json]
  vitriol caduceus accounts free [--apply] [--json] [--strict]
                                  [--no-optional] [--no-existing-auth]
                                  [--no-browser] [--plans] [--template]
                                  [--file PATH] [--force]
  vitriol caduceus accounts signup [--only a,b] [--start-at id] [--status]
                                  [--reset] [--no-open] [--no-sync] [--redo] [--json]
  vitriol caduceus accounts discover [--apply] [--json] [--strict]
  vitriol caduceus accounts oauth list
  vitriol caduceus accounts oauth login <id> [--no-browser]
                                  [--import-from-system] [--no-import-system]
                                  [--social google|github] [--timeout MS]
  vitriol caduceus accounts oauth status

Config: ${ACCOUNTS_FILE}
Credentials are referenced by env-var name (credential_env), never stored in
the config. Supported OAuth flows: ${OAUTH_PROVIDERS.map((p) => p.id).join(", ")}.`);
}

function printHelp() {
  log(`vitriol caduceus — local Caduceus gateway for Officina

Usage:
  vitriol caduceus setup [--opencode-key-stdin] [--no-install] [--force]
                          [--free] [--no-browser]
  vitriol caduceus up
  vitriol caduceus down
  vitriol caduceus status
  vitriol caduceus sync
  vitriol caduceus accounts init|sync|status|keys|enable|import-keys
  vitriol caduceus keys [--all] [--json]   # present provider keys (masked)
  vitriol caduceus enable [--sync] [--disable-missing] [--dry-run] [--json]
  vitriol caduceus import-keys <keys-file> [--dry-run] [--no-sync] [--json]
  vitriol caduceus keepalive [--status|--install|--remove|--dry-run|--json|--quiet]
  vitriol caduceus stats [--today|--month|--json]
  vitriol caduceus preflight [--json] [--start-local]
  vitriol caduceus recover-session [--file PATH] [--cwd DIR] [--deep] [--dry-run]
  vitriol caduceus session-doctor [--latest|--file PATH] [--cwd DIR] [--json]
  vitriol caduceus accounts signup
  vitriol caduceus signup [--only a,b] [--start-at id] [--status]
                          [--reset] [--no-open] [--no-sync] [--redo] [--json]
  vitriol caduceus accounts oauth list|login|status
  vitriol caduceus cache status|enable|disable [--providers a,b] [--json]
  vitriol caduceus egress status|set|clear|test [--proxy URL] [--no-proxy LIST]
  vitriol caduceus service install|status|remove
  vitriol caduceus migrate [--dry-run|--apply] [--no-service] [--json]
  vitriol caduceus doctor [--json]
  vitriol caduceus use <model-id> [--local] [--json]
  vitriol caduceus config get [--json]
  vitriol caduceus config set cache auto|local|cloud|off
  vitriol caduceus config set praetor on|off

Setup installs the OmniRoute vendor package@${PIN} under ${VENDOR_DIR}, configures OpenCode Go,
creates a local inference key, disables compression/semantic-cache/memory, and
writes ${ENV_FILE}. Accounts sync bulk-imports API-key providers from
${ACCOUNTS_FILE}; accounts free adds zero-key free providers and reuses existing
OpenCode credentials. Cache overrides enable provider-native prompt caching on
opencode-go/glm/zai. Egress is fixed-only by policy (no quota-based rotation).`);
}

async function main() {
  const argv = process.argv.slice(2);
  const command = argv[0] || "status";
  const flags = parseFlags(argv.slice(1));

  if (command === "help" || command === "--help" || command === "-h" || flags.help) {
    printHelp();
    return;
  }
  if (command === "accounts") return cmdAccounts(parseFlags(argv.slice(1)));
  if (command === "cache") return cmdCache(parseFlags(argv.slice(1)));
  if (command === "egress") return cmdEgress(parseFlags(argv.slice(1)));
  if (command === "service") return cmdService(parseFlags(argv.slice(1)));
  if (command === "migrate") return cmdMigrate(parseFlags(argv.slice(1)));
  if (command === "signup") return cmdSignup(parseFlags(argv.slice(1)));
  if (command === "keys") return cmdKeys(parseFlags(argv.slice(1)));
  if (command === "enable") return cmdAccountsEnable(parseFlags(argv.slice(1)));
  if (command === "import-keys") return cmdImportKeys(parseFlags(argv.slice(1)));
  if (command === "keepalive") return cmdKeepalive(parseFlags(argv.slice(1)));
  if (command === "stats") return cmdStats(parseFlags(argv.slice(1)));
  if (command === "preflight") return cmdPreflight(parseFlags(argv.slice(1)));
  if (command === "recover-session") return cmdRecoverSession(parseFlags(argv.slice(1)));
  if (command === "session-doctor") return cmdSessionDoctor(parseFlags(argv.slice(1)));
  if (command === "doctor") return cmdDoctor(parseFlags(argv.slice(1)));
  if (command === "use") return cmdUse(parseFlags(argv.slice(1)));
  if (command === "config") return cmdConfig(parseFlags(argv.slice(1)));
  if (command === "setup") return setup(flags);
  if (command === "up") return cmdUp();
  if (command === "down") return cmdDown();
  if (command === "status") return cmdStatus();
  if (command === "sync") return cmdSync();
  printHelp();
  fail(`unknown command: ${command}`, 2);
}

main().catch((error) => {
  const message = error instanceof Error ? error.message : String(error);
  process.stderr.write(`[caduceus] ${message}\n`);
  process.exit(Number(error?.exitCode) || 1);
});
