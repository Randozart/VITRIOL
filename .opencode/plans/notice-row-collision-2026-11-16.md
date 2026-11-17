# "next:" text collides on the bottom row and flickers (2026-11-16, PLAN)

Owner: *"the 'next: ' text leaks into the bottom section and conflicts with
something else on that line, resulting in flickering."*

## What I traced

Two different producers use the string `next: `, and they land in two different
places:

| producer | string | where it lands |
|---|---|---|
| `composer-predictions/predict.ts:61` | `▸ next: <clipped task> · <hints>` | a **belowEditor widget** → the status band, one row above the ctx row |
| `resilient-default/index.ts:956` (`/nimbus`) | `nimbus · ctx 12k · 3 lane(s) · next: <lane>` + a **multi-line table** | `ctx.ui.notify(...)` → the RPC `notify` → **`state.notice`** |

And `state.notice` is rendered by `render_footer` (`layout.rs:1634`) — appended to
the **keybinding hints row**:

```rust
} else if let Some((msg, _)) = &state.notice {
    spans.push(Span::styled(format!("  ·  {}", trunc(msg, 40)), theme::gold_muted()));
}
```

That is the collision, and it explains all three symptoms:

1. **"leaks into the bottom section"** — a notice (including `/nimbus`'s
   `next: …`) is appended to the row that carries `enter send · → complete ·
   esc dissolve · …`, so unrelated text appears in the keybinding strip.
2. **"conflicts with something else on that line"** — the hints already fill most
   of a normal width; the notice is pushed past the edge and clipped mid-word,
   or squeezes against `^v tools`/`no telemetry`.
3. **"flickering"** — `state.notice` is only ever **set, never cleared** (grep:
   no `notice = None` anywhere). Every tool/engine/watchdog/route event
   overwrites it, so the tail of that row re-renders on each event — a row that
   shimmers for the whole session and never expires.

Secondary finding (also real, also mine to own): `trunc(msg, 40)` on a
**multi-line** notice (`/nimbus` sends `head \n table`) keeps the leading 40
chars of the first line only — the table is silently dropped, and if the first
line is empty the footer shows a bare ` · `.

## Why the band hypothesis is weaker (but still checked)

The `▸ next:` widget goes through the settled band (`stabilize_band_parts`), so
its row appears/disappears at most once per `BAND_SETTLE` (700 ms) — a pop, not
a shimmer. It can still *look* like a conflict when the ctx row and the prediction
row swap during the settle window (settled `ind_ctx` split vs live lines), which
is why the fix below also pins the band's per-key slots.

## The fix

### 1. Notices get their own row (and expire)

- `AppState` gains `notice_shown_at: Option<Instant>`; `apply_notify` /
  `local_dispatch` set it. A notice older than `NOTICE_TTL` (proposed 6 s for
  `info`, 12 s for `warning`/`error`, never auto-hidden for `error`) stops
  rendering.
- `render_footer` no longer appends the notice. The row above it (a new
  one-line "notice strip", on `theme::PANEL`, above the hints) shows
  `first line only, whitespace collapsed, truncated to the *available* width`
  — never a fixed 40.
- Invariant: **the hints row is a pure function of the keybindings** — no state
  other than `widgets.is_empty()` may change it. That kills the shimmer by
  construction.

### 2. Multi-line notices are rendered honestly

`notify()` callers send multi-line strings (the `/nimbus` table). The strip shows
the first line + `… (+N lines)`, and a `/notices` command (or the existing
`/diag`) surfaces the full text. No more silent truncation of a table into 40
chars.

### 3. Band rows become per-key slots (defensive)

The band currently paints the *live* line list into the *settled* height. Give
each belowEditor widget key a settled row count (`band_slots: HashMap<String,
Settled>`), so one widget appearing can never displace another's row — the
prediction row in particular.

### 4. Diagnostics, so the next report is measurable

The `/diag` overlay gains a "bottom band" block: each belowEditor key with its
line count and settled slot, the ctx row presence, the notice text + age, and
the footer width used. If a flicker survives all of the above, the overlay says
which input changed between frames.

## Tests

- **Parity**: with no notice, the footer row is byte-identical to today.
- **No collision**: with a notice set, the hints row is unchanged and the notice
  appears on its own row.
- **Expiry**: a notice older than its TTL stops rendering; an `error` does not.
- **Multi-line**: a `head \n table` notice renders `head … (+N lines)`, never a
  bare ` · `.
- **Width**: a notice longer than the row is truncated to the available width,
  not a fixed 40.
- **Band slots**: two belowEditor widgets, one appearing/disappearing, never
  move each other's rows (render two frames, compare row indices).

## Trade-off

An expiring notice means a message can vanish before it is read. Mitigations:
`error` never expires, `warning` lasts twice as long as `info`, and the full
text stays available in `/diag`. If the owner prefers permanent notices, the
TTL becomes `None` by default and only the *row separation* fix ships.

## Files

- `officina/tui/src/tui/state.rs` — notice timestamp/TTL, first-line helper.
- `officina/tui/src/tui/layout.rs` — `render_footer` split into hints + notice
  strip; band slot rendering.
- `officina/tui/src/tui/mod.rs` — diag block additions.
- tests in `layout.rs`/`state.rs` per the list above.

---

# As landed (2026-11-16)

| item | status |
|---|---|
| notices on their own row above the hints | **done** |
| hints row = pure keybindings | **done** |
| multi-line notices → `head … (+N lines)` | **done** |
| width-aware truncation (no fixed 40) | **done** |
| `/notices` shows the full text | **done** |
| `/band` dumps the band composition | **done** |
| per-key band slots | **deferred** (see below) |
| notice TTL | **not shipped** (see below) |

## Why no TTL

The shimmer was the *collision*, not the persistence: once the notice has its own
row, a notice that stays put is stable. Expiring notices would have traded a
visual bug for a "did I miss a message?" bug, and the owner never asked for it.
`/notices` (full text) plus `/band` (composition) make anything unexpected
inspectable.

## Why per-key band slots are deferred

The band's height is settled (`stabilize_band_parts`) and the ctx row is
bottom-anchored, so a widget appearing cannot displace the ctx row any more.
Per-key slots would add a third settling layer to the same code path we just
stabilised twice; the measured evidence (dumps + tests) does not justify that
risk yet. `/band` exists so the next report can show whether slots are needed.

## The test that proves it

`hints_row_never_carries_a_notice` compares the hints row **byte-for-byte** with
and without a notice, at width 200. The negative control matters and nearly
fooled me: at width 120 the hints row is already full, so the old append was
clipped and the test passed even with the bug restored. At 200 the control fails
as it should — which also explains the owner's "flicker": the notice's tail was
being clipped mid-word as notices changed, not shown in full.
