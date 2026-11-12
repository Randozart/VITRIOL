// longjob rules (2026-11-12, I1-I7).
import { describe, expect, it } from "vitest";
import {
  hasFgOptOut,
  isLongRunner,
  jobDir,
  jobsToAnnounce,
  rewriteToDispatch,
  shellQuote,
  snapshotJob,
} from "./longjob.ts";

describe("isLongRunner", () => {
  it("matches known long-runners", () => {
    for (const c of [
      "cargo build --release",
      "cargo test --lib",
      "CARGO_TARGET_DIR=/scratch/x cargo build",
      "make build",
      "brievc build c4probe.bv --library --verify-bodies",
      "npm install",
      "npm run build",
      "pytest -q",
      "go build ./...",
      "cmake -B build",
      "env FOO=1 make all",
      "sudo cargo build",
    ]) {
      expect(isLongRunner(c), c).toBe(true);
    }
  });

  it("leaves short commands alone (I4 precision)", () => {
    for (const c of [
      "ls -la",
      "git status",
      "git commit -m x",
      "cat /tmp/r2.json",
      "echo hi",
      "head -c 400 /tmp/o2/r2.log",
    ]) {
      expect(isLongRunner(c), c).toBe(false);
    }
  });

  it("#fg always wins (I4)", () => {
    expect(isLongRunner("cargo build --release #fg")).toBe(false);
    expect(hasFgOptOut("cargo build #FG")).toBe(true);
    expect(hasFgOptOut("cargo build")).toBe(false);
  });
});

describe("rewriteToDispatch", () => {
  it("forks detached, writes pid/log, and returns a pointer line", () => {
    const paths = jobDir("/tmp/lj", "j-1");
    const out = rewriteToDispatch("cargo build --release", paths, "j-1");
    expect(out).toContain("mkdir -p '/tmp/lj/j-1'");
    expect(out).toContain("setsid bash -c");
    expect(out).toContain("echo $! > '/tmp/lj/j-1/pid'");
    expect(out).toContain("longjob j-1 dispatched");
    expect(out).toContain("bg_status j-1");
  });

  it("quotes payloads with quotes, dollars and newlines", () => {
    const cmd = `echo 'a b' && echo "$X"`;
    const out = rewriteToDispatch(cmd, jobDir("/tmp/lj", "j-2"), "j-2");
    expect(out).toContain(shellQuote(`cd "$PWD"; ${cmd}`));
    // the quoted payload must survive as ONE argument
    expect(out.includes(`'\\''a b'\\''`)).toBe(true);
  });
});

describe("snapshotJob + announce-once", () => {
  const fake = (files: Record<string, string>) => {
    const store = files;
    return {
      files: store,
      readFile: (p: string) => {
        const base = p.split("/").pop()!;
        if (base in store) return store[base];
        throw new Error("missing");
      },
      tailLines: (p: string, n: number) => {
        const base = p.split("/").pop()!;
        return (store[base] ?? "").split("\n").slice(-n).join("\n");
      },
      readdir: () => (Object.keys(store).length ? ["j-a"] : []),
    };
  };

  it("missing exit file = running; exit file = finished", () => {
    const running = snapshotJob("/tmp/lj", "j-a", fake({ "out.log": "compiling" }));
    expect(running?.running).toBe(true);
    expect(running?.exit).toBeNull();
    expect(running?.tail).toBe("compiling");
    const done = snapshotJob("/tmp/lj", "j-a", fake({ "out.log": "ok", exit: "exit=0" }));
    expect(done?.running).toBe(false);
    expect(done?.exit).toBe(0);
  });

  it("announces finished-unannounced jobs exactly once", () => {
    const deps = fake({ "out.log": "ok", exit: "exit=0" });
    expect(jobsToAnnounce("/tmp/lj", deps)).toEqual(["j-a"]);
    // announce flag written → no longer in the list
    deps.files.announced = "1";
    expect(jobsToAnnounce("/tmp/lj", deps)).toEqual([]);
  });

  it("never announces still-running jobs (I2)", () => {
    const deps = fake({ "out.log": "compiling" });
    expect(jobsToAnnounce("/tmp/lj", deps)).toEqual([]);
  });
});
