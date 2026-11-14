import { describe, expect, it } from "vitest";
import {
  contextChars,
  isLedgerMessage,
  renderLedger,
} from "./ledger.ts";

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

describe("isLedgerMessage", () => {
  it("recognizes only ledger custom messages", () => {
    expect(isLedgerMessage({ role: "custom", customType: "lc-ledger" })).toBe(true);
    expect(isLedgerMessage({ role: "user", customType: "lc-ledger" })).toBe(false);
    expect(isLedgerMessage({ role: "custom", customType: "lc-tasks" })).toBe(false);
  });
});
