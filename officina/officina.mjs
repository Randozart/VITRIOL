#!/usr/bin/env node
// OFFICINA — the VITRIOL coding workshop (2026-08-31).
//
// The first-party agent harness, bundled with the engine it is coupled to:
// `vitriol officina` from any directory claims the terminal and hands you a
// programming environment tuned for local inference, speaking the engine's
// Vitriolum visual language and reporting live engine truth. Runtime is
// pi-coding-agent (MIT, pinned 0.83.0) as a LIBRARY/CLI; this entry
// binds OUR extensions + OUR theme + the VITRIOL endpoint. First-Party
// Mandate: AGENTS.md 2026-08-31; naming decision: POST-MIGRATION-PLAN.md
// (2026-08-31, owner-approved "VITRIOL Officina").
//
// Undo: `tris code` falls back to little-coder when this package is not
// installed (config flag scaffold.mode decides which path runs).

import { existsSync, readdirSync, mkdirSync, writeFileSync, readFileSync } from "node:fs";
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
// Launch env file: ~/.vitriol/officina/env (KEY=VALUE lines, # comments).
// Read BEFORE extension binding so kill switches and arming flags
// (OFFICINA_ROUTE_MODE etc.) have one canonical, inspectable home — the
// TUI SUBSYSTEMS row and AGENTS.md document the same names. File env LOSES
// to real environment variables ( ??= semantics): an explicit shell export
// always wins.
try {
  const envFile = join(homedir(), ".vitriol", "officina", "env");
  if (existsSync(envFile)) {
    for (const line of readFileSync(envFile, "utf-8").split("\n")) {
      const t = line.trim();
      if (!t || t.startsWith("#")) continue;
      const eq = t.indexOf("=");
      if (eq <= 0) continue;
      const k = t.slice(0, eq).trim();
      let v = t.slice(eq + 1).trim();
      if ((v.startsWith('"') && v.endsWith('"')) || (v.startsWith("'") && v.endsWith("'"))) v = v.slice(1, -1);
      process.env[k] ??= v;
    }
  }
} catch {
  // env file is optional; a bad one must never block launch
}
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

// Officina branding (owner request 2026-08-31): pi reads APP_NAME/APP_TITLE
// from `piConfig.name` in its own package.json (dist/config.js) — the same
// field marks the install as a fork/rebrand, which also disables pi's
// first-time-setup prompts. Set it idempotently so the workshop presents as
// "officina" everywhere pi renders its own name (logo line, terminal title,
// /help text, update strings). The CONFIG DIR stays ~/.pi — repointing
// piConfig.configDir would orphan the user's keybindings/sessions, and the
// directory is invisible plumbing anyway. npm install resets this file;
// the check below re-applies it on the next startup.
function ensureBranding() {
  try {
    const pkgPath = join(pkgRoot, "package.json");
    const pkg = JSON.parse(readFileSync(pkgPath, "utf-8"));
    if (pkg.piConfig?.name === "officina") return;
    pkg.piConfig = { ...(pkg.piConfig ?? {}), name: "officina" };
    writeFileSync(pkgPath, JSON.stringify(pkg, null, "\t") + "\n");
  } catch {
    // cosmetic; an unbranded run just says "pi" where it would say "officina"
  }
}
ensureBranding();

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
  // Module hooks don't cross process boundaries: the fullscreen path below
  // spawns pi as a child, so the hook must be re-registered there via
  // NODE_OPTIONS. runtime/register-hooks.mjs reads these env vars.
  process.env.OFFICINA_PKG_DIST = join(pkgRoot, "dist");
  const hookImport = pathToFileURL(join(here, "runtime", "register-hooks.mjs")).href;
  process.env.NODE_OPTIONS = [process.env.NODE_OPTIONS, `--import ${hookImport}`]
    .filter(Boolean)
    .join(" ");
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
  // Pin the composer to the bottom — CLASSIC MODE ONLY. In docked mode the
  // patched interactive-mode fills remaining viewport rows natively
  // ([officina P2] OfficinaSplit), so a pre-push here would just insert a
  // scrollback gap above the first frame.
  if (!docked) {
    const reserve = Math.max(2, (process.stdout.rows ?? 40) - 8);
    process.stdout.write("\n".repeat(reserve));
  }
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
