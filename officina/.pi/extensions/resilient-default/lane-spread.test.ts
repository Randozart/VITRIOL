import { describe, expect, it } from "vitest";
import { busyPeerLanes, laneKey, spreadPick } from "./lane-spread.ts";

const lanes = [
  { provider: "caduceus", id: "freeinference/deepseek-v4-flash" },
  { provider: "caduceus", id: "dahl/MiniMaxAI/MiniMax-M2.7" },
  { provider: "caduceus", id: "agentrouter/gpt-5.6-sol" },
];

describe("lane spread", () => {
  it("picks the first free lane by default", () => {
    expect(spreadPick(lanes, new Set())).toBe(lanes[0]);
  });

  it("skips a lane another live session is turning on", () => {
    const busy = new Set([laneKey(lanes[0])]);
    expect(spreadPick(lanes, busy)?.id).toBe(lanes[1].id);
  });

  it("falls back to the priority order when every lane is busy", () => {
    const busy = new Set(lanes.map(laneKey));
    expect(spreadPick(lanes, busy)).toBe(lanes[0]);
  });

  it("ignores local peers and empty lists", () => {
    expect(busyPeerLanes([{ tier: "local", model: "llamacpp/Lapis Occultus" }]).size).toBe(0);
    expect(
      busyPeerLanes([{ tier: "cloud", model: "caduceus/dahl/x" }]).has("caduceus/dahl/x"),
    ).toBe(true);
    expect(spreadPick([], new Set())).toBeUndefined();
  });
});
