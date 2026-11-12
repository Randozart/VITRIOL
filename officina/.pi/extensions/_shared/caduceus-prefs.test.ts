import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  allowPaidOwners,
  applyPrefs,
  capCandidates,
  normalizePrefs,
  prefsPath,
  readPrefs,
  type CaduceusPrefs,
  type PrefCandidate,
} from "./caduceus-prefs.ts";

const cand = (id: string, owner: string, free = true): PrefCandidate => ({ id, owner, free });

const POOL: PrefCandidate[] = [
  cand("opencode-go/deepseek-v4.1-flash", "opencode-go"),
  cand("opencode-go/qwen3.8-flash", "opencode-go"),
  cand("dahl/MiniMax-M2.7", "dahl"),
  cand("agentrouter/gpt-5.6-sol", "agentrouter"),
  cand("glm/glm-4.7-flash", "glm"),
];

const ids = (list: PrefCandidate[]): string[] => list.map((c) => c.id);

describe("normalizePrefs", () => {
  it("rejects absent, corrupt, array and unsupported shapes", () => {
    expect(normalizePrefs(null)).toBeNull();
    expect(normalizePrefs([])).toBeNull();
    expect(normalizePrefs({})).toBeNull();
    expect(normalizePrefs({ version: 2 })).toBeNull();
    expect(normalizePrefs({ version: 1, per_owner: "nope" })).toEqual({ version: 1 });
  });

  it("clamps caps and keeps valid provider rows", () => {
    const prefs = normalizePrefs({
      version: 1,
      free_only: false,
      per_owner: 99,
      max_candidates: 0,
      providers: {
        "OpenCode-Go": { routing: "off", rank: 4.9, allow_paid: true },
        broken: "nope",
        empty: {},
        badrouting: { routing: "sometimes" },
      },
      models: { pinned: ["a/b", ""], denied: "nope" },
    });
    expect(prefs).toEqual({
      version: 1,
      free_only: false,
      per_owner: 32,
      max_candidates: 1,
      providers: { "opencode-go": { routing: "off", rank: 4, allow_paid: true } },
      models: { pinned: ["a/b"], denied: [] },
    });
  });
});

describe("readPrefs", () => {
  const dirs: string[] = [];
  const tmpFile = (contents: string): string => {
    const dir = mkdtempSync(join(tmpdir(), "caduceus-prefs-"));
    dirs.push(dir);
    const path = join(dir, "caduceus-prefs.json");
    writeFileSync(path, contents);
    return path;
  };
  afterEach(() => {
    for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
  });

  it("returns null when the file is missing (I1 parity path)", () => {
    expect(readPrefs(join(tmpdir(), "definitely-missing-caduceus-prefs.json"))).toBeNull();
  });

  it("returns null for invalid JSON (I6 corrupt-safe)", () => {
    expect(readPrefs(tmpFile("{not json"))).toBeNull();
  });

  it("reads a valid document", () => {
    const prefs = readPrefs(tmpFile(JSON.stringify({ version: 1, free_only: false })));
    expect(prefs?.free_only).toBe(false);
  });

  it("honours the env seam", () => {
    const path = tmpFile(JSON.stringify({ version: 1 }));
    const prev = process.env.OFFICINA_CADUCEUS_PREFS_FILE;
    process.env.OFFICINA_CADUCEUS_PREFS_FILE = path;
    try {
      expect(prefsPath()).toBe(path);
      expect(readPrefs()).toEqual({ version: 1 });
    } finally {
      if (prev === undefined) delete process.env.OFFICINA_CADUCEUS_PREFS_FILE;
      else process.env.OFFICINA_CADUCEUS_PREFS_FILE = prev;
    }
  });
});

describe("applyPrefs", () => {
  it("returns the pool untouched without prefs (I1)", () => {
    expect(applyPrefs(POOL, null)).toBe(POOL);
  });

  it("keeps order for a neutral document", () => {
    expect(ids(applyPrefs(POOL, { version: 1 }))).toEqual(ids(POOL));
  });

  it("drops denied models and routing-off owners", () => {
    const prefs: CaduceusPrefs = {
      version: 1,
      providers: { glm: { routing: "off" } },
      models: { denied: ["dahl/MiniMax-M2.7"], pinned: [] },
    };
    expect(ids(applyPrefs(POOL, prefs))).toEqual([
      "opencode-go/deepseek-v4.1-flash",
      "opencode-go/qwen3.8-flash",
      "agentrouter/gpt-5.6-sol",
    ]);
  });

  it("moves pinned models to the front in listed order", () => {
    const prefs: CaduceusPrefs = {
      version: 1,
      models: { pinned: ["glm/glm-4.7-flash", "dahl/MiniMax-M2.7"], denied: [] },
    };
    expect(ids(applyPrefs(POOL, prefs))).toEqual([
      "glm/glm-4.7-flash",
      "dahl/MiniMax-M2.7",
      "opencode-go/deepseek-v4.1-flash",
      "opencode-go/qwen3.8-flash",
      "agentrouter/gpt-5.6-sol",
    ]);
  });

  it("puts explicitly ranked owners first, ascending, unranked stable", () => {
    const prefs: CaduceusPrefs = {
      version: 1,
      providers: { glm: { rank: 0 }, agentrouter: { rank: 1 } },
    };
    expect(ids(applyPrefs(POOL, prefs))).toEqual([
      "glm/glm-4.7-flash",
      "agentrouter/gpt-5.6-sol",
      "opencode-go/deepseek-v4.1-flash",
      "opencode-go/qwen3.8-flash",
      "dahl/MiniMax-M2.7",
    ]);
  });

  it("does not add candidates (I2)", () => {
    const prefs: CaduceusPrefs = {
      version: 1,
      providers: { unseen: { rank: 0 } },
      models: { pinned: ["not/in-catalog"], denied: [] },
    };
    expect(ids(applyPrefs(POOL, prefs))).toEqual(ids(POOL));
  });
});

describe("capCandidates", () => {
  const many: PrefCandidate[] = [
    ...Array.from({ length: 10 }, (_, i) => cand(`opencode-go/m${i}`, "opencode-go")),
    ...Array.from({ length: 10 }, (_, i) => cand(`dahl/m${i}`, "dahl")),
  ];

  it("applies the env caps (historical behaviour)", () => {
    expect(capCandidates(many, null, { perOwner: 3, max: 5 })).toHaveLength(5);
    expect(
      capCandidates(many, null, { perOwner: 2, max: 50 }).filter((c) => c.owner === "opencode-go"),
    ).toHaveLength(2);
  });

  it("lets explicit prefs caps win over env", () => {
    const prefs: CaduceusPrefs = { version: 1, per_owner: 3, max_candidates: 4 };
    const capped = capCandidates(many, prefs, { perOwner: 1, max: 100 });
    expect(capped).toHaveLength(4);
    expect(capped.filter((c) => c.owner === "opencode-go")).toHaveLength(3);
    expect(capped.filter((c) => c.owner === "dahl")).toHaveLength(1);
  });
});

describe("allowPaidOwners", () => {
  it("collects only explicitly opted-in owners", () => {
    const prefs: CaduceusPrefs = {
      version: 1,
      providers: { glm: { allow_paid: true }, dahl: { allow_paid: false } },
    };
    expect([...allowPaidOwners(prefs)]).toEqual(["glm"]);
    expect(allowPaidOwners(null).size).toBe(0);
  });
});
