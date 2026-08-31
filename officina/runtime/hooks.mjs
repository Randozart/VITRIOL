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

const TARGET_SUFFIX = "/dist/modes/interactive/interactive-mode.js";

// pkgDir arrives via register()'s initialize data — hooks run on a separate
// thread, so main-thread env changes after register() are not visible here.
let pkgDist = null;
let dockedActive = true;
export function initialize(data) {
  const d = data && data.pkgDist;
  if (d && existsSync(join(d, "modes", "interactive", "interactive-mode.js"))) {
    pkgDist = d;
  }
  dockedActive = !(data && data.docked === false);
}

function originalUrl() {
  return pkgDist ? pathToFileURL(join(pkgDist, "modes", "interactive", "interactive-mode.js")).href : null;
}

function patchedSourcePath() {
  // runtime/patched/interactive-mode.officina.js lives next to this hook's
  // sibling directory inside officina/runtime
  const runtimeDir = fileURLToPath(new URL(".", import.meta.url));
  const p = join(normalize(runtimeDir), "patched", "interactive-mode.officina.js");
  return existsSync(p) ? p : null;
}

export function resolve(specifier, context, nextResolve) {
  const orig = dockedActive ? originalUrl() : null;
  if (orig && context.parentURL && String(context.parentURL).startsWith("file:")) {
    const parentPath = fileURLToPath(context.parentURL);
    if (parentPath.endsWith(TARGET_SUFFIX)) {
      // Relative imports from INSIDE the patched source must resolve as if
      // they came from the original file. Re-anchor via a synthetic parent.
      // orig is already a file:// URL string - pass it straight through
      return nextResolve(specifier, { ...context, parentURL: orig });
    }
  }
  return nextResolve(specifier, context);
}

export function load(url, context, nextLoad) {
  const orig = dockedActive ? originalUrl() : null;
  if (dockedActive && orig && url === orig) {
    const patched = patchedSourcePath();
    if (patched) {
      const source = readFileSync(patched, "utf-8");
      return { format: "module", source, shortCircuit: true };
    }
  }
  return nextLoad(url, context);
}
