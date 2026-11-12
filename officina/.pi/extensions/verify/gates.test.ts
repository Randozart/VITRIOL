// Tests for verify gate discovery (pure).
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, describe, expect, it } from "vitest";
import { detectGates, loadConfig, vitriolSignature, type Gate } from "./gates.ts";

const roots: string[] = [];
function fixture(specs: Record<string, string>): string {
  const root = mkdtempSync(join(tmpdir(), "verify-gates-"));
  roots.push(root);
  for (const [rel, content] of Object.entries(specs)) {
    const p = join(root, rel);
    mkdirSync(join(p, ".."), { recursive: true });
    writeFileSync(p, content);
  }
  return root;
}
afterAll(() => roots.forEach((r) => rmSync(r, { recursive: true, force: true })));

const byLabel = (gates: Gate[], label: string) => gates.filter((g) => g.label === label);

describe("detectGates", () => {
  it("root node project: typecheck cheap+auto, test full, vitest parser", () => {
    const root = fixture({
      "package.json": JSON.stringify({
        scripts: { typecheck: "tsc --noEmit", test: "vitest --run" },
        devDependencies: { vitest: "^3" },
      }),
      "tsconfig.json": "{}",
    });
    const { gates } = detectGates(root);
    const tc = byLabel(gates, "typecheck")[0]!;
    expect(tc.tier).toBe("cheap");
    expect(tc.auto).toBe(true);
    expect(tc.parser).toBe("tsc");
    const test = byLabel(gates, "npm test")[0]!;
    expect(test.tier).toBe("full");
    expect(test.parser).toBe("vitest");
    expect(test.auto).toBe(false);
  });

  it("no typecheck script but tsconfig → npx tsc", () => {
    const root = fixture({
      "package.json": JSON.stringify({ scripts: { test: "vitest" }, devDependencies: { vitest: "1" } }),
      "tsconfig.json": "{}",
    });
    const gates = detectGates(root).gates;
    expect(byLabel(gates, "tsc")[0]!.cmd).toEqual(["npx", "tsc", "--noEmit"]);
  });

  it("rust project: check+fmt cheap auto at root, test full lazy", () => {
    const root = fixture({ "Cargo.toml": "[package]\nname='x'\n" });
    const gates = detectGates(root).gates;
    expect(byLabel(gates, "cargo check")[0]!.auto).toBe(true);
    expect(byLabel(gates, "cargo check")[0]!.parser).toBe("cargo");
    expect(byLabel(gates, "cargo fmt")[0]!.tier).toBe("cheap");
    expect(byLabel(gates, "cargo test")[0]!.tier).toBe("full");
    expect(byLabel(gates, "cargo test")[0]!.auto).toBe(false);
  });

  it("excludes engine/node_modules/target subtrees and dotted dirs", () => {
    const root = fixture({
      "engine/Cargo.toml": "[package]\nname='engine'\n",
      "node_modules/x/package.json": "{}",
      "target/Cargo.toml": "[package]\nname='t'\n",
      ".opencode/package.json": "{}",
      "package.json": JSON.stringify({ scripts: { test: "node -e 1" } }),
    });
    const gates = detectGates(root).gates;
    expect(gates.every((g) => !g.rel.startsWith("engine") && !g.rel.startsWith("target"))).toBe(true);
    expect(gates.some((g) => g.label === "npm test")).toBe(true);
  });

  it("subdir manifests are discovered but lazy (auto=false)", () => {
    const root = fixture({ "sub/Cargo.toml": "[package]\nname='s'\n" });
    const gates = detectGates(root).gates;
    const check = byLabel(gates, "cargo check")[0]!;
    expect(check.rel).toBe("sub");
    expect(check.auto).toBe(false);
  });

  it("build-patch.mjs → canaries gate", () => {
    const root = fixture({
      "package.json": "{}",
      "runtime/build-patch.mjs": "// canary",
    });
    const gates = detectGates(root).gates;
    const can = byLabel(gates, "build-patch canaries")[0]!;
    expect(can.tier).toBe("cheap");
    expect(can.cmd).toEqual(["node", "runtime/build-patch.mjs"]);
  });

  it("verify.json include is authoritative; disable filters", () => {
    const root = fixture({
      "package.json": JSON.stringify({ scripts: { test: "vitest" } }),
      ".officina/verify.json": JSON.stringify({
        include: [{ id: "::custom", dir: "", rel: ".", label: "custom", cmd: ["true"], tier: "cheap", parser: "generic", auto: true }],
        disable: ["::test"],
      }),
    });
    const { gates, config } = detectGates(root);
    expect(gates.map((g) => g.id)).toEqual(["::custom"]);
    expect(config.auto).toBe(true);
  });

  it("VITRIOL signature adds officina + officina/tui gates", () => {
    const root = fixture({
      "officina/package.json": JSON.stringify({ scripts: { typecheck: "tsc", test: "vitest" }, devDependencies: { vitest: "1" } }),
      "officina/tui/Cargo.toml": "[package]\nname='tui'\n",
    });
    expect(vitriolSignature(root)).toBe(true);
    const gates = detectGates(root).gates;
    const ids = gates.map((g) => g.id);
    expect(ids).toContain("officina::typecheck");
    expect(ids).toContain("officina/tui::cargo-check");
    expect(byLabel(gates, "cargo check").find((g) => g.rel === "officina/tui")!.auto).toBe(true);
  });

  it("generic caps stop crate storms (cargo ≤ 3 from generic)", () => {
    const specs: Record<string, string> = {};
    for (const name of ["aaa", "bbb", "ccc", "ddd", "eee"]) specs[`${name}/Cargo.toml`] = "[package]\nname='c'\n";
    const root = fixture(specs);
    const cargoChecks = detectGates(root).gates.filter((g) => g.id.endsWith("::cargo-check"));
    expect(cargoChecks.length).toBeLessThanOrEqual(3);
  });

  it("deterministic ids + no dupes", () => {
    const root = fixture({
      "officina/package.json": JSON.stringify({ scripts: { typecheck: "tsc" } }),
      "officina/tui/Cargo.toml": "[package]\nname='t'\n",
    });
    const gates = detectGates(root).gates;
    expect(new Set(gates.map((g) => g.id)).size).toBe(gates.length);
  });
});

describe("loadConfig", () => {
  it("missing file → defaults", () => {
    const root = mkdtempSync(join(tmpdir(), "verify-cfg-"));
    roots.push(root);
    const cfg = loadConfig(root);
    expect(cfg).toEqual({ auto: true, disable: [] });
  });
  it("invalid json → defaults", () => {
    const root = fixture({ ".officina/verify.json": "{nope" });
    expect(loadConfig(root).auto).toBe(true);
  });
  it("auto:false honored", () => {
    const root = fixture({ ".officina/verify.json": JSON.stringify({ auto: false }) });
    expect(loadConfig(root).auto).toBe(false);
  });
});
