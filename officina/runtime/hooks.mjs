// Loader hook — serves the Officina-patched interactive-mode to the pinned
// pi runtime (2026-08-31, layout fork steps 2-4).
//
// Mechanism: module.register() routes ALL resolves/loads through here.
// When the docked layout is active and the requested module is pi's
// interactive-mode.js, the LOAD returns our patched source while RESOLVE
// still pins every relative dependency to the original package directory —
// so the 5k-line module keeps its whole import graph without any rewriting.
//
// Everything else resolves/loads untouched.
//
// Provenance: original work, this repo. Fork plan:
// docs/LAYOUT-FORK-2026-08-31.md (Rule 9 conscious divergence).

import { existsSync, readFileSync } from "node:fs";
import { dirname, join, normalize } from "node:path";
import { pathToFileURL, fileURLToPath } from "node:url";

// [officina] patched targets: docked interactive-mode + session selector
// (empty-state honesty fix, owner report 2026-08-31).
// Paths are relative to the pi-coding-agent PACKAGE ROOT — the markdown
// component lives in the nested @earendil-works/pi-tui package, the others
// in pi-coding-agent's own dist (owner bugfix 2026-09-01: the markdown
// patch previously keyed off a dist-relative path and never loaded).
const PATCHED = new Map([
  ["dist/modes/interactive/interactive-mode.js", "interactive-mode.officina.js"],
  ["dist/modes/interactive/components/session-selector.js", "session-selector.officina.js"],
  ["node_modules/@earendil-works/pi-tui/dist/components/markdown.js", "markdown.officina.js"],
]);
function originalUrlFor(parentPath) {
  if (!pkgDist) return null;
  for (const rel of PATCHED.keys()) {
    if (parentPath.endsWith(rel)) return pathToFileURL(join(pkgRoot, rel)).href;
  }
  return null;
}

// pkgDir arrives via register()'s initialize data — hooks run on a separate
// thread, so main-thread env changes after register() are not visible here.
let pkgDist = null; // pi-coding-agent's dist/ directory
let pkgRoot = null; // pi-coding-agent's package root
let dockedActive = true;
export function initialize(data) {
  const d = data && data.pkgDist;
  if (d && existsSync(join(d, "modes", "interactive", "interactive-mode.js"))) {
    pkgDist = d;
    pkgRoot = dirname(d); // strip the trailing dist/
  }
  dockedActive = !(data && data.docked === false);
}

function patchedSourcePath() {
  // runtime/patched/interactive-mode.officina.js lives next to this hook's
  // sibling directory inside officina/runtime
  const runtimeDir = fileURLToPath(new URL(".", import.meta.url));
  const p = join(normalize(runtimeDir), "patched", "interactive-mode.officina.js");
  return existsSync(p) ? p : null;
}

export function resolve(specifier, context, nextResolve) {
  if (dockedActive && context.parentURL && String(context.parentURL).startsWith("file:")) {
    const orig = originalUrlFor(fileURLToPath(context.parentURL));
    if (orig) {
      // Relative imports from INSIDE a patched source must resolve as if
      // they came from the original file. Re-anchor via a synthetic parent.
      // orig is already a file:// URL string - pass it straight through
      return nextResolve(specifier, { ...context, parentURL: orig });
    }
  }
  return nextResolve(specifier, context);
}

export function load(url, context, nextLoad) {
  if (dockedActive) {
    for (const [rel, patchedName] of PATCHED) {
      const orig = pkgRoot ? pathToFileURL(join(pkgRoot, rel)).href : null;
      if (orig && url === orig) {
        const source = readFileSync(join(fileURLToPath(new URL(".", import.meta.url)), "patched", patchedName), "utf-8");
        return { format: "module", source, shortCircuit: true };
      }
    }
  }
  return nextLoad(url, context);
}
