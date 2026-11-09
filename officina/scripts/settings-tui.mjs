#!/usr/bin/env node
// Interactive settings menu for VITRIOL engine config and Officina env.
//
// Reads ~/.vitriol/config (INI, comments become explanations) and
// ~/.vitriol/officina/env (known keys get curated explanations), then edits
// values in place with an atomic write. Engine changes need a service restart.

import { chmodSync, copyFileSync, existsSync, readFileSync, renameSync, statSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import {
  Input,
  ProcessTerminal,
  SettingsList,
  Text,
  TuiMainScreen,
  getTerminalColorMode,
  matchesKey,
  parseColor,
  styleText,
} from "@earendil-works/pi-tui";
import { parseEnvFile, upsertEnvFile } from "./omniroute-lib.mjs";

const HOME = process.env.HOME || homedir();
const CONFIG = process.env.VITRIOL_CONFIG || join(HOME, ".vitriol", "config");
const ENV_FILE = process.env.OFFICINA_ENV_FILE || join(HOME, ".vitriol", "officina", "env");

const colorMode = getTerminalColorMode();
const accent = parseColor("oklch(76% 0.13 195)");
const accent2 = parseColor("oklch(72% 0.16 305)");
const green = parseColor("oklch(78% 0.15 145)");
const muted = parseColor("oklch(65% 0.02 250)");
const warn = parseColor("oklch(78% 0.14 75)");
const bold = (text, color = accent) => styleText(text, { fg: color, bold: true }, colorMode);
const dim = (text) => styleText(text, { fg: muted, dim: true }, colorMode);
const good = (text) => styleText(text, { fg: green, bold: true }, colorMode);
const hot = (text) => styleText(text, { fg: warn, bold: true }, colorMode);

const ENGINE_ENUMS = {
  "gpu.exclude_secondary": ["true", "false"],
  "vitriol.mode": ["off", "stream", "sync", "async"],
  "vitriol.output_cache": ["off", "on"],
  "vitriol.predictive_prefetch": ["off", "on"],
  "vitriol.verbose": ["false", "true"],
  "memory.mode": ["off", "on"],
  "memory.semantic_mode": ["off", "on"],
  "server.ctx_resize": ["0", "1"],
  "server.cache_idle_slots": ["off", "on"],
  "kv.mode": ["standard", "offload", "sparse"],
  "kv.score": ["off", "probe"],
  "kv.quant_mode": ["f16", "q8_0", "q4_0", "tq3_0", "tq3_1s", "tq3_4s"],
  "kv.quant_mode_v": ["f16", "q8_0", "q4_0", "tq3_0", "tq3_1s", "tq3_4s"],
  "engine.mode": ["vitriol-dma", "native"],
  "spec.type": ["", "mtp", "draft"],
  "spec.draft_ngl": ["all", "auto"],
  "chimera.mode": ["auto", "cuda", "vulkan", "off"],
};

const ENV_DOCS = {
  OFFICINA_OMNIROUTE: "Enable the OmniRoute provider extension (1 = on, 0 = off).",
  OFFICINA_OMNIROUTE_BASE_URL: "OpenAI-compatible OmniRoute base URL, normally http://127.0.0.1:20128/v1.",
  OFFICINA_OMNIROUTE_API_KEY: "Local OmniRoute inference key. Secret; edit with $EDITOR.",
  OFFICINA_CACHE_MODE: "Cloud cache strategy: auto keeps local low-context; non-local providers use cache-safe mode.",
  OFFICINA_PRAETOR: "Arm the advisory Praetor extension (1 = on, 0 = off).",
  OFFICINA_PRAETOR_BUDGET: "Maximum characters of Praetor diagnostics injected after an edit.",
  OFFICINA_PRAETOR_TIMEOUT_MS: "Praetor validate/report timeout in milliseconds.",
  OFFICINA_EGRESS: "Set 0 to disable fixed egress even when egress.json is configured.",
  OFFICINA_NO_PRAETOR: "Hard kill switch for Praetor, overriding OFFICINA_PRAETOR.",
  OFFICINA_CACHE_CLEAR_PCT: "Cloud-mode context percentage at which prefix-clearing tools may run.",
  OFFICINA_CACHE_FORCE_CLEAR: "Set 1 to allow prefix clearing below the cloud threshold.",
};

const ENV_VALUES = {
  OFFICINA_OMNIROUTE: ["1", "0"],
  OFFICINA_CACHE_MODE: ["auto", "local", "cloud", "off"],
  OFFICINA_PRAETOR: ["1", "0"],
  OFFICINA_EGRESS: ["1", "0"],
  OFFICINA_NO_PRAETOR: ["0", "1"],
  OFFICINA_CACHE_FORCE_CLEAR: ["0", "1"],
};

const isSecret = (key) => /(?:API_KEY|TOKEN|SECRET|PASSWORD|CREDENTIAL)/i.test(key);

function atomicWrite(path, content) {
  const mode = existsSync(path) ? statSync(path).mode & 0o777 : 0o600;
  const tmp = `${path}.tmp-${process.pid}`;
  writeFileSync(tmp, content, { mode });
  chmodSync(tmp, mode);
  renameSync(tmp, path);
}

function backupOnce(path) {
  const backup = `${path}.bak`;
  if (existsSync(path) && !existsSync(backup)) {
    try { copyFileSync(path, backup); chmodSync(backup, statSync(path).mode & 0o777); } catch {}
  }
}

function parseIni(path) {
  const lines = readFileSync(path, "utf8").split(/\r?\n/);
  const entries = [];
  let section = "";
  let docs = [];
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    const trimmed = line.trim();
    if (!trimmed) { docs = []; continue; }
    if (trimmed.startsWith("#") || trimmed.startsWith(";")) {
      docs.push(trimmed.replace(/^[#;]\s?/, ""));
      continue;
    }
    const sec = trimmed.match(/^\[([^\]]+)\]$/);
    if (sec) { section = sec[1]; docs = []; continue; }
    const kv = line.match(/^(\s*)([A-Za-z0-9_.-]+)\s*=\s*(.*)$/);
    if (!kv) continue;
    entries.push({
      id: `engine:${section}:${kv[2]}`,
      section,
      key: kv[2],
      value: kv[3].trim(),
      doc: docs.join(" ").replace(/\s+/g, " ").slice(0, 700),
      lineIndex: i,
    });
    docs = [];
  }
  return { path, lines, entries };
}

function engineValues(entry) {
  if (ENGINE_ENUMS[`${entry.section}.${entry.key}`]) return ENGINE_ENUMS[`${entry.section}.${entry.key}`];
  if (/^(on|off)$/i.test(entry.value)) return ["on", "off"];
  if (/^(true|false)$/i.test(entry.value)) return ["true", "false"];
  if (/^(yes|no)$/i.test(entry.value)) return ["yes", "no"];
  return undefined;
}

function writeEngine(path, lines, entry, value) {
  backupOnce(path);
  const original = lines[entry.lineIndex];
  const replaced = original.replace(/^(\s*[A-Za-z0-9_.-]+\s*=\s*).*$/, `$1${value}`);
  lines[entry.lineIndex] = replaced === original ? `${entry.key} = ${value}` : replaced;
  atomicWrite(path, `${lines.join("\n")}`);
}

function textSubmenu(current, done) {
  const input = new Input({ prompt: "  > " });
  input.setValue(current);
  input.focused = true;
  input.onSubmit = (value) => done(String(value).trim());
  input.onEscape = () => done(undefined);
  return input;
}

function parseEnvDocs(path) {
  const docs = new Map();
  if (!existsSync(path)) return docs;
  const lines = readFileSync(path, "utf8").split(/\r?\n/);
  let comments = [];
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed) { comments = []; continue; }
    if (trimmed.startsWith("#")) { comments.push(trimmed.replace(/^#\s?/, "")); continue; }
    const match = trimmed.match(/^([A-Za-z_][A-Za-z0-9_]*)\s*=/);
    if (match) {
      docs.set(match[1], comments.join(" ").replace(/\s+/g, " ").slice(0, 700));
      comments = [];
    }
  }
  return docs;
}

function envItem(key, value, fileDoc = "") {
  const secret = isSecret(key);
  const doc = ENV_DOCS[key] ?? fileDoc ?? "Officina environment setting.";
  const values = secret ? undefined : ENV_VALUES[key];
  return {
    id: `env:${key}`,
    label: `${key}`,
    description: `${doc}${secret ? " Value is hidden and cannot be edited here." : ""}`,
    currentValue: secret ? (value ? "•••• (set)" : "(unset)") : (value || "(unset)"),
    values,
    submenu: values || secret ? undefined : textSubmenu,
    secret,
    key,
  };
}

function buildItems() {
  const items = [];
  const engine = existsSync(CONFIG) ? parseIni(CONFIG) : { path: CONFIG, lines: [], entries: [] };
  const engineMap = new Map(engine.entries.map((entry) => [entry.id, entry]));

  for (const entry of engine.entries) {
    const values = engineValues(entry);
    const shown = entry.value.length > 60 ? `${entry.value.slice(0, 57)}…` : (entry.value || "(empty)");
    items.push({
      id: entry.id,
      label: `${entry.section} · ${entry.key}`,
      description: entry.doc || `${entry.section}.${entry.key} has no inline explanation in ~/.vitriol/config.`,
      currentValue: shown,
      values,
      submenu: values ? undefined : (_current, done) => textSubmenu(entry.value, done),
      engineEntry: entry,
      engineLines: engine.lines,
      enginePath: engine.path,
      rawValue: entry.value,
    });
  }

  const env = existsSync(ENV_FILE) ? parseEnvFile(readFileSync(ENV_FILE, "utf8")) : new Map();
  const envDocs = parseEnvDocs(ENV_FILE);
  const envKeys = new Set([...Object.keys(ENV_DOCS), ...env.keys()]);
  for (const key of [...envKeys].sort()) {
    items.push(envItem(key, env.get(key) ?? "", envDocs.get(key) ?? ""));
  }
  return items;
}

function printList(json) {
  const items = buildItems().map((item) => ({
    id: item.id,
    label: item.label,
    currentValue: item.currentValue,
    description: item.description,
    values: item.values,
  }));
  if (json) {
    process.stdout.write(`${JSON.stringify(items, null, 2)}\n`);
    return;
  }
  for (const item of items) {
    process.stdout.write(`${item.label}\n  ${item.currentValue}\n  ${item.description}\n`);
  }
}

function main() {
  const args = process.argv.slice(2);
  if (args.includes("--list") || args.includes("--json")) {
    printList(args.includes("--json"));
    return;
  }
  if (args.includes("--help") || args.includes("-h")) {
    process.stdout.write("usage: vitriol settings [--list] [--json]\n");
    return;
  }
  if (!process.stdin.isTTY) {
    console.error("[settings] interactive TUI requires a TTY");
    process.exit(1);
  }

  const items = buildItems();
  const terminal = new ProcessTerminal();
  const tui = new TuiMainScreen(terminal);
  const title = new Text(
    `${bold("⚙  VITRIOL settings")}  ${dim("engine config + Officina env")}\n` +
    `${dim("Changes are written immediately. Engine changes apply after:")} ${hot("systemctl restart vitriol-server")}`,
    1,
    0,
  );
  const status = new Text("", 1, 0);

  const theme = {
    label: (text, selected) => selected ? bold(text, accent) : text,
    value: (text, selected) => selected ? bold(text, green) : dim(text),
    description: (text) => dim(text),
    cursor: bold("▸ ", accent2),
    hint: (text) => dim(text),
  };

  const list = new SettingsList(
    items,
    18,
    theme,
    (id, newValue) => {
      const item = items.find((entry) => entry.id === id);
      if (!item || item.secret) return;
      if (item.engineEntry) {
        const display = item.rawValue;
        const old = display;
        writeEngine(item.enginePath, item.engineLines, item.engineEntry, newValue);
        item.engineEntry.value = newValue;
        item.rawValue = newValue;
        item.currentValue = newValue || "(empty)";
        status.setText(`${good("saved")} ${dim(item.label)}  ${dim(old)} → ${newValue || "(empty)"}  ${dim("(restart engine to apply)")}`);
      } else {
        const content = existsSync(ENV_FILE) ? readFileSync(ENV_FILE, "utf8") : "";
        atomicWrite(ENV_FILE, upsertEnvFile(content, { [item.key]: newValue }));
        item.currentValue = newValue || "(unset)";
        status.setText(`${good("saved")} ${dim(item.key)} = ${newValue}  ${dim("(next Officina session)")}`);
      }
      tui.requestRender();
    },
    () => { tui.stop(); process.exit(0); },
    { enableSearch: true },
  );

  tui.addChild(title);
  tui.addChild(list);
  tui.addChild(status);
  tui.setFocus(list);
  tui.addInputListener((data) => {
    if (matchesKey(data, "ctrl+c")) {
      tui.stop();
      process.exit(0);
    }
    return undefined;
  });
  tui.start();
}

main();
