// findEngineModel call-discipline gate (2026-11-19 incident).
//
// The 2026-11-19 "Cannot read properties of undefined (reading 'runtime')"
// outage was a DETACHED method call: findEngineModel captured
// `const find = registry.find` and invoked it bare, so `this` was undefined
// inside pi's ModelRegistry.find (`this.runtime.getModel(...)`). Every
// officina/lapis route died at the local-lane resolution step.
//
// These tests pin the behaviour: find must be invoked ATTACHED to the
// registry object. The fake registry's find asserts its receiver.

import { describe, expect, it } from "vitest";
import {
  ENGINE_PROVIDER_ID,
  LEGACY_ENGINE_PROVIDER_IDS,
  findEngineModel,
} from "./engine-provider.ts";

/** A registry whose find THROWS when detached (this === undefined). */
function strictRegistry(
  models: {
    provider: string;
    id: string;
  }[],
): { find: (provider: string, modelId: string) => unknown } {
  return {
    find(provider: string, modelId: string) {
      if (this === undefined || this === null) {
        throw new TypeError(
          "Cannot read properties of undefined (reading 'runtime')",
        );
      }
      called.push(`${provider}/${modelId}`);
      return (
        models.find((m) => m.provider === provider && m.id === modelId) ??
        undefined
      );
    },
  };
}
let called: string[] = [];

describe("findEngineModel call discipline", () => {
  it("calls find attached — this is the registry, not undefined", () => {
    called = [];
    const registry = strictRegistry([
      { provider: "llamacpp", id: "Lapis Occultus" },
    ]);
    const hit = findEngineModel(registry, "Lapis Occultus");
    expect(hit).toEqual({ provider: "llamacpp", id: "Lapis Occultus" });
    // The legacy alias resolved — proves lookups actually ran.
    expect(called).toContain("llamacpp/Lapis Occultus");
  });

  it("probes the canonical id before the legacy aliases", () => {
    called = [];
    const registry = strictRegistry([
      { provider: ENGINE_PROVIDER_ID, id: "Lapis Occultus" },
    ]);
    findEngineModel(registry, "Lapis Occultus");
    expect(called[0]).toBe(`${ENGINE_PROVIDER_ID}/Lapis Occultus`);
    expect(called).not.toContain(
      `${LEGACY_ENGINE_PROVIDER_IDS[0]}/Lapis Occultus`,
    );
  });

  it("returns fallback when no spelling resolves", () => {
    called = [];
    const registry = strictRegistry([]);
    const hit = findEngineModel(registry, "missing", "fb");
    expect(hit).toBe("fb");
    // Canonical + every legacy spelling probed.
    expect(called).toHaveLength(1 + LEGACY_ENGINE_PROVIDER_IDS.length);
  });

  it("returns fallback when the registry has no find at all", () => {
    const hit = findEngineModel<unknown>({} as never, "x", "fb");
    expect(hit).toBe("fb");
  });
});
