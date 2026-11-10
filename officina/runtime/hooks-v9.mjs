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
  ["dist/modes/interactive/components/tool-execution.js", "components/tool-execution.js"],
  ["node_modules/@earendil-works/pi-tui/dist/components/markdown.js", "markdown.officina.js"],
  // [officina P12] the BUNDLED CLI (dist/bundle/cli-runtime.js) never imports
  // the vendor interactive-mode.js above — its chunks inline their own
  // InteractiveMode/CustomEditor. Serve the P12-patched chunk (composer
  // wrap-edge hook) so P11 works in docked/bundle mode.
  ["dist/bundle/chunks/chunk-44DLKSKE.js", "chunks/chunk-44DLKSKE.js"],
]);
function originalUrlFor(parentPath) {
  if (!pkgDist) return null;
  for (const rel of PATCHED.keys()) {
    if (parentPath.endsWith(rel)) return pathToFileURL(join(pkgRoot, rel)).href;
  }
  // Relative imports from INSIDE a patched bundle chunk must resolve as if
  // they came from the original chunk (same directory, ./chunk-*.js peers).
  if (parentPath.includes("/patched/chunks/")) {
    const base = pathToFileURL(join(pkgRoot, "dist", "bundle", "chunks")).href;
    return base;
  }
  return null;
}

// pkgDir arrives via register()'s initialize data — hooks run on a separate
// thread, so main-thread env changes after register() are not visible here.
let pkgDist = null; // pi-coding-agent's dist/ directory
let pkgRoot = null; // pi-coding-agent's package root
let dockedActive = true;
export function initialize(data) {
  { const _t = process.env.HOOKS_TRACE_FILE; if (_t) { try { require("node:fs").appendFileSync(_t, "INIT v8-SELFID pid=" + process.pid + " selfdir=" + fileURLToPath(new URL(".", import.meta.url)) + " data=" + JSON.stringify(data) + " pkgRoot-before=" + pkgRoot + "\n"); } catch {} } }
  const d = data && data.pkgDist;
  // Sticky: initialize() can run more than once (register from both the
  // launcher and the child's NODE_OPTIONS --import). Only adopt a new
  // pkgDist when we have none yet, or when it differs from the current one.
  // Do NOT re-derive pkgRoot from a failed existsSync guard — the guard's
  // vendor file (dist/modes/interactive/interactive-mode.js) is absent in
  // bundle mode, where the chunk hooks still must serve.
  if (d && (pkgDist === null || pkgDist !== d)) {
    pkgDist = d;
    pkgRoot = dirname(d); // strip the trailing dist/
  }
  dockedActive = !(data && data.docked === false);
  { const _t = process.env.HOOKS_TRACE_FILE; if (_t) { try { import("node:fs").then(f=>f.appendFileSync(_t, "INIT after pkgRoot=" + pkgRoot + "\n")); } catch {} } }
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
  { const _t = process.env.HOOKS_TRACE_FILE; if (_t) { try { require("node:fs").appendFileSync(_t, "LOAD ENTRY pid=" + process.pid + " url=" + url + " dockedActive=" + dockedActive + " pkgRoot=" + pkgRoot + "\n"); } catch (e) { import("node:fs").then(f=>f.appendFileSync(_t, "SYNC-REQ-ERR " + (e && e.message || String(e)) + "\n")); } } }
  { const _t = process.env.HOOKS_TRACE_FILE; if (_t) { try { require("node:fs").appendFileSync(_t, "LOAD " + url + (url.includes("chunk-44DLKSKE") ? " *** CHUNK MATCH-CHECK pkgRoot=" + (pkgRoot || "null") : "") + "\n"); } catch {} } }
  if (dockedActive) {
    { const _t = process.env.HOOKS_TRACE_FILE; if (_t && url && url.includes("chunk-44DLKSKE")) { try { import("node:fs").then(f=>f.appendFileSync(_t, "CMP url=[" + url + "] pkgRoot=[" + pkgRoot + "]\n")); } catch {} } }
    for (const [rel, patchedName] of PATCHED) {
      { const _t = process.env.HOOKS_TRACE_FILE; if (_t) { try { require("node:fs").appendFileSync(_t, "LOOP TOP pid=" + process.pid + " rel=" + rel + "\n"); } catch {} } }
      const orig = pkgRoot ? pathToFileURL(join(pkgRoot, rel)).href : null;
      { const _t = process.env.HOOKS_TRACE_FILE; if (_t && url && url.includes("chunk-44DLKSKE")) { try { import("node:fs").then(f=>f.appendFileSync(_t, "CMP rel=[" + rel + "] orig=[" + orig + "] eq=" + (orig === url) + "\n")); } catch {} } }
      { const _t = process.env.HOOKS_TRACE_FILE; if (_t) { try { require("node:fs").appendFileSync(_t, "IFCHK rel=" + rel + " urlT=" + typeof url + " origT=" + typeof orig + " url=" + JSON.stringify(url) + " orig=" + JSON.stringify(orig) + " eq=" + (orig === url) + "\n"); } catch {} } }
      if (orig && url === orig) {
        { const _t = process.env.HOOKS_TRACE_FILE; if (_t) { try { require("node:fs").appendFileSync(_t, "SERVE ENTER rel=" + rel + "\n"); } catch {} } }
        const patchedPath = join(fileURLToPath(new URL(".", import.meta.url)), "patched", patchedName);
        { const _t = process.env.HOOKS_TRACE_FILE; if (_t) { try { require("node:fs").appendFileSync(_t, "SERVE path=" + patchedPath + " exists=" + existsSync(patchedPath) + "\n"); } catch {} } }
        let source;
        try {
          source = readFileSync(patchedPath, "utf-8");
        } catch (e) {
          { const _t = process.env.HOOKS_TRACE_FILE; if (_t) { try { require("node:fs").appendFileSync(_t, "SERVE READ ERROR " + e.message + "\n"); } catch {} } }
          return nextLoad(url, context);
        }
        { const _t = process.env.HOOKS_TRACE_FILE; if (_t) { try { require("node:fs").appendFileSync(_t, "SERVE RETURNING len=" + source.length + "\n"); } catch {} } }
        return { format: "module", source, shortCircuit: true };
      }
    }
  }
  return nextLoad(url, context);
}
