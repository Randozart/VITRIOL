#!/usr/bin/env node
// OFFICINA — the VITRIOL coding workshop (2026-08-31).
//
// The first-party agent harness, bundled with the engine it is coupled to:
// `vitriol officina` from any directory claims the terminal and hands you a
// programming environment tuned for local inference, speaking the engine's
// Vitriolum visual language and reporting live engine truth. Runtime is
// pi-coding-agent (Apache-2.0, pinned 0.83.0) as a LIBRARY/CLI; this entry
// binds OUR extensions + OUR theme + the VITRIOL endpoint. First-Party
// Mandate: AGENTS.md 2026-08-31; naming decision: POST-MIGRATION-PLAN.md
// (2026-08-31, owner-approved "VITRIOL Officina").
//
// Undo: `tris code` falls back to little-coder when this package is not
// installed (config flag scaffold.mode decides which path runs).

import { existsSync, readdirSync, mkdirSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createRequire } from "node:module";

const here = dirname(fileURLToPath(import.meta.url));
const pkgRoot = join(here, "node_modules", "@earendil-works", "pi-coding-agent");

// Opt out of pi's startup update banner (owner request 2026-08-31) and any
// other startup network ops. PI_SKIP_VERSION_CHECK=1 kills the
// "Update Available" line; PI_OFFLINE=1 would also block package/update
// checks. The workshop manages its own pins — the banner is noise.
process.env.PI_SKIP_VERSION_CHECK ??= "1";
// Session continuity: pi's picker is per-project (cwd), so prior sessions
// for THIS folder are reachable from the session manager; -c/--continue and
// -r/--resume pass through for direct resume. Advertised in the widget-free
// zone (the TUI footer) via pi's own UI — nothing bespoke to maintain.

// Resolve pi's CLI from OUR node_modules (pinned in officina/package.json).
// The package "exports" map blocks ALL subpaths (even ./package.json), so
// resolve the package root by walking node_modules directly.
const require = createRequire(pathToFileURL(join(here, "package.json")));
let cli;
try {
  const bin = require(join(pkgRoot, "package.json")).bin;
  const binPath = typeof bin === "string" ? bin : bin["pi-coding-agent"] ?? bin.pi ?? Object.values(bin)[0];
  cli = join(pkgRoot, binPath);
  if (!existsSync(cli)) throw new Error(`bin missing: ${cli}`);
} catch (e) {
  console.error("officina: cannot resolve pi-coding-agent CLI:", e.message);
  console.error("Run:  cd " + here + " && npm install");
  process.exit(1);
}

// Ensure TAB is formally unbound from editor autocomplete in the user's
// pi keybindings, so the agent-mode TAB toggle doesn't raise a conflict
// banner on every startup (owner request 2026-08-31). Merge-preserving:
// any existing keybindings.json content survives. Opt-out: set the key
// back to whatever you prefer; we only ever touch "tui.input.tab".
function ensureTabUnbound() {
  try {
    const dir = join(process.env.HOME || homedir(), ".pi", "agent");
    const p = join(dir, "keybindings.json");
    let cfg = {};
    if (existsSync(p)) {
      cfg = JSON.parse(readFileSync(p, "utf-8"));
      if (cfg["tui.input.tab"] !== undefined) return; // owner-managed, hands off
    }
    cfg["tui.input.tab"] = [];
    mkdirSync(dir, { recursive: true });
    writeFileSync(p, JSON.stringify(cfg, null, 2) + "\n");
  } catch {
    // cosmetic optimization; a missed unbind only costs a startup banner
  }
}
ensureTabUnbound();

// Bind OUR stack, then the user's own flags (pi's --extension/--theme flags
// are repeatable, so user-supplied ones ADD to ours).
const bound = [];
const extDir = join(here, ".pi", "extensions");
for (const dir of existsSync(extDir) ? readdirSync(extDir, { withFileTypes: true }) : []) {
  if (dir.isDirectory() && existsSync(join(extDir, dir.name, "index.ts"))) {
    bound.push("--extension", join(extDir, dir.name));
  }
}
const theme = join(here, "theme", "officina.json");
if (existsSync(theme)) bound.push("--theme", theme);
const model = process.env.TRIS_LC_MODEL;
if (model) bound.push("--model", model);

// Layout fork (docs/LAYOUT-FORK-2026-08-31.md): when docked, a loader hook
// serves our patched interactive-mode instead of the stock one. Must be
// registered BEFORE the pi CLI is imported.
import { register } from "node:module";
const docked = (process.env.OFFICINA_LAYOUT || "docked") !== "classic";
if (docked) {
  register("./runtime/hooks.mjs", {
    parentURL: pathToFileURL(join(here, "officina.mjs")),
    data: { pkgDist: join(pkgRoot, "dist"), docked: true },
  });
}

// pi's CLI parses process.argv itself (argv[0] is the program path it cares
// about least); rewrite it so pi sees the bound flags before the user's.
process.argv = [process.argv[0], cli, ...bound, ...process.argv.slice(2)];

// ── Full terminal claim ────────────────────────────────────────────────
// pi's inline TUI has no alt-screen mode of its own. When we're on a TTY
// we claim the screen (ANSI 1049h: enter alternate buffer, clear, home),
// run pi as a child with inherited stdio, and ALWAYS restore the terminal
// (1049l) on exit — shell scrollback comes back untouched. Opt-out:
// TRIS_NO_FULLSCREEN=1 (some multiplexers/copilots prefer inline).
if (process.stdout.isTTY && process.env.TRIS_NO_FULLSCREEN !== "1") {
  // Claim the screen AND paint the engine's background (Vitriolum BG
  // #0d1117, blue-slate near-black) via OSC 11, so the workshop has the
  // VITRIOL TUI's atmosphere even where messages have no explicit bg.
  // OSC 111 on every exit path restores the terminal's own background.
  const OFFICINA_BG = "\x1b]11;#0d1117\x07";
  const RESTORE_BG = "\x1b]111\x07";
  process.stdout.write("\x1b[?1049h" + OFFICINA_BG + "\x1b[H\x1b[2J");
  // Pin the composer to the bottom (owner request 2026-08-31): pre-push the
  // render origin down so the editor sits at the screen floor on a fresh
  // session; as content grows past the reserve, the view scrolls naturally
  // - the composer stays pinned, like Crush/OpenCode.
  const reserve = Math.max(2, (process.stdout.rows ?? 40) - 8);
  process.stdout.write("\n".repeat(reserve));
  const restore = () => process.stdout.write("\x1b[?1049l" + RESTORE_BG);
  process.on("exit", restore);
  for (const sig of ["SIGINT", "SIGTERM", "SIGHUP"]) {
    process.on(sig, () => {
      restore();
      process.exit(0);
    });
  }
  const { spawnSync } = await import("node:child_process");
  // after the argv rewrite: [execPath, cli, ...bound+user flags]
  const res = spawnSync(process.execPath, process.argv.slice(1), { stdio: "inherit" });
  restore();
  process.exit(res.status ?? 0);
}

await import(pathToFileURL(cli).href);
