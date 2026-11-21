// dream-gen extension tests (2026-11-21, plan dream-rung2-3-plan D1).
// Pinned: I-E2 (byte-bounded, whole-file skip), I-P3 (provenance),
// I-E1 family (corpus hash separate from eval).
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { readTranscript, sessionDirFor, writeCorpus } from "./index.ts";
import {
  buildCorpus,
  type DreamPair,
} from "../_shared/dream-generator-core.ts";

let dir: string;
beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "dream-gen-"));
});
afterEach(() => rmSync(dir, { recursive: true, force: true }));

const MSG = (role: string, content: string, ts = "T"): string =>
  JSON.stringify({
    id: "x",
    parentId: null,
    type: "message",
    timestamp: ts,
    message: { role, content, timestamp: ts },
  });

describe("readTranscript (I-E2)", () => {
  it("extracts user+assistant text rows in order", () => {
    const p = join(dir, "s.jsonl");
    writeFileSync(
      p,
      [
        JSON.stringify({ type: "session", id: "1" }),
        MSG("user", "What is the KV budget?", "T1"),
        MSG("assistant", "40 GiB.", "T2"),
      ].join("\n"),
    );
    const rows = readTranscript(p);
    expect(rows).toEqual([
      { role: "user", text: "What is the KV budget?", ts: "T1" },
      { role: "assistant", text: "40 GiB.", ts: "T2" },
    ]);
  });

  it("skips non-message and malformed rows", () => {
    const p = join(dir, "s.jsonl");
    writeFileSync(
      p,
      [
        JSON.stringify({ type: "model_change", id: "m" }),
        "not json at all",
        MSG("user", "q?", "T1"),
      ].join("\n"),
    );
    expect(readTranscript(p)).toHaveLength(1);
  });

  it("skips a file that exceeds the byte budget whole (I-E2, not mid-pair)", () => {
    const p = join(dir, "big.jsonl");
    const big = MSG("user", "x".repeat(10_000), "T1");
    writeFileSync(p, big + "\n" + MSG("assistant", "answer 42", "T2"));
    // budget smaller than the file → nothing
    expect(readTranscript(p, 100)).toEqual([]);
  });

  it("returns [] for a missing file", () => {
    expect(readTranscript(join(dir, "nope.jsonl"))).toEqual([]);
  });
});

describe("sessionDirFor", () => {
  it("encodes the cwd with dashes", () => {
    expect(sessionDirFor("/a/b")).toBe(
      join(homedir(), ".pi", "agent", "sessions", "-a-b"),
    );
  });
});
import { homedir } from "node:os";

describe("writeCorpus (I-E1 family + append-only)", () => {
  it("writes one JSONL line per pair and embeds the hash", async () => {
    const pair: DreamPair = {
      id: "s#0@T",
      question: "q",
      answer: "40",
      provenance: "s#0@T",
      class: "number",
      mustContain: ["40"],
    };
    const corpus = await buildCorpus([pair], { createdAt: "C" });
    const path = writeCorpus(corpus);
    const lines = readFileSyncSafe(path).split("\n").filter(Boolean);
    expect(lines).toHaveLength(1);
    const doc = JSON.parse(lines[0]!);
    expect(doc.hash).toBe(corpus.hash);
    expect(doc.provenance).toBe("s#0@T");
  });

  it("is idempotent for an identical corpus (same hash → same file, I-P4)", async () => {
    const pair: DreamPair = {
      id: "s#0@T",
      question: "q",
      answer: "40",
      provenance: "s#0@T",
      class: "number",
      mustContain: ["40"],
    };
    const a = await buildCorpus([pair], { createdAt: "C" });
    const b = await buildCorpus([pair], { createdAt: "C" });
    expect(a.hash).toBe(b.hash);
    expect(writeCorpus(a)).toBe(writeCorpus(b));
  });
});

function readFileSyncSafe(p: string): string {
  // local import to avoid a circular top-level import in the test file
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  const { readFileSync } = require("node:fs") as typeof import("node:fs");
  return readFileSync(p, "utf8");
}
