import { afterEach, describe, expect, it } from "vitest";
import { budgetStatus, canSpendCloud } from "./budget.ts";

const now = new Date().toISOString();
afterEach(() => {
  delete process.env.OFFICINA_FREE_DAILY_TOKENS;
  delete process.env.OFFICINA_FREE_MONTHLY_TOKENS;
  delete process.env.OFFICINA_PAID_ALLOWED;
});

describe("free budget", () => {
  it("counts BILLED cloud tokens only, never local or zero-cost lanes", () => {
    process.env.OFFICINA_FREE_DAILY_TOKENS = "100";
    const b = budgetStatus([
      { ts: now, reason: "user", tier: "local", tokensIn: 99_999, tokensOut: 1 },
      { ts: now, reason: "user", tier: "cloud", tokensIn: 1_000_000, tokensOut: 0, costEstimate: 0 },
      { ts: now, reason: "user", tier: "cloud", tokensIn: 40, tokensOut: 10, costEstimate: 0.002 },
    ]);
    expect(b.dailyTokens).toBe(50);
    expect(b.allowed).toBe(true);
  });

  it("refuses when the free daily cap is reached", () => {
    process.env.OFFICINA_FREE_DAILY_TOKENS = "10";
    expect(
      canSpendCloud(),
    ).toBeTypeOf("boolean");
    const b = budgetStatus([
      { ts: now, reason: "user", tier: "cloud", tokensIn: 10, tokensOut: 0, costEstimate: 0.001 },
    ]);
    expect(b.allowed).toBe(false);
  });

  it("free-lane floods never exhaust the cap", () => {
    process.env.OFFICINA_FREE_DAILY_TOKENS = "10";
    const b = budgetStatus([
      { ts: now, reason: "user", tier: "cloud", tokensIn: 9_000_000, tokensOut: 0, costEstimate: 0 },
    ]);
    expect(b.dailyTokens).toBe(0);
    expect(b.allowed).toBe(true);
  });

  it("never allows automatic paid", () => {
    delete process.env.OFFICINA_PAID_ALLOWED;
    expect(budgetStatus([]).paidAllowed).toBe(false);
  });
});
