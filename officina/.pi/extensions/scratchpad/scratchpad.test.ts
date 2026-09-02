import { describe, it, expect } from "vitest";
import {
  applyUpdate,
  emptyDoc,
  parseScratchpad,
  renderScratchpadBlock,
  scratchpadConfig,
  serializeScratchpad,
  totalLines,
  type ScratchpadDoc,
} from "./state.ts";
import { lastCopyIsCurrent } from "./index.ts";

const doc = (over: Partial<ScratchpadDoc>): ScratchpadDoc => ({
  facts: [],
  leads: [],
  dead: [],
  ...over,
});

describe("scratchpadConfig", () => {
  it("defaults and kill switch", () => {
    const c = scratchpadConfig({});
    expect(c.enabled).toBe(true);
    expect(c.cap).toBe(60);
    expect(c.dir).toBe(".officina");
    expect(c.maxLineChars).toBe(200);
    expect(scratchpadConfig({ OFFICINA_SCRATCHPAD: "0" }).enabled).toBe(false);
    expect(scratchpadConfig({ OFFICINA_SCRATCHPAD_CAP: "20" }).cap).toBe(20);
    expect(scratchpadConfig({ OFFICINA_SCRATCHPAD_CAP: "0" }).cap).toBe(10); // floor
  });
});

describe("applyUpdate", () => {
  const cfg = scratchpadConfig({});

  it("upsert replaces named sections and leaves others untouched", () => {
    const start = doc({ facts: ["old fact"], leads: ["lead one", "lead two"] });
    const v = applyUpdate(start, { facts: ["new fact"] }, cfg);
    expect(v.error).toBeUndefined();
    expect(v.doc?.facts).toEqual(["new fact"]);
    expect(v.doc?.leads).toEqual(["lead one", "lead two"]); // pruned by omission
    expect(v.doc?.dead).toEqual([]);
  });

  it("empty array clears a section", () => {
    const v = applyUpdate(doc({ dead: ["x"] }), { dead: [] }, cfg);
    expect(v.doc?.dead).toEqual([]);
  });

  it("reset wipes everything before applying", () => {
    const start = doc({ facts: ["a"], leads: ["b"], dead: ["c"] });
    const v = applyUpdate(start, { reset: true, facts: ["fresh"] }, cfg);
    expect(v.doc?.leads).toEqual([]);
    expect(v.doc?.dead).toEqual([]);
    expect(v.doc?.facts).toEqual(["fresh"]);
  });

  it("rejects non-arrays, empty entries, overlong lines", () => {
    expect(applyUpdate(emptyDoc(), { facts: "nope" as unknown as string[] }, cfg).error).toContain("array");
    expect(applyUpdate(emptyDoc(), { facts: ["  "] }, cfg).error).toContain("empty entry");
    expect(applyUpdate(emptyDoc(), { facts: ["x".repeat(201)] }, cfg).error).toContain("200");
  });

  it("enforces the total line cap with pruning guidance", () => {
    const tight = scratchpadConfig({ OFFICINA_SCRATCHPAD_CAP: "10" });
    const start = doc({ facts: Array.from({ length: 6 }, (_, i) => `f${i}`), leads: Array.from({ length: 4 }, (_, i) => `l${i}`) });
    const v = applyUpdate(start, { dead: ["one more"] }, tight);
    expect(v.error).toContain("cap exceeded");
    expect(v.error).toContain("11 lines > 10");
  });
});

describe("parse/serialize round trip", () => {
  it("round-trips sections and drops empty ones from the file", () => {
    const d = doc({ facts: ["n1", "n2"], leads: ["l1"] });
    const text = serializeScratchpad(d);
    expect(text).toContain("# Scratchpad");
    expect(text).not.toContain("## dead");
    const back = parseScratchpad(text);
    expect(back.facts).toEqual(["n1", "n2"]);
    expect(back.leads).toEqual(["l1"]);
    expect(back.dead).toEqual([]);
  });

  it("parses to empty on garbage", () => {
    const back = parseScratchpad("hello\nworld\n## unknown\n- x");
    expect(totalLines(back)).toBe(0);
  });
});

describe("renderScratchpadBlock", () => {
  it("renders empty string for an empty notebook", () => {
    expect(renderScratchpadBlock(emptyDoc(), 60)).toBe("");
  });

  it("includes header, counts, and section entries", () => {
    const b = renderScratchpadBlock(doc({ facts: ["a"], dead: ["b"] }), 60);
    expect(b).toContain("Scratchpad");
    expect(b).toContain("[2/60 lines]");
    expect(b).toContain("### facts");
    expect(b).toContain("- a");
    expect(b).toContain("### dead");
    expect(b).not.toContain("### leads");
  });
});

describe("lastCopyIsCurrent", () => {
  const msg = (content: string) => ({ role: "custom", customType: "lc-scratchpad", content });

  it("false when never injected", () => {
    expect(lastCopyIsCurrent([{ role: "user", content: "hi" }], "X")).toBe(false);
  });

  it("true when the tail copy matches exactly, even with newer non-scratch messages", () => {
    expect(lastCopyIsCurrent([msg("A"), msg("B"), { role: "user", content: "q" }], "B")).toBe(true);
    expect(lastCopyIsCurrent([msg("A"), msg("B")], "B")).toBe(true);
  });

  it("false when the tail copy differs", () => {
    expect(lastCopyIsCurrent([msg("A"), msg("B")], "A")).toBe(false);
  });
});
