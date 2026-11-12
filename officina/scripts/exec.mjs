#!/usr/bin/env node
// officina exec — headless one-shot through the SAME harness the TUI runs
// (codex-infra stage 3, 2026-11-12; plan
// .opencode/plans/officina-exec-infra-2026-11-12.md).
//
// `officina exec "task" [--lane auto|lapis|nimbus] [--tools csv]
//             [--max-minutes N] [--json] [--detach] [--list] [--no-trust]`
//
// Invariant I1: exec NEVER passes --session — it cannot resume, patch, or
// quarantine any existing session file. The permissions guard's headless
// fail-closed applies unchanged; lanes/prefs/routing are the TUI's exact
// extension stack (same -e set as RpcBridge::officina_extensions_to_carry).
//
// Exit: pi's exit code (0 ok). `--list` exits 0.

import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, openSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const OFFICINA = resolve(fileURLToPath(new URL("..", import.meta.url)));
const PI_BIN = join(OFFICINA, "node_modules", ".bin", "pi");
const EXEC_ROOT = join(homedir(), ".local", "share", "officina", "exec");
const DEFAULT_TOOLS = "+grep,+find,+ls";

/** Same contract as the TUI's envfile::load_officina_env (main.rs): the
 *  launch env applies to pi, but shell exports win. */
function loadOfficinaEnv() {
  const path = join(homedir(), ".vitriol", "officina", "env");
  if (!existsSync(path)) return;
  for (const line of readFileSync(path, "utf8").split(/\r?\n/)) {
    const m = line.match(/^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$/);
    if (m && process.env[m[1]] === undefined) process.env[m[1]] = m[2];
  }
}

function parseArgs(argv) {
  const opts = {
    lane: "auto",
    tools: DEFAULT_TOOLS,
    maxMinutes: 30,
    json: false,
    detach: false,
    list: false,
    trust: true,
    task: "",
  };
  const rest = [];
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--lane") opts.lane = argv[++i] ?? "auto";
    else if (a === "--tools") opts.tools = argv[++i] ?? DEFAULT_TOOLS;
    else if (a === "--max-minutes") opts.maxMinutes = Number(argv[++i] ?? 30);
    else if (a === "--json") opts.json = true;
    else if (a === "--detach") opts.detach = true;
    else if (a === "--list") opts.list = true;
    else if (a === "--no-trust") opts.trust = false;
    else if (a === "-h" || a === "--help") opts.help = true;
    else rest.push(a);
  }
  opts.task = rest.join(" ").trim();
  return opts;
}

/** Same rule as RpcBridge::officina_extensions_to_carry: every officina
 *  extension dir with an index.ts; project-owned dirs shadow by name. */
function extensionsToCarry(cwd) {
  const exts = join(OFFICINA, ".pi", "extensions");
  if (!existsSync(exts)) return [];
  const shadowed = new Set();
  for (const root of [join(cwd, ".pi", "extensions"), join(cwd, ".officina", "extensions")]) {
    try {
      for (const ent of readdirSync(root)) shadowed.add(ent);
    } catch {
      // no project extensions
    }
  }
  return readdirSync(exts)
    .filter((name) => name !== "_shared" && !shadowed.has(name))
    .filter((name) => existsSync(join(exts, name, "index.ts")))
    .sort()
    .map((name) => join(exts, name));
}

function buildArgs(opts) {
  const args = ["--print"];
  if (opts.json) args.push("--mode", "json");
  if (opts.trust) args.push("-a");
  args.push("--tools", opts.tools);
  if (opts.lane === "lapis") args.push("--model", "officina/lapis");
  if (opts.lane === "nimbus") args.push("--model", "officina/nimbus");
  for (const ext of extensionsToCarry(process.cwd())) args.push("-e", ext);
  args.push(opts.task);
  return args;
}

function slug(text, max = 40) {
  return (
    text
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, max) || "task"
  );
}

function isAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

function runForeground(opts) {
  const args = buildArgs(opts);
  const child = spawn("node", [PI_BIN, ...args], { stdio: "inherit" });
  const killer = setTimeout(() => {
    child.kill("SIGTERM");
    setTimeout(() => child.kill("SIGKILL"), 5_000);
  }, opts.maxMinutes * 60_000);
  child.on("exit", (code) => {
    clearTimeout(killer);
    process.exit(code ?? 1);
  });
}

function runDetached(opts) {
  mkdirSync(EXEC_ROOT, { recursive: true });
  const dir = join(EXEC_ROOT, `${Date.now()}-${slug(opts.task)}`);
  mkdirSync(dir, { recursive: true });
  const args = buildArgs(opts);
  const logPath = join(dir, "output.log");
  const log = openSync(logPath, "a");
  const child = spawn("node", [PI_BIN, ...args], {
    detached: true,
    stdio: ["ignore", log, log],
    cwd: process.cwd(),
  });
  writeFileSync(
    join(dir, "meta.json"),
    `${JSON.stringify(
      {
        pid: child.pid,
        task: opts.task,
        lane: opts.lane,
        cwd: process.cwd(),
        started: new Date().toISOString(),
        maxMinutes: opts.maxMinutes,
      },
      null,
      2,
    )}\n`,
  );
  // Completion watcher: notify + done marker; hard timeout mirrors fg mode.
  const watcher = [
    "end=$((SECONDS + " + opts.maxMinutes * 60 + "))",
    `while kill -0 ${child.pid} 2>/dev/null; do`,
    "  [ $SECONDS -ge $end ] && kill -9 " + child.pid + " 2>/dev/null && echo timeout=1 >> " + JSON.stringify(logPath),
    "  sleep 5",
    "done",
    `wait ${child.pid} 2>/dev/null; ec=$?`,
    `echo "exit=$ec" > ${JSON.stringify(join(dir, "done"))}`,
    "command -v notify-send >/dev/null && notify-send -a officina \"exec done\" \"exit=$ec · " +
      opts.task.replace(/"/g, "").slice(0, 60) + "\" || true",
  ].join("\n");
  const w = spawn("bash", ["-c", watcher], { detached: true, stdio: "ignore" });
  w.unref();
  child.unref();
  console.log(`detached · pid ${child.pid} · ${dir}`);
  console.log(`follow: tail -f ${logPath}`);
}

function listRuns() {
  if (!existsSync(EXEC_ROOT)) {
    console.log("no exec runs");
    return;
  }
  for (const dir of readdirSync(EXEC_ROOT).sort().reverse()) {
    const full = join(EXEC_ROOT, dir);
    let meta = {};
    try {
      meta = JSON.parse(readFileSync(join(full, "meta.json"), "utf8"));
    } catch {
      continue;
    }
    const alive = isAlive(meta.pid);
    const done = existsSync(join(full, "done"));
    const state = done ? `done (${readFileSync(join(full, "done"), "utf8").trim()})` : alive ? "running" : "lost";
    console.log(`${state.padEnd(22)} ${meta.started ?? "?"} · ${meta.task ?? "?"}`);
    console.log(`${" ".repeat(22)} ${full}`);
  }
}

loadOfficinaEnv();
const opts = parseArgs(process.argv.slice(2));
if (opts.help) {
  console.log(
    "usage: officina exec \"task\" [--lane auto|lapis|nimbus] [--tools csv] [--max-minutes N] [--json] [--detach] [--list] [--no-trust]",
  );
  process.exit(0);
}
if (opts.list) {
  listRuns();
  process.exit(0);
}
if (!opts.task) {
  console.error("officina exec: a task prompt is required");
  process.exit(2);
}
if (!existsSync(PI_BIN)) {
  console.error(`officina exec: pi not found at ${PI_BIN}`);
  process.exit(2);
}
if (opts.detach) runDetached(opts);
else runForeground(opts);
