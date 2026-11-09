#!/usr/bin/env node
// Prompt-cache report for an Officina/pi session JSONL.

import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { computeCacheReport, parseSessionJsonl } from "./cache-report-lib.mjs";

function expand(path) {
  return path.startsWith("~/") ? join(homedir(), path.slice(2)) : path;
}

function latestSession(root) {
  let best = null;
  const walk = (dir) => {
    let names;
    try {
      names = readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const ent of names) {
      const path = join(dir, ent.name);
      if (ent.isDirectory()) {
        walk(path);
      } else if (ent.isFile() && ent.name.endsWith(".jsonl")) {
        try {
          const mtime = statSync(path).mtimeMs;
          if (!best || mtime > best.mtime) best = { path, mtime };
        } catch {
          // ignore unreadable
        }
      }
    }
  };
  walk(root);
  return best?.path ?? null;
}

function parseArgs(argv) {
  const opts = { json: false };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--json") opts.json = true;
    else if (arg === "--file") opts.file = argv[++i];
    else if (arg === "--sessions") opts.sessions = argv[++i];
    else if (arg === "--cwd") opts.cwd = argv[++i];
    else if (arg === "--help" || arg === "-h") opts.help = true;
    else if (!arg.startsWith("--")) opts.file ??= arg;
  }
  return opts;
}

function fmt(n) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) {
    console.log(`usage: cache-report [--file SESSION.jsonl] [--sessions DIR] [--cwd DIR] [--json]

Reports prompt-cache reads/writes, hit rate, and likely misses for a pi session.
Default session root: ~/.pi/agent/sessions (latest .jsonl by mtime).
--cwd scopes to that project's encoded session directory.`);
    return;
  }

  const sessionsRoot = expand(opts.sessions ?? "~/.pi/agent/sessions");
  const root = opts.cwd
    ? join(
        sessionsRoot,
        `--${resolve(expand(opts.cwd)).replace(/^[/\\]/, "").replace(/[/\\:]/g, "-")}--`,
      )
    : sessionsRoot;
  const file = opts.file ? expand(opts.file) : latestSession(root);
  if (!file || !existsSync(file)) {
    console.error(`[cache-report] no session JSONL found (${file ?? root})`);
    process.exit(1);
  }

  const report = computeCacheReport(parseSessionJsonl(readFileSync(file, "utf8")));
  if (opts.json) {
    console.log(JSON.stringify({ file, ...report }, null, 2));
    return;
  }

  console.log(`Session: ${file}`);
  console.log(`Turns: ${report.turns}  model switches: ${report.modelSwitches}`);
  console.log(`Prompt: ${fmt(report.promptTokens)}  cache read: ${fmt(report.cacheRead)}  cache write: ${fmt(report.cacheWrite)}`);
  console.log(`Hit rate: ${(report.hitRate * 100).toFixed(1)}%`);
  console.log(`Misses: ${report.missCount}  missed tokens: ${fmt(report.missedTokens)}  extra cost: $${report.missedCost.toFixed(6)}`);
  console.log(`Likely idle-TTL misses: ${report.idleMisses}`);
  if (report.hitRate === 0 && report.turns > 1) {
    console.log("No cache activity reported. Check provider cache support / OFFICINA_CACHE_MODE.");
  }
}

main();
