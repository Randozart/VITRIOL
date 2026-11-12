// Tests for the quick-add helpers and loose JSON parsing (panel stage 4).
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { enableAccount, parseJsonLoose, writeEnvKey } from "./index.ts";

const dirs: string[] = [];
const tmpDir = (): string => {
  const dir = mkdtempSync(join(tmpdir(), "caduceus-add-"));
  dirs.push(dir);
  return dir;
};
afterEach(() => {
  for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
});

describe("writeEnvKey", () => {
  it("appends a missing key and keeps the rest of the file", () => {
    const dir = tmpDir();
    writeFileSync(join(dir, "env"), "A=1\nB=2\n");
    writeEnvKey(dir, "GLM_API_KEY", "sk-test");
    const text = readFileSync(join(dir, "env"), "utf8");
    expect(text).toContain("A=1");
    expect(text).toContain("B=2");
    expect(text).toContain("GLM_API_KEY=sk-test");
  });

  it("replaces an existing assignment in place (no duplicates)", () => {
    const dir = tmpDir();
    writeFileSync(join(dir, "env"), "A=1\nGLM_API_KEY=old\nB=2\n");
    writeEnvKey(dir, "GLM_API_KEY", "new");
    const text = readFileSync(join(dir, "env"), "utf8");
    expect(text).toContain("GLM_API_KEY=new");
    expect(text).not.toContain("GLM_API_KEY=old");
    expect(text.match(/GLM_API_KEY=/g)).toHaveLength(1);
  });

  it("creates the env file when absent", () => {
    const dir = tmpDir();
    writeEnvKey(dir, "NEW_KEY", "v");
    expect(readFileSync(join(dir, "env"), "utf8")).toContain("NEW_KEY=v");
  });
});

describe("enableAccount", () => {
  const accounts = (enabled: boolean): string =>
    JSON.stringify({
      accounts: [
        { provider: "glm", credential_env: "GLM_API_KEY", enabled },
        { provider: "dahl", credential_env: "DAHL_API_KEY", enabled: false },
      ],
      oauth: [],
    });

  it("flips only the requested provider to enabled", () => {
    const dir = tmpDir();
    writeFileSync(join(dir, "accounts.json"), accounts(false));
    expect(enableAccount(dir, "GLM")).toBe(true);
    const doc = JSON.parse(readFileSync(join(dir, "accounts.json"), "utf8"));
    expect(doc.accounts[0].enabled).toBe(true);
    expect(doc.accounts[1].enabled).toBe(false);
  });

  it("returns false for unknown providers and missing files", () => {
    const dir = tmpDir();
    writeFileSync(join(dir, "accounts.json"), accounts(false));
    expect(enableAccount(dir, "nope")).toBe(false);
    expect(enableAccount(join(dir, "missing"), "glm")).toBe(false);
  });
});

describe("parseJsonLoose", () => {
  it("extracts JSON from noisy CLI output and strips ANSI", () => {
    const out = "\x1b[32mok\x1b[0m\n[{ \"id\": \"glm\" }]\ntrailing";
    expect(parseJsonLoose(out)).toEqual([{ id: "glm" }]);
  });

  it("returns null when no JSON is present", () => {
    expect(parseJsonLoose("nothing here")).toBeNull();
    expect(parseJsonLoose("")).toBeNull();
  });
});
