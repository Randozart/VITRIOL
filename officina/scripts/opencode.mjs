#!/usr/bin/env node
// Import OpenCode sessions into Officina/pi sessions.

import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { ProcessTerminal, SelectList, Text, TuiMainScreen, getTerminalColorMode, matchesKey, parseColor, styleText } from "@earendil-works/pi-tui";
import {
  DEFAULT_OPTIONS,
  FULL_OPTIONS,
  filterSessionsByCwd,
  loadSessionFromDb,
  normalizeCwd,
  normalizeOpenCodeSession,
  parseSessionList,
  sessionDirForCwd,
  sessionStats,
  toSessionJsonl,
  writeSession,
} from "./opencode-import-lib.mjs";

const HOME = process.env.HOME || homedir();
const DB = process.env.OPENCODE_DB || join(HOME, ".local", "share", "opencode", "opencode.db");
const OPENCODE_BIN =
  process.env.OPENCODE_BIN ||
  (existsSync(join(HOME, ".opencode", "bin", "opencode"))
    ? join(HOME, ".opencode", "bin", "opencode")
    : "opencode");
for (const stream of [process.stdout, process.stderr]) {
  stream.on("error", (error) => {
    if (error?.code === "EPIPE") process.exit(0);
  });
}

const HERE = dirname(fileURLToPath(import.meta.url));
const VITRIOL = join(HERE, "..", "..", "scripts", "vitriol");

const VALUE_FLAGS = new Set(["last", "max-tool-chars", "max-chars", "max-reasoning-chars"]);

function parseArgs(argv) {
  const flags = { _: [] };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (!arg.startsWith("--")) { flags._.push(arg); continue; }
    const key = arg.slice(2);
    if (VALUE_FLAGS.has(key) && argv[i + 1] && !argv[i + 1].startsWith("--")) {
      flags[key] = argv[++i];
    } else {
      flags[key] = true;
    }
  }
  return flags;
}

function fail(message, code = 1) {
  process.stderr.write(`[opencode] ${message}\n`);
  process.exit(code);
}

function listOpenCodeSessions(cwd) {
  const res = spawnSync(OPENCODE_BIN, ["session", "list", "-n", "200", "--format", "json"], {
    encoding: "utf8",
    timeout: 60_000,
  });
  if (res.status !== 0) fail(`opencode session list failed: ${(res.stderr || res.error?.message || "").trim()}`);
  return filterSessionsByCwd(parseSessionList(res.stdout), cwd);
}

function withStats(sessions) {
  return sessions.map((session) => ({ ...session, stats: sessionStats(DB, session.id) }));
}

function fmtBytes(bytes) {
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KiB`;
  return `${bytes} B`;
}

function importOptions(flags) {
  const base = flags.full ? FULL_OPTIONS : DEFAULT_OPTIONS;
  return {
    ...base,
    last: flags.last ? Number(flags.last) : base.last,
    maxChars: flags["max-chars"] ? Number(flags["max-chars"]) : base.maxChars,
    maxToolChars: flags["max-tool-chars"] ? Number(flags["max-tool-chars"]) : base.maxToolChars,
    maxReasoningChars: flags["max-reasoning-chars"] ? Number(flags["max-reasoning-chars"]) : base.maxReasoningChars,
  };
}

function resolveSession(id, cwd) {
  const sessions = listOpenCodeSessions(cwd);
  if (sessions.length === 0) fail(`no OpenCode sessions for ${cwd}`);
  if (!id || id === "latest") return sessions[0];
  const exact = sessions.find((session) => session.id === id);
  if (exact) return exact;
  const partial = sessions.filter((session) => session.id.includes(id));
  if (partial.length === 1) return partial[0];
  fail(partial.length > 1 ? `ambiguous session id: ${id}` : `session not found in this project: ${id}`);
}

function importSession(id, flags, cwd = process.cwd()) {
  const session = resolveSession(id, cwd);
  const source = loadSessionFromDb(DB, session.id);
  if (!source) fail(`session not found in OpenCode DB: ${session.id}`);
  const options = importOptions(flags);
  const normalized = normalizeOpenCodeSession(source, options);
  if (normalized.blocks.length === 0) fail(`session ${session.id} has no importable text/tool history`);

  const text = toSessionJsonl(source, normalized, { cwd: session.directory || cwd });
  const report = {
    source: { id: session.id, title: session.title, directory: session.directory, updated: session.updated },
    mode: options.full ? "full" : "compact",
    stats: normalized.stats,
    bytes: Buffer.byteLength(text),
    destinationDir: sessionDirForCwd(session.directory || cwd),
  };

  if (flags["dry-run"]) {
    if (flags.json) process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
    else {
      process.stdout.write(`Dry run: ${session.title} (${session.id})\n`);
      process.stdout.write(`  project: ${session.directory}\n`);
      process.stdout.write(`  source:  ${report.stats.sourceMessages} messages / ${report.stats.sourceParts} parts\n`);
      process.stdout.write(`  import:  ${report.stats.selected} blocks, ${report.stats.chars} chars, ${fmtBytes(report.bytes)}\n`);
      process.stdout.write(`  dest:    ${report.destinationDir}\n`);
    }
    return report;
  }

  const written = writeSession(session.directory || cwd, source, normalized, { cwd: session.directory || cwd });
  report.path = written.path;
  report.bytes = written.bytes;

  if (flags.json) process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
  else {
    process.stdout.write(`Imported ${session.title} (${session.id})\n`);
    process.stdout.write(`  ${report.stats.selected} blocks, ${fmtBytes(report.bytes)} -> ${written.path}\n`);
  }

  if (!flags["no-launch"]) {
    // Rust TUI is canonical (2026-11-10). Launch it directly; fall back to
    // the deprecated vitriol/JS path only when the binary is absent.
    const launchArgs = flags.fork ? ["--fork", written.path] : ["--session", written.path];
    const configured = process.env.OFFICINA_BIN || "officina";
    let launched = spawnSync(configured, launchArgs, { stdio: "inherit" });
    if (launched.error?.code === "ENOENT") {
      launched = spawnSync(VITRIOL, ["officina", ...launchArgs], { stdio: "inherit" });
    }
    if (launched.status !== 0) process.exit(launched.status ?? 1);
  }
  return report;
}

function printList(sessions, json) {
  if (json) {
    process.stdout.write(`${JSON.stringify(sessions, null, 2)}\n`);
    return;
  }
  if (sessions.length === 0) {
    process.stdout.write("No OpenCode sessions for this project.\n");
    return;
  }
  for (const session of sessions) {
    const updated = new Date(Number(session.updated ?? 0)).toLocaleString();
    process.stdout.write(`${session.id}  ${String(session.title).slice(0, 70)}\n  ${updated}  ${fmtBytes(session.stats.bytes)}\n`);
  }
}

function tui(sessions, flags, cwd) {
  if (!process.stdin.isTTY) {
    printList(sessions, false);
    return;
  }
  const terminal = new ProcessTerminal();
  const tuiApp = new TuiMainScreen(terminal);
  const mode = getTerminalColorMode();
  const accent = parseColor("oklch(76% 0.13 195)");
  const muted = parseColor("oklch(65% 0.02 250)");
  const bold = (s) => styleText(s, { fg: accent, bold: true }, mode);
  const dim = (s) => styleText(s, { fg: muted, dim: true }, mode);

  const header = new Text(`${bold("⌘ OpenCode sessions")}  ${dim(normalizeCwd(cwd))}\n${dim("Enter choose · q quit")}`, 1, 0);
  const hint = new Text("", 1, 0);
  let current;
  let selected = null;

  const actionItems = [
    { value: "compact", label: "Import compact", description: "Recent text + bounded tool history" },
    { value: "fork", label: "Fork compact", description: "Import, then open with --fork" },
    { value: "full", label: "Import full", description: "All messages; larger context" },
    { value: "cancel", label: "Cancel", description: "Back to session list" },
  ];

  const theme = {
    selectedPrefix: () => bold("▸ "),
    selectedText: (t) => bold(t),
    description: (t) => dim(t),
    scrollInfo: (t) => dim(t),
    noMatch: (t) => dim(t),
  };

  function showSessions() {
    const items = sessions.map((session) => ({
      value: session.id,
      label: `${session.title}`,
      description: `${session.id} · ${fmtBytes(session.stats.bytes)} · ${new Date(Number(session.updated)).toLocaleDateString()}`,
    }));
    const list = new SelectList(items, 14, theme);
    list.onSelect = (item) => {
      selected = sessions.find((session) => session.id === item.value);
      showActions();
    };
    list.onCancel = () => { tuiApp.stop(); process.exit(0); };
    current = list;
    tuiApp.addChild(list);
    tuiApp.setFocus(list);
    tuiApp.requestRender();
  }

  function showActions() {
    if (current) tuiApp.removeChild(current);
    const list = new SelectList(actionItems, actionItems.length, theme);
    list.onSelect = (item) => {
      if (item.value === "cancel") {
        showSessionsAndStart();
        return;
      }
      tuiApp.stop();
      const nextFlags = { ...flags, fork: item.value === "fork", full: item.value === "full", "no-launch": false };
      importSession(selected.id, nextFlags, cwd);
      process.exit(0);
    };
    list.onCancel = () => showSessionsAndStart();
    current = list;
    tuiApp.addChild(list);
    tuiApp.setFocus(list);
    tuiApp.requestRender();
  }

  function showSessionsAndStart() {
    if (current) tuiApp.removeChild(current);
    showSessions();
  }

  tuiApp.addChild(header);
  showSessions();
  tuiApp.addChild(hint);
  tuiApp.addInputListener((data) => {
    if (matchesKey(data, "ctrl+c") || data === "q") {
      tuiApp.stop();
      process.exit(0);
    }
    return undefined;
  });
  tuiApp.start();
}

function main() {
  const flags = parseArgs(process.argv.slice(2));
  if (flags.help) {
    process.stdout.write(`usage: vitriol opencode [list|import <id>] [options]

  --json                 machine output
  --fork                 open the imported session with --fork
  --full                 all messages, larger caps
  --dry-run              report size/destination without writing
  --no-launch            write the Officina session but do not open it
  --last N               optional: import only the last N blocks (default: all)
  --max-tool-chars N     per-tool input/output guard (default 500; full 4000)
  --max-chars N          optional total transcript budget (default: none)
`);
    return;
  }

  const cwd = process.cwd();
  const command = flags._[0] || "pick";
  if (command === "list") {
    printList(withStats(listOpenCodeSessions(cwd)), flags.json);
    return;
  }
  if (command === "import") {
    importSession(flags._[1] ?? "latest", flags, cwd);
    return;
  }
  if (command !== "pick") fail(`unknown command: ${command}`);

  const sessions = withStats(listOpenCodeSessions(cwd));
  if (sessions.length === 0) fail(`no OpenCode sessions for ${cwd}`);
  tui(sessions, flags, cwd);
}

main();
