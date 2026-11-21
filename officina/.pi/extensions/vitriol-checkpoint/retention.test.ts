// Retention ring gate (2026-11-19 cap breach).
//
// Turn checkpoints are full ~2.5 GiB slot states written into the engine's
// slot_save_path. The engine's cache_max_gb budget scan does NOT count them
// (server-task.cpp disk_scan matches only cache-*/slot* prefixes), so a long
// session once wrote hundreds of GiB. The writer enforces its own ring:
// keep-last-K per session. These tests pin the pruning decision logic.

import { describe, expect, it } from "vitest";
import { overBudgetTurnFiles, isTurnFile, prunableTurnFiles } from "./client.ts";

const STEM = "2026-11-14T13-45-57-796Z_abc123.jsonl.patched-x";

function ownTurns(turns: number[], stem = STEM): string[] {
  return turns.map((t) => `${stem}-turn-${t}.bin`);
}

describe("prunableTurnFiles retention ring", () => {
  it("keeps the newest K turns of this session", () => {
    const files = [...ownTurns([10, 20, 30, 40, 50]), "slot0.bin"];
    const out = prunableTurnFiles(files, STEM, 3, 50);
    // 10/20 pruned, 30/40/50 kept.
    expect(out.sort()).toEqual([`${STEM}-turn-10.bin`, `${STEM}-turn-20.bin`]);
  });

  it("never prunes other sessions' files", () => {
    const other = "other-session-turn-1.bin";
    const files = [...ownTurns([10, 20]), other, "cache-abc.bin", "slot0.bin"];
    const out = prunableTurnFiles(files, STEM, 1, 20);
    expect(out).toEqual([`${STEM}-turn-10.bin`]);
    expect(out).not.toContain(other);
  });

  it("keeps the just-saved turn even when it is the oldest number", () => {
    // Rewind writes low turn numbers; the state we just saved must survive.
    const files = ownTurns([5, 50, 60, 70]);
    const out = prunableTurnFiles(files, STEM, 3, 5);
    expect(out).not.toContain(`${STEM}-turn-5.bin`);
    // 70, 60, 5 kept → 50 pruned.
    expect(out).toEqual([`${STEM}-turn-50.bin`]);
  });

  it("ignores non-turn files and malformed names", () => {
    const files = [
      `${STEM}-turn-x.bin`, // malformed turn
      `${STEM}-turn.bin`,
      `${STEM}-turn-10.bin.bak`,
      `slot0.bin`,
      "cache-1234.bin",
    ];
    const out = prunableTurnFiles(files, STEM, 3, 10);
    expect(out).toEqual([]);
  });

  it("deduplicates: a turn saved twice is kept once", () => {
    const files = ownTurns([10, 10, 20, 30]);
    const out = prunableTurnFiles(files, STEM, 1, 30);
    expect(out.sort()).toEqual([`${STEM}-turn-10.bin`, `${STEM}-turn-20.bin`]);
  });

  it("keep < 1 is clamped to at least one survivor", () => {
    const files = ownTurns([10, 20]);
    const out = prunableTurnFiles(files, STEM, 0, 20);
    expect(out).toEqual([`${STEM}-turn-10.bin`]);
  });
});

// Global budget sweep (2026-11-21): the per-session ring still let DEAD
// sessions hold 3 x ~2.5 GiB each — 63 GiB observed on disk against a 40 GiB
// cache_max_gb. The writer now caps turn files across ALL sessions.
const GiB = 1024 ** 3;
const turn = (stem: string, n: number, sizeGb: number, mtimeMs: number) => ({
  name: `${stem}-turn-${n}.bin`,
  size: sizeGb * GiB,
  mtimeMs,
});

describe("overBudgetTurnFiles global budget", () => {
  it("deletes oldest first until the total fits", () => {
    const files = [
      turn("s1", 10, 3, 1000),
      turn("s2", 20, 3, 2000),
      turn("s3", 30, 3, 3000),
    ];
    const out = overBudgetTurnFiles(files, 4 * GiB);
    expect(out).toEqual([
      "s1-turn-10.bin",
      "s2-turn-20.bin",
    ]);
  });

  it("under budget deletes nothing (negative control)", () => {
    expect(overBudgetTurnFiles([turn("s1", 10, 1, 1000)], 8 * GiB)).toEqual([]);
  });

  it("ignores engine spills — cache_max_gb owns those", () => {
    const files = [
      { name: "cache-abc-def.bin", size: 100 * GiB, mtimeMs: 1 },
      { name: "slot0.bin", size: 100 * GiB, mtimeMs: 1 },
      turn("s1", 10, 1, 1000),
    ];
    expect(overBudgetTurnFiles(files, 8 * GiB)).toEqual([]);
  });

  it("never deletes a protected file, even when it is the oldest", () => {
    const files = [turn("s1", 10, 5, 1000), turn("s1", 20, 5, 2000)];
    const out = overBudgetTurnFiles(files, 6 * GiB, new Set(["s1-turn-10.bin"]));
    expect(out).toEqual(["s1-turn-20.bin"]);
  });

  it("isTurnFile matches only turn checkpoints", () => {
    expect(isTurnFile("a.jsonl.patched-b-turn-90.bin")).toBe(true);
    expect(isTurnFile("cache-48dd-4bcb.bin")).toBe(false);
    expect(isTurnFile("turn-90.bin")).toBe(false);
    expect(isTurnFile("a-turn-90.json")).toBe(false);
  });
});
