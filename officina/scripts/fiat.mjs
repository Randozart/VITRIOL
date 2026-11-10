#!/usr/bin/env node
// Fiat Lux — the free-tier merry-go-round.
//
// One screen listing every free tier Officina can reach, each on its effort
// ladder rung. Enter runs the matching flow:
//   active       -> nothing to do (already reachable)
//   auth-reused  -> `accounts free --apply` (reuses ~/.local/share/opencode/auth.json)
//   oauth        -> `accounts oauth login <id>` (owner clicks consent in the browser)
//   api-key      -> masked key prompt -> accounts.json env ref (never a literal)
// After any success the Fiat Lux coupling is regenerated from the live catalog.

import { existsSync, readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  ProcessTerminal,
  SelectList,
  Text,
  TuiMainScreen,
  getTerminalColorMode,
  matchesKey,
  parseColor,
  styleText,
} from "@earendil-works/pi-tui";
import {
  buildFiatLuxCoupling,
  membershipState,
  writeFiatLuxCouplings,
} from "./fiat-lux-lib.mjs";
import { upsertEnvFile } from "./caduceus-lib.mjs";

const FIAT_LIB_HELPERS = { upsertEnvFile };

const HOME = process.env.HOME || homedir();
const HERE = dirname(fileURLToPath(import.meta.url));
const VITRIOL = join(HERE, "..", "..", "scripts", "vitriol");
const CATALOG_CACHE = join(HOME, ".vitriol", "officina", "caduceus-models.json");
const COUPLINGS_FILE = join(HOME, ".vitriol", "officina", "couplings.json");
const ACCOUNTS_FILE = join(HOME, ".vitriol", "officina", "accounts.json");
const OPENCODE_AUTH = process.env.OPENCODE_AUTH_FILE || join(HOME, ".local", "share", "opencode", "auth.json");
const ORIGIN = (process.env.OFFICINA_CADUCEUS_BASE_URL || "http://127.0.0.1:20128/v1").replace(/\/+$/, "");

const OAUTH_BACKEND_BY_PROVIDER = new Map([
  ["claude-code", "claude-code"],
  ["codex", "codex"],
  ["copilot", "copilot"],
  ["gemini", "gemini"],
  ["antigravity", "antigravity"],
  ["kiro", "kiro"],
  ["amazon-q", "amazon-q"],
  ["freebuff", "freebuff"],
  ["openference", "openference"],
  ["agy", "agy"],
]);

for (const stream of [process.stdout, process.stderr]) {
  stream.on("error", (error) => {
    if (error?.code === "EPIPE") process.exit(0);
  });
}

function fail(message, code = 1) {
  process.stderr.write(`[fiat] ${message}\n`);
  process.exit(code);
}

function runVetriol(args, { inherit = true } = {}) {
  const res = spawnSync(VITRIOL, args, {
    encoding: "utf8",
    stdio: inherit ? "inherit" : ["pipe", "pipe", "pipe"],
    timeout: 600_000,
  });
  return { status: res.status ?? 1, stdout: res.stdout ?? "", stderr: res.stderr ?? "" };
}

function readCatalog() {
  if (existsSync(CATALOG_CACHE)) {
    try {
      const parsed = JSON.parse(readFileSync(CATALOG_CACHE, "utf8"));
      if (Array.isArray(parsed?.data)) return parsed.data;
    } catch {
      // fall through to a live fetch
    }
  }
  const res = runVetriol(["caduceus", "sync", "--json"], { inherit: false });
  if (res.status !== 0) fail(`catalog unavailable: run 'vitriol caduceus up' first${res.stderr ? ` — ${res.stderr.trim().slice(0, 200)}` : ""}`);
  try {
    const parsed = JSON.parse(res.stdout);
    if (Array.isArray(parsed?.data)) return parsed.data;
  } catch {
    // best-effort: try the raw model list
  }
  if (Array.isArray(parsedModelsFrom(res.stdout))) return parsedModelsFrom(res.stdout);
  fail("catalog unavailable and no cached catalog; run 'vitriol caduceus up' first");
  return [];
}

function parsedModelsFrom(text) {
  try {
    const json = JSON.parse(text);
    const list = Array.isArray(json) ? json : json?.data ?? json?.models;
    return Array.isArray(list) ? list : null;
  } catch {
    return null;
  }
}

function connectedProviders() {
  const res = runVetriol(["caduceus", "status", "--json"], { inherit: false });
  if (res.status !== 0) return new Set();
  try {
    const json = JSON.parse(res.stdout);
    const list = Array.isArray(json) ? json : json?.connections ?? json?.providers ?? [];
    return new Set(
      list
        .map((row) => String(row?.provider ?? "").trim().toLowerCase())
        .filter(Boolean),
    );
  } catch {
    return new Set();
  }
}

function authProviders() {
  if (!existsSync(OPENCODE_AUTH)) return new Set();
  try {
    const auth = JSON.parse(readFileSync(OPENCODE_AUTH, "utf8"));
    return new Set(Object.keys(auth ?? {}).map((k) => k.toLowerCase()));
  } catch {
    return new Set();
  }
}

function oauthProviders() {
  const res = runVetriol(["caduceus", "accounts", "oauth", "list", "--json"], { inherit: false });
  if (res.status !== 0) return new Set();
  try {
    const json = JSON.parse(res.stdout);
    const list = Array.isArray(json) ? json : json?.providers ?? [];
    return new Set(
      list
        .map((row) => (typeof row === "string" ? row : String(row?.id ?? "")))
        .map((id) => id.trim().toLowerCase())
        .filter(Boolean),
    );
  } catch {
    return new Set();
  }
}

// A model is usable in the chat composer when it emits text. Image-only
// models (aihorde SDXL pool: type "image", output_modalities ["image"]) are
// not chat models and must not join Fiat Lux.
export function isChatModel(row) {
  if (!row || typeof row !== "object") return false;
  if (row.type === "image") return false;
  const modalities = Array.isArray(row.output_modalities) ? row.output_modalities : null;
  if (modalities && modalities.length > 0) return modalities.includes("text");
  return true; // no modality metadata = assume chat
}

function freeMembership(catalog) {
  // One row per (provider, model) that is free AND a text-output chat model,
  // classified by effort rung.
  const ctx = {
    connectedProviders: connectedProviders(),
    authProviders: authProviders(),
    oauthProviders: oauthProviders(),
  };
  const rows = [];
  const seen = new Set();
  for (const row of catalog) {
    if (!isChatModel(row)) continue;
    const id = String(row?.id ?? "");
    if (!id) continue;
    const [provider, ...rest] = id.split("/");
    const model = rest.join("/") || id;
    if (seen.has(id)) continue;
    seen.add(id);
    rows.push({
      id,
      provider: provider.toLowerCase(),
      model,
      name: String(row?.name ?? model),
      state: membershipState(provider, ctx),
    });
  }
  rows.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  return rows;
}

function regenFiatLux(catalog) {
  const coupling = buildFiatLuxCoupling(catalog, ORIGIN);
  if (!coupling) return 0;
  writeFiatLuxCouplings(COUPLINGS_FILE, coupling);
  return coupling.models.length;
}

function promptKey(provider) {
  const envName = `${String(provider).toUpperCase().replace(/[^A-Z0-9]+/g, "_")}_API_KEY`;
  return new Promise((resolve) => {
    process.stderr.write(`[fiat] Paste the API key for ${provider} (stored as env ref ${envName}; input is masked): `);
    const wasRaw = process.stdin.isRaw;
    if (process.stdin.isTTY) process.stdin.setRawMode(true);
    let value = "";
    let done = false;
    const onData = (chunk) => {
      if (done) return;
      for (const ch of Buffer.from(chunk).toString("utf8")) {
        if (ch === "\n" || ch === "\r") {
          done = true;
          if (process.stdin.isTTY) process.stdin.setRawMode(wasRaw);
          process.stdin.removeListener("data", onData);
          process.stderr.write("\n");
          resolve(value.trim() ? { env: envName, value: value.trim() } : null);
          return;
        }
        if (ch === "\x03") {
          done = true;
          if (process.stdin.isTTY) process.stdin.setRawMode(wasRaw);
          process.stdin.removeListener("data", onData);
          process.stderr.write("\n[fiat] cancelled\n");
          resolve(null);
          return;
        }
        if (ch >= " " && ch !== "\x7f") value += ch;
      }
    };
    process.stdin.on("data", onData);
  });
}

function addAccountEnvRef(provider, envName, secret) {
  // The env file holds the secret (mode 600); accounts.json holds ONLY the env-ref name.
  const { upsertEnvFile } = importLib();
  const envFile = join(HOME, ".vitriol", "officina", "env");
  let content = existsSync(envFile) ? readFileSync(envFile, "utf8") : "";
  content = upsertEnvFile(content, { [envName]: secret });
  writeAtomic(envFile, content);
  const accounts = readAccounts();
  if (!accounts.accounts.some((a) => a.provider === provider)) {
    accounts.accounts.push({ provider, name: provider, credential_env: envName, enabled: true });
  }
  writeJsonAtomic(ACCOUNTS_FILE, accounts);
}

// Pure helpers imported once at module load.
const importLib = () => importSync();
function importSync() {
  // fiat.mjs is ESM; the helpers are pure ESM too. We loaded them at the top
  // of this module via a static import below; this closure just re-exposes them.
  return FIAT_LIB_HELPERS;
}

function readAccounts() {
  if (!existsSync(ACCOUNTS_FILE)) return { accounts: [], oauth: [] };
  try {
    const parsed = JSON.parse(readFileSync(ACCOUNTS_FILE, "utf8"));
    return {
      accounts: Array.isArray(parsed.accounts) ? parsed.accounts : [],
      oauth: Array.isArray(parsed.oauth) ? parsed.oauth : [],
    };
  } catch {
    return { accounts: [], oauth: [] };
  }
}

function writeAtomic(path, text) {
  const { writeFileSync, renameSync, mkdirSync } = require("node:fs");
  mkdirSync(join(path, ".."), { recursive: true });
  const tmp = `${path}.tmp-${process.pid}`;
  writeFileSync(tmp, text, { mode: 0o600 });
  renameSync(tmp, path);
}

function writeJsonAtomic(path, value) {
  writeAtomic(path, `${JSON.stringify(value, null, 2)}\n`);
}

function subscribe(row) {
  if (row.state === "active") {
    log(`already active: ${row.id}`);
    return true;
  }
  if (row.state === "auth-reused") {
    const res = runVetriol(["caduceus", "accounts", "free", "--apply"]);
    return res.status === 0;
  }
  if (row.state === "oauth") {
    const res = runVetriol(["caduceus", "accounts", "oauth", "login", row.provider]);
    return res.status === 0;
  }
  // api-key rung
  const prompted = promptKey(row.provider);
  if (!prompted) return false;
  addAccountEnvRef(row.provider, prompted.env, prompted.value);
  const res = runVetriol(["caduceus", "accounts", "sync"]);
  return res.status === 0;
}

function log(message) {
  process.stdout.write(`${message}\n`);
}

function printRows(rows, json) {
  if (json) {
    process.stdout.write(`${JSON.stringify(rows, null, 2)}\n`);
    return;
  }
  const badge = {
    active: "✓ active",
    "auth-reused": "● reuse auth",
    oauth: "○ one-click",
    "api-key": "🔑 paste key",
  };
  for (const row of rows) {
    log(`${badge[row.state]}  ${row.provider.padEnd(22)} ${row.model}  (${row.name})`);
  }
}

function tui(rows, flags) {
  if (!process.stdin.isTTY) {
    printRows(rows, false);
    return;
  }
  const terminal = new ProcessTerminal();
  const tuiApp = new TuiMainScreen(terminal);
  const mode = getTerminalColorMode();
  const accent = parseColor("oklch(76% 0.13 195)");
  const muted = parseColor("oklch(65% 0.02 250)");
  const bold = (s) => styleText(s, { fg: accent, bold: true }, mode);
  const dim = (s) => styleText(s, { fg: muted, dim: true }, mode);

  const header = new Text(
    `${bold("⌘ Fiat Lux — free tiers")}\n${dim("Enter subscribe · r refresh · q quit")}`,
    1,
    0,
  );
  const theme = {
    selectedPrefix: () => bold("▸ "),
    selectedText: (t) => bold(t),
    description: (t) => dim(t),
    scrollInfo: (t) => dim(t),
    noMatch: (t) => dim(t),
  };

  let current = null;
  let liveRows = rows;

  function showList() {
    if (current) tuiApp.removeChild(current);
    const items = liveRows.map((row) => ({
      value: row.id,
      label: `${row.provider} · ${row.model}`,
      description: `${row.state} — ${row.name}`,
    }));
    const list = new SelectList(items, 14, theme);
    list.onSelect = (item) => {
      const row = liveRows.find((r) => r.id === item.value);
      if (!row) return;
      tuiApp.stop();
      const ok = subscribe(row);
      const n = regenFiatLux(readCatalog());
      log(ok ? `subscribed: ${row.id}; fiat-lux now has ${n} model(s)` : `subscription failed for ${row.id}`);
      process.exit(ok ? 0 : 1);
    };
    list.onCancel = () => {
      tuiApp.stop();
      process.exit(0);
    };
    current = list;
    tuiApp.addChild(list);
    tuiApp.setFocus(list);
    tuiApp.requestRender();
  }

  tuiApp.addChild(header);
  showList();
  tuiApp.addInputListener((data) => {
    if (matchesKey(data, "ctrl+c") || data === "q") {
      tuiApp.stop();
      process.exit(0);
    }
    if (data === "r") {
      liveRows = freeMembership(readCatalog());
      showList();
    }
    return undefined;
  });
  tuiApp.start();
}

function main() {
  const argv = process.argv.slice(2);
  const flags = {};
  const positional = [];
  for (const arg of argv) {
    if (arg.startsWith("--")) flags[arg.slice(2)] = true;
    else positional.push(arg);
  }
  if (flags.help) {
    log(`usage: vitriol fiat [list|subscribe <provider>] [options]

  --json    machine output

Fiat Lux is the free-tier merry-go-round: one screen, every zero-cost
provider, minimal owner effort (click consent / paste key).`);
    return;
  }

  const catalog = readCatalog();
  const rows = freeMembership(catalog);
  if (rows.length === 0) fail("no free tiers found — run 'vitriol caduceus up' and try again");

  const command = positional[0] || "pick";
  if (command === "list") {
    printRows(rows, flags.json);
    return;
  }
  if (command === "subscribe") {
    const provider = positional[1];
    if (!provider) fail("usage: vitriol fiat subscribe <provider>");
    const row = rows.find((r) => r.provider === provider.toLowerCase()) ?? rows.find((r) => r.id.includes(provider));
    if (!row) fail(`no free tier for provider: ${provider}`);
    const ok = subscribe(row);
    const n = regenFiatLux(readCatalog());
    log(ok ? `subscribed: ${row.id}; fiat-lux now has ${n} model(s)` : `subscription failed for ${row.id}`);
    process.exit(ok ? 0 : 1);
  }
  if (command !== "pick") fail(`unknown command: ${command}`);

  // Keep the fiat-lux coupling fresh on every open, then show the picker.
  const n = regenFiatLux(catalog);
  tui(rows, flags);
}

main();
