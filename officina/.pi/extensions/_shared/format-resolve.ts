// Formatter binary resolution (2026-11-14): project-local first, then PATH.
//
// Shared by format-gate (automatic) and the `code` tool (explicit) so both
// pick the same binary. Kept out of format-gate/format.ts (which stays pure)
// and out of the extensions (jiti gives each import site its own module
// instance anyway; this module holds only a process-local cache).

import { execFileSync } from "node:child_process";
import { accessSync, constants, existsSync } from "node:fs";
import { dirname, join } from "node:path";

const cache = new Map<string, string | null>();

function executable(path: string): boolean {
  try {
    accessSync(path, constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

/** Resolve `<cmd>` for an edited file: nearest node_modules/.bin, then PATH. */
export function resolveFormatterExecutable(file: string, cmd: string): string | null {
  const key = `${dirname(file)}::${cmd}`;
  const hit = cache.get(key);
  if (hit !== undefined) return hit;

  let dir = dirname(file);
  let found: string | null = null;
  for (let i = 0; i < 6; i++) {
    const candidate = join(dir, "node_modules", ".bin", cmd);
    if (existsSync(candidate) && executable(candidate)) {
      found = candidate;
      break;
    }
    const parent = dirname(dir);
    if (!parent || parent === dir) break;
    dir = parent;
  }
  if (!found) {
    let ok = cache.get(`PATH::${cmd}`);
    if (ok === undefined) {
      try {
        execFileSync("sh", ["-c", `command -v ${cmd}`], { stdio: "pipe" });
        ok = cmd;
      } catch {
        ok = "";
      }
      cache.set(`PATH::${cmd}`, ok);
    }
    found = ok ? ok : null;
  }
  cache.set(key, found);
  return found;
}

/** Test hook: forget cached probes (tests inject fake binaries). */
export function resetFormatterCache(): void {
  cache.clear();
}
