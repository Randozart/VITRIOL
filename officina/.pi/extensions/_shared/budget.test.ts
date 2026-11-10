import { afterEach, describe, expect, it } from "vitest";
import { budgetStatus, canSpendCloud } from "./budget.ts";

const now = new Date().toISOString();
afterEach(() => {
  delete process.env.OFFICINA_FREE_DAILY_TOKENS;
  delete process.env.OFFICINA_FREE_MONTHLY_TOKENS;
  delete process.env.OFFICINA_PAID_ALLOWED;
});

describe("free budget", () => {
  it("counts cloud tokens only, never local", () => {
    process.env.OFFICINA_FREE_DAILY_TOKENS = "100";
    const b = budgetStatus([
      { ts: now, reason: "user", tier: "local", tokensIn: 99_999, tokensOut: 1 },
      { ts: now, reason: "user", tier: "cloud", tokensIn: 40, tokensOut: 10 },
    ]);
    expect(b.dailyTokens).toBe(50);
    expect(b.allowed).toBe(true);
  });

  it("refuses when the free daily cap is reached", () => {
    process.env.OFFICINA_FREE_DAILY_TOKENS = "10";
    expect(
      canSpendCloud(),
    ).toBeTypeOf("boolean");
    const b = budgetStatus([{ ts: now, reason: "user", tier: "cloud", tokensIn: 10, tokensOut: 0 }]);
    expect(b.allowed).toBe(false);
  });

  it("never allows automatic paid", () => {
    delete process.env.OFFICINA_PAID_ALLOWED;
    expect(budgetStatus([]).paidAllowed).toBe(false);
  });
});
