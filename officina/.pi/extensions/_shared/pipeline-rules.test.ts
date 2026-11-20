// Pipeline confirm-gate tests (2026-11-19). Pin I8: PIPELINE never
// implements without the owner's explicit confirmation. Fail-closed.

import { describe, expect, it } from "vitest";
import {
  PLAN_CONFIRMED_MARKER,
  gateDecision,
  gateOpen,
  parseMarker,
} from "./pipeline-rules.ts";

describe("PIPELINE confirm gate", () => {
  it("opens only on the exact standalone marker line", () => {
    expect(gateOpen(PLAN_CONFIRMED_MARKER)).toBe(true);
    expect(gateOpen(`  ${PLAN_CONFIRMED_MARKER}  `)).toBe(true); // trimmed
    expect(gateOpen(`findings:\n- a\n${PLAN_CONFIRMED_MARKER}`)).toBe(true);
  });

  it("stays closed on inline mentions (negative control)", () => {
    // A research note quoting the marker must NOT open the gate.
    expect(gateOpen(`say exactly: ${PLAN_CONFIRMED_MARKER} when ready`)).toBe(
      false,
    );
    expect(gateOpen(`${PLAN_CONFIRMED_MARKER} (not yet)`)).toBe(false);
    expect(gateOpen(`PIPELINE: PLAN CONFIRMED!!`)).toBe(false);
    expect(gateOpen("")).toBe(false);
    expect(gateOpen("research notes only")).toBe(false);
  });

  it("gate decision is fail-closed: await until the marker appears", () => {
    expect(gateDecision("research done, awaiting owner", 0).action).toBe(
      "await-confirmation",
    );
    const d = gateDecision(`plan:\n- x\n${PLAN_CONFIRMED_MARKER}`, 0);
    expect(d.action).toBe("proceed");
  });

  it("re-exports the grind completion grammar", () => {
    expect(parseMarker("GRIND: DONE").kind).toBe("done");
    expect(parseMarker("GRIND: BLOCKED: no creds").kind).toBe("blocked");
    expect(parseMarker("mid-work").kind).toBe("none");
  });
});
