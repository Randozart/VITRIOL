import { describe, expect, it } from "vitest";
import { jsImportedNames, pyImportedNames, renderImportNotice, unusedImports } from "./importlint.ts";

describe("pyImportedNames", () => {
  it("collects plain, aliased and from-imports", () => {
    const src = [
      "import os",
      "import os.path as osp",
      "from collections import OrderedDict, defaultdict as dd",
      "from x import (  # trailing comment",
      ")",
    ].join("\n");
    expect(pyImportedNames(src).sort()).toEqual(["OrderedDict", "dd", "os", "osp"].sort());
  });
});

describe("jsImportedNames", () => {
  it("collects default, named, aliased and namespace imports", () => {
    const src = [
      "import fs from 'node:fs'",
      "import { a, b as bee } from './x.js'",
      "import * as ns from 'mod'",
      "import 'side-effect'",
    ].join("\n");
    expect(jsImportedNames(src).sort()).toEqual(["a", "bee", "fs", "ns"].sort());
  });
});

describe("unusedImports", () => {
  it("flags names never referenced in the body", () => {
    const src = "import os\nimport sys\nprint(os.getcwd())";
    expect(unusedImports(src, pyImportedNames(src), "py")).toEqual(["sys"]);
  });
  it("counts aliased names only by their local binding", () => {
    const src = "from json import loads as jl\nprint(jl('[]'))";
    expect(unusedImports(src, pyImportedNames(src), "py")).toEqual([]);
  });
  it("word boundaries: no substring false positive", () => {
    const src = "import re\nprint(research)";
    expect(unusedImports(src, pyImportedNames(src), "py")).toEqual(["re"]);
  });
  it("js usage counts after stripping import lines", () => {
    const src = "import { used, dead } from './x.js'\nexport const y = used()";
    expect(unusedImports(src, jsImportedNames(src), "js")).toEqual(["dead"]);
  });
});

describe("renderImportNotice", () => {
  it("empty when clean", () => {
    expect(renderImportNotice("a.py", [], 6)).toBe("");
  });
  it("caps shown names", () => {
    const out = renderImportNotice("a.py", ["a", "b", "c"], 2);
    expect(out).toContain("a, b");
    expect(out).toContain("(+1 more)");
  });
});
