# vitriol-tui Spring Cleaning — 2026-09-01

## Status: DONE (2026-09-01) — 151 tests pass, release build clean
## Date: 2026-09-01

## Principle (owner directive)

**Disable, never delete.** Disabled tabs (Hermetis, Rebis, Officina, Gpu)
stay compiled and out of `Tab::ALL` — that pattern is correct and stays.
Retired-system references (REBIS, Hermetis-now-removed) are gated behind
env kill switches (Rule 15 convention) with `// RETIRED 2026-09-01: …`
comments at every site. Setting `VITRIOL_TUI_ENABLE_REBIS=1` (and
`VITRIOL_TUI_ENABLE_HERMETIS=1`) restores the retired surfaces wholesale.

## Ground truth (verified 2026-09-01)

- ONE inference engine: llama-server :8279, Qwen3.8-27B-Q3_K_M "Lapis
  Occultus", `--parallel 2` (slots 0/1, n_ctx 81920 each), `-ts 22,14`
  (AGENTS.md table says 26,10/27,9 — stale vs the 2026-08-31 Q3_K_M
  retarget), systemd-managed (`vitriol-server.service`), single log
  `/tmp/opencode/vitriol_gen.log`.
- Hermetis: REMOVED by owner (was :7980). ascensusd :8283 out of TUI scope.
- REBIS gateway (Sol/Luna/Mercury, :8247/:8280, /tmp/{mellum,shim,
  rebis-supervise}.log): retired, ports dead, logs absent.
- 6 of 7 TUI log sources point at nonexistent files; only Gen resolves.

## A. Disable-with-comment (REBIS + Hermetis era)

| Site | Change |
|---|---|
| `poller.rs poll_rebis` | early-return behind `VITRIOL_TUI_ENABLE_REBIS` gate; comment |
| `poller.rs poll_hermetis`/`poll_embed` | early-return behind `VITRIOL_TUI_ENABLE_HERMETIS` gate; comment (Hermetis removed by owner) |
| `app.rs LOG_ORDER` | visible order = Gen, Slots (new); Luna/Mercury/Supervise/Embed/Hermetis/Traffic gated behind env flags, variants + tails kept |
| `control.rs Action::all()` | visible list drops LaunchRebis/StopRebis (gated, kept) and retires Tria-Prima Start/Stop/Restart for systemd actions (B.1) |
| `ui.rs:723-726` Logs liveness chips | gate on the same flags |
| `ui.rs footer hint` | `:8279/:7980/:4779` → `:8279 gen` |
| `app.rs tab_all_matches_labels` test | fix: 7 tabs, last = Guide; comment on de-tab history |

## B. Fix stale-live content (correction, not deletion)

| Site | Change |
|---|---|
| `control.rs` Start/Stop/Restart/Doctor | target live path: `systemctl --user {start,stop,restart} vitriol-server.service`; Doctor stays `scripts/vitriol` doctor if present else `serve --help` probe |
| `app.rs SWEEP_GPU_OPTS` | "GPU0 — RTX 3060 (12 GiB)" / "GPU1 — GTX 1070 Ti (8 GiB)" |
| `app.rs SWEEP_TS_PRESETS` | `22,14` (default/live) `26,10` `27,9` |
| `app.rs` sweep csv | `/tmp/rebis-sweep.csv` → `/tmp/vitriol-sweep.csv` |
| `subsystems.rs` phantom keys | verify against `~/.vitriol/config`; disable-with-comment phantom rows |

## C. Observability additions

1. **Slots panel** (Dashboard, DECODE card extended):
   - keep per-slot `n_ctx` in `parse_slots` (currently dropped)
   - per-slot: busy, decoded/remain progress, **context-fill bar** (filled
     vs window — AGENTS "window ≠ depth"), prompt-eval phase
     (`is_processing && n_decoded == 0`)
   - **slot transition history**: timestamped list of state changes
     (poller detects transitions, App keeps ring buffer)
2. **Logs tab**: new `Slots` pseudo-source rendering the transition
   history; missing-file chips render dim + "absent"
3. **GEN card**: `n_parallel` from `/slots` len; totals row
   (prompt_tokens_total, tokens_predicted_total, lifetime t/s from
   predicted_seconds — parsed today, rendered nowhere); `top_ops` behind
   verbose toggle
4. Footer occupancy stays `slots busy/total`

## Bonus doc fix

AGENTS.md qwen38 table: add note that live Q3_K_M retarget config runs
`-ts 22,14` (2026-08-31), distinct from the 27,9/26,10 profiles.

## Verification

- `cargo build --release && cargo test` in vitriol-tui (fix broken test)
- Eyeball: 7 header tabs; Logs = Gen + Slots chips, absent markers;
  Controls lists live actions; Dashboard slots show ctx fill
- `VITRIOL_TUI_ENABLE_REBIS=1` restores Rebis polling/chips/actions
- No outer-repo code changes; commit on main

## Results (2026-09-01)

**A. Disable-with-comment** — all landed:
- `poller.rs`: `rebis_enabled()` / `hermetis_enabled()` env gates;
  `poll_rebis` / `poll_hermetis` / `poll_embed` skipped (defaults published)
  unless re-enabled. Log tails kept (cheap local stats; restore is instant).
- `LogSource::log_order()` (was const LOG_ORDER): visible chips = GEN +
  SLOTS; retired sources gated behind the two env flags, variants intact.
- `control.rs`: `Action::rebis_enabled()` gate — default list is the five
  fixed actions; LaunchRebis/StopRebis hidden, steps retained.
- Footer unreachable-hint: `:8279` only.
- Broken `tab_all_matches_labels` test fixed (7 tabs, Guide last) with the
  de-tab history comment.

**B. Stale-live corrections**:
- Controls Start/Stop/Restart/Doctor → systemd `vitriol-server.service`
  (`systemctl --user …`); a selected profile applies via
  `vitriol config load <name>` before start/restart. Doctor adds the
  launcher's own `doctor` step. Old flags-only machinery (`launch_args`)
  kept `#[allow(dead_code)]` with a RETIRED comment.
- Sweep: GPU labels → RTX 3060 / GTX 1070 Ti; `SWEEP_TS_PRESETS`
  ["22,14","26,10","27,9"] with 22,14 as the split default (was "3,1");
  csv → `/tmp/vitriol-sweep.csv`.
- subsystems.rs: verified against live `~/.vitriol/config` — `engine`,
  `model.expert_count`, `vitriol.mode` all real; only `[spagyric] profile`
  unconfigured (legitimately renders Unknown until set). No code change.
- LOGS number keys: 1=GEN 2=SLOTS (3-8 still reach the retired sources for
  env-gated restore); footer hint updated.

**C. Observability additions**:
- `SlotSnapshot.n_ctx` parsed (was dropped) + `is_prompt_eval()`.
- GEN card: per-slot lines with context fill (`slot 0 idle · ctx 0/81k`) +
  lifetime totals row (prompt/gen k-tokens, lifetime t/s) from /metrics —
  parsed since day one, never rendered before. `n_parallel` now derived
  from /slots length.
- DECODE card: prompt-eval slots show a phase label instead of a 0% bar.
- SLOTS pseudo-log-source: poller-derived transition history
  (acquired/released/new task + prompt-eval + token counts), 120-line
  ring, diffed in `App::observe_slots` on every snapshot.
- Chips for missing files render dim `·absent` instead of silently empty.

**Test fixes** (all four failures were stale assertions / pre-existing):
- `tab_all_matches_labels` (10 tabs / Rebis) → 7 / Guide.
- control: Start steps (launch-script flags) → config-load + unit start;
  action list 7 (incl LaunchRebis) → 5 gated.
- subsystems `service_rows_reflect_port_trio`: expected 7 → 5 (HERMETIS/
  EMBED rows were retired earlier without updating the test).
- theme `lerp_color_endpoints_and_mid`: PRE-EXISTING breakage on committed
  code (palette retuned, test constant stale) — expected midpoint updated
  to (0x20,0xDC,0xA8) for #3fb950→#00ffff.

**AGENTS.md**: added the LIVE CONFIG note — `~/.vitriol/config` runs
`-ts 22,14` (2026-08-31 retarget); the 26,10/27,9 splits are profile
history, not the current operating point.

Follow-ups noted, not done (scope discipline): real ts preset cycling in
the Sweep UI (presets exist as a constant; the selector is a larger UI
change), Hermetis journald tailing (moot — service removed).
