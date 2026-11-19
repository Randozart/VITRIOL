// Retention ring gate (2026-11-19 cap breach).
//
// Turn checkpoints are full ~2.5 GiB slot states written into the engine's
// slot_save_path. The engine's cache_max_gb budget scan does NOT count them
// (server-task.cpp disk_scan matches only cache-*/slot* prefixes), so a long
// session once wrote hundreds of GiB. The writer enforces its own ring:
// keep-last-K per session. These tests pin the pruning decision logic.

import { describe, expect, it } from "vitest";
import { prunableTurnFiles } from "./client.ts";

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
