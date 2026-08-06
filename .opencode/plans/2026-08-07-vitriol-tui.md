# vitriol-tui: Ratatui Ops Dashboard

> 2026-08-07 — Standalone Rust TUI for operating VITRIOL. Supersedes
> `.opencode/plans/TUI_DASHBOARD.md` (the Python/Textual design, never built).

## Goal

A terminal operations dashboard for VITRIOL: live status, GPU telemetry, log
tails, and full control (start/stop/restart services, doctor, model/profile
switch, Spagyric sweep launch). Built in **Ratatui** (Rust) — not Textual —
per explicit user preference. Aesthetic: **Zellij + btop**, dark alchemical
green + gold ("Vitriolum" theme), matching the VITRIOL logo.

## Decision History

- 2026-08-06: `TUI_DASHBOARD.md` planned a Textual (Python) TUI. Never built.
- 2026-08-07: User chose **Ratatui** over Textual ("much prefer Ratatui").
- 2026-08-07: Scope = "everything you would want to do with VITRIOL in the
  interface" — full ops dashboard, not just status.
- 2026-08-07: Aesthetic = Zellij (colored nested borders) + btop (block-fill
  gauges, sparklines, process table).
- 2026-08-07: Theme palette derived from **Alka Officina** dark theme
  (`Desktop/Projects/alka-lang/vscode/themes/officina-dark.json`) — itself
  alchemy-inspired — re-centered on VITRIOL logo green + gold. Red/brass
  rejected.

## "Vitriolum" Theme Spec

Base from Alka Officina (GitHub-dark family); signature colors = VITRIOL
green + gold.

| Role | Color | Source |
|---|---|---|
| background | `#0D1117` | Officina `editor.background` |
| panel | `#161B22` | Officina `editor.lineHighlightBackground` |
| border dim | `#21262D` | Officina `statusBar.background` |
| primary (VITRIOL green) | `#39FF14` | Officina "Safety" — borders, headers, gauges |
| gold (VITRIOL gold) | `#FFD700` | Officina "Sovereignty" — titles, active accents |
| nominal / running | `#39FF14` | green |
| active / streaming (decode) | `#00FFFF` | Officina "Solvent" — vitriol is the solvent |
| idle / wait | `#FFD700` | gold |
| warn | `#FF5F1F` | Officina "Antidote" |
| down / critical | `#FF4444` | Officina "Substrate" |
| text | `#E0E0E0` | Officina foreground |
| muted | `#8B949E` | Officina punctuation |

**Semantics (alchemical):** solvent (cyan) = process flowing; gold = work
completed (nominal); green = living fire (healthy); red = substrate (crash).

**Service glyphs** (alchemical symbols, U+1F700+, render in most terminal
fonts; fallback to plain text label if glyphs render as tofu):

| Service | Glyph | Element | Meaning |
|---|---|---|---|
| gen | 🜂 | fire | the forge |
| hermetis | 🜄 | water | sealed flask of memory |
| embed | 🜁 | air | spirit / meaning |
| GPU | 🜃 | earth | silicon matter |

## Architecture

**Crate:** standalone `vitriol-tui/` at the VITRIOL repo root. No coupling to
`vitriol-calibrate` or `vitriol-daemon` (user decision — keep separate).
Rust 1.94.

**Dependencies:**

```toml
[dependencies]
ratatui = "0.29"
crossterm = "0.28"
ureq = "2"          # blocking HTTP — no async runtime needed
serde = { version = "1", features = ["derive"] }
serde_json = "1"
ctrlc = "3"
```

**Threads:**

- **Main** — ratatui event loop: crossterm poll + 2 s tick. Draws from the
  latest snapshot.
- **Data poller** — background thread: HTTP health/stats + `nvidia-smi` + log
  tails → `mpsc` snapshots. Keeps a `VecDeque` of decode-t/s samples (ring,
  ~120 entries) for the sparkline.
- **Control executor** — actions spawn the existing scripts as subprocesses
  (see Controls); status streamed back to the UI via channel.

**Control mechanism:** shell out to the tested bash logic — reuse, don't
reimplement:
- `scripts/launch_vitriol_full.sh status|stop|logs|doctor|help`
- `scripts/launch_copula.sh`
- `spagyric_sweep.py`

## Layout (Zellij + btop)

```
┌ tabs: DASHBOARD · GPU · LOGS · CONTROLS · HERMETIS ──────────────┐
│                                                                  │
│ DASHBOARD:                                                       │
│   🜂 GEN     model · ctx · reasoning · decode t/s [sparkline]    │
│   🜄 HERMETIS  health · episodes/nodes/sessions                   │
│   🜁 EMBED    health · mode                                       │
│   🜃 GPU      VRAM [██░░] · util · temp (btop gauges)             │
│                                                                  │
│ GPU:      btop-style gauges (VRAM/util/temp) + process list       │
│ LOGS:     live tail, [1/2/3] switch gen/hermetis/embed           │
│ CONTROLS: start/stop/restart · doctor · model/profile pick        │
│           · run Spagyric sweep (detached, tail its log)           │
│ HERMETIS: per-project stats · recent stores · search              │
└───────────────────────────────────────────────────────────────────┘
  [q] quit  [Tab] switch tab  [1/2/3] log pane  [r] refresh  [t] theme
```

Borders: green inactive / gold on the focused panel (Zellij feel). Gauges:
block-fill with green→gold gradient. Decode sparkline: green.

## Data Sources

| Widget | Data | Source | Interval |
|---|---|---|---|
| gen health | model, ctx, reasoning, slots | `http://127.0.0.1:8279/health`, `/v1/models` | 2 s |
| decode t/s | tokens/sec | `/health` `slot.tokens_predicted`/`predicting_ms` or log `slot print_timing` | 2 s |
| hermetis | health, episodes/nodes/sessions | `http://127.0.0.1:8090/health`, `/hermetis/stats` | 2 s |
| embed | health, mode | `http://127.0.0.1:8081/health` | 2 s |
| GPU | VRAM used/total, util, temp, clocks, processes | `nvidia-smi --query-gpu=... --format=csv,noheader` | 2 s |
| logs | live tail (last N lines) | gen/hermetis/embed log files | 2 s |
| doctor | check results | `launch_vitriol_full.sh doctor` | on demand |
| models | Spagyric profiles | `profiles/` configs + `/v1/models` | on demand |

## Controls ("everything you'd want to do")

- start / stop / restart each service (gen, hermetis, embed) + the whole stack
- `vitriol doctor` — run checks, show live results
- model/profile switch — pick a Spagyric profile (deepseek / mellum2) or custom
  `--model/--ngl/--ctx/--parallel`, then restart gen with those knobs
- launch a Spagyric sweep (detached) + tail its log
- log pane switching, manual refresh, theme toggle

## CLI Wiring

- `vitriol tui` → dispatches to `vitriol-tui`
- auto `cargo build --release` on first `tui` if the binary is missing
- plain subcommands (status/logs/doctor/stop) stay for scripting

## Implementation Phases

### V1 — skeleton + dashboard
- `vitriol-tui/` crate, ratatui event loop, theme module (`theme.rs` with the
  Vitriolum palette)
- DASHBOARD tab: gen/hermetis/embed health + GPU gauges, decode sparkline
- data poller (HTTP + nvidia-smi), mpsc snapshots

### V2 — GPU + LOGS
- GPU tab: btop-style gauges, clocks, process table
- LOGS tab: live tail, [1/2/3] source switch

### V3 — CONTROLS
- start/stop/restart (shell out to scripts), doctor with live results
- model/profile switch + gen restart

### V4 — sweep + HERMETIS
- Spagyric sweep launcher + detached log tail
- HERMETIS tab: per-project stats, recent stores, search

### V5 — wiring + docs
- `vitriol tui` subcommand + auto-build
- `docs/CURRENT_ARCHITECTURE.md` update, this plan's results section
- commit

## Risks / Trade-offs

- **Tofu glyphs:** alchemical symbols may not render on all terminals —
  fallback to plain labels (verify on the target terminal).
- **Blocking HTTP (ureq):** 2 s poll in a thread; an unresponsive server must
  not stall the UI — poller isolates each request with its own timeout.
- **Script shell-out:** launch logic stays in bash (tested); TUI is a caller.
  Error reporting must surface stderr cleanly.
- **Context budget:** live log tails can be large — cap displayed lines.

## Baseline

No TUI exists today (Textual plan never built). Baseline for this work:
manual `vitriol status` / `vitriol logs` / `nvidia-smi` / curl for all data.
No performance baseline to preserve; the dashboard is additive.

## Documentation Requirements

- Provenance: this is a new user-owned tool; no third-party code copied.
  Ratatui/crossterm/ureq are standard permissive crates (MIT/Apache-2.0).
  Alka Officina palette is user-owned (alka-lang), borrowed with attribution
  in `theme.rs`.
- Doc comments on every function/struct (AGENTS.md §5.2).
- Flat control flow, ≤15 cyclomatic, ≤6 params, ≤100-line functions.

## Results

<!-- living section: fill as phases land -->
- **V1 — DONE (2026-08-07)**: `vitriol-tui/` crate ships. DASHBOARD tab only:
  banner + GEN/HERMETIS/EMBED cards + GPU card (btop gauges, VRAM/UTIL/temp/
  power/processes) + DECODE sparkline. Poller thread (2 s, ureq, per-request
  3 s timeout) + `nvidia-smi` parsing + gen-log `eval time` decode parser
  (anchored on `eval time =` so prompt lines never count). Vitriolum theme,
  alchemical glyphs 🜂🜄🜁🜃 verified rendering in tmux. Decode t/s via log
  because gen `/health` returns only `{"status":"ok"}` (llama.cpp
  `server-context.cpp:3700`). Live check: gen correctly down (GPU blocked by
  avatar), hermetis+embed up, GTX 1070 Ti VRAM/util/temp live.
  Verification: 8 unit tests, clippy `-D warnings` clean, fmt clean, Praetor
  validate PASS, release binary 3.6 MB. pty smoke test clean.
- **V2 — DONE (2026-08-07)**: GPU + LOGS tabs. GPU tab = btop gauges (VRAM/
  UTIL/TEMP/SM CLK/MEM CLK/POWER) + full process table; clocks + power.limit
  added to the nvidia-smi query. LOGS tab = incremental log tails (byte-offset
  ring, truncation-reset) for gen/hermetis/embed with [1/2/3] source switch +
  ANSI stripping. Tab bar (Tab/BackTab cycles, active underlined). Verified in
  tmux: GPU gauges live (0.70/8.00 GiB 9%, 822 MHz, 35W/180W), LOGS shows the
  real gen log — live evidence of the current state (n_ctx=32768, flash-attn,
  then `cudaMalloc 448.00 MiB failed: out of memory` — avatar still holds the
  GPU).   Gates: 10 unit tests (env tests serialized via Mutex — they raced
  otherwise), clippy clean, fmt clean, Praetor PASS.
- **V3 — DONE (2026-08-07)**: CONTROLS tab. Actions: start/stop/restart stack,
  doctor, and one "load profile" entry per discovered profile. Process control
  shells out to `scripts/launch_vitriol_full.sh` (`--no-setup`, `stop`,
  `doctor`); profile load runs `vitriol config load <name>` (the config CLI —
  syncs `~/.vitriol` + auto-installs bundled) then stops and relaunches with
  the profile's parsed knobs (`--model/--ngl/--ctx/--threads/--parallel`).
  Profiles discovered from repo `profiles/` (bundled) + `~/.vitriol/profiles`
  (installed, shadowing), INI parsed section-aware. Executor: background
  thread, sequential steps, streamed stdout+stderr lines, [x] abort kills the
  child, nonzero exit marks the action failed. Verified live in tmux: doctor
  streamed (PASS binary/model/ldd/RUNPATH/port/disk, FAIL cap_ipc_lock → exit
  1 → "✗ failed: run doctor"). Gates: 14 tests, clippy clean, Praetor PASS.
- **V4 — DONE (2026-08-07)**: HERMETIS tab + Spagyric sweep launcher. HERMETIS
  tab = stats (episodes/nodes/sessions), RECENT STORES (new
  `GET /hermetis/recent` endpoint in `libvitriol/hermetis_server.py`, polled
  into the snapshot), and SEARCH (type a query, Enter runs one-shot
  `/hermetis/search` POST on a background thread; hits show score/kind/source/
  snippet). Sweep: new `sweep: <profile>` CONTROLS action runs
  `spagyric_sweep.py --model/--ngl/--ctx/--output /tmp/opencode/sweep_<name>_<ts>.csv`
  streamed into the CONTROL LOG (no-op echo when a profile lacks `model.path`).
  Project-id fix: `default_project_id` now sanitizes the FULL cwd path
  (`/`→`_`, 120 cap) matching hermetis `_project_id` — the TUI reads the real
  project (VITRIOL: 71 episodes / 42 sessions), not an empty basename project.
  Search timeout raised 10 s → 30 s after measuring retrieval at ~15 s on the
  CPU bge server. Verified live: search "gpu" returned `[0.81] episode ↳
  hop1_direct` with real snippets. NOTE: `/hermetis/recent` activates on the
  next hermetis restart (running server predates the endpoint; panel degrades
  to "no episodes yet" meanwhile). Gates: 16 tests, clippy clean, Praetor PASS.
- **V5 — DONE (2026-08-07)**: `vitriol tui` subcommand wired into
  `scripts/vitriol` (replaced the old Textual handler; builds the Ratatui TUI
  on first use, `exec`s it with `VITRIOL_REPO` exported so repo detection
  works from any cwd). Old dead `libvitriol/vitriol-tui.py` removed.
  `docs/CURRENT_ARCHITECTURE.md` gains §10 (dashboard spec + tab table) and
  repo-map entries; headings renumbered. Verified: `./scripts/vitriol tui`
  launches the dashboard in tmux with the correct project id.
  **All phases V1–V5 complete.**
