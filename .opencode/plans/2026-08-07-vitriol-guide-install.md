# vitriol install + TUI launch semantics + config editing + GUIDE tab

Date: 2026-08-07.

## 1. Goal

Four usability features so VITRIOL is self-installing and self-explanatory:

1. **Install script** — put VITRIOL on PATH (your machine + an easy guide for others).
2. **No auto-launch TUI** — launching the TUI keeps the interface up without
   starting the stack (start only explicitly from CONTROLS, or headless/regular
   CLI mode).
3. **Config editing in the TUI** — create, change, remove config files/profiles
   from inside the native Ratatui app (not only shell-out).
4. **GUIDE tab** — explain *everything* about VITRIOL: settings, sweep, provenance,
   papers, and code inspection.

## 1b. Alchemical relical metaphysics (Tria Prima) + port renumber

The three live processes map 1:1 onto the **three alchemical principles
(Tria Prima)**. Each port encodes an atomic transmutation as `<X>→<Y>` (element
atomic numbers), rhyming with the main gen port's original *lead→gold* joke.

| Port | Process | Principle | Glyph | Encoding | Reads as |
|------|---------|-----------|-------|----------|----------|
| **8279** | gen | Sulfur | 🜍 | 82→79 (Pb→Au) | *lead→gold* — the Opus (unchanged) |
| **7980** | hermetis | Mercury | ☿ | 79→80 (Au→Hg) | *gold returns to the mercurial flux* — the mind that re-dissolves and holds gold |
| **4779** | embed | Salt | 🜔 | 47→79 (Ag→Au) | *silver→gold* — a second Opus, the reflective body refined (essence of text) |

Non-canonical (deliberately avoids the common dev port 8080). Story: **salt(4779)
refines → mercury(7980) fluxes/records → sulfur(8279) completes the gold.**

Alchemically themed (non-port) layers — **Spagyric** (autotuner), **Chimera**
(dual-backend MoE routing), **Ascensus** (cloud escalation), **Copula** (the
OpenCode bond) — fill the SUBSYSTEMS tab, not the port scheme.

### 1b.1 SUBSYSTEMS tab (new)
Diagnostic + system-wide config per logical layer: glyph row + status dot + live
count; Enter expands inline. Subsumes listing for: Spagyric (sweep controller,
frozen profiles), Chimera (--chimera-mode, layer split, mtp), Ascensus (GEMINI_*
env, escalation count/state [ascensus] episodes), Copula (COPULA_ENABLED/
AUTO_CONTEXT, URLs, ingest counters), and the port trio (gen/hermetis/embed).

### 1b.2 Tabs: CONFIG → **PROFILES**, plus keep CONFIG distinct
Per user decision: **Profiles tab** = system-wide config/profile editing (create/
rename/delete, form-style + raw $EDITOR); **SUBSYSTEMS tab** = diagnostic, kept
separate (not merged). Hermetis keeps its data tab; its config folds into the
system-wide config/profile editor.
   papers, and code inspection.

## 2. Requirements (from discussion)

- Installer targets: **PATH symlinks + build** (build the Rust binaries the TUI
  already auto-builds; symlink `vitriol`, `vitriol-tui` into a bin dir). It must
  not assume a destination directory — usable for the user and for other people.
- `vitriol` with **no args → TUI**; `vitriol --help` still prints help; `vitriol run`
  / `vitriol serve` / other subcommands unchanged; `vitriol tui` still works
  (idempotent to `vitriol`).
- TUI launch must NOT bring up gen/hermetis/embed on startup. Only an explicit
  CONTROLS `start stack` / `restart` / profile-load, or the standalone
  `launch_vitriol_full.sh`, starts processes.
- Config editing in-TUI: **hybrid** — form-style scrolled editing for common knobs
  PLUS a raw `$EDITOR` subprocess for full INI (per prior decision).
- GUIDE tab: render bundled `docs/*.md` + `docs/provenance/*.md` + the corpus in a
  scrolled pane; rows link out to open the repo code in `$EDITOR` and papers/web
  URLs in a browser; TUI stays a terminal app (no we fetch without browser launch).

## 3. Evidence (file:line, verified in investigation)

- TUI currently is `vitriol-tui/` (standalone Rust; ratatui+crossterm+ureq);
  `vitriol tui` auto-builds on first use then `exec`s (`scripts/vitriol:2362-2372`).
  No-arg defaults to `help` (`scripts/vitriol:1588 CMD="${1:-help}"`).
- `vitriol` is already a symlink to `scripts/vitriol` (top-level). Building on that:
  an install script can symlink `vitriol` from anywhere and also beside the release
  binary.
- CONTROLS already shells out (`vitri-tui/src/control.rs`) to
  `launch_vitriol_full.sh {start|stop|restart|doctor}` + `vitriol config load`.
- Config is INI at `~/.vitriol/config`; profiles in `~/.vitriol/profiles/<name>/`
  and bundled `profiles/<name>/` (installed shadows bundled), parsed in
  `vitri-tui/src/profile.rs`. `config_set/show/reset` already exist in bash
  (`scripts/vitriol:248-337`).
- Hermetis used edn: docs observe it never auto-starts (poller only reads);
  the poller/shell-out path is the only spawn. Confirm control-spawn only fires on
  a CONTROLS Enter — currently true (Control::Start is a listed action, not auto).

## 4. Design

### 4.1 Install script `scripts/install.sh` (and `vitriol install` subcommand)
- Detect repo root, build `vitriol-tui` (and any repo Rust binaries) with
  `cargo build --release`.
- Install to a prefix dir: `~/bin` (put on PATH). Symlink `vitriol`,
  `vitriol-tui`, and future `hermetis-server`/`pymander` tools.
- Idempotent: re-run updates symlinks; no destructive moves; no sudo needed.
- Print a short "you're set" message giving the next step (`vitriol`).

### 4.2 No-arg → TUI (in `scripts/vitriol`)
- Change dispatch: `CMD="${1:-tui}"`. `--help`/`-h`/`help` still print usage.
- Keep `tui` explicit subcommand for parity and for scripts.
- No behavior change to run/serve/stop/bench/config/calibrate/setup/pymander.

### 4.3 TUI: no auto-launch
- Audit: the TUI app only shells out on CONTROLS actions (verified). Ensure no
  `Start` is queued at startup or on first tick. Add an explicit guard/test: `App`
  lacks any auto-spawn and `poller` is read-only.
- The TUI is pure monitoring until the user runs a CONTROLS action.

### 4.3b Port renumber + centralize (Tria Prima)
- New constants module = single source of truth for ports (env-overridable):
  `GEN_PORT=8279`, `HERM_PORT=7980`, `EMBED_PORT=4779`.
- Migrate all scattered literals (`8090`, `8081`, `8279`) to the module:
  `launch_vitriol_full.sh:26-27`, `launch_copula.sh`, `hermetis_server.py:33`
  `DEFAULT_PORT`, pymander client URLs, `plugins/copula.ts:12` (repo AND
  `~/.config/opencode/plugins/copula.ts`), `docs/{CURRENT_ARCHITECTURE,
  CONFIG_REFERENCE,OPENCODE_SETUP,copula}.md`, env example, TUI `config.rs`,
  tests.
- Kill stale servers; verify stack boots on new ports (8279 stays).

### 4.4 TUI config editor (hybrid)
- New **PROFILES** tab (renamed from CONFIG per user decision; see §1b.2).
  - Lists the active config (`~/.vitriol/config`) keys grouped by section, with a
    form-style scrolled editor for common knobs (device, model.path/context/threads/
    ngl, server.parallel, memory.semantic_mode, etc).
  - Actions: **add** (new profile from current config → `profiles_save`), **remove**
    (delete a profile, confirm), **save-as** new name), **raw edit** (open
    `$EDITOR` on the selected config / profile file via shell-out, reuse existing
    tile stream from `control.rs`).
  - Uses existing profile discovery + INI parse; writes via a small `config` module
    (local to write-back of `~/.vitriol/config` + profiles) — reuse, don't fork the
    bash `write_config` (AGENTS DRY), but keep contract: after write, `parse_config`
    reload (`vitriol config load` or generate).
- Themes key error handling: refuse to clobber values to unknown keys; validate
  name on add/delete (safe filesystem chars); node write via temp+rename.

### 4.4b SUBSYSTEMS tab (new)
- Rows per layer (§1b.1): Spagyric, Chimera, Ascensus, Copula + the port trio.
  Glyph + status dot + live count; Enter expands inline to config knobs.
- Same hybrid edit machinery as PROFILES; diagnostics data read from Hermetis
  (`/hermetis/stats`) and process probes (poller), never auto-spawn.

### 4.5 GUIDE tab
- A `guide.rs` module: loads markdown/front-matter from
  - `docs/` (water: `CURRENT_ARCHITECTURE.md`, `hermetis.md`, `spagyric-autotuner.md`,
    `CONFIG_DEFAULTS_GUIDE.md`, `RECOMMENDED_SETTINGS.md`, `copula.md`),
  - `docs/provenance/*.md` (kimi-k3-in-c, pymander),
  - the corpus `docs/pymander/*.md`.
- Renders in a scrolled READER pane; fuzzy searchable section index. Items with
  `tags` (settings vs sweep vs provenance) filterable.
- **Actions**: `o` open the doc-derived repo source in `$EDITOR` (map docs → the
  module/file each documents), `w` open papers/URLs in browser via
  `xdg-open` if present (filtered to `http(s)`). Show license note per item from
  `provenance` header (`PROVENANCE:` line) in a footer.

## 5. Files
- `scripts/install.sh` (new), `scripts/vitriol` (dispatch no-arg → tui).
- `vitrit-tui/src/{main,app,ui}.rs` — no-arg/launch no-op, add PROFILES + GUIDE +
  SUBSYSTEMS tabs, tab registry `Tab::ALL`.
- `vitrit-tui/src/config_edit.rs` (new: INI read/write, form model), possibly
  `vitrit-tui/src/guide.rs` (new: doc loader + search index + open actions),
  `vitrit-tui/src/subsystems.rs` (new: per-layer diagnostics model).
- New ports constants module (Rust + bash + python share env contract).
- `control.rs` — reuse `spawn` stream for raw-edit + open actions.
- Tests: config round-trip, no-auto-launch invariant, no-arg mapping,
  guide load from repo, provenance header parse, port-constant centralization (no
  stray port literals).

## 6. Gates
- `cargo test` in `vitrit-tui` green; `cargo clippy --all-targets -- -D warnings`;
  `cargo fmt --check`; bash `bash -n scripts/vitriol scripts/install.sh`; praetor
  PASS on changed files.

## 7. Non-$targets
- No auto-install of deps / system packages; install is symlink+build only.
- GUIDE renderer is simple text (no true markdown engine dep beyond what we add
  to ratatui); if a doc set grows, defer.

## 8. Results
(fill as I go)