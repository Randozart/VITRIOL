// verify — gate discovery (pure, unit-tested; no side effects beyond reads).
//
// Standards ride IN the tool: detectGates() builds the project's verification
// set from manifests (package.json / Cargo.toml / pyproject / go.mod /
// build-patch canaries) + a VITRIOL signature special-case for this monorepo.
// `.officina/verify.json` (optional) overrides wholesale via `include`.
//
// Tiers: `cheap` = typecheck/lint/fmt (session-start auto-baseline eligible
// when gate.auto), `full` = test suites (lazy — only on /verify or the tool).
// Caps keep monorepos with dozens of manifests from spawning storm runs.
// Kill switch handled by index.ts (OFFICINA_NO_VERIFY=1).

import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

export type Tier = "cheap" | "full";

export type ParserId = "tsc" | "cargo" | "rustfmt" | "ruff" | "vitest" | "pytest" | "go" | "generic";

export interface Gate {
  id: string; // "<relDir>::<tool>" — stable key for baselines/reports
  dir: string; // absolute run cwd
  rel: string; // relative dir (".", "officina", "officina/tui")
  label: string; // display: "tsc", "npm test", "cargo check"
  cmd: string[]; // argv
  tier: Tier;
  parser: ParserId;
  auto: boolean; // eligible for the session-start cheap baseline
}

export interface VerifyConfig {
  auto: boolean;
  disable: string[];
  include?: Gate[];
}

const EXCLUDE_DIRS = new Set([
  "engine", ".git", "node_modules", "target", "build", "dist",
  ".opencode", ".pi", ".officina", "vendor", "legacy",
]);

// Per-toolchain caps on GENERIC discovery (root + one level down). The
// VITRIOL signature and verify.json `include` bypass these.
const CAPS: Record<string, number> = { cargo: 3, node: 2, py: 1, go: 1 };

/** Read optional `.officina/verify.json`. Missing/invalid → defaults. */
export function loadConfig(cwd: string): VerifyConfig {
  const path = join(cwd, ".officina", "verify.json");
  try {
    const raw = JSON.parse(readFileSync(path, "utf8")) as Partial<VerifyConfig>;
    return {
      auto: raw.auto !== false,
      disable: Array.isArray(raw.disable) ? raw.disable.map(String) : [],
      include: Array.isArray(raw.include) && raw.include.length > 0 ? (raw.include as Gate[]) : undefined,
    };
  } catch {
    return { auto: true, disable: [] };
  }
}

/** True for the VITRIOL monorepo layout (officina/ + officina/tui/). */
export function vitriolSignature(cwd: string): boolean {
  return (
    existsSync(join(cwd, "officina", "package.json")) &&
    existsSync(join(cwd, "officina", "tui", "Cargo.toml"))
  );
}

function hasScripts(dir: string, script: string): boolean {
  try {
    const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8")) as {
      scripts?: Record<string, unknown>;
      devDependencies?: Record<string, unknown>;
      dependencies?: Record<string, unknown>;
    };
    return Boolean(pkg.scripts?.[script]);
  } catch {
    return false;
  }
}

function hasVitest(dir: string): boolean {
  try {
    const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8")) as {
      devDependencies?: Record<string, unknown>;
      dependencies?: Record<string, unknown>;
    };
    return Boolean(pkg.devDependencies?.vitest || pkg.dependencies?.vitest);
  } catch {
    return false;
  }
}

function nodeGates(dir: string, rel: string, auto: boolean): Gate[] {
  const out: Gate[] = [];
  const base = rel === "." ? "" : `${rel}::`;
  if (hasScripts(dir, "typecheck")) {
    out.push({ id: `${base}typecheck`, dir, rel, label: "typecheck", cmd: ["npm", "run", "typecheck"], tier: "cheap", parser: "tsc", auto });
  } else if (existsSync(join(dir, "tsconfig.json"))) {
    out.push({ id: `${base}tsc`, dir, rel, label: "tsc", cmd: ["npx", "tsc", "--noEmit"], tier: "cheap", parser: "tsc", auto });
  }
  if (hasScripts(dir, "test")) {
    out.push({
      id: `${base}test`, dir, rel, label: "npm test", cmd: ["npm", "test"], tier: "full",
      parser: hasVitest(dir) ? "vitest" : "generic", auto: false,
    });
  }
  if (existsSync(join(dir, "runtime", "build-patch.mjs"))) {
    out.push({
      id: `${base}canaries`, dir, rel, label: "build-patch canaries",
      cmd: ["node", "runtime/build-patch.mjs"], tier: "cheap", parser: "generic", auto,
    });
  }
  return out;
}

function cargoGates(dir: string, rel: string, auto: boolean): Gate[] {
  const base = rel === "." ? "" : `${rel}::`;
  return [
    { id: `${base}cargo-check`, dir, rel, label: "cargo check", cmd: ["cargo", "check", "--message-format=short"], tier: "cheap", parser: "cargo", auto },
    { id: `${base}cargo-fmt`, dir, rel, label: "cargo fmt", cmd: ["cargo", "fmt", "--check"], tier: "cheap", parser: "rustfmt", auto },
    { id: `${base}cargo-test`, dir, rel, label: "cargo test", cmd: ["cargo", "test"], tier: "full", parser: "generic", auto: false },
  ];
}

function pyGates(dir: string, rel: string): Gate[] {
  const base = rel === "." ? "" : `${rel}::`;
  return [
    { id: `${base}ruff`, dir, rel, label: "ruff", cmd: ["ruff", "check", "."], tier: "cheap", parser: "ruff", auto: rel === "." },
    { id: `${base}pytest`, dir, rel, label: "pytest", cmd: ["python3", "-m", "pytest", "-q"], tier: "full", parser: "pytest", auto: false },
  ];
}

function goGates(dir: string, rel: string): Gate[] {
  const base = rel === "." ? "" : `${rel}::`;
  return [
    { id: `${base}go-vet`, dir, rel, label: "go vet", cmd: ["go", "vet", "./..."], tier: "cheap", parser: "go", auto: rel === "." },
    { id: `${base}go-test`, dir, rel, label: "go test", cmd: ["go", "test", "./..."], tier: "full", parser: "generic", auto: false },
  ];
}

/** Official VITRIOL gate set (the monorepo special-case, AGENTS.md gates). */
export function vitriolGates(cwd: string): Gate[] {
  const officina = join(cwd, "officina");
  const tui = join(officina, "tui");
  return [
    ...nodeGates(officina, "officina", true),
    ...cargoGates(tui, "officina/tui", true),
  ];
}

/** Shallow listing of candidate manifest dirs (root + one level, capped). */
function manifestDirs(cwd: string): Array<{ dir: string; rel: string }> {
  const dirs: Array<{ dir: string; rel: string }> = [{ dir: cwd, rel: "." }];
  let entries: string[] = [];
  try {
    entries = readdirSync(cwd);
  } catch {
    return dirs;
  }
  for (const e of entries.sort()) {
    if (e.startsWith(".") || EXCLUDE_DIRS.has(e)) continue;
    try {
      if (statSync(join(cwd, e)).isDirectory()) dirs.push({ dir: join(cwd, e), rel: e });
    } catch {
      /* unreadable entry — skip */
    }
  }
  return dirs;
}

function genericGates(cwd: string): Gate[] {
  const found: Gate[] = [];
  const count: Record<string, number> = { cargo: 0, node: 0, py: 0, go: 0 };
  const budget = (kind: string) => count[kind]! < CAPS[kind]!;
  for (const { dir, rel } of manifestDirs(cwd)) {
    const auto = rel === ".";
    if (existsSync(join(dir, "package.json")) && budget("node")) {
      count.node!++;
      found.push(...nodeGates(dir, rel, auto));
    }
    if (existsSync(join(dir, "Cargo.toml")) && budget("cargo")) {
      count.cargo!++;
      found.push(...cargoGates(dir, rel, auto));
    }
    if ((existsSync(join(dir, "pyproject.toml")) || existsSync(join(dir, "setup.py"))) && budget("py")) {
      count.py!++;
      found.push(...pyGates(dir, rel));
    }
    if (existsSync(join(dir, "go.mod")) && budget("go")) {
      count.go!++;
      found.push(...goGates(dir, rel));
    }
  }
  return found;
}

/**
 * Final gate set. Precedence: verify.json `include` (authoritative) >
 * generic discovery + VITRIOL signature, then `disable` filter.
 */
export function detectGates(cwd: string): { gates: Gate[]; config: VerifyConfig } {
  const config = loadConfig(cwd);
  let gates: Gate[];
  if (config.include) {
    gates = config.include.map((g) => ({ ...g, dir: g.dir ?? join(cwd, g.rel === "." ? "" : g.rel) }));
  } else {
    gates = genericGates(cwd);
    if (vitriolSignature(cwd)) {
      const sig = vitriolGates(cwd);
      const have = new Set(gates.map((g) => g.id));
      gates.push(...sig.filter((g) => !have.has(g.id)));
    }
  }
  if (config.disable.length > 0) {
    const off = new Set(config.disable);
    gates = gates.filter((g) => !off.has(g.id));
  }
  // de-dup ids (signature vs generic overlap), first wins
  const seen = new Set<string>();
  gates = gates.filter((g) => (seen.has(g.id) ? false : (seen.add(g.id), true)));
  return { gates, config };
}
