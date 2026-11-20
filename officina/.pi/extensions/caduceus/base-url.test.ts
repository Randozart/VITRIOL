// Provider base-URL policy tests (2026-11-16): the reroute proxy only helps a
// session if the provider addresses it.
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  GATEWAY_BASE_URL,
  ROUTER_BASE_URL,
  resolveBaseUrl,
  routerInstalled,
  routerUnitPath,
} from "./base-url.ts";

let home = "";

function makeHome(withUnit: boolean): string {
  home = mkdtempSync(join(tmpdir(), "officina-baseurl-"));
  if (withUnit) {
    const dir = join(home, ".config", "systemd", "user");
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, "vitriol-caduceus-router.service"), "[Unit]\n");
  }
  return home;
}

afterEach(() => {
  if (home) rmSync(home, { recursive: true, force: true });
  home = "";
});

// Hermeticity: a developer shell (or launch env) may export
// OFFICINA_CADUCEUS_BASE_URL — the oven's shell tier would win over every
// test expectation. Clear it before each case; the oven tests own the
// shell-tier coverage.
beforeEach(() => {
  delete process.env.OFFICINA_CADUCEUS_BASE_URL;
});

describe("resolveBaseUrl", () => {
  it("P2: no proxy installed ⇒ the gateway, unchanged", () => {
    const env = { HOME: makeHome(false) } as NodeJS.ProcessEnv;
    expect(routerInstalled(env)).toBe(false);
    expect(resolveBaseUrl(env)).toBe(GATEWAY_BASE_URL);
  });

  it("routes through the proxy when its unit is installed", () => {
    const env = { HOME: makeHome(true) } as NodeJS.ProcessEnv;
    expect(routerInstalled(env)).toBe(true);
    expect(resolveBaseUrl(env)).toBe(ROUTER_BASE_URL);
  });

  it("P1: an explicit base URL always wins", () => {
    const env = {
      HOME: makeHome(true),
      OFFICINA_CADUCEUS_BASE_URL: "http://127.0.0.1:19999/v1/",
    } as NodeJS.ProcessEnv;
    expect(resolveBaseUrl(env)).toBe("http://127.0.0.1:19999/v1");
  });

  it("P3: OFFICINA_CADUCEUS_ROUTER=0 is a kill switch", () => {
    const env = {
      HOME: makeHome(true),
      OFFICINA_CADUCEUS_ROUTER: "0",
    } as NodeJS.ProcessEnv;
    expect(routerInstalled(env)).toBe(false);
    expect(resolveBaseUrl(env)).toBe(GATEWAY_BASE_URL);
  });

  it("resolves the unit path against the given HOME", () => {
    const env = { HOME: "/tmp/officina-home" } as NodeJS.ProcessEnv;
    expect(routerUnitPath(env)).toBe(
      "/tmp/officina-home/.config/systemd/user/vitriol-caduceus-router.service",
    );
  });
});
