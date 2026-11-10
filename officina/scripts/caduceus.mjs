#!/usr/bin/env node
// OmniRoute bootstrap for Officina. Keeps the local VITRIOL engine as the default
// provider and registers OmniRoute as an explicit OpenAI-compatible gateway.
//
// PROVENANCE: integration glue only; OmniRoute itself is MIT and installed as a
// pinned vendor package. No OmniRoute source is copied into this repo.

import { spawn, spawnSync } from "node:child_process";
import { createInterface } from "node:readline/promises";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { homedir, tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
import { parseEnvFile, systemdUnit, upsertEnvFile } from "./omniroute-lib.mjs";
import {
  findOAuthProvider,
  OAUTH_PROVIDERS,
  parseOAuthCallback,
} from "./omniroute-oauth-lib.mjs";
import {
  buildFreeAccountTemplate,
  buildImportPayload,
  catalogMap,
  mapOpenCodeAuth,
  maskSecret,
  normalizeAccountsConfig,
  parseCliJson,
  selectOptionalFreeProviders,
  stripAnsi,
  summarizeImportResults,
} from "./omniroute-accounts-lib.mjs";

const HOME = process.env.HOME || homedir();
const PIN = "3.8.53";
const PLAYWRIGHT_PIN = "1.62.1";
const PORT = 20128;
const ORIGIN = `http://127.0.0.1:${PORT}`;
const VENDOR_DIR = join(HOME, ".vitriol", "vendor", "omniroute");
const BIN = join(VENDOR_DIR, "node_modules", ".bin", "omniroute");
const API_MODULE = join(VENDOR_DIR, "node_modules", "omniroute", "bin", "cli", "api.mjs");
const ENV_FILE = join(HOME, ".vitriol", "officina", "env");
const CACHE_FILE = join(HOME, ".vitriol", "officina", "omniroute-models.json");
const ACCOUNTS_FILE = join(HOME, ".vitriol", "officina", "accounts.json");
const EGRESS_FILE = join(HOME, ".vitriol", "officina", "egress.json");
const EGRESS_ENV_FILE = join(HOME, ".vitriol", "officina", "egress.env");
const SERVICE_UNIT = join(HOME, ".config", "systemd", "user", "vitriol-omniroute.service");
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
  process.stderr.write(`[omniroute] ${message}\n`);
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
    const res = systemctlUser(["start", "vitriol-omniroute.service"]);
    if (!res.ok) fail(`could not start vitriol-omniroute.service: ${res.out}`);
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
    const res = systemctlUser(["stop", "vitriol-omniroute.service"]);
    if (!res.ok) {
      warn(`could not stop vitriol-omniroute.service: ${res.out}`);
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
  if (!hasCommand("npm")) fail("npm is required to install OmniRoute");
  log(`[omniroute] installing omniroute@${PIN} + playwright@${PLAYWRIGHT_PIN} under ${VENDOR_DIR}`);
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
  if (after !== PIN) fail(`OmniRoute install finished but version is ${after ?? "missing"}`);
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
  log("[omniroute] installing Chromium for browser-backed free providers");
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
    log(`[omniroute] provider ${PROVIDER_ID} already configured`);
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

  log(`[omniroute] adding provider ${PROVIDER_ID}`);
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
  log("[omniroute] creating local inference key for Officina");
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
  if (!key) fail("could not parse the inference key from OmniRoute");
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
    warn(`OmniRoute DB not found at ${DB_FILE}; relying on per-request headers`);
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
  return env.get("OFFICINA_OMNIROUTE_API_KEY") || "";
}

async function setup(flags) {
  if (!flags["no-install"]) installOmniRoute(Boolean(flags.force));
  else if (!existsSync(BIN)) fail(`OmniRoute not installed at ${BIN} (remove --no-install)`);

  let startedByUs = false;
  try {
    log(`[omniroute] ensuring gateway on ${ORIGIN}`);
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
      OFFICINA_OMNIROUTE: "1",
      OFFICINA_OMNIROUTE_BASE_URL: `${ORIGIN}/v1`,
      OFFICINA_OMNIROUTE_API_KEY: apiKey,
    });

    log("");
    log("OmniRoute ready.");
    log(`  base URL: ${ORIGIN}/v1`);
    log(`  models:   ${catalog.ok ? catalog.data.length : "unavailable"}`);
    log(`  env:      ${ENV_FILE}`);
    log("");
    log("Next: vitriol officina, then /model -> omniroute/<model>.");

    if (flags.free) {
      log("");
      log("[omniroute] --free: adding zero-key providers and reusing local auth");
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
  if (!existsSync(BIN)) fail(`OmniRoute not installed at ${BIN}; run: vitriol omniroute setup`);
  const started = await startServer();
  log(started ? "OmniRoute started." : "OmniRoute already running.");
}

async function cmdDown() {
  if (!existsSync(BIN)) fail(`OmniRoute not installed at ${BIN}`);
  const stopped = await stopServer();
  if (!stopped) fail("OmniRoute did not stop");
  log("OmniRoute stopped.");
}

async function cmdStatus() {
  const env = readEnv();
  const apiKey = resolveInferenceKey(env) || process.env.OFFICINA_OMNIROUTE_API_KEY || "";
  const up = await serverUp();
  const version = installedVersion();
  const catalog = up ? await fetchModels(apiKey) : { ok: false, status: 0, data: [] };
  const connections = providerConnectionCount(PROVIDER_ID);

  log(`OmniRoute ${version ?? "not installed"}`);
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
  const apiKey = resolveInferenceKey(env) || process.env.OFFICINA_OMNIROUTE_API_KEY || "";
  if (!apiKey) fail("missing OFFICINA_OMNIROUTE_API_KEY; run: vitriol omniroute setup");
  if (!(await serverUp())) fail(`gateway is down at ${ORIGIN}; run: vitriol omniroute up`);
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

async function cmdService(flags) {
  const sub = flags._[0] || "status";
  if (sub === "status") {
    const enabled = systemctlUser(["is-enabled", "vitriol-omniroute.service"]);
    const active = systemctlUser(["is-active", "vitriol-omniroute.service"]);
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
    if (!existsSync(BIN)) fail(`OmniRoute not installed at ${BIN}; run: vitriol omniroute setup`);
    mkdirSync(join(HOME, ".config", "systemd", "user"), { recursive: true });
    writeEgressEnv(readEgress());
    const unit = systemdUnit({
      bin: "%h/.vitriol/vendor/omniroute/node_modules/.bin/omniroute",
      envFile: "%h/.vitriol/officina/egress.env",
      port: PORT,
    });
    writeFileSync(SERVICE_UNIT, unit, { mode: 0o644 });
    if (await serverUp()) {
      log("[omniroute] stopping the running gateway to free the port for the user service");
      await stopServer();
    }
    const reload = systemctlUser(["daemon-reload"]);
    if (!reload.ok) fail(`systemctl --user daemon-reload failed: ${reload.out}`);
    const enable = systemctlUser(["enable", "--now", "vitriol-omniroute.service"]);
    if (!enable.ok) fail(`could not enable vitriol-omniroute.service: ${enable.out}`);
    const linger = spawnSync("loginctl", ["enable-linger", process.env.USER || ""], {
      encoding: "utf8",
      timeout: 15_000,
    });
    if (linger.status !== 0) {
      warn("loginctl enable-linger failed; the service may not start until you log in");
    }
    if (!(await waitForServer())) fail("service started but gateway is not healthy");
    log(`Installed and started ${SERVICE_UNIT}`);
    log("Next: vitriol omniroute service status");
    return;
  }

  if (sub === "remove") {
    systemctlUser(["disable", "--now", "vitriol-omniroute.service"]);
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
  if (!version) add("omniroute binary", "fail", `missing at ${BIN}`);
  else if (version === PIN) add("omniroute binary", "ok", version);
  else add("omniroute binary", "warn", `${version} (pinned ${PIN})`);

  const up = await serverUp();
  add("gateway", up ? "ok" : "fail", up ? ORIGIN : `down at ${ORIGIN}`);

  const env = readEnv();
  const apiKey = resolveInferenceKey(env) || process.env.OFFICINA_OMNIROUTE_API_KEY || "";
  add("inference key", apiKey ? "ok" : "fail", apiKey ? `set (...${apiKey.slice(-4)})` : "missing");

  if (existsSync(ENV_FILE)) {
    const mode = statSync(ENV_FILE).mode & 0o777;
    add("env permissions", mode === 0o600 ? "ok" : "warn", `${mode.toString(8)} ${ENV_FILE}`);
  } else {
    add("env permissions", "warn", `missing ${ENV_FILE}`);
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

  const enabled = systemctlUser(["is-enabled", "vitriol-omniroute.service"]);
  const active = systemctlUser(["is-active", "vitriol-omniroute.service"]);
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

  if (!requested) fail("usage: vitriol omniroute use <model-id> [--local]");
  if (!existsSync(BIN)) fail(`OmniRoute not installed at ${BIN}; run: vitriol omniroute setup`);
  await startServer();

  const env = readEnv();
  const apiKey = resolveInferenceKey(env) || process.env.OFFICINA_OMNIROUTE_API_KEY || "";
  if (!apiKey) fail("missing inference key; run: vitriol omniroute setup");
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

  settings.defaultProvider = "omniroute";
  settings.defaultModel = model.id;
  writeJsonAtomic(PI_SETTINGS, settings);
  chmodSync(PI_SETTINGS, 0o600);

  if (flags.json) {
    log(JSON.stringify({ provider: "omniroute", model: model.id, settings: PI_SETTINGS }, null, 2));
  } else {
    log(`Default model set to omniroute/${model.id}`);
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
      omniroute: readEnv().get("OFFICINA_OMNIROUTE") ?? "unset",
      envFile: ENV_FILE,
    };
    if (flags.json) {
      log(JSON.stringify(report, null, 2));
    } else {
      log(`cache:     ${report.cache}`);
      log(`praetor:   ${report.praetor}`);
      log(`egress:    ${report.egress}`);
      log(`omniroute: ${report.omniroute}`);
      log(`env file:  ${report.envFile}`);
    }
    return;
  }

  if (sub !== "set") fail(`unknown config command: ${sub}`);
  const key = flags._[1];
  const raw = flags._[2];
  if (!key || !raw) fail("usage: vitriol omniroute config set <cache|praetor> <value>");
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
  if (!existsSync(BIN)) fail(`OmniRoute not installed at ${BIN}; run: vitriol omniroute setup`);
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
    fail(`accounts config not found at ${path}; run: vitriol omniroute accounts init`);
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
  log("Add provider entries with credential_env names, then run: vitriol omniroute accounts sync");
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
  const dir = mkdtempSync(join(tmpdir(), "vitriol-omniroute-"));
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
      if (!/^[A-Za-z_]/.test(env)) env = `OMNIROUTE_${env}`;
      return {
        provider: provider.id,
        name: provider.name ?? provider.id,
        website: provider.website ?? "",
        credential_env: env,
      };
    })
    .sort((a, b) => a.provider.localeCompare(b.provider));
}

async function cmdAccountsFree(flags) {
  if (!existsSync(BIN)) fail(`OmniRoute not installed at ${BIN}; run: vitriol omniroute setup`);

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
  if (!existsSync(BIN)) fail(`OmniRoute not installed at ${BIN}; run: vitriol omniroute setup`);
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
  if (!existsSync(BIN)) fail(`OmniRoute not installed at ${BIN}; run: vitriol omniroute setup`);

  const loaded = loadAccountConfig(flags);
  const catalog = loadProviderCatalog();
  const payload = buildImportPayload(loaded, catalog, process.env, {
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
  const dir = mkdtempSync(join(tmpdir(), "vitriol-omniroute-"));
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
    const credential = credentialEnv
      ? process.env[String(credentialEnv)]?.trim()
        ? `env:${credentialEnv} (${maskSecret(process.env[String(credentialEnv)])})`
        : `env:${credentialEnv} (missing)`
      : account.allow_no_credential || account.allowNoCredential
        ? "no credential"
        : "missing credential_env";
    return {
      provider,
      name,
      configured: configured.has(`${provider.toLowerCase()}\0${name.toLowerCase()}`),
      credential,
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

  log(`Accounts config: ${loaded.path}`);
  log(`Gateway: ${up ? `up (${ORIGIN})` : "down"}`);
  if (connectionError) log(`Connection read failed: ${connectionError}`);
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

  if (!existsSync(BIN)) fail(`OmniRoute not installed at ${BIN}`);
  if (sub === "status") {
    if (!(await serverUp())) fail(`gateway is down at ${ORIGIN}; run: vitriol omniroute up`);
    run(BIN, ["--base-url", ORIGIN, "oauth", "status"], { inherit: true, timeout: 60_000 });
    return;
  }

  if (sub !== "login") fail(`unknown accounts oauth command: ${sub}`);
  const provider = findOAuthProvider(String(id ?? ""));
  if (!provider) {
    fail(`unsupported OAuth provider: ${id ?? "(missing)"}; try: vitriol omniroute accounts oauth list`);
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
  if (sub === "free") return cmdAccountsFree(flags);
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
  log(`vitriol omniroute accounts — bulk API-key import + assisted OAuth

Usage:
  vitriol omniroute accounts init [--file PATH] [--force]
  vitriol omniroute accounts sync [--file PATH] [--dry-run] [--json]
                                  [--only ID] [--strict]
  vitriol omniroute accounts status [--file PATH] [--json]
  vitriol omniroute accounts free [--apply] [--json] [--strict]
                                  [--no-optional] [--no-existing-auth]
                                  [--no-browser] [--plans] [--template]
                                  [--file PATH] [--force]
  vitriol omniroute accounts discover [--apply] [--json] [--strict]
  vitriol omniroute accounts oauth list
  vitriol omniroute accounts oauth login <id> [--no-browser]
                                  [--import-from-system] [--no-import-system]
                                  [--social google|github] [--timeout MS]
  vitriol omniroute accounts oauth status

Config: ${ACCOUNTS_FILE}
Credentials are referenced by env-var name (credential_env), never stored in
the config. Supported OAuth flows: ${OAUTH_PROVIDERS.map((p) => p.id).join(", ")}.`);
}

function printHelp() {
  log(`vitriol omniroute — local OmniRoute gateway for Officina

Usage:
  vitriol omniroute setup [--opencode-key-stdin] [--no-install] [--force]
                          [--free] [--no-browser]
  vitriol omniroute up
  vitriol omniroute down
  vitriol omniroute status
  vitriol omniroute sync
  vitriol omniroute accounts init|sync|status
  vitriol omniroute accounts oauth list|login|status
  vitriol omniroute cache status|enable|disable [--providers a,b] [--json]
  vitriol omniroute egress status|set|clear|test [--proxy URL] [--no-proxy LIST]
  vitriol omniroute service install|status|remove
  vitriol omniroute doctor [--json]
  vitriol omniroute use <model-id> [--local] [--json]
  vitriol omniroute config get [--json]
  vitriol omniroute config set cache auto|local|cloud|off
  vitriol omniroute config set praetor on|off

Setup installs omniroute@${PIN} under ${VENDOR_DIR}, configures OpenCode Go,
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
  process.stderr.write(`[omniroute] ${message}\n`);
  process.exit(Number(error?.exitCode) || 1);
});
