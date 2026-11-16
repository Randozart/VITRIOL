// Where the Caduceus provider points (2026-11-16).
//
// The routing proxy (`vitriol caduceus router --install`) reroutes a
// quota-spent or dead lane to the panel's fallback, but only if the *provider*
// addresses it. Deciding that here keeps it out of the launch environment: the
// unit file written by `--install` (and removed by `--remove`) IS the switch.
//
// Invariants:
//   P1 — an explicit OFFICINA_CADUCEUS_BASE_URL always wins (scripted setups).
//   P2 — no unit file ⇒ the gateway origin, byte-identical to before.
//   P3 — OFFICINA_CADUCEUS_ROUTER=0 is a kill switch even when installed.

import { existsSync } from "node:fs";
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

/** Is the reroute proxy installed? */
export function routerInstalled(env: NodeJS.ProcessEnv = process.env): boolean {
  if (
    /^(0|false|off|no)$/i.test(
      String(env.OFFICINA_CADUCEUS_ROUTER ?? "").trim(),
    )
  ) {
    return false;
  }
  try {
    return existsSync(routerUnitPath(env));
  } catch {
    return false;
  }
}

/** Provider base URL: explicit env, else the proxy when installed, else the gateway. */
export function resolveBaseUrl(env: NodeJS.ProcessEnv = process.env): string {
  const explicit = String(env.OFFICINA_CADUCEUS_BASE_URL ?? "").trim();
  if (explicit) return explicit.replace(/\/+$/, "");
  if (routerInstalled(env)) return ROUTER_BASE_URL;
  return GATEWAY_BASE_URL;
}
