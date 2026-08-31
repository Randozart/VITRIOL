import { describe, it, expect, afterAll } from "vitest";
import { existsSync } from "node:fs";
import { RepoMapClient, capText, repoMapConfig } from "./client.ts";

// Unit tests: pure config + budget-capping logic.
// Live tests (RM_LIVE=1): real shim against the repo-map repo itself —
//   RM_LIVE=1 npx vitest run .pi/extensions/repo-map/repo-map.test.ts

describe("repoMapConfig", () => {
  it("defaults: enabled, home-relative paths, 2000-char output cap", () => {
    const cfg = repoMapConfig({ HOME: "/home/tester" });
    expect(cfg.enabled).toBe(true);
    expect(cfg.pythonBin).toContain("/home/tester/venvs/repo-map/bin/python");
    expect(cfg.repomapDir).toContain("repo-map");
    expect(cfg.maxOutputChars).toBe(2000);
  });

  it("kill switch TRIS_NO_REPO_MAP=1 disables the extension", () => {
    expect(repoMapConfig({ TRIS_NO_REPO_MAP: "1" }).enabled).toBe(false);
  });

  it("env overrides for python bin and repo-map dir", () => {
    const cfg = repoMapConfig({ TRIS_REPO_MAP_PY: "/opt/py", TRIS_REPO_MAP_DIR: "/opt/rm" });
    expect(cfg.pythonBin).toBe("/opt/py");
    expect(cfg.repomapDir).toBe("/opt/rm");
  });
});

describe("capText", () => {
  it("passes short text through untouched", () => {
    expect(capText("hello", 2000)).toBe("hello");
  });

  it("caps at maxChars with a truncation note", () => {
    const out = capText("x".repeat(5000), 2000);
    expect(out.length).toBeLessThan(2200);
    expect(out).toContain("truncated 3000 chars");
  });

  it("exact boundary is not truncated", () => {
    const s = "y".repeat(100);
    expect(capText(s, 100)).toBe(s);
  });
});

describe("RepoMapClient availability", () => {
  it("reports unavailable when the interpreter path does not exist", () => {
    const client = new RepoMapClient(repoMapConfig({ TRIS_REPO_MAP_PY: "/nonexistent/python" }));
    expect(client.isAvailable()).toBe(false);
  });

  it("request rejects cleanly when shim cannot start", async () => {
    const client = new RepoMapClient(repoMapConfig({ TRIS_REPO_MAP_PY: "/nonexistent/python" }));
    await expect(client.request("ping", {})).rejects.toThrow(/unavailable/);
  });
});

const LIVE = process.env.RM_LIVE === "1" && existsSync(repoMapConfig().pythonBin);
describe.skipIf(!LIVE)("repo-map shim live", () => {
  const client = new RepoMapClient(repoMapConfig());
  afterAll(() => client.shutdown());

  it("answers ping", async () => {
    expect(await client.request("ping", {})).toBe("pong");
  }, 90_000);

  it("indexes itself and outlines a file", async () => {
    const repo = repoMapConfig().repomapDir;
    const idx = await client.index(repo);
    expect(idx).toMatch(/server\.py|d[ée]finitions?|definitions/i);
    const out = await client.request("outline", { file: "server.py", repo });
    expect(out).toContain("def ");
  }, 120_000);

  it("where_is finds build for a known symbol", async () => {
    const repo = repoMapConfig().repomapDir;
    await client.ensureIndexed(repo);
    const hits = await client.request("where_is", { query: "build", repo });
    expect(hits.length).toBeGreaterThan(0);
  }, 120_000);
});
