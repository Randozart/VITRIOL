#!/usr/bin/env node
// Config drift detector for the Officina harness (verification-rollout T0.3).
//
// Answers one question, the question nobody asked in 2026-09-03: "did the
// config silently change since we last blessed it?"
//
//   config-drift.mjs            check live config against last-known-good
//   config-drift.mjs --bless    record the current config as good
//   config-drift.mjs --json     machine-readable report
//
// The live config is READ-ONLY here. The only file this writes is the
// last-known-good store (mode 600), and only on --bless.
//
// Exit codes: 0 clean, 1 schema violation (a required key is gone),
//             2 drift (fingerprint differs from last known good).

import { readFileSync, writeFileSync, existsSync, mkdirSync, chmodSync } from "node:fs";
import { join, dirname } from "node:path";
import { homedir } from "node:os";
import {
  diffFingerprints,
  fingerprintConfig,
  parseEnvFile,
  validateConfig,
} from "./config-schema.mjs";

const HOME = process.env.HOME || homedir();

function arg(name, fallback) {
  const i = process.argv.indexOf(name);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

const ENV_FILE = arg("--env", join(HOME, ".vitriol", "officina", "env"));
const SETTINGS_FILE = arg("--settings", join(HOME, ".pi", "agent", "settings.json"));
const STORE_FILE = arg(
  "--store",
  join(HOME, ".vitriol", "officina", "config-last-known-good.json"),
);
const asJson = process.argv.includes("--json");
const bless = process.argv.includes("--bless");

function readEnv(path) {
  if (!existsSync(path)) return null;
  try {
    return parseEnvFile(readFileSync(path, "utf8"));
  } catch {
    return null;
  }
}

function readSettings(path) {
  if (!existsSync(path)) return undefined;
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch {
    return undefined;
  }
}

function readStore(path) {
  if (!existsSync(path)) return null;
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch {
    return null;
  }
}

const env = readEnv(ENV_FILE);
const settings = readSettings(SETTINGS_FILE);
const validation = validateConfig({ env, settings });
const { hash, pairs } = fingerprintConfig({ env: env ?? {}, settings });
const previous = readStore(STORE_FILE);
const diff = previous ? diffFingerprints(previous.pairs, pairs) : null;

const report = {
  envFile: ENV_FILE,
  envFound: env !== null,
  settingsFile: SETTINGS_FILE,
  settingsFound: settings !== undefined,
  storeFile: STORE_FILE,
  hash,
  pairs,
  errors: validation.errors,
  warnings: validation.warnings,
  drift: diff
    ? { clean: diff.clean, added: diff.added, removed: diff.removed, changed: diff.changed }
    : null,
  previousHash: previous?.hash ?? null,
  blessedAt: previous?.blessedAt ?? null,
};

if (bless) {
  if (!validation.ok) {
    if (asJson) console.log(JSON.stringify({ ...report, blessed: false }, null, 2));
    else {
      console.error("✗ refusing to bless: schema violations present");
      for (const e of validation.errors) console.error(`  ${e.key}: ${e.why}`);
    }
    process.exit(1);
  }
  mkdirSync(dirname(STORE_FILE), { recursive: true });
  const payload = { hash, pairs, blessedAt: new Date().toISOString() };
  writeFileSync(STORE_FILE, `${JSON.stringify(payload, null, 2)}\n`, { mode: 0o600 });
  chmodSync(STORE_FILE, 0o600);
  if (asJson) console.log(JSON.stringify({ ...report, blessed: true }, null, 2));
  else console.log(`✓ blessed ${STORE_FILE} (fingerprint ${hash})`);
  process.exit(0);
}

if (asJson) {
  console.log(JSON.stringify(report, null, 2));
} else {
  console.log(`config fingerprint: ${hash}`);
  if (!env) console.log(`  env file missing: ${ENV_FILE}`);
  if (settings === undefined) console.log(`  settings file missing: ${SETTINGS_FILE}`);
  for (const e of validation.errors) console.log(`  ✗ ${e.key}: ${e.why}`);
  for (const w of validation.warnings) console.log(`  ! ${w.key}: ${w.why}`);
  if (!diff) {
    console.log(`  (no last-known-good at ${STORE_FILE}; run --bless to arm drift checks)`);
  } else if (diff.clean) {
    console.log(`  ✓ no drift since ${report.blessedAt}`);
  } else {
    console.log(`  ⚠ drift since ${report.blessedAt}:`);
    for (const [k, v] of diff.added) console.log(`    + ${k}=${v}`);
    for (const [k] of diff.removed) console.log(`    - ${k}`);
    for (const c of diff.changed) console.log(`    ~ ${c.key}: ${c.from} -> ${c.to}`);
    console.log("    (intentional? re-run with --bless to accept)");
  }
}

if (!validation.ok) process.exit(1);
if (diff && !diff.clean) process.exit(2);
process.exit(0);
