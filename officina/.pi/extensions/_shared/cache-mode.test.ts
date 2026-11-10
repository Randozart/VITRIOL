import { describe, expect, it } from "vitest";
import {
  cacheClearPercent,
  cacheModeSetting,
  getCacheModeState,
  isCloudCacheMode,
  providerIsLocal,
  resolveCacheMode,
  setCacheModeFromProvider,
  shouldDeferToolClear,
} from "./cache-mode.ts";

describe("cacheModeSetting", () => {
  it("defaults to auto", () => {
    expect(cacheModeSetting({})).toBe("auto");
  });
  it("accepts known values only", () => {
    expect(cacheModeSetting({ OFFICINA_CACHE_MODE: "cloud" })).toBe("cloud");
    expect(cacheModeSetting({ OFFICINA_CACHE_MODE: "bogus" })).toBe("auto");
  });
});

describe("resolveCacheMode", () => {
  it("is local for the local engine in auto mode", () => {
    expect(resolveCacheMode("auto", "llamacpp").cloud).toBe(false);
    expect(resolveCacheMode("auto", undefined).cloud).toBe(false);
  });
  it("is cloud for non-local providers in auto mode", () => {
    expect(resolveCacheMode("auto", "caduceus").cloud).toBe(true);
    expect(resolveCacheMode("auto", "opencode-go").cloud).toBe(true);
  });
  it("honours forced local/cloud/off", () => {
    expect(resolveCacheMode("local", "caduceus").cloud).toBe(false);
    expect(resolveCacheMode("cloud", "llamacpp").cloud).toBe(true);
    expect(resolveCacheMode("off", "caduceus").cloud).toBe(false);
  });
  it("treats empty provider as local", () => {
    expect(providerIsLocal("")).toBe(true);
    expect(providerIsLocal("llamacpp")).toBe(true);
    expect(providerIsLocal("zai")).toBe(false);
  });
});

describe("shared state", () => {
  it("updates on provider selection", () => {
    setCacheModeFromProvider("caduceus", { OFFICINA_CACHE_MODE: "auto" });
    expect(isCloudCacheMode()).toBe(true);
    expect(getCacheModeState().provider).toBe("caduceus");
    setCacheModeFromProvider("llamacpp", { OFFICINA_CACHE_MODE: "auto" });
    expect(isCloudCacheMode()).toBe(false);
  });
});

describe("clear policy", () => {
  it("parses and clamps the threshold", () => {
    expect(cacheClearPercent({})).toBe(92);
    expect(cacheClearPercent({ OFFICINA_CACHE_CLEAR_PCT: "75" })).toBe(75);
    expect(cacheClearPercent({ OFFICINA_CACHE_CLEAR_PCT: "1" })).toBe(50);
    expect(cacheClearPercent({ OFFICINA_CACHE_CLEAR_PCT: "999" })).toBe(100);
  });

  it("defers clearing in cloud mode below the threshold", () => {
    setCacheModeFromProvider("caduceus", { OFFICINA_CACHE_MODE: "auto" });
    expect(shouldDeferToolClear(40, {})).toBe(true);
    expect(shouldDeferToolClear(null, {})).toBe(true);
    expect(shouldDeferToolClear(95, {})).toBe(false);
    expect(shouldDeferToolClear(40, { OFFICINA_CACHE_FORCE_CLEAR: "1" })).toBe(false);
  });

  it("never defers for local mode", () => {
    setCacheModeFromProvider("llamacpp", { OFFICINA_CACHE_MODE: "auto" });
    expect(shouldDeferToolClear(10, {})).toBe(false);
  });
});
