import { describe, it, expect } from "vitest";
import { makeComponent } from "./index.ts";

// Live regression for issue #51 (and the #48 reopen — same root cause from a
// different code path). pi paints the dispatch tool-result panel with a 1-char
// background-color left margin + fill; any line we return wider than
// `width - 1` overflows pi-tui and crashes the session, including on
// `--resume` because pi re-renders saved tool results from session history.
//
// The user's crash log showed a 134-char sub-coder report sentence rendered
// at terminal width 133 → 135 > 133.
//
// v1.9.4 fixed this by truncating to width-2. v1.9.5 (PR #49 by
// @steverhoades) replaced the truncation with **word-wrap**: a 134-char
// sentence becomes two visual lines that together preserve the full
// sentence, instead of dropping the tail. Both behaviors satisfy the
// width invariant — this test asserts (a) no emitted line exceeds and (b)
// the wide content is preserved across the wrapped lines (no data loss).

const stripAnsi = (s: string) => s.replace(/\x1b\[[0-9;]*[a-zA-Z]/g, "");
const visibleWidth = (s: string) => stripAnsi(s).length;

describe("issue #51 — dispatch renderResult doesn't overflow", () => {
  it("wraps a wide sub-coder report line to fit the pi-supplied width", () => {
    const wideSentence =
      "There is **no `rate_limits` table**. The entire file defines a single class, `ConversationStore`, which manages only one SQLite table:";
    // Sanity: this is the exact 134-char shape from the user's crash log.
    expect(wideSentence.length).toBeGreaterThan(133);
    const comp = makeComponent([
      "✓ Storage schema",
      "**Report: `bot/storage.py` Schema Analysis**",
      "",
      wideSentence,
      "",
      "  …",
      "(Ctrl+O to expand)",
    ]);
    const out = comp.render(133);
    // Width invariant: no emitted line exceeds the pi-supplied width.
    const max = Math.max(...out.map((l) => visibleWidth(l)));
    expect(max).toBeLessThanOrEqual(133);
    // The wide sentence wraps but is preserved: rejoining the wrapped lines
    // (collapsing whitespace) reproduces the original prose verbatim. That's
    // the user-visible win over v1.9.4's truncate-with-ellipsis.
    const wrappedRoundtrip = out
      .map((l) => stripAnsi(l))
      .join(" ")
      .replace(/\s+/g, " ")
      .trim();
    expect(wrappedRoundtrip).toContain(
      "There is **no `rate_limits` table**. The entire file defines a single class, `ConversationStore`, which manages only one SQLite table:",
    );
    // And it's wrapped, not truncated — there are more emitted lines than
    // input lines because the long sentence split.
    expect(out.length).toBeGreaterThan(7);
  });

  it("survives a narrow terminal (40 cols) without throwing", () => {
    const comp = makeComponent([
      "very long content " + "x".repeat(500),
      "another long line " + "y".repeat(200),
    ]);
    const out = comp.render(40);
    expect(Math.max(...out.map((l) => visibleWidth(l)))).toBeLessThanOrEqual(40);
  });

  it("preserves short lines unchanged", () => {
    const comp = makeComponent(["short", "tiny"]);
    expect(comp.render(133)).toEqual(["short", "tiny"]);
  });

  it("chunks long whitespace-free tokens (URLs/paths) so wrapping has room to split", () => {
    // A 200-char URL-ish token has no spaces; without sanitizeLongTokens it
    // would defeat word-wrap and overflow at any narrow width.
    const url = "https://example.com/" + "a".repeat(200);
    const comp = makeComponent([url]);
    const out = comp.render(60);
    expect(Math.max(...out.map((l) => visibleWidth(l)))).toBeLessThanOrEqual(60);
  });
});
