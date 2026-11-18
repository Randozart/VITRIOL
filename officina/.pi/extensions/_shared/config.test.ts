// Knob inventory gate (2026-11-18).

import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { KNOBS, envFlag, envNumber, envString } from "./config.ts";

const here = dirname(fileURLToPath(import.meta.url));
const extensionsDir = resolve(here, "..");

function discover(): string[] {
  const names = new Set<string>();
  const walk = (dir: string): void => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        if (entry.name === "node_modules" || entry.name.startsWith(".")) continue;
        walk(full);
        continue;
      }
      if (!entry.name.endsWith(".ts") || entry.name.endsWith(".test.ts")) continue;
      for (const m of readFileSync(full, "utf8").matchAll(/\b(OFFICINA_[A-Z0-9_]+)\b/g)) {
        names.add(m[1]);
      }
    }
  };
  walk(extensionsDir);
  return [...names].sort();
}

describe("knob inventory", () => {
  it("declares every OFFICINA_* the sources use", () => {
    const undeclared = discover().filter((n) => !KNOBS.includes(n));
    expect(undeclared).toEqual([]);
  });

  it("keeps the inventory honest (no stale knobs)", () => {
    const found = new Set(discover());
    expect(KNOBS.filter((n) => !found.has(n))).toEqual([]);
  });
});

describe("typed readers", () => {
  // Negative controls: the point of these helpers is that an unset or nonsense
  // value cannot masquerade as "on" or as a number.
  it("reads flags conservatively", () => {
    expect(envFlag("X", { X: "1" })).toBe(true);
    expect(envFlag("X", { X: "true" })).toBe(true);
    expect(envFlag("X", { X: "0" })).toBe(false);
    expect(envFlag("X", { X: "" })).toBe(false);
    expect(envFlag("X", {})).toBe(false);
  });

  it("falls back on unparseable numbers", () => {
    expect(envNumber("X", 7, { X: "12" })).toBe(12);
    expect(envNumber("X", 7, { X: "abc" })).toBe(7);
    expect(envNumber("X", 7, {})).toBe(7);
  });

  it("trims strings", () => {
    expect(envString("X", { X: "  v  " })).toBe("v");
    expect(envString("X", {})).toBe("");
  });
});
