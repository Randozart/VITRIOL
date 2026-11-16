// Telemetry scope (2026-11-16): a session shows the facts of its OWN provider.
import { describe, expect, it } from "vitest";
import { engineRowsVisible, telemetryScope } from "./telemetry-scope.ts";

describe("telemetryScope", () => {
  it("maps the real provider ids to their own facts", () => {
    expect(telemetryScope("llamacpp", "Lapis Occultus")).toBe("engine");
    expect(telemetryScope("officina", "lapis")).toBe("engine");
    expect(telemetryScope("officina", "nimbus")).toBe("lanes");
    expect(telemetryScope("caduceus", "dahl/MiniMaxAI/MiniMax-M2.7")).toBe(
      "cloud",
    );
    expect(telemetryScope("opencode-go", "deepseek-v4.1-flash")).toBe("cloud");
    expect(telemetryScope("cerebras", "llama-4")).toBe("cloud");
  });

  it("is case/space tolerant and defaults to silence without a provider", () => {
    expect(telemetryScope(" LLaMacpp ", "x")).toBe("engine");
    expect(telemetryScope("Officina", "NIMBUS")).toBe("lanes");
    expect(telemetryScope(undefined, "whatever")).toBe("none");
    expect(telemetryScope("", "")).toBe("none");
    // An unknown virtual model must not inherit the engine rows.
    expect(telemetryScope("officina", "mystery")).toBe("cloud");
  });

  it("keeps the engine rows exclusive to the engine scope (P2)", () => {
    expect(engineRowsVisible("engine")).toBe(true);
    expect(engineRowsVisible("lanes")).toBe(false);
    expect(engineRowsVisible("cloud")).toBe(false);
    expect(engineRowsVisible("none")).toBe(false);
  });
});
