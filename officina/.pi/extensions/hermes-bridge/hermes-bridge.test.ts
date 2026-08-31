import { describe, it, expect } from "vitest";
import { existsSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  bridgeConfig,
  formatHits,
  headLines,
  readPersistentFacts,
  runSearch,
  sanitizeFtsQuery,
  searchSql,
  loadSqlite,
} from "./bridge.ts";

describe("bridgeConfig", () => {
  it("defaults: on, ~/.hermes paths, read-only caps", () => {
    const cfg = bridgeConfig({ HOME: "/h" });
    expect(cfg.enabled).toBe(true);
    expect(cfg.stateDb).toContain("/h/.hermes/state.db");
    expect(cfg.maxRows).toBe(5);
    expect(cfg.snippetChars).toBe(300);
  });

  it("kill switch", () => {
    expect(bridgeConfig({ TRIS_NO_HERMES_BRIDGE: "1" }).enabled).toBe(false);
  });
});

describe("SQL + FTS sanitization", () => {
  it("searchSql is read-only (no write verbs) and joins the store shape", () => {
    const sql = searchSql().toLowerCase();
    expect(sql).toContain("messages_fts match");
    expect(sql).toContain("order by rank");
    expect(sql).toContain("join messages");
    for (const verb of ["insert", "update", "delete", "drop"]) expect(sql).not.toContain(verb);
  });

  it("quotes and operators cannot break fts syntax", () => {
    expect(sanitizeFtsQuery('checkpoint " AND ')).toBe("checkpoint AND");
    expect(sanitizeFtsQuery("plain  words")).toBe("plain words");
    expect(sanitizeFtsQuery("   ")).toBe("");
  });
});

describe("formatting + reads", () => {
  it("empty result is an explicit no-matches string", () => {
    expect(formatHits([], bridgeConfig({}))).toContain("no matches");
  });

  it("rows render date, role, session, capped snippet", () => {
    const line = formatHits(
      [{ timestamp: 1787243595, role: "tool", session_id: "20260820_180301_7a1b5a", snippet: "the answer" }],
      bridgeConfig({}),
    );
    expect(line).toContain("2026-08");
    expect(line).toContain("20260820_180301");
    expect(line).toContain("the answer");
  });

  it("persistent facts: missing dir is empty string, files are head-capped", () => {
    expect(readPersistentFacts("/nonexistent", 50)).toBe("");
    const dir = mkdtempSync(join(tmpdir(), "tris-hb-"));
    try {
      writeFileSync(join(dir, "MEMORY.md"), Array.from({ length: 100 }, (_, i) => `line ${i}`).join("\n"));
      const out = readPersistentFacts(dir, 3);
      expect(out).toContain("### MEMORY.md");
      expect(out).toContain("line 2");
      expect(out).not.toContain("line 50");
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  it("missing db returns null (caller renders the reason)", async () => {
    expect(await runSearch("/nonexistent/state.db", "x", bridgeConfig({}))).toBeNull();
  });
});

const DB = bridgeConfig().stateDb;
describe.skipIf(!existsSync(DB))("live read-only search (real state.db)", () => {
  it("searches past sessions and hits real rows (not an error string)", async () => {
    const out = await runSearch(DB, "VITRIOL", bridgeConfig({}));
    expect(out).not.toBeNull();
    expect(out).not.toContain("search failed");
    expect(out).toMatch(/•|no matches/);
  }, 30_000);

  it("opening readOnly cannot write", async () => {
    const { DatabaseSync } = loadSqlite();
    const db = new DatabaseSync(DB, { readOnly: true });
    expect(() => db.prepare("UPDATE sessions SET id = id").run()).toThrow();
    db.close();
  });
});
