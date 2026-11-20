// Where the Caduceus provider points (2026-11-16).
//
// 2026-11-20 (seam-resistance M2): precedence now lives in the OVEN
// (_shared/resolve-config.ts) — this module is a thin adapter over the
// resolved snapshot so existing callers keep their API. The env-file
// demotion rule (a year-old BASE_URL pin must not bypass an installed
// router — the 049ab57 defect) is enforced in the oven, and its
// negative-control test is resolve-config.test.ts (I-oven-1).
//
// Invariants (enforced by the oven; documented here for callers):
//   P1 — a SHELL-exported OFFICINA_CADUCEUS_BASE_URL always wins.
//   P2 — no unit file ⇒ the gateway origin, byte-identical to before.
//   P3 — OFFICINA_CADUCEUS_ROUTER=0 is a kill switch even when installed.

import { resetConfigCache, resolveConfig } from "../_shared/resolve-config.ts";
import { homedir } from "node:os";
import { join } from "node:path";

/** The gateway (OmniRoute vendor) origin. */
export const GATEWAY_BASE_URL = "http://127.0.0.1:20128/v1";
/** The routing proxy's port (`vitriol caduceus router`). */
export const ROUTER_PORT = 20129;
export const ROUTER_BASE_URL = `http://127.0.0.1:${ROUTER_PORT}/v1`;

/** The router unit path, resolved against the *current* HOME (testable). */
export function routerUnitPath(env: NodeJS.ProcessEnv = process.env): string {
  const home = env.HOME || homedir();
  return join(
    home,
    ".config",
    "systemd",
    "user",
    "vitriol-caduceus-router.service",
  );
}

/**
 * Is the reroute proxy installed? (oven-resolved; env param kept for compat.)
 * The oven detects the unit via OFFICINA_TEST_HOME (its test seam); callers
 * that pass env.HOME get the same effect by bridging it here.
 */
export function routerInstalled(env: NodeJS.ProcessEnv = process.env): boolean {
  const saved = process.env.OFFICINA_TEST_HOME;
  const savedKill = process.env.OFFICINA_CADUCEUS_ROUTER;
  process.env.OFFICINA_TEST_HOME = String(env.HOME ?? saved ?? "");
  // Bridge the P3 kill switch (callers may pass it as a param).
  if (env.OFFICINA_CADUCEUS_ROUTER !== undefined) {
    process.env.OFFICINA_CADUCEUS_ROUTER = env.OFFICINA_CADUCEUS_ROUTER;
  }
  try {
    resetConfigCache(); // HOME differs per call — never serve a stale snapshot
    return resolveConfig().caduceusRouterInstalled.value;
  } finally {
    if (saved === undefined) delete process.env.OFFICINA_TEST_HOME;
    else process.env.OFFICINA_TEST_HOME = saved;
    if (savedKill === undefined) delete process.env.OFFICINA_CADUCEUS_ROUTER;
    else process.env.OFFICINA_CADUCEUS_ROUTER = savedKill;
  }
}

/** Provider base URL: oven-resolved (see module docstring for precedence). */
export function resolveBaseUrl(env: NodeJS.ProcessEnv = process.env): string {
  const saved = process.env.OFFICINA_TEST_HOME;
  const savedShell = process.env.OFFICINA_CADUCEUS_BASE_URL;
  const savedKill = process.env.OFFICINA_CADUCEUS_ROUTER;
  const shell = env.OFFICINA_CADUCEUS_BASE_URL;
  process.env.OFFICINA_TEST_HOME = String(env.HOME ?? saved ?? "");
  // Bridge BOTH seams the tests use: HOME (unit detection) and an explicit
  // BASE_URL / kill switch passed as params (shell tier in the oven's model).
  if (shell !== undefined) process.env.OFFICINA_CADUCEUS_BASE_URL = shell;
  if (env.OFFICINA_CADUCEUS_ROUTER !== undefined) {
    process.env.OFFICINA_CADUCEUS_ROUTER = env.OFFICINA_CADUCEUS_ROUTER;
  }
  try {
    resetConfigCache();
    return resolveConfig().caduceusBaseUrl.value;
  } finally {
    if (saved === undefined) delete process.env.OFFICINA_TEST_HOME;
    else process.env.OFFICINA_TEST_HOME = saved;
    if (savedShell === undefined) delete process.env.OFFICINA_CADUCEUS_BASE_URL;
    else process.env.OFFICINA_CADUCEUS_BASE_URL = savedShell;
    if (savedKill === undefined) delete process.env.OFFICINA_CADUCEUS_ROUTER;
    else process.env.OFFICINA_CADUCEUS_ROUTER = savedKill;
  }
}
