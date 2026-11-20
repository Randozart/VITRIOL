// resolve-config — the ONE place Officina configuration precedence lives.
//
// 2026-11-20 (seam-resistance research, M2): before this module, 24 files
// read process.env directly with per-file precedence folklore. The caduceus
// router bypass (049ab57) was exactly this class: ~/.vitriol/officina/env
// pinned OFFICINA_CADUCEUS_BASE_URL and base-url.ts's P1 rule honoured it,
// silently bypassing the running router. A precedence rule nobody had
// written down defeated a service nobody remembered was there.
//
// The oven: layers call resolveConfig() and read a FROZEN snapshot. They
// never see raw process.env. Precedence (shell export beats env file beats
// detection beats default) is stated HERE, once, and every key records which
// rule won — so `why(key)` answers "where did this value come from?", the
// question every incident investigation starts with.
//
// Scope: only keys where precedence has actually bitten or plausibly could.
// The long tail of single-reader knobs keeps reading env directly (M3's
// tripwire warns on new ones so the oven grows deliberately, not all at
// once).
//
// Env-file (??= semantics, read once): ~/.vitriol/officina/env or
// OFFICINA_ENV_FILE. jiti-safe: resolved snapshot cached on globalThis.

import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

type Source = "shell" | "env-file" | "default";

export interface ResolvedValue<T = string> {
  value: T;
  source: Source;
}

export interface ResolvedConfig {
  /** Provider base URL for the Caduceus lane (the bypass key). */
  caduceusBaseUrl: ResolvedValue;
  /** Whether the reroute router unit is installed (detection rule). */
  caduceusRouterInstalled: ResolvedValue<boolean>;
  /** Prefs/state surfaces shared by proxy + TUI. */
  caduceusPrefsFile: ResolvedValue;
  caduceusVendorApi: ResolvedValue;
  /** Frozen view: never re-resolved mid-session. */
  readonly frozen: true;
}

const GATEWAY_BASE_URL = "http://127.0.0.1:20128/v1";
const ROUTER_BASE_URL = "http://127.0.0.1:20129/v1";

export const ENV_FILE_PATH =
  process.env.OFFICINA_ENV_FILE ||
  join(homedir(), ".vitriol", "officina", "env");

/** Parse `KEY=value` lines (comments/blank lines skipped). ??= semantics. */
export function parseEnvFile(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of String(text ?? "").split("\n")) {
    const t = line.trim();
    if (!t || t.startsWith("#")) continue;
    const eq = t.indexOf("=");
    if (eq <= 0) continue;
    out[t.slice(0, eq).trim()] = t.slice(eq + 1).trim();
  }
  return out;
}

/** Is the reroute router unit installed? (mirror of caduceus/base-url.ts) */
function routerUnitInstalled(): boolean {
  if (
    /^(0|false|off|no)$/i.test(
      String(process.env.OFFICINA_CADUCEUS_ROUTER ?? ""),
    )
  ) {
    return false;
  }
  try {
    // Same path base-url.ts derives (HOME + systemd user dir). Tests inject
    // a temp HOME via OFFICINA_TEST_HOME to simulate "unit not installed".
    const home = process.env.OFFICINA_TEST_HOME || homedir();
    return existsSync(
      join(
        home,
        ".config",
        "systemd",
        "user",
        "vitriol-caduceus-router.service",
      ),
    );
  } catch {
    return false;
  }
}

interface Cache {
  config?: ResolvedConfig;
  why?: Map<string, Source>;
}
const CACHE_KEY = "__officinaResolvedConfig";

/** Resolve once per process; frozen. envFile injectable for tests. */
export function resolveConfig(envFileText?: string): ResolvedConfig {
  const g = globalThis as unknown as { [CACHE_KEY]?: Cache };
  if (!envFileText && g[CACHE_KEY]?.config) return g[CACHE_KEY].config!;

  const envFile = parseEnvFile(envFileText ?? readEnvFile());
  const why = new Map<string, Source>();

  /** Precedence: shell (process.env) > env file > fallback default. */
  const pick = (key: string, fallback: string): ResolvedValue => {
    const shell = process.env[key];
    if (shell !== undefined && shell !== "") {
      why.set(key, "shell");
      return { value: shell, source: "shell" };
    }
    const file = envFile[key];
    if (file !== undefined && file !== "") {
      why.set(key, "env-file");
      return { value: file, source: "env-file" };
    }
    why.set(key, "default");
    return { value: fallback, source: "default" };
  };

  const routerInstalled = routerUnitInstalled();

  // THE BYPASS KEY (049ab57): explicit BASE_URL wins — but the env FILE's
  // copy is DEMOTED below router detection. Rationale: the env file is
  // persistent state nobody revisits; a year-old pin must not silently
  // defeat a service installed later. Shell exports remain absolute wins
  // (scripted setups), stated in the env file itself since 049ab57.
  const shellBase = process.env.OFFICINA_CADUCEUS_BASE_URL;
  const fileBase = envFile["OFFICINA_CADUCEUS_BASE_URL"];
  let caduceusBaseUrl: ResolvedValue;
  let caduceusRouterInstalled: ResolvedValue<boolean>;
  if (shellBase !== undefined && shellBase !== "") {
    caduceusBaseUrl = { value: shellBase.replace(/\/+$/, ""), source: "shell" };
    why.set("OFFICINA_CADUCEUS_BASE_URL", "shell");
    caduceusRouterInstalled = { value: routerInstalled, source: "default" };
  } else if (routerInstalled) {
    caduceusBaseUrl = { value: ROUTER_BASE_URL, source: "default" };
    caduceusRouterInstalled = { value: true, source: "default" };
    why.set("OFFICINA_CADUCEUS_BASE_URL", "default");
    why.set("OFFICINA_CADUCEUS_ROUTER_INSTALLED", "default");
  } else {
    if (fileBase !== undefined && fileBase !== "") {
      caduceusBaseUrl = {
        value: fileBase.replace(/\/+$/, ""),
        source: "env-file",
      };
      why.set("OFFICINA_CADUCEUS_BASE_URL", "env-file");
    } else {
      caduceusBaseUrl = { value: GATEWAY_BASE_URL, source: "default" };
      why.set("OFFICINA_CADUCEUS_BASE_URL", "default");
    }
    caduceusRouterInstalled = { value: false, source: "default" };
  }

  const config: ResolvedConfig = {
    caduceusBaseUrl,
    caduceusRouterInstalled,
    caduceusPrefsFile: pick(
      "OFFICINA_CADUCEUS_PREFS_FILE",
      join(homedir(), ".vitriol", "officina", "caduceus-prefs.json"),
    ),
    caduceusVendorApi: pick(
      "OFFICINA_CADUCEUS_VENDOR_API",
      join(
        homedir(),
        ".vitriol",
        "vendor",
        "omniroute",
        "node_modules",
        "omniroute",
        "bin",
        "cli",
        "api.mjs",
      ),
    ),
    frozen: true,
  };

  if (!envFileText) {
    g[CACHE_KEY] = { config, why };
  }
  return config;
}

/** Which rule won for `key`? For `vitriol doctor` and debugging. */
export function why(key: string): Source | undefined {
  const g = globalThis as unknown as { [CACHE_KEY]?: Cache };
  resolveConfig();
  return g[CACHE_KEY]?.why?.get(key);
}

function readEnvFile(): string {
  try {
    return readFileSync(ENV_FILE_PATH, "utf-8");
  } catch {
    return "";
  }
}

/** Test hook: forget the cached snapshot. */
export function resetConfigCache(): void {
  const g = globalThis as unknown as { [CACHE_KEY]?: Cache };
  delete g[CACHE_KEY];
}
