// Config Resolution Oven tests (2026-11-20, seam-resistance M2).
//
// The bypass invariant (I-oven-1): an env-FILE OFFICINA_CADUCEUS_BASE_URL can
// no longer defeat an installed router — the exact defect that silently
// bypassed the reroute proxy (049ab57). Shell exports still win (scripted
// setups), also pinned.

import { describe, expect, it } from "vitest";
import {
  parseEnvFile,
  resolveConfig,
  resetConfigCache,
  why,
} from "./resolve-config.ts";

// The router unit exists on this host — tests that need "no unit" inject a
// temp HOME where the unit path does not exist (OFFICINA_TEST_HOME).
const UNIT_ENV = { OFFICINA_TEST_HOME: "/tmp/definitely-no-unit-home" };

function withEnv(env: Record<string, string>, envFileText = "") {
  const saved: Record<string, string | undefined> = {};
  for (const [k, v] of Object.entries(env)) {
    saved[k] = process.env[k];
    process.env[k] = v;
  }
  resetConfigCache();
  const cfg = resolveConfig(envFileText);
  for (const [k, v] of Object.entries(saved)) {
    if (v === undefined) delete process.env[k];
    else process.env[k] = v;
  }
  resetConfigCache();
  return cfg;
}

describe("resolve-config oven", () => {
  it("parses env files: KEY=value, comments, blanks skipped", () => {
    const parsed = parseEnvFile(
      "# comment\n\nA=1\n  B  =  spaced  \nbroken-line\n",
    );
    expect(parsed).toEqual({ A: "1", B: "spaced" });
  });

  it("I-oven-1 NEGATIVE CONTROL: env-file BASE_URL cannot defeat an installed router", () => {
    // Router unit installed (default detection on this host), env file pins
    // the gateway — the exact 049ab57 defect shape. The oven must return the
    // ROUTER and record the env-file value as demoted.
    const savedUnit = process.env.OFFICINA_CADUCEUS_ROUTER_UNIT;
    const savedBase = process.env.OFFICINA_CADUCEUS_BASE_URL;
    delete process.env.OFFICINA_CADUCEUS_ROUTER_UNIT;
    delete process.env.OFFICINA_CADUCEUS_BASE_URL;
    resetConfigCache();
    const cfg = resolveConfig(
      "OFFICINA_CADUCEUS_BASE_URL=http://127.0.0.1:20128/v1\n",
    );
    expect(cfg.caduceusBaseUrl.value).toBe("http://127.0.0.1:20129/v1");
    expect(cfg.caduceusBaseUrl.source).toBe("default");
    expect(cfg.caduceusRouterInstalled.value).toBe(true);
    if (savedUnit !== undefined)
      process.env.OFFICINA_CADUCEUS_ROUTER_UNIT = savedUnit;
    resetConfigCache();
  });

  it("I-oven-1 companion: shell export still wins absolutely", () => {
    const cfg = withEnv(
      { OFFICINA_CADUCEUS_BASE_URL: "http://example.test:9/v1/" },
      "OFFICINA_CADUCEUS_BASE_URL=http://127.0.0.1:20128/v1\n",
    );
    expect(cfg.caduceusBaseUrl.value).toBe("http://example.test:9/v1");
    expect(cfg.caduceusBaseUrl.source).toBe("shell");
  });

  it("no router unit + env-file URL → env file serves (detection demoted)", () => {
    const cfg = withEnv(
      UNIT_ENV,
      "OFFICINA_CADUCEUS_BASE_URL=http://127.0.0.1:20128/v1\n",
    );
    expect(cfg.caduceusBaseUrl.value).toBe("http://127.0.0.1:20128/v1");
    expect(cfg.caduceusBaseUrl.source).toBe("env-file");
    expect(cfg.caduceusRouterInstalled.value).toBe(false);
  });

  it("nothing anywhere → gateway default", () => {
    const cfg = withEnv(UNIT_ENV, "");
    expect(cfg.caduceusBaseUrl.value).toBe("http://127.0.0.1:20128/v1");
    expect(cfg.caduceusBaseUrl.source).toBe("default");
  });

  it("why() records the winning rule for doctor output", () => {
    // withEnv resolves while the shell override is live, so the why-map from
    // THAT resolution is the honest record (why() re-resolves live env).
    const saved = process.env.OFFICINA_CADUCEUS_BASE_URL;
    process.env.OFFICINA_CADUCEUS_BASE_URL = "http://shell.test";
    resetConfigCache();
    resolveConfig(""); // resolves WITH the override; why-map captured now
    expect(why("OFFICINA_CADUCEUS_BASE_URL")).toBe("shell");
    if (saved === undefined) delete process.env.OFFICINA_CADUCEUS_BASE_URL;
    else process.env.OFFICINA_CADUCEUS_BASE_URL = saved;
    resetConfigCache();
    // Restore check: after withEnv restored the env, a fresh resolution
    // records the default rule.
    resetConfigCache();
    resolveConfig("");
    expect(why("OFFICINA_CADUCEUS_BASE_URL")).toBe("default");
  });

  it("snapshot is frozen across calls (jiti-safe cache)", () => {
    const a = resolveConfig("");
    const b = resolveConfig("");
    expect(a).toBe(b);
  });
});
