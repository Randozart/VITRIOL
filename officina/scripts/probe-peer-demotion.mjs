#!/usr/bin/env node
// Live peer-demotion probe (2026-11-18).
//
// The peer arm of `decideTier` was only ever unit-tested. This reads the LIVE
// routing shards (what each running session last published) and reports, per
// session, whether the engine can serve them all — i.e. whether the peer arm
// would demote somebody right now. Read-only: it never switches a model.
//
//   node scripts/probe-peer-demotion.mjs
//
// Exit codes: 0 = decided (printed), 2 = no live shards, 3 = engine unreachable.

import { readdirSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const DIR = join(homedir(), ".vitriol", "officina", "routing-registry.d");
const ENGINE = process.env.VITRIOL_ENGINE_URL || "http://127.0.0.1:8279";
const TTL_MS = 60_000;

function shards(now = Date.now()) {
  let files = [];
  try {
    files = readdirSync(DIR).filter((f) => f.endsWith(".json"));
  } catch {
    return [];
  }
  const out = [];
  for (const file of files) {
    try {
      const doc = JSON.parse(readFileSync(join(DIR, file), "utf8"));
      for (const entry of doc.entries ?? []) {
        if (typeof entry.ts === "number" && now - entry.ts <= TTL_MS)
          out.push(entry);
      }
    } catch {
      // a torn shard costs one probe row, never the probe
    }
  }
  return out;
}

async function slots() {
  try {
    const res = await fetch(`${ENGINE}/slots`, {
      signal: AbortSignal.timeout(2000),
    });
    if (!res.ok) return null;
    return await res.json();
  } catch {
    return null;
  }
}

const live = shards();
const engine = await slots();
if (live.length === 0) {
  console.log("no live shards (no session has published a decision in 60 s)");
  process.exit(2);
}
if (!engine) {
  console.log(`engine unreachable at ${ENGINE} — cannot read slot occupancy`);
  process.exit(3);
}

const processing = engine.filter((s) => s.is_processing).length;
const freeSlots = Math.max(0, engine.length - processing);
const now = Date.now();

// Seniority: oldest session with the most context owns the last free slot; the
// newest/smallest are the cheap ones to bounce (the router's peer policy).
const ranked = live
  .map((e) => ({
    ...e,
    ageMs: now - (e.sessionStartedAt ?? e.ts),
  }))
  .sort((a, b) => b.ageMs - a.ageMs || b.ctxTokens - a.ctxTokens);

console.log(
  `slots ${engine.length} · processing ${processing} · free ${freeSlots}`,
);
for (const [i, e] of ranked.entries()) {
  const rank = i + 1;
  // Mirror the router rule: no free slot and I am not the senior-most session.
  const demote = freeSlots === 0 && rank > 1;
  console.log(
    `${demote ? "DEMOTE" : "keep  "} rank ${rank}/${ranked.length} · pid ${e.tuiPid} · ` +
      `${e.tier} ${e.model} · ctx ${Math.round((e.ctxTokens ?? 0) / 1000)}k · ` +
      `${Math.round(e.ageMs / 1000)}s old`,
  );
}
if (freeSlots === 0 && ranked.length === 1) {
  console.log("single session holds the only slot — nothing to demote");
}
