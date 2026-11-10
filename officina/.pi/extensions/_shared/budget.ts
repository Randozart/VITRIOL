// Free-only budget firewall. Automatic routing never selects paid models;
// caps apply to free tokens so a misconfigured provider cannot burn quota.

import { readRouteEntries, summarizeRoutes, type RouteLedgerEntry } from "./route-ledger.ts";

function num(name: string, dflt: number): number {
  const n = Number(process.env[name]);
  return Number.isFinite(n) && n >= 0 ? Math.floor(n) : dflt;
}

export interface BudgetStatus {
  paidAllowed: boolean;
  dailyCap: number;
  monthlyCap: number;
  dailyTokens: number;
  monthlyTokens: number;
  dailyLeft: number;
  monthlyLeft: number;
  allowed: boolean;
}

export function budgetStatus(entries?: RouteLedgerEntry[]): BudgetStatus {
  const dailyCap = num("OFFICINA_FREE_DAILY_TOKENS", 2_000_000);
  const monthlyCap = num("OFFICINA_FREE_MONTHLY_TOKENS", 40_000_000);
  const rows = entries ?? readRouteEntries(31);
  const today = new Date().toISOString().slice(0, 10);
  const month = today.slice(0, 7);
  let daily = 0;
  let monthly = 0;
  for (const e of rows) {
    if (e.tier !== "cloud") continue; // local tokens are free
    const tokens = (e.tokensIn ?? 0) + (e.tokensOut ?? 0) + (e.cacheRead ?? 0) + (e.cacheWrite ?? 0);
    if (e.ts.slice(0, 10) === today) daily += tokens;
    if (e.ts.slice(0, 7) === month) monthly += tokens;
  }
  return {
    paidAllowed: process.env.OFFICINA_PAID_ALLOWED === "1",
    dailyCap,
    monthlyCap,
    dailyTokens: daily,
    monthlyTokens: monthly,
    dailyLeft: Math.max(0, dailyCap - daily),
    monthlyLeft: Math.max(0, monthlyCap - monthly),
    allowed: daily < dailyCap && monthly < monthlyCap,
  };
}

export function canSpendCloud(): boolean {
  return budgetStatus().allowed;
}

export function budgetSummaryText(): string {
  const b = budgetStatus();
  return `free tokens today ${b.dailyTokens}/${b.dailyCap} · month ${b.monthlyTokens}/${b.monthlyCap} · paid ${b.paidAllowed ? "allowed (manual)" : "never"}`;
}
