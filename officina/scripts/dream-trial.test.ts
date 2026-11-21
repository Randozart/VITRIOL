// dream-trial.sh dry-run gate (2026-11-21, plan dream-rung2-3-plan D2).
// Pins I-P4 (reversible: a corrupt snapshot is refused, config untouched)
// and the dry-run contract (no systemd, no writes) without a live engine.
import { mkdtempSync, rmSync, writeFileSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFileSync } from "node:child_process";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

let dir: string;
const SCRIPT = join(process.cwd(), "..", "scripts", "dream-trial.sh");

function run(args: string[], env: Record<string, string>): { code: number; out: string } {
  try {
    const out = execFileSync("bash", [SCRIPT, ...args], {
      env: { ...process.env, OFFICINA_DRY_RUN: "1", ...env },
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
    return { code: 0, out };
  } catch (e) {
    const err = e as { status?: number; stdout?: string; stderr?: string };
    return {
      code: err.status ?? 1,
      out: `${err.stdout ?? ""}${err.stderr ?? ""}`,
    };
  }
}
const env = (config: string, snap: string) => ({
  DREAM_TRIAL_CONFIG: config,
  DREAM_TRIAL_SNAPSHOT: snap,
  DREAM_TRIAL_MODEL: "/nonexistent/tiny.gguf",
});

beforeEach(() => { dir = mkdtempSync(join(tmpdir(), "dream-trial-")); });
afterEach(() => rmSync(dir, { recursive: true, force: true }));

describe("dream-trial.sh dry-run", () => {
  it("swap-in is read-only under OFFICINA_DRY_RUN (no config write, no restart)", () => {
    const config = join(dir, "config");
    const orig = "model = big\ncontext = 262144\n";
    writeFileSync(config, orig);
    const { code, out } = run(["swap-in"], env(config, join(dir, "snap")));
    expect(code).toBe(0);
    expect(out).toContain("DRY: would snapshot");
    expect(out).toContain("DRY: systemctl restart");
    // config untouched, no snapshot file created
    expect(readFileSync(config, "utf8")).toBe(orig);
  });

  it("swap-out refuses a corrupt (tiny) snapshot and leaves config intact (I-P4)", () => {
    const config = join(dir, "config");
    const orig = "model = big\ncontext = 262144\n";
    writeFileSync(config, orig);
    const snap = join(dir, "snap");
    writeFileSync(snap, "x"); // corrupt: 1 byte
    const { code, out } = run(["swap-out"], env(config, snap));
    expect(code).not.toBe(0);
    expect(out).toContain("refusing to restore");
    expect(readFileSync(config, "utf8")).toBe(orig);
  });

  it("swap-out refuses when no snapshot exists", () => {
    const config = join(dir, "config");
    writeFileSync(config, "model = big\n");
    const { code, out } = run(["swap-out"], env(config, join(dir, "nope")));
    expect(code).not.toBe(0);
    expect(out).toContain("no snapshot");
  });

  it("status prints config + dry-run state and takes no action", () => {
    const config = join(dir, "config");
    writeFileSync(config, "model = big\n");
    const { code, out } = run(["status"], env(config, join(dir, "snap")));
    expect(code).toBe(0);
    expect(out).toContain(config);
    expect(out).toContain("dry-run:   1");
  });
});
