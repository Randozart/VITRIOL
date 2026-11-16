# Two asks: divider → underscores, and "run a decision model in VITRIOL"

2026-11-16 09:25 — PLAN mode. Findings + plan, nothing implemented yet.

## Item 1 — the gap-row divider: `─` → `_` so it hugs the row below

### What it is
`officina/tui/src/tui/layout.rs`, in `render_chat_and_editor`, the "gap row"
block (≈line 745). It paints one row of full-width box-drawing dashes between
the transcript and the bottom status band (indicators / ctx row / composer):

```rust
let gap = Rect { y: area.y + chat_area.height, height: 1, ..area };
let line = Line::from(vec![Span::styled(
    "─".repeat(gap.width as usize),
    Style::new().fg(theme::BORDER_DIM),
)]);
frame.render_widget(Paragraph::new(line).style(Style::new().bg(theme::BG)), gap);
```

### Findings
- `─` (U+2500, BOX DRAWINGS LIGHT HORIZONTAL) is the only divider of this
  "hug the status band" kind. Other `─` uses are unrelated:
  - `layout.rs:785` — the ctx-bar rule (right of the label, on the PANEL tone).
  - `layout.rs:1762` — the models-page header underline.
  - `markdown.rs` — table rules; not the chat divider.
- No test asserts the gap-row glyph; the render is exercised indirectly by the
  PTY smoke test (isolated HOME) and by the 287 bin tests (chat_lines etc.).
- The gap row also carries the fire-tint base and the scroll/held hint text
  (`tail_hint_area`), which is drawn *on top* of the rule row. Underscore does
  not break that: the hint is rendered after the gap row and overwrites the
  same cells.

### Plan (small, one edit + one test)
1. Change the gap-row glyph from `"─"` to `"_"` (U+005F underscore). Keep
   `BORDER_DIM` fg and `BG` bg exactly — only the character changes, so the
   fire tint and hint overlays keep working unchanged.
2. Add a regression test in `tui/mod.rs` (render-level): build an AppState,
   render into a `ratatui` backend buffer at a fixed size, assert the row at
   `area.y + chat_area.height` is `_` across its width (the first and last
   cells), and that the hint row still paints when scrolled.
3. Rebuild + install TUI (mandatory for operator-visible Rust TUI changes):
   `cargo build --release`, `cp target/release/officina ~/.local/bin/officina`,
   confirm in a fresh PTY session that the line above the composer is all `_`.
4. Commit (own stage commit).

Risks: none structural. Aesthetic only. If the owner prefers the rule to
*not* hug (i.e. stay one blank row above the composer), that's a separate
layout decision — this change keeps the current one-row gap, just swaps the
glyph.

## Item 2 — "download a decision model and run it in VITRIOL"

### What already exists (Stage B, landed 170df35)
- `officina/.pi/extensions/hang-watchdog/decision.ts` — the adapter is written
  and tested. It POSTs to a **SystemOne-style endpoint** (single `liveness`
  question: ALIVE/HUNG/UNKNOWN with `answer_confidence`,
  `action.act_probability`, `abstention`, and a `usage` coverage block).
- `/decision set <url>` wires that endpoint into the watchdog (0600 config
  file, env override).
- **No model, endpoint, or inference server exists yet** on this host. The
  watchdog currently runs the free-lane judge (cloud) or `unavailable`.

### What "run it in VITRIOL" needs
The watchdog expects an HTTP service that *is* a decision model. VITRIOL
(llama.cpp fork) can serve that: a dedicated instance on its own port,
prompted with the watchdog evidence and forced to answer the contract, with a
thin shim translating its completion into the SystemOne reply shape.

### Hardware reality (measured now)
- Two RTX 3060 12GB (the 1070 Ti from the historical notes is gone — the live
  config's `tensor_split = 27,21` is the 3060 pair). Daily driver:
  Qwen3.8-27B (IQ3_S, ~11.5 GiB weights) at c262144 + MTP n=2 — it owns both
  GPUs.
- `~/Models/gemma-4-12b-it-Q4_K_M.gguf` is already on disk (12B Q4_K_M ≈ 7.5
  GiB).
- RAM 39 GiB, /scratch 28 GB free. HF CDN stalls → ranged-resume downloader
  needed for any download.

### Constraint (owner-stated, 2026-11-16)
"No model downloads/GPU/routing changes; endpoint is operator-configured."
Downloading a model and standing up a VITRIOL instance is exactly a model +
GPU + (minor) routing change. So this is a *scope change* that needs the
owner's explicit go-ahead, with these options:

**Option A — reuse the 27B (no download).** Run a second VITRIOL instance
pointed at the same GGUF on a different port with a small context (judge
prompt is short). Problem: a second 27B instance won't fit alongside the
daily driver on the two 3060s (each ~12 GiB, the driver already fills both).
Would require time-slicing (stop the driver, run the judge, restart) — not a
24/7 decision service. **Not viable as a live endpoint.**

**Option B — Gemma 12B from disk (no download).** A second instance on
`gemma-4-12b-it-Q4_K_M.gguf` with `-ngl 99` on one GPU + small ctx. A 12B Q4
(≈7.5 GiB weights + KV) fits one 3060 12GB, but the daily 27B split 27,21
already occupies both GPUs. Same time-slice problem as A unless the judge
instance runs only when the driver is idle (governor Yielded state) — not
24/7.

**Option C — download a small dedicated judge (new model).** Pull a compact
judge (e.g. Gemma-2 2B / Phi-3 mini / Llama-3.2 1–3B class, Q4, ≈1–2 GiB) via
the ranged-resume downloader into `~/Models/`, and run it **dedicated on one
GPU** with a small ctx (4k) as a 24/7 SystemOne endpoint. The 3060 pair can
hold the 27B (split) + a 2B judge with room to spare; the judge's KV at 4k
is negligible. This is the only route that gives a *live, always-on*
decision lane without touching the daily driver. Requires: the download
(scope change), a small shim service (Node or Rust) that wraps VITRIOL's
`/completion` into the SystemOne contract, and a `/decision set` to point the
watchdog at it.

### Recommendation
- Item 1: do it — one-line change, one test, rebuild/install/commit.
- Item 2: recommend **Option C** *if* the owner lifts the "no download/GPU"
  constraint; otherwise leave the lane wired-but-unconfigured (current state,
  which already degrades cleanly to the free lane / unavailable).

### Invariants (if Item 2 proceeds)
- I1: daily-driver fingerprint unchanged — the judge instance must not
  perturb the live 27B config (separate unit/port, no shared KV mirror).
- I2: watchdog without a configured endpoint stays byte-identical (free lane)
  — pinned by the existing runtime matrix.
- I3: the decision lane is advisory only — a failed gate can never become
  ALIVE; the shim must pass `usage.coverage` truthfully.
- I4: no silent free-lane retry when the judge endpoint is configured (I4
  already pinned by `decision.test.ts`).
- I5: judge model weights/KV fit on one GPU without evicting the daily
  driver's resident weights (fingerprint diff before/after).

### Open questions for the owner
1. Lift the no-download constraint? (Option C needs a ~1–2 GiB fetch.)
2. If no download: is time-slicing acceptable (judge only runs when the
   driver is idle), or leave the lane unconfigured?
3. Which small judge model to pull (Gemma-2 2B vs Phi-3 mini vs Llama-3.2 1B)?
4. Shim language: Node (fits the existing watchdog stack) or Rust (fits
   VITRIOL's stack)?

## As landed (2026-11-16)

### Item 1 — divider glyph
`render_chat_and_editor` gap row now draws `_` instead of `─`
(`officina/tui/src/tui/layout.rs`). Colour/overlay unchanged; fire dots still
patch bg only. TUI rebuilt + installed (`~/.local/bin/officina`).

### Item 2 — decision model (shipped, heuristic-first)
- Judge weights: `~/Models/gemma-3-1b-it-Q4_K_M.gguf` (769 MiB, sha256
  `8270790f…98ebd`). It loads on its own port — a 12B/27B judge does NOT fit
  alongside the resident daily driver (both 3060s are at ~10.9 GiB).
- **Finding:** a 1B judge is not a reliable *structured* classifier at T=0
  (it loops; JSON output degenerates). The daily driver's
  `/v1/chat/completions` classifies ALIVE/HUNG correctly with a one-word
  system prompt, but is expensive/slow to use as an always-on judge.
- **As landed:** `officina/.pi/extensions/hang-watchdog/decision-shim.cjs`
  answers from deterministic heuristics first (keyword + stall-ratio:
  ALIVE/HUNG, zero latency, no GPU) and only falls back to VITRIOL
  `/v1/chat/completions` for ambiguous evidence; unreachable judge ⇒
  UNKNOWN/abstained (never a silent ALIVE). 13 tests in `shim.test.mjs`.

### Item 3 — connection errors vs full re-prefill (diagnosis)
Measured on the live engine after a restart: a TCP RST mid-prefill, a bare
close, and an abort all left the prompt cache intact (`prompt_n=4,
cache_n=7400`). The cache is healthy (hit ratio ≈0.89). The residual
uncached traffic comes from the LCP prefix cache's fingerprint (FNV-1a over
count + first 64 + last 64 tokens), from session switches (cold cache), and
from long cold prompts — NOT from connection errors. No code change made.

### Item 4 — status band flash + question panel z-order (2026-11-16)
- `AppState::stabilize_band_rows` adds one-frame hysteresis to the status
  band height (indicator rows + ctx row). A datum that exists for a single
  frame can no longer shift the transcript. Band surface painted first so a
  reserved row reads as panel tone.
- The context-fill row now survives `reset_for_rehydrate` (recovery respawns
  the same conversation), so it cannot appear/disappear across a heal.
- `render_question_panel` moved to the end of the chat screen: the ask menu
  used to be painted before the transcript and composer, which drew over its
  upper border (operator: "the question popped BEHIND the text").
- Tests: `band_height_ignores_a_one_frame_candidate`,
  `band_height_adopts_a_candidate_that_persists`,
  `rehydrate_keeps_the_context_row_so_the_layout_cannot_jump`,
  `question_panel_draws_over_the_transcript`.
