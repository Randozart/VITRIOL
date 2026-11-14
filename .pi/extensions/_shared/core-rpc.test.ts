import { afterEach, describe, expect, it } from "vitest";
import { coreBinaryPath, coreCall, coreEnabled, resetCoreClient } from "./core-rpc.ts";

afterEach(() => {
  delete process.env.OFFICINA_ROUTER_CORE;
  delete process.env.OFFICINA_CORE_BIN;
  resetCoreClient();
});

describe("core sidecar client", () => {
  it("is off unless OFFICINA_ROUTER_CORE=rust", async () => {
    expect(coreEnabled()).toBe(false);
    expect(await coreCall("ping", {})).toBeUndefined();
    process.env.OFFICINA_ROUTER_CORE = "rust";
    expect(coreEnabled()).toBe(true);
  });

  it("fails open when the binary is missing", async () => {
    process.env.OFFICINA_ROUTER_CORE = "rust";
    process.env.OFFICINA_CORE_BIN = "/nonexistent/vitriol-core-rpc";
    expect(await coreCall("ping", {})).toBeUndefined();
  });

  it.skipIf(!coreBinaryPath())("answers ping from the built sidecar", async () => {
    process.env.OFFICINA_ROUTER_CORE = "rust";
    const result = (await coreCall("ping", {}, 2_000)) as { ok?: boolean; version?: string };
    expect(result?.ok).toBe(true);
    expect(typeof result?.version).toBe("string");
  });

  it.skipIf(!coreBinaryPath())("batches admission decisions", async () => {
    process.env.OFFICINA_ROUTER_CORE = "rust";
    const result = (await coreCall(
      "admit",
      {
        promptTokens: 170_000,
        reserveTokens: 16_384,
        lanes: [
          { id: "big", declared: 400_000, invalid: false, cooling: false, ownerPaused: false },
          { id: "paused", declared: 400_000, invalid: false, cooling: false, ownerPaused: true },
        ],
      },
      2_000,
    )) as { decisions?: Record<string, string> };
    expect(result?.decisions?.["big"]).toBe("admit");
    expect(result?.decisions?.["paused"]).toBe("owner-paused");
  });
});
