# GUIDE optimization docs + markdown render + profile save/load + sweep→profile + Ascensus secrets

Date: 2026-08-07.

## 1. Goal

Five linked usability upgrades for the VITRIOL TUI:

1. **GUIDE lists curated optimization docs** (not the old `docs/` sprawl), each
   explaining one optimization: lever → config key, silicon rationale, measured
   numbers, status, undo path.
2. **Markdown rendered properly** (CommonMark via `pulldown-cmark`) into styled
   ratatui lines, like OpenCode renders it.
3. **PROFILES save/load** in the TUI (two-pane: active config + profile list).
4. **Spagyric sweep builds a profile** (winner config written to
   `~/.vitriol/profiles/<name>-swept/`).
5. **Ascensus secret management**: Gemini key + model editable in the
   SUBSYSTEMS tab, persisted to `~/.vitriol/secrets` (0600, never in repo),
   consumed by `plugins/copula.ts`.

## 2. Commit 1 — `docs/optimizations/*.md`

New curated directory; GUIDE discovers ONLY this. One doc per optimization.
Each template: lever + config key, silicon rationale (Pascal CC 6.1, PCIe 3.0
x16, no AVX2), measured result (cited), status (validated/refuted/recorded),
undo path.

| Doc | Status |
|---|---|
| decode-batch-amortization.md | validated — ubatch → `[engine] ubatch_size`, 3.6× @ R16 |
| kv-offload.md | validated — `[kv] mode` |
| kv-quantization.md | validated — `[kv] quant_mode` |
| prefill-pipelining.md | validated |
| context-size.md | validated — `[model] context` |
| threads-and-parallel.md | validated — `[model] threads`, `[server] parallel`, 2.3× @ p8 |
| weights-as-code-r2-fold.md | refuted — 92.8× packed bytes |
| iq-lut-on-pascal.md | refuted — SMEM 48 KB |
| activation-delta.md | refuted — 14.5× worse |
| input-prefolding.md | refuted |
| dead-lane-skip.md | recorded, not an autotune knob |

Sources: `.opencode/plans/2026-08-06-spagyric-*.md`, `docs/OPTIMIZATION_PLAN*`,
`docs/CONTEXT_OFFLOADING_STRATEGIES.md`, `docs/KV_QUANT_SESSION.md`,
`docs/prefill-optimization-plan.md`, `docs/spagyric-autotuner.md` §3.

## 3. Commit 2 — markdown render + GUIDE retarget

- Add `pulldown-cmark` (MIT) to `vitriol-tui/Cargo.toml`.
- New `vitriol-tui/src/markdown.rs`: `render(text, width) -> Vec<Line<'static>>`.
  Walk parser events; headings (bold green by level), strong/emphasis, inline
  code (cyan on panel), fenced code blocks (muted on panel), bullet/ordered
  lists (prefix + indent), blockquotes (`│ `), rules (`─`), links (text + URL
  muted), tables (`│` cells). Hard-wrap paragraphs to `width` so the reader's
  scroll math stays logical-line based.
- `guide.rs`: discover only `docs/optimizations/`; drop docs/ root + pymander;
  `Kind::Optimization`; add `summary` (first non-heading paragraph).
- `ui.rs` GUIDE reader: use `markdown::render`; provenance footer stays.

## 4. Commit 3 — PROFILES two-pane save/load

- `app.rs`: `profile_prompt: Option<String>` (save-name input),
  `profile_list_selection: usize`, `profile_focus` pane enum; methods:
  `profile_pane_toggle`, `profile_save_prompt`, `profile_save_commit`
  (validate `^[a-zA-Z0-9_-]+$`, write `~/.vitriol/profiles/<name>/config` via
  `config_edit::render_entries` + `meta` with name/description/created),
  `profile_load_selected` (copy `<profile>/config` → `~/.vitriol/config`
  atomic, reload `config_file`), `profile_list_move`,
  `profile_delete_selected` (confirm), `profile_reload_list`.
- `ui.rs` `render_profiles_tab`: two panes (config rows | profile list);
  keys `,`/`.` focus toggle, `s` save-as prompt, `l`/Enter load, `d` delete,
  `r` reload. No restart on load (CONTROLS handles full load+restart).

## 5. Commit 4 — sweep → profile

- `libvitriol/spagyric_sweep.py`: `--build-profile <name>` +
  `--profiles-dir` (default `~/.vitriol/profiles`). After the grid, pick
  per-knob winner among rows with `correct=PASS`: ubatch/threads → max
  `decode_tps`; parallel → max `concurrent_tps`. Write
  `<profiles>/<name>-swept/config` (+ meta): ubatch → `[engine] ubatch_size`,
  threads → `[model] threads`, parallel → `[server] parallel`, plus `[model]
  path/ngl/context` from args, header comment with measured t/s + date.
  Never clobbers (suffix). Log profile path.
- `control.rs`: `SweepAndSave(String)` action (`sweep+save: <name>`), passes
  `--build-profile <name>`; existing `sweep:` stays CSV-only.

## 6. Commit 5 — Ascensus secrets

- `config.rs`: `secrets_path()` → `~/.vitriol/secrets`.
- New `vitriol-tui/src/secrets.rs`: `Secrets { api_key, model }`;
  `load(path)`, `save(path)` (atomic + `chmod 0600`), `mask()` (`••••` + last 4,
  never prints full key). INI `[ascensus] api_key/model`.
- `subsystems.rs`: ASCENSUS row reads secrets file (env still honored);
  status `Up` when key present. Value: `key set · model <m>` vs `no key`.
- SUBSYSTEMS tab: Enter on ASCENSUS row opens inline `api_key` + `model` editor
  (form pattern), save writes `~/.vitriol/secrets`. Secret key input masked on
  display.
- `plugins/copula.ts`: ascensus reads `~/.vitriol/secrets` first, env fallback;
  sync installed `~/.config/opencode/plugins/copula.ts`.
- Safety: secrets never written under the repo; profile save/load/config
  exports never touch the file. Verify `git status` + `git grep` for key
  patterns post-commit.

## 7. Gates (each commit)

`cargo test` + `cargo clippy --all-targets -- -D warnings` + `cargo fmt --check`
+ `praetor validate --warn` on changed files. Final `cargo build --release` +
relaunch (avoid the stale-release-binary trap).

## 8. Risks

- `pulldown-cmark` fetch needs network at build time (cargo network verified).
- Sweep winner = max t/s per knob with PASS; may not be the most stable config —
  documented in the profile header, user can re-tune.
- GUIDE content authoring is the bulk (~11 docs); numbers cited from existing
  plans/docs, never invented.

## 9. Results

(fill as I go)
