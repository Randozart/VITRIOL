// Route ledger — durable, privacy-safe routing telemetry.
//
// Records ONLY routing metadata: no prompts, no completions, no keys.
// File: ~/.vitriol/officina/route-ledger-YYYYMMDD.jsonl (mode 600), 30-day
// retention. Shared by resilient-default and adaptive-router.

import {
  appendFileSync,
  chmodSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
} from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

export interface RouteLedgerEntry {
  ts: string;
  sessionId?: string;
  reason: string;
  tier: "local" | "cloud";
  provider?: string;
  model?: string;
  owner?: string;
  localState?: "up" | "down" | "busy" | "unknown";
  profile?: string;
  candidatesSkipped?: string[];
  latencyMs?: number;
  tokensIn?: number;
  tokensOut?: number;
  cacheRead?: number;
  cacheWrite?: number;
  costEstimate?: number;
  outcome?: "ok" | "error" | "cancelled";
  errorClass?: string;
}

interface LedgerSummary {
  requests: number;
  localRequests: number;
  cloudRequests: number;
  failures: number;
  tokensIn: number;
  tokensOut: number;
  cacheRead: number;
  cacheWrite: number;
  costEstimate: number;
  byModel: Record<string, { requests: number; tokensIn: number; tokensOut: number; failures: number; cost: number }>;
  byTier: { local: number; cloud: number };
}

const RETENTION_DAYS = 30;
let lastCleanup = 0;

function ledgerDir(): string {
  return process.env.OFFICINA_ROUTE_LEDGER_DIR || join(homedir(), ".vitriol", "officina");
}

function dayStamp(date = new Date()): string {
  return date.toISOString().slice(0, 10).replace(/-/g, "");
}

function ledgerPath(day = dayStamp()): string {
  return join(ledgerDir(), `route-ledger-${day}.jsonl`);
}

function cleanup(): void {
  const now = Date.now();
  if (now - lastCleanup < 3_600_000) return;
  lastCleanup = now;
  try {
    const cutoff = now - RETENTION_DAYS * 86_400_000;
    for (const name of readdirSync(ledgerDir())) {
      if (!/^route-ledger-\d{8}\.jsonl$/.test(name)) continue;
      const stamp = name.slice("route-ledger-".length, -".jsonl".length);
      const at = Date.parse(`${stamp.slice(0, 4)}-${stamp.slice(4, 6)}-${stamp.slice(6, 8)}T00:00:00Z`);
      if (Number.isFinite(at) && at < cutoff) rmSync(join(ledgerDir(), name), { force: true });
    }
  } catch {
    // telemetry must never break routing
  }
}

export function recordRoute(entry: RouteLedgerEntry): void {
  try {
    mkdirSync(ledgerDir(), { recursive: true, mode: 0o700 });
    const path = ledgerPath();
    appendFileSync(path, `${JSON.stringify({ ...entry, ts: entry.ts || new Date().toISOString() })}\n`, {
      mode: 0o600,
    });
    chmodSync(path, 0o600);
    cleanup();
  } catch {
    // best-effort
  }
}

export function readRouteEntries(days = 30): RouteLedgerEntry[] {
  const out: RouteLedgerEntry[] = [];
  const cutoff = Date.now() - days * 86_400_000;
  try {
    const names = readdirSync(ledgerDir())
      .filter((name) => /^route-ledger-\d{8}\.jsonl$/.test(name))
      .sort()
      .slice(-days);
    for (const name of names) {
      const path = join(ledgerDir(), name);
      const stamp = name.slice("route-ledger-".length, -".jsonl".length);
      const at = Date.parse(`${stamp.slice(0, 4)}-${stamp.slice(4, 6)}-${stamp.slice(6, 8)}T00:00:00Z`);
      if (Number.isFinite(at) && at + 86_400_000 < cutoff) continue;
      if (!existsSync(path)) continue;
      for (const line of readFileSync(path, "utf8").split("\n")) {
        if (!line.trim()) continue;
        try {
          out.push(JSON.parse(line) as RouteLedgerEntry);
        } catch {
          // ignore a torn line
        }
      }
    }
  } catch {
    // no ledger yet
  }
  out.sort((a, b) => a.ts.localeCompare(b.ts));
  return out;
}

export function summarizeRoutes(entries: RouteLedgerEntry[]): LedgerSummary {
  const summary: LedgerSummary = {
    requests: 0,
    localRequests: 0,
    cloudRequests: 0,
    failures: 0,
    tokensIn: 0,
    tokensOut: 0,
    cacheRead: 0,
    cacheWrite: 0,
    costEstimate: 0,
    byModel: {},
    byTier: { local: 0, cloud: 0 },
  };
  for (const e of entries) {
    summary.requests += 1;
    if (e.tier === "local") summary.localRequests += 1;
    else summary.cloudRequests += 1;
    summary.byTier[e.tier] += 1;
    if (e.outcome === "error") summary.failures += 1;
    summary.tokensIn += e.tokensIn ?? 0;
    summary.tokensOut += e.tokensOut ?? 0;
    summary.cacheRead += e.cacheRead ?? 0;
    summary.cacheWrite += e.cacheWrite ?? 0;
    summary.costEstimate += e.costEstimate ?? 0;
    const key = `${e.provider ?? "?"}/${e.model ?? "?"}`;
    const row = (summary.byModel[key] ??= { requests: 0, tokensIn: 0, tokensOut: 0, failures: 0, cost: 0 });
    row.requests += 1;
    row.tokensIn += e.tokensIn ?? 0;
    row.tokensOut += e.tokensOut ?? 0;
    row.failures += e.outcome === "error" ? 1 : 0;
    row.cost += e.costEstimate ?? 0;
  }
  return summary;
}

export function lastRouteEntry(): RouteLedgerEntry | null {
  const entries = readRouteEntries(2);
  return entries.length ? entries[entries.length - 1] : null;
}

export function todaySummary(): LedgerSummary {
  return summarizeRoutes(readRouteEntries(1).filter((e) => e.ts.slice(0, 10) === new Date().toISOString().slice(0, 10)));
}
