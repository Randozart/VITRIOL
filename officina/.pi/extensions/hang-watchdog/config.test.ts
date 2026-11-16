// Decision-lane config (2026-11-16): /decision writes a 0600 file the judge
// consults, env vars override for throwaway runs, and the key never leaks.
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  ascensusGeminiKey,
  decisionConfigPath,
  readDecisionConfig,
  resolveDecisionConfig,
  type DecisionConfig,
  writeDecisionConfig,
} from "./config.ts";

let dir: string;
beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "wd-decision-config-"));
  vi.stubEnv("HOME", dir);
});
afterEach(() => {
  vi.unstubAllEnvs();
  rmSync(dir, { recursive: true, force: true });
});

const cfg = (over: Partial<DecisionConfig> = {}): DecisionConfig => ({
  version: 1,
  url: "http://127.0.0.1:8000/v1/systemone",
  key: "",
  floor: 0.9,
  ...over,
});

describe("decision config", () => {
  it("round-trips url/key/floor through the 0600 file", () => {
    expect(writeDecisionConfig(cfg({ key: "s3cr1t" }))).toBe(true);
    const got = readDecisionConfig();
    expect(got?.url).toBe("http://127.0.0.1:8000/v1/systemone");
    expect(got?.key).toBe("s3cr1t");
    expect(got?.floor).toBe(0.9);
    const path = decisionConfigPath();
    expect(
      path.endsWith(join(".vitriol", "officina", "watchdog-decision.json")),
    ).toBe(true);
    expect(statSync(path).mode & 0o777).toBe(0o600);
  });

  it("tolerates missing and corrupt files", () => {
    expect(readDecisionConfig()).toBeNull();
    mkdirSync(join(dir, ".vitriol", "officina"), { recursive: true });
    writeFileSync(decisionConfigPath(), "{corrupt");
    expect(readDecisionConfig()).toBeNull();
  });

  it("env vars override the file; unset env falls back to the file", () => {
    writeDecisionConfig(cfg({ key: "file-key" }));
    const resolved = resolveDecisionConfig({});
    expect(resolved.endpoint).toBe("http://127.0.0.1:8000/v1/systemone");
    expect(resolved.key).toBe("file-key");
    expect(resolved.floor).toBe(0.9);

    const overridden = resolveDecisionConfig({
      OFFICINA_WATCHDOG_DECISION_URL: "http://192.0.2.1:9000/v1/systemone",
      OFFICINA_WATCHDOG_DECISION_KEY: "env-key",
      OFFICINA_WATCHDOG_DECISION_MIN_SCORE: "0.5",
    });
    expect(overridden.endpoint).toBe("http://192.0.2.1:9000/v1/systemone");
    expect(overridden.key).toBe("env-key");
    expect(overridden.floor).toBe(0.5);
  });

  it("clamps an invalid floor and returns empty url when unset", () => {
    writeDecisionConfig(cfg({ url: "", floor: 5 }));
    const got = readDecisionConfig();
    expect(got?.url).toBe("");
    expect(got?.floor).toBe(0.9);
    expect(resolveDecisionConfig({}).endpoint).toBe("");
  });
});

describe("decision lane modes (2026-11-16)", () => {
  it("derives the mode: endpoint ⇒ http, nothing ⇒ heuristic", () => {
    writeDecisionConfig(cfg({ url: "http://127.0.0.1:9/x", floor: 0.8 }));
    expect(resolveDecisionConfig({}).mode).toBe("http");
    writeDecisionConfig(cfg({ url: "", floor: 0.8 }));
    expect(resolveDecisionConfig({}).mode).toBe("heuristic");
  });

  it("honours an explicit gemini mode and its knobs", () => {
    writeDecisionConfig({
      version: 1,
      mode: "gemini",
      url: "",
      key: "AIza-from-file",
      floor: 0.6,
      margin: 0.2,
      model: "gemini-decision-1",
      redact: false,
    });
    const got = resolveDecisionConfig({});
    expect(got.mode).toBe("gemini");
    expect(got.floor).toBe(0.6);
    expect(got.margin).toBe(0.2);
    expect(got.redact).toBe(false);
    expect(got.key).toBe("AIza-from-file");
  });

  it("lets env override every knob", () => {
    writeDecisionConfig(cfg({ url: "", floor: 0.9 }));
    const got = resolveDecisionConfig({
      OFFICINA_WATCHDOG_DECISION_MODE: "gemini",
      OFFICINA_WATCHDOG_DECISION_MIN_SCORE: "0.55",
      OFFICINA_WATCHDOG_DECISION_MARGIN: "0.25",
      OFFICINA_WATCHDOG_DECISION_MODEL: "gemini-decision-1",
      OFFICINA_WATCHDOG_DECISION_REDACT: "0",
      GEMINI_API_KEY: "AIza-env",
    } as NodeJS.ProcessEnv);
    expect(got).toMatchObject({
      mode: "gemini",
      floor: 0.55,
      margin: 0.25,
      redact: false,
      key: "AIza-env",
    });
  });

  it("defaults the margin to 0.15 and redaction to on", () => {
    writeDecisionConfig(cfg({ url: "http://127.0.0.1:9/x", floor: 0.8 }));
    const got = resolveDecisionConfig({} as NodeJS.ProcessEnv);
    expect(got.margin).toBe(0.15);
    expect(got.redact).toBe(true);
    expect(got.model).toBe("gemini-decision-1");
  });
});

describe("ascensusGeminiKey", () => {
  it("sniffs a bare AIza api_key (ascensusd's own rule)", () => {
    const text = "provider = zai-coding-plan\napi_key = sk-zai\n";
    expect(ascensusGeminiKey("/x", () => text)).toBe("");
    const withGemini = `provider = zai-coding-plan\napi_key = AIza${"A".repeat(35)}\n`;
    expect(ascensusGeminiKey("/x", () => withGemini)).toMatch(/^AIza/);
  });

  it("prefers a [gemini] section when present", () => {
    const text = [
      "provider = zai-coding-plan",
      "api_key = AIza" + "B".repeat(35),
      "[gemini]",
      "api_key = AIza" + "C".repeat(35),
      "model = gemini-decision-1",
    ].join("\n");
    const got = ascensusGeminiKey("/x", () => text);
    expect(got).toBe("AIza" + "C".repeat(35));
  });

  it("tolerates a missing file", () => {
    expect(
      ascensusGeminiKey("/nope", () => {
        throw new Error("ENOENT");
      }),
    ).toBe("");
  });
});
