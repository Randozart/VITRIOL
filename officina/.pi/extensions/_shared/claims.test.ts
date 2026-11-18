// Cross-session claim registry (2026-11-18).
//
// Two sessions in one project used to be able to edit the same file with zero
// signal. These tests pin the contract that makes the warning trustworthy:
// claims are per-pid shards, another session's claim is visible, self is not, a
// stale shard expires, and NOTHING here can throw into a tool call.

import { mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  CLAIM_TTL_MS,
  claimNote,
  foreignClaims,
  normalizePath,
  publishClaims,
  readClaims,
} from "./claims.ts";

const dirs: string[] = [];
function dir(): string {
  const d = mkdtempSync(join(tmpdir(), "claims-"));
  dirs.push(d);
  return d;
}
afterEach(() => {
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
});

describe("claim shards", () => {
  it("one file per pid, so two sessions cannot clobber each other", () => {
    const d = dir();
    publishClaims({ pid: 101, sessionId: "a", paths: ["src/a.rs"] }, d, 1_000);
    publishClaims({ pid: 202, sessionId: "b", paths: ["src/b.rs"] }, d, 1_000);
    expect(readdirSync(d).sort()).toEqual(["101.json", "202.json"]);
    expect(
      readClaims(1_001, CLAIM_TTL_MS, d)
        .map((c) => c.pid)
        .sort(),
    ).toEqual([101, 202]);
  });

  it("replaces this pid's claims instead of appending forever", () => {
    const d = dir();
    publishClaims({ pid: 101, sessionId: "a", paths: ["x", "y"] }, d, 1_000);
    publishClaims({ pid: 101, sessionId: "a", paths: ["z"] }, d, 1_100);
    expect(readClaims(1_200, CLAIM_TTL_MS, d)[0].paths).toEqual(["z"]);
  });

  it("expires a session that went quiet", () => {
    const d = dir();
    publishClaims({ pid: 101, sessionId: "a", paths: ["x"] }, d, 1_000);
    expect(readClaims(1_000 + CLAIM_TTL_MS + 1, CLAIM_TTL_MS, d)).toEqual([]);
  });

  it("survives a torn shard (one bad file costs one warning)", () => {
    const d = dir();
    writeFileSync(join(d, "999.json"), "{not json");
    publishClaims({ pid: 101, sessionId: "a", paths: ["x"] }, d, 1_000);
    expect(readClaims(1_001, CLAIM_TTL_MS, d).map((c) => c.pid)).toEqual([101]);
  });

  it("is silent before the directory exists", () => {
    expect(readClaims(1_000, CLAIM_TTL_MS, "/nonexistent/claims-dir")).toEqual(
      [],
    );
  });
});

describe("foreign claims", () => {
  it("reports another session's claim on the same path", () => {
    const d = dir();
    publishClaims(
      { pid: 202, sessionId: "b", paths: ["src/tui/layout.rs"] },
      d,
      1_000,
    );
    const found = foreignClaims(
      ["src/tui/layout.rs"],
      101,
      1_001,
      CLAIM_TTL_MS,
      d,
    );
    expect(found).toEqual([{ path: "src/tui/layout.rs", pid: 202 }]);
  });

  it("never reports self (negative control)", () => {
    const d = dir();
    publishClaims(
      { pid: 101, sessionId: "a", paths: ["src/own.rs"] },
      d,
      1_000,
    );
    expect(foreignClaims(["src/own.rs"], 101, 1_001, CLAIM_TTL_MS, d)).toEqual(
      [],
    );
  });

  it("normalises ./ and trailing slashes so spellings compare equal", () => {
    const d = dir();
    publishClaims(
      { pid: 202, sessionId: "b", paths: ["./src/x.rs"] },
      d,
      1_000,
    );
    expect(
      foreignClaims(["src/x.rs/"], 101, 1_001, CLAIM_TTL_MS, d),
    ).toHaveLength(1);
    expect(normalizePath("./a/b/")).toBe("a/b");
  });
});

describe("claim note", () => {
  it("names the other session and the file, compactly", () => {
    expect(claimNote([{ path: "/repo/src/tui/layout.rs", pid: 3911 }])).toBe(
      "⚠ layout.rs also open in session 3911",
    );
    expect(
      claimNote([
        { path: "a.rs", pid: 1 },
        { path: "b.rs", pid: 2 },
      ]),
    ).toBe("⚠ a.rs (+1) also open in session 1");
  });

  it("is silent when there is no conflict", () => {
    expect(claimNote([])).toBeNull();
  });
});
