// Codex-infra stage 2: desktop notification gating (I3).
import { describe, expect, it } from "vitest";
import { desktopNotifyEligible } from "./index.ts";

const base = { WAYLAND_DISPLAY: "wayland-0" } as NodeJS.ProcessEnv;

describe("desktopNotifyEligible", () => {
  it("fires past the default 20 s threshold on a graphical session", () => {
    expect(desktopNotifyEligible(base, 21)).toBe(true);
    expect(desktopNotifyEligible(base, 19)).toBe(false);
  });

  it("honours the kill switch and missing display", () => {
    expect(desktopNotifyEligible({ ...base, OFFICINA_NO_DESKTOP_NOTIFY: "1" }, 99)).toBe(false);
    expect(desktopNotifyEligible({} as NodeJS.ProcessEnv, 99)).toBe(false);
  });

  it("honours a custom threshold", () => {
    expect(
      desktopNotifyEligible({ ...base, OFFICINA_DESKTOP_NOTIFY_MIN_S: "60" }, 45),
    ).toBe(false);
    expect(
      desktopNotifyEligible({ ...base, OFFICINA_DESKTOP_NOTIFY_MIN_S: "60" }, 61),
    ).toBe(true);
  });
});
