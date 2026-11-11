import { describe, expect, it } from "vitest";
import {
  contextChars,
  isLedgerMessage,
  relocateLedger,
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

describe("relocateLedger", () => {
  it("appends at the tail when absent, without mutating input", () => {
    const input: unknown[] = [{ role: "user", content: "hi" }];
    const out = relocateLedger(input, "[ledger: msgs=1]");
    expect(out).toHaveLength(2);
    expect(isLedgerMessage(out[1])).toBe(true);
    expect(input).toHaveLength(1);
    expect(out[0]).toBe(input[0]); // identity preserved
  });

  it("relocates a mid-history ledger to the tail — never accumulates", () => {
    const sys = { role: "system", content: "sys" };
    const u1 = { role: "user", content: "u1" };
    let msgs: unknown[] = relocateLedger([sys, u1], "[ledger: v1]");
    // Conversation grew after the ledger sat at the tail.
    const grown = [...msgs, { role: "assistant", content: "a1" }, { role: "user", content: "u2" }];
    msgs = relocateLedger(grown, "[ledger: v2]");
    // [sys, u1] + old ledger + [a1, u2] + fresh ledger, old one dropped.
    expect(msgs).toHaveLength(5);
    const ledgers = msgs.filter(isLedgerMessage);
    expect(ledgers).toHaveLength(1);
    expect((ledgers[0] as { content: string }).content).toContain("v2");
    // Ledger is the LAST message.
    expect(isLedgerMessage(msgs[msgs.length - 1])).toBe(true);
    expect(msgs[msgs.length - 2]).toEqual({ role: "user", content: "u2" });
  });

  // I4 (prefix stability): turn N+1's context extends turn N's up to the
  // ledger position — the engine-visible prefix through the real
  // conversation is untouched, so KV reuse survives the whole exchange.
  it("keeps a growing shared prefix across consecutive turns", () => {
    const sys = { role: "system", content: "sys" };
    let msgs: unknown[] = [sys, { role: "user", content: "u1" }];
    let prev: unknown[] = relocateLedger(msgs, "[ledger: t1]");
    for (let t = 2; t <= 5; t++) {
      const next = relocateLedger(
        [...prev, { role: "assistant", content: `a${t - 1}` }, { role: "user", content: `u${t}` }],
        `[ledger: t${t}]`,
      );
      // Every NON-LEDGER message of the previous pass appears unchanged,
      // in order, in the new pass (identity = reference for kept ones).
      const prevReal = prev.filter((m) => !isLedgerMessage(m));
      for (let i = 0; i < prevReal.length; i++) {
        expect(next[i]).toBe(prevReal[i]);
      }
      // Break point is at most the ledger slot: everything before the new
      // ledger equals the previous real prefix + the delta exchange.
      prev = next;
    }
    expect(prev.filter(isLedgerMessage)).toHaveLength(1);
  });
});
