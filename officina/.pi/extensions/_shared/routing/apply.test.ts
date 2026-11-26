// Actuation contract (2026-11-18): one place switches models.
//
// `applyModel` exists so that no router can invent its own latch. These tests
// pin the three things the old nine call sites each got wrong in some way:
// intent is announced BEFORE the switch, the switch is recorded with owner and
// reason, and a refusal can never break a turn.

import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  clearProgrammaticSwitch,
  wasProgrammaticSwitch,
} from "../model-switch-intent.ts";
import { applyModel, modelLabel } from "./apply.ts";

const emitted: unknown[] = [];
vi.mock("../events.ts", () => ({
  emitHarnessEvent: (rec: unknown) => emitted.push(rec),
  harnessEvent: (
    src: string,
    ev: string,
    fields: Record<string, unknown> = {},
  ) => ({
    src,
    ev,
    ...fields,
  }),
}));

const cloud = { provider: "caduceus", id: "opencode-go/deepseek-v4.1-flash" };

beforeEach(() => {
  emitted.length = 0;
  clearProgrammaticSwitch();
});
afterEach(() => clearProgrammaticSwitch());

describe("modelLabel", () => {
  it("names the lane the way the ledger does", () => {
    expect(modelLabel(cloud)).toBe("caduceus/opencode-go/deepseek-v4.1-flash");
    expect(modelLabel({ provider: "llamacpp" })).toBe("llamacpp");
    expect(modelLabel(undefined)).toBe("?");
  });
});

describe("applyModel", () => {
  it("keeps an operator menu action manual even after a programmatic switch", async () => {
    const pi = { setModel: async () => true };
    await applyModel(pi, cloud, { owner: "adaptive-router", reason: "overflow" });
    await applyModel(pi, cloud, { owner: "model-catalog", reason: "operator selection", operator: true });
    expect(wasProgrammaticSwitch()).toBe(false);
  });

  it("announces the intent so a router is not fooled by its own switch", async () => {
    // pi reports the operator and an extension identically (source "set"), so a
    // router's model_select handler leans on this marker to stay out of the way.
    const pi = { setModel: async () => true };
    expect(wasProgrammaticSwitch()).toBe(false);
    await applyModel(pi, cloud, {
      owner: "adaptive-router",
      reason: "overflow spill",
    });
    expect(wasProgrammaticSwitch()).toBe(true);
  });

  it("records who switched and why", async () => {
    const pi = { setModel: async () => true };
    await applyModel(pi, cloud, {
      owner: "phase-model",
      reason: "phase: review",
    });
    expect(emitted).toHaveLength(1);
    expect(emitted[0]).toMatchObject({ src: "lc-route", ev: "switch" });
    expect((emitted[0] as { detail: string }).detail).toBe(
      "phase-model: caduceus/opencode-go/deepseek-v4.1-flash — phase: review",
    );
  });

  it("records a refusal instead of swallowing it", async () => {
    const pi = { setModel: () => false };
    expect(
      await applyModel(pi, cloud, { owner: "coupling", reason: "manual swap" }),
    ).toBe(false);
    expect((emitted[0] as { detail: string }).detail).toContain("(refused)");
  });

  it("never throws when the switch does", async () => {
    const pi = {
      setModel: () => {
        throw new Error("no auth");
      },
    };
    await expect(
      applyModel(pi, cloud, { owner: "x", reason: "y" }),
    ).resolves.toBe(false);
  });

  it("returns false when the host has no setModel at all (older hosts, tests)", async () => {
    expect(await applyModel({}, cloud, { owner: "x", reason: "y" })).toBe(
      false,
    );
  });
});

describe("one actuation point", () => {
  // The bug class this replaces: nine call sites, each with its own latch. A
  // direct `pi.setModel` anywhere else silently bypasses the intent marker (so
  // routers mistake it for the operator) and the switch record (so a surprise
  // model change is unexplainable).
  it("is the only file that calls pi.setModel", () => {
    const dir = dirname(fileURLToPath(import.meta.url));
    const offenders: string[] = [];
    const walk = (p: string): void => {
      for (const entry of readdirSync(p, { withFileTypes: true })) {
        const full = join(p, entry.name);
        if (entry.isDirectory()) {
          if (entry.name === "node_modules" || entry.name.startsWith(".")) continue;
          walk(full);
          continue;
        }
        if (!entry.name.endsWith(".ts") || entry.name.endsWith(".test.ts")) continue;
        if (entry.name === "apply.ts") continue;
        const text = readFileSync(full, "utf8");
        // Ignore comments: several files explain the old behaviour.
        const code = text
          .split("\n")
          .filter((l) => !l.trim().startsWith("//"))
          .join("\n");
        if (/\bpi\.setModel\s*\(/.test(code)) offenders.push(entry.name);
      }
    };
    walk(resolve(dir, "..", ".."));
    expect(offenders).toEqual([]);
  });
});
