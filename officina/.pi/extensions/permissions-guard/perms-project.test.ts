import {
  mkdtempSync,
  existsSync,
  readdirSync,
  writeFileSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  addProjectRule,
  clearProjectRules,
  leadingProgram,
  loadProjectStore,
  matchesProjectRule,
  normalizeCommand,
  projectStorePath,
} from "./perms-project.ts";

function tempHome(): string {
  return mkdtempSync(join(tmpdir(), "officina-perms-"));
}

describe("perms-project", () => {
  // I3: exact by default. Allowing one command must not cover a compound.
  it("command rules match exactly, never by prefix", () => {
    const home = tempHome();
    const cwd = "/tmp/proj-a";
    addProjectRule(
      cwd,
      { tool: "bash", kind: "command", pattern: "make -j12" },
      home,
    );
    const rules = loadProjectStore(cwd, home).rules;
    expect(matchesProjectRule(rules, "bash", "make  -j12 ")).not.toBeNull();
    expect(
      matchesProjectRule(rules, "bash", "make -j12 && rm -rf /"),
    ).toBeNull();
    expect(matchesProjectRule(rules, "bash", "make -j8")).toBeNull();
    expect(matchesProjectRule(rules, "edit", "make -j12")).toBeNull();
  });

  it("program rules cover only the leading program", () => {
    const home = tempHome();
    const cwd = "/tmp/proj-b";
    addProjectRule(
      cwd,
      { tool: "bash", kind: "program", pattern: "cargo" },
      home,
    );
    const rules = loadProjectStore(cwd, home).rules;
    expect(
      matchesProjectRule(rules, "bash", "cargo build --release"),
    ).not.toBeNull();
    expect(
      matchesProjectRule(rules, "bash", "FOO=1 cargo test"),
    ).not.toBeNull();
    expect(matchesProjectRule(rules, "bash", "make test")).toBeNull();
  });

  it("extracts the leading program past env prefixes and wrappers", () => {
    expect(leadingProgram("FOO=1 cargo test")).toBe("cargo");
    expect(leadingProgram("sudo systemctl restart x")).toBe("systemctl");
    expect(leadingProgram("/usr/bin/git status")).toBe("git");
    expect(leadingProgram("   ")).toBe("");
  });

  // I1: a corrupt store must grant nothing.
  it("fails closed on a corrupt store", () => {
    const home = tempHome();
    const cwd = "/tmp/proj-c";
    const path = projectStorePath(cwd, home);
    addProjectRule(cwd, { tool: "bash", kind: "command", pattern: "ls" }, home);
    writeFileSync(path, "{ not json");
    expect(loadProjectStore(cwd, home).rules).toEqual([]);
    expect(
      matchesProjectRule(loadProjectStore(cwd, home).rules, "bash", "ls"),
    ).toBeNull();
  });

  // I4: project isolation.
  it("keeps rules per project", () => {
    const home = tempHome();
    addProjectRule(
      "/tmp/proj-one",
      { tool: "bash", kind: "command", pattern: "make" },
      home,
    );
    addProjectRule(
      "/tmp/proj-two",
      { tool: "bash", kind: "command", pattern: "cargo" },
      home,
    );
    const one = loadProjectStore("/tmp/proj-one", home).rules;
    const two = loadProjectStore("/tmp/proj-two", home).rules;
    expect(one.map((r) => r.pattern)).toEqual(["make"]);
    expect(two.map((r) => r.pattern)).toEqual(["cargo"]);
    expect(matchesProjectRule(one, "bash", "cargo build")).toBeNull();
  });

  // I5: no store ⇒ no rules (parity with today's behaviour).
  it("is empty when nothing was ever stored", () => {
    const home = tempHome();
    expect(loadProjectStore("/tmp/proj-none", home).rules).toEqual([]);
  });

  // I6: the store never lands inside the project.
  it("never writes inside the project directory", () => {
    const home = tempHome();
    const cwd = tempHome();
    addProjectRule(cwd, { tool: "bash", kind: "command", pattern: "ls" }, home);
    expect(existsSync(projectStorePath(cwd, home))).toBe(true);
    expect(readdirSync(cwd)).toEqual([]);
    expect(projectStorePath(cwd, home).startsWith(home)).toBe(true);
  });

  it("normalises whitespace and clears on request", () => {
    expect(normalizeCommand("  a   b \n c ")).toBe("a b c");
    const home = tempHome();
    const cwd = "/tmp/proj-clear";
    addProjectRule(cwd, { tool: "bash", kind: "command", pattern: "ls" }, home);
    expect(clearProjectRules(cwd, home)).toBe(true);
    expect(loadProjectStore(cwd, home).rules).toEqual([]);
    rmSync(home, { recursive: true, force: true });
  });
});
