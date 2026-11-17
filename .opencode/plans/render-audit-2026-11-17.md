# Render audit — every defect found, why the tests missed it, and the plan

**Date:** 2026-11-17 18:55
**Mode:** PLAN (research only — nothing changed)
**Trigger:** owner reported two render bugs in an hour ("several UI bugs just lingering and waiting
to be found"). This is the systematic sweep. Companion doc:
`fire-base-taper-regression-2026-11-17.md`.

## 1. Findings

Severity: **A** = operator sees it today · **B** = wrong under a condition the owner hits less
often · **C** = latent trap for the next change.

| id | sev | evidence | what happens | why no test caught it |
|---|---|---|---|---|
| **FIRE-1** a.k.a. bottom-two-rows-identical | A | `fire.rs:374` `rn = (rows-1-r)/(rows-1)` + gate `rn > 0.0` | the bottom **body** row gets `rn == 0`, is exempt from the taper, and matches the rule row exactly | the test was written around it: `layout.rs:4546` "the assert must look one row higher", plus `assert_eq!(flat.last(), tapered.last())` enshrines the exemption |
| **BAND-1** phantom prompt in the band | A | `.pi/extensions/composer-predictions/index.ts:150` `placement: "belowEditor"` | the predicted continuation is drawn as a band row between the ctx row and the route row; mid-typing it reads like the operator's own text | no test asserts *what a band row may contain* |
| **CACHE-1** stale markdown frames | A | `state.rs:4088` key `(idx, text.chars().count(), width)` | a text change **with unchanged length** serves the cached render: bash `\r` progress lines, in-place token fixes, equal-length replacements | the cache is tested for correctness of a warm hit, never for same-length invalidation |
| **WIDTH-1** every width is `chars().count()` | A | `layout.rs:159,233,768,948,1227,1348,1391,1663,1915,1984`, `trunc` `:3805`, `trunc_str` `state.rs:5891`, `truncate_spans` `markdown.rs:106`; **no `unicode-width` dep** | CJK/emoji occupy 2 cells but count 1: borders misalign, text is pushed past the area, model-picker columns drift, notice/header truncation overshoots | fixtures are ASCII/border glyphs only, all of which are 1 cell wide |
| **COMPOSER-1** composer undsersizes its box | B | `layout.rs:3823` `composer_rows()` — its own doc says "word-wrapped text can need MORE rows… callers treat this as a lower bound" | a long prose prompt renders more rows than the box was given, so its last line is clipped exactly when the operator is writing it | tests feed short single-row inputs |
| **WRAP-1** transcript clips instead of wrapping | B | `layout.rs:582` `lines.into_iter().skip(start).take(visible)` painted with no `.wrap()` | any line wider than the pane (long unbroken token, URL, code line, wide-char run) is cut at the right edge with no ellipsis | `chat_lines` pre-wraps markdown at `width-6`, so the tests that matter never produce an overlong line |
| **SCROLL-1** scroll math is line-count based | C | `layout.rs:555` `max_scroll = total.saturating_sub(visible)` | correct **only because** WRAP-1 keeps 1 logical line = 1 row. Fixing WRAP-1 with `.wrap()` silently breaks scrolling (can't reach the top, drift) | no test couples the two |
| **BAND-2** notice row latches forever | B | `layout.rs:35` `notice_row_sticky \|= notice.is_some()`; only ever set; `reset_band()` (`state.rs:1732`) does not clear it | after the first notice of the process, one row stays reserved permanently — the composer is a row shorter forever, and a session switch does not reclaim it | one test asserts it *becomes* sticky, none asserts it is released |
| **HEADER-1** header arithmetic by magic number | C | `layout.rs:233` `right_w = chars+2`, `avail = mid_w - 24` | at narrow widths or with a long session label, the model chip collides with the session label; wide chars make it worse | header tests use a comfortable 120-column frame |

Also noted, not yet verified: `tint_rows` is computed from the content tail (`fire.rs`), so under
`scroll > 0` the tint may decorate rows that are not the ones on screen. Needs a measurement
before it earns a severity.

## 2. The pattern behind all of them

Three structural habits, each cheap to break and expensive to notice:

1. **Width is assumed equal to character count.** No `unicode-width` dependency exists. Every
   alignment, truncation and cell budget in the UI shares this assumption, so it is one bug with
   eleven sites rather than eleven bugs.
2. **Tests assert the current output.** FIRE-1's test skips the broken row; BAND-1 has no
   invariant; SCROLL-1's correctness is accidental. A test that documents a defect is worse than
   none, because it converts the defect into a contract.
3. **Layout arithmetic carries unnamed constants** (`- 6`, `- 24`, `+ 2`, `max(8)`, `take(3)`)
   whose interactions nobody is checking at the edges (narrow terminal, wide glyphs, 1-row band).

## 3. Plan

### Stage 1 — one width model (unblocks the rest)
Add `unicode-width` and a single helper module `tui/width.rs`:

```rust
pub fn cols(s: &str) -> usize          // display width
pub fn trunc_cols(s: &str, w: usize) -> String   // ellipsis-aware, never exceeds w
pub fn pad_cols(s: &str, w: usize) -> String
```

Replace every `chars().count()` used as geometry (the eleven sites above) and the three ad-hoc
truncators. Purely mechanical, high value: it fixes CJK/emoji misalignment *and* removes the
guesswork from the remaining stages.
Tests: table-driven fixtures with CJK, emoji, combining marks, braille, box drawing — assert
`cols()` and that `trunc_cols` never exceeds the budget (property, generated over a char pool).

### Stage 2 — FIRE-1 (owner-visible, tiny)
`rn` from `from_base` (`0.5/rows` at the bottom body row) plus a base inset
`taper_scale = 1 - taper*(0.35 + 0.65*sqrt(rn))` so the first step is visible at 2…6 rows.
Rewrite the test to assert `width(rule) > width(first body row)` and prove the negative control
(restoring the old `rn`) fails. Details in the companion doc.

### Stage 3 — BAND-1 (owner-visible, small)
The predictions widget must not be mistakable for input. Cheapest correct fix, in order:
1. never render a prediction that is a prefix of, or identical to, the current composer text;
2. prefix the row with `≈ ` and render it muted;
3. where the ghost already exists (`state.prompt_ghost`), prefer the ghost and drop the widget.
Test: a render-invariant asserting the band never contains a verbatim copy of `state.input`.

### Stage 4 — CACHE-1 (one line, real staleness)
Key on content, not length: `(idx, hash64(text), width)` — or bump a per-entry generation counter
when the entry mutates. Test: render, mutate the text to an equal-length string, render again,
assert the output changed (negative control: same text ⇒ cache hit, same render).

### Stage 5 — COMPOSER-1 + WRAP-1/SCROLL-1 (do the pair together)
- `composer_rows` counts visual rows with the same wrap rule the widget uses (word wrap), or the
  composer paragraph switches to hard character wrapping so the count is exact — pick one and
  state it in a comment; today it is "hard wrap, but the widget word-wraps".
- Transcript: either keep clipping **and** add an ellipsis on overlong lines, or enable `.wrap()`
  **and** switch scrolling to visual rows (ratatui can report it via `Paragraph::line_count`).
  The second is the right fix, the first is the safe one; decide explicitly rather than by default.

### Stage 6 — BAND-2 + HEADER-1 (hygiene)
- Release the notice row when the notice is gone for N settled frames (reuse the `Settled` dwell
  used by the band) instead of latching forever; clear it in `reset_band()`.
- Header: space the three cells with `Layout` constraints (`Min`, `Length`) instead of `- 24`,
  and truncate with `trunc_cols`.

### Stage 7 — `render-invariants` module (the durable part)
A test module that asserts *semantic* properties of a composed frame, each with a negative
control (flip the offending condition behind a `cfg(test)` knob and watch it fail):

1. band rows never contain the composer's text (BAND-1)
2. the bottom two fire rows differ, for every row count (FIRE-1)
3. the ctx row does not move when an indicator appears/disappears (band settling)
4. no drawn row exceeds the frame width (WIDTH-1)
5. every truncated line ends with `…` when it was cut (WRAP-1)
6. the notice strip is never on the hints row; the hints row contains only keybindings
7. rendered output is unchanged when the frame is re-rendered with no state change (idempotence —
   catches stale caches like CACHE-1 for free)

## 4. Acceptance

- All Stage 1–2 tests fail on the pre-fix code (negative controls recorded in the commit message).
- Live check at 80 and 200 columns, levels that produce 2/4/6 fire rows, `taper` 0/0.55/1.0, and a
  prompt containing CJK + emoji + a 400-character URL.
- No visual regressions in: composer wrap, scroll to top, band stability, notice dismissal.

## 5. Ordering rationale

Stage 1 first because it is the shared root cause and mechanical (low risk, wide effect). FIRE-1
and BAND-1 next because they are the operator's two complaints and are small. CACHE-1 is a
one-liner with a real user-visible symptom. The WRAP/SCROLL pair must be decided together — doing
either alone converts a clipping bug into a scrolling bug. The invariants module last, so it can
encode everything learned here.

## 6. Shipped (2026-11-17, BUILD — all seven stages)

| stage | commit | outcome |
|---|---|---|
| S1 width model | `1f3fc4a`… (see log) | `tui/width.rs` (`cols`, `trunc_cols`, `pad_cols`, `pad_cols_right`, `hard_wrap_rows`), `unicode-width` 0.2, every geometry `chars().count()` replaced; 6 tests |
| S2 FIRE-1 | fire commit | `rn` derived from `from_base` (every body row tapers) + `BASE_INSET = 0.18`; `tint_rows` aligned to the same fraction; taper test rewritten with a same-row differential and a verified negative control; `flame_meets_its_line` now asserts body ⊆ rule (the rule may be the wider pedestal) + rule continuity |
| S3 BAND-1 | band commit | `band_echoes_draft()` refuses any band row that equals/prefixes the draft — applied to every extension widget; screen-differential test with a negative control (band force-settled, since `BAND_SETTLE` hides it for 700 ms) |
| S4 CACHE-1 | cache commit | `entry_rev` hashes TEXT (was `t.len()`), `md_cache` keyed by an FNV-1a content fingerprint (was a char count); test asserts a same-length change changes both the key and the render |
| S5 COMPOSER-1 / WRAP-1 / SCROLL-1 | composer commit | composer height comes from the renderer (`Paragraph::line_count`, hence ratatui's `unstable-rendered-line-info`); overlong transcript lines elide with `…` via `elide_line`, preserving the 1-line=1-row invariant that scrolling depends on (stated at both sites) |
| S6 BAND-2 / HEADER-1 | header commit | notice row is `Settled` (reserved, then released; dropped by `reset_band`) instead of latched forever; header laid out by constraint (session ≤ ⅓ row, chip gets the remainder) with cell widths |
| S7 render invariants | `adee09c` | four properties: buffer bounds (with the stringify trap documented), ctx-row stability, render idempotence, hints row purity |

Final state: **344 cargo tests**, `cargo fmt --check` clean, **zero warnings**, clippy clean, installed `a30f65f2`.

### Notable discoveries during implementation

1. **The first I4 test was wrong, and it taught the lesson again.** It asserted
   display width on `screen()`'s string, which concatenates one symbol per buffer
   cell — so every double-width glyph inflated the dump by one column and a
   40-cell row "measured" 42. Same shape as FIRE-1's test: the assertion described
   the *tool*, not the *behaviour*. Fixed by asserting on the buffer.
2. **CACHE-1 had two layers.** Fixing the `md_cache` key was not enough: the
   per-entry cache above it (`entry_rev`) hashed `t.len()`. The test caught it
   because it asserted the render changed, not merely the key.
3. **The composer estimate could not be made correct by refinement** — only by
   asking the widget. That is why an unstable ratatui feature is enabled, and it
   is documented next to the dependency.
4. **Two existing composer tests were pinned to the old estimate** (exact row
   counts). They are now relational (`rows == renderer's line_count`, narrower ⇒
   more rows), which is the property they were always trying to express.

### Still open (not defects, decisions)

- `tint_rows` under `scroll > 0`: whether the text tint decorates the rows on
  screen or the rows at the content tail. Needs a measurement, not a guess.
- Enabling `.wrap()` on the transcript would let long lines flow instead of
  eliding; that requires switching scroll maths to visual rows (`SCROLL-1`), which
  is a larger change than this audit's scope.
