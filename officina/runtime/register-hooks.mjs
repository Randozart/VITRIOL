// Loader-hook bootstrap for child processes. officina.mjs registers
// runtime/hooks.mjs in its own process, but module hooks do NOT cross
// process boundaries — and the fullscreen launcher spawns pi as a child
// with inherited stdio. This entry is injected into the child via
// NODE_OPTIONS="--import <this file>" so the docked layout fork applies
// there too (2026-08-31, PTY-proven gap: without it the child silently
// ran stock interactive-mode and docked mode never engaged).
//
// Provenance: original work, this repo (docs/LAYOUT-FORK-2026-08-31.md).
import { register } from "node:module";

const docked = (process.env.OFFICINA_LAYOUT || "docked") !== "classic";
if (docked && process.env.OFFICINA_PKG_DIST) {
  register("./hooks.mjs", {
    parentURL: import.meta.url,
    data: { pkgDist: process.env.OFFICINA_PKG_DIST, docked: true },
  });
}
