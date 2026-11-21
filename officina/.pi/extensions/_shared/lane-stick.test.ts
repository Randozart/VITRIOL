// Lane continuation-stick table (2026-11-21).
//
// The stick keeps a tool loop on the same physical model (warm weights). It
// was unconditional, which pinned whole sessions to local: Nimbus lost its
// cloud-first priority and Lapis kept a contended slot. These pin the table
// in the plan, with the parity cells (I1) alongside the two escapes.
import { describe, expect, it } from "vitest";
import { stickDecision, type Lane, type StickInput } from "./lane-stick.ts";

const input = (over: Partial<StickInput> = {}): StickInput => ({
  lane: "lapis",
  previousIsLocal: true,
  contended: false,
  cloudFits: true,
  ...over,
});

describe("stickDecision", () => {
  /** I1 — previous on the lane's preferred tier: always stick (parity). */
  it("lane_stick_parity_matrix", () => {
    for (const lane of ["lapis", "nimbus"] as Lane[]) {
      for (const contended of [true, false]) {
        for (const cloudFits of [true, false]) {
          const d = stickDecision(
            input({ lane, previousIsLocal: false, contended, cloudFits }),
          );
          expect(d, `${lane}: a CLOUD previous always sticks`).toEqual({
            stick: true,
            reason: "",
          });
        }
      }
    }
    // Lapis on an uncontended local slot is the normal tool loop: stick.
    for (const cloudFits of [true, false]) {
      expect(
        stickDecision(input({ lane: "lapis", contended: false, cloudFits })),
      ).toEqual({ stick: true, reason: "" });
    }
  });

  it("nimbus_leaves_local_when_cloud_available", () => {
    const d = stickDecision(
      input({ lane: "nimbus", previousIsLocal: true, cloudFits: true }),
    );
    expect(d.stick).toBe(false);
    expect(d.reason).toContain("cloud-first");
  });

  it("nimbus_stays_local_without_cloud", () => {
    const d = stickDecision(
      input({ lane: "nimbus", previousIsLocal: true, cloudFits: false }),
    );
    expect(d.stick).toBe(true);
  });

  it("lapis_leaves_contended_local", () => {
    const d = stickDecision(
      input({ lane: "lapis", previousIsLocal: true, contended: true }),
    );
    expect(d.stick).toBe(false);
    expect(d.reason).toContain("contended");
  });

  it("lapis_stays_on_contended_local_with_nowhere_to_go", () => {
    const d = stickDecision(
      input({
        lane: "lapis",
        previousIsLocal: true,
        contended: true,
        cloudFits: false,
      }),
    );
    expect(d.stick).toBe(true);
  });

  it("nimbus ignores contention when it is already cloud", () => {
    expect(
      stickDecision(
        input({ lane: "nimbus", previousIsLocal: false, contended: true }),
      ).stick,
    ).toBe(true);
  });
});
