import { describe, expect, it } from "vitest";
import { contextChars, renderLedger, upsertLedger } from "./ledger.ts";

describe("renderLedger", () => {
  it("renders the compact line", () => {
    const out = renderLedger({ messages: 42, approxTokens: 12_345, files: ["a.ts", "b.py"], edits: 7 }, 4);
    expect(out).toContain("msgs=42");
    expect(out).toContain("ctx≈12k tok");
    expect(out).toContain("edits=7");
    expect(out).toContain("files=[a.ts, b.py]");
  });
  it("caps files and omits when empty", () => {
    const out = renderLedger({ messages: 1, approxTokens: 5, files: ["a", "b", "c"], edits: 0 }, 2);
    expect(out).toContain("[b, c]");
    const empty = renderLedger({ messages: 1, approxTokens: 5, files: [], edits: 0 }, 4);
    expect(empty).not.toContain("files=");
  });
});

describe("contextChars", () => {
  it("sums string and text-block content", () => {
    expect(contextChars([{ content: "abcd" }, { content: [{ type: "text", text: "efgh" }] }, { content: [{ type: "image", image: "zz" }] }])).toBe(8);
  });
});

describe("upsertLedger", () => {
  it("appends when absent, without mutating input", () => {
    const input: unknown[] = [{ role: "user", content: "hi" }];
    const out = upsertLedger(input, "[ledger: msgs=1]");
    expect(out).toHaveLength(2);
    expect((out[1] as { customType: string }).customType).toBe("lc-ledger");
    expect(input).toHaveLength(1);
  });
  it("replaces in place on repeat passes — never accumulates", () => {
    let msgs: unknown[] = [{ role: "user", content: "hi" }];
    msgs = upsertLedger(msgs, "[ledger: v1]");
    msgs = upsertLedger(msgs, "[ledger: v2]");
    const ledgers = msgs.filter((m) => (m as { customType?: string }).customType === "lc-ledger");
    expect(msgs).toHaveLength(2);
    expect(ledgers).toHaveLength(1);
    expect((ledgers[0] as { content: string }).content).toContain("v2");
    expect((ledgers[0] as { display: boolean }).display).toBe(false);
  });
});
