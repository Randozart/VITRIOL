// Decode band payload tests (2026-11-19).
//
// The bug these exist for: the "engine down" line was a belowEditor WIDGET while
// the live decode line was a band ROW, and neither path cleared the other. An
// engine flap therefore left two rows about the same subject in a three-row
// strip; which one you could read depended on the layout's settled height, so the
// row flickered instead of saying anything. One subject, one row.

import { describe, expect, it, afterEach } from "vitest";
import { DECODE_DOWN_LINE, decodeBandRows } from "./decode.ts";
import { bandLines, resetBand, setBandRow } from "../_shared/band.ts";

const PROGRESS = ["◈ VITRIOL  ▮▮▮▯▯ slots 1/2   12.4 tok/s", "◈ ~/proj"];

afterEach(() => resetBand());

describe("the decode row owns exactly one band tier", () => {
  it("down publishes a state row and no progress row", () => {
    const rows = decodeBandRows(false, PROGRESS);
    expect(rows.state).toEqual([DECODE_DOWN_LINE]);
    // Even when a progress payload is passed: stale numbers must not sit under
    // a down notice, they are the same subject.
    expect(rows.progress).toBeNull();
  });

  it("up publishes a progress row and no state row", () => {
    const rows = decodeBandRows(true, PROGRESS);
    expect(rows.progress).toEqual(PROGRESS);
    expect(rows.state).toBeNull();
  });

  it("up with nothing to say publishes nothing at all (negative control)", () => {
    // Not "an empty row": a null, so the strip shrinks honestly instead of
    // carrying a blank line that the layout then settles around.
    const rows = decodeBandRows(true, []);
    expect(rows.state).toBeNull();
    expect(rows.progress).toBeNull();
  });

  it("i2 recovery leaves no trace of the outage", () => {
    // The cycle test that would have caught the original bug: up -> down -> up
    // must be indistinguishable from steady up.
    const steady = decodeBandRows(true, PROGRESS);
    decodeBandRows(false, []);
    const recovered = decodeBandRows(true, PROGRESS);
    expect(recovered).toEqual(steady);
    expect(JSON.stringify(recovered)).toBe(JSON.stringify(steady));
  });
});

describe("the assembled strip holds one decode row across a flap", () => {
  it("a down state row replaces the progress row instead of joining it", () => {
    // Simulates the render loop: each frame publishes the payload for the
    // current snapshot, exactly as index.ts does.
    // Clear-then-publish: rows are keyed by producer id, so publishing a state
    // row must clear the progress row first (and vice versa).
    const publish = (up: boolean) => {
      const rows = decodeBandRows(up, PROGRESS);
      setBandRow("vitriol-decode", "progress", null);
      setBandRow("vitriol-decode", "state", null);
      if (rows.state) setBandRow("vitriol-decode", "state", rows.state);
      if (rows.progress) setBandRow("vitriol-decode", "progress", rows.progress);
    };

    publish(true);
    expect(bandLines().filter((l) => l.includes("VITRIOL"))).toHaveLength(1);

    publish(false);
    const down = bandLines();
    expect(down).toContain(DECODE_DOWN_LINE);
    // The old bug: the progress line stayed, so both rows competed.
    expect(down.some((l) => l.includes("tok/s"))).toBe(false);

    publish(true);
    const back = bandLines();
    expect(back).toEqual(PROGRESS);
    expect(back).not.toContain(DECODE_DOWN_LINE);
  });
});
