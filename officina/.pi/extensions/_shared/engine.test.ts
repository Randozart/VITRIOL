import { describe, expect, it } from "vitest";
import { classifyFetchError } from "./engine.ts";

describe("classifyFetchError — stalled vs down (2026-09-04)", () => {
  const abortErr = () => {
    const e = new Error("The operation was aborted");
    e.name = "AbortError";
    return e;
  };

  it("timeout abort after TCP connect = stalled (engine alive, endpoint queue-backed)", () => {
    expect(classifyFetchError(abortErr())).toBe("stalled");
  });

  it("connection refused = down", () => {
    const e = new TypeError("fetch failed");
    (e as unknown as { cause: { code: string } }).cause = { code: "ECONNREFUSED" };
    expect(classifyFetchError(e)).toBe("down");
  });

  it("connection reset = down", () => {
    const e = new TypeError("fetch failed");
    (e as unknown as { cause: { code: string } }).cause = { code: "ECONNRESET" };
    expect(classifyFetchError(e)).toBe("down");
  });

  it("unknown failures default to down (conservative)", () => {
    expect(classifyFetchError(new Error("something odd"))).toBe("down");
  });
});
