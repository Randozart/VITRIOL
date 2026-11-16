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
  decisionConfigPath,
  readDecisionConfig,
  resolveDecisionConfig,
  writeDecisionConfig,
  type DecisionConfig,
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
