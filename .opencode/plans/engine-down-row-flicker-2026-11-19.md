# The "engine down" row flicker: two writers, neither one clears up after itself (2026-11-19)

Owner report: *"I'm still seeing occasional flickering on the 'VITRIOL engine down' row, but
I can't make out what is trying to also live there."*

The thing trying to live there is **the decode row's own last progress line**. Both states are
written by the same extension, and neither path clears the other one's output.

---

## 1. The mechanism (read off the code, no runtime needed)

`officina/.pi/extensions/vitriol-decode/index.ts` has two disjoint output paths:

```ts
// lines 68-82 — DOWN
if (!last.up) {
  ui.setWidget("vitriol-decode", ["◈ VITRIOL  engine down (tris up to start)"],
               { placement: "belowEditor" });
  return;                    // ← returns BEFORE the band is written
}
// lines ~120-133 — UP
setBandRow("vitriol-decode", "progress", lines);   // ← never touches the widget above
```

The consequences, in both directions:

| transition | widget `vitriol-decode` | band row `vitriol-decode` |
|---|---|---|
| engine up (steady) | **stale "engine down" left from before** (never cleared) | live slots/tok-s |
| engine down | live "engine down" | **stale progress left from before** (never cleared) |

Verified: `vitriol-decode/index.ts` contains exactly two `setWidget` calls (`engine-fire`, and the
down-state `vitriol-decode`) and **no `clearWidget` at all** — so once the down line is written it
is permanent. It disappears only when the TUI drops its whole widget map on a session switch
(`tui/mod.rs:3906`, `state.rs:3073`). That is why the owner calls it *occasional*: the stale row is
cleared by an unrelated event, then returns on the next flap.

So at the first flap the strip holds **two rows about the same thing**: a live one and a stale
one. `layout.rs` then decides which you see:

- `indicators` flattens every `belowEditor` widget, sorts `officina-band => 0`, everything else
  `=> 9`, and finally `.take(3)` (`layout.rs:446-473`). The legacy `vitriol-decode` widget sorts
  **last**, so it is always the line that gets clipped when the strip is full.
- The *height* of the strip is settled independently, with `BAND_SETTLE = 700 ms`
  (`state.rs:19`), and the engine poll cadence is **also 700 ms**
  (`_shared/engine.ts:26`). Two oscillators on the same period: the line count and the applied
  height drift in and out of phase, so a row is admitted, clipped, and re-admitted. That is the
  flicker, and it is why it never settles into something readable.

`Settled` is doing its job (it exists to stop exactly this class of movement — "the ctx field
looks like a different field", 2026-11-16); the defect is that a *stale* row is feeding it.

## 2. Why the down path exists at all, and why it is the last one of its kind

The band registry (`_shared/band.ts`, 2026-11-18) made the strip a contract: tiers
`state > progress > suggestion`, one widget `officina-band`, `BAND_MAX_ROWS = 3`, and this rule
in its own header —

> **NEW producers must not appear here at all** — they register a row in `_shared/band.ts`.

`layout.rs:455-467` repeats it: *"Legacy producers (pre-registry sessions still running, and the
decode fallback): keep them renderable but behind the registry's strip."*

The decode **down** path is that fallback, kept when six other producers were migrated in S2
(2026-11-18). It was not migrated because it is not a *band row* — it is the one state the
extension can be in where it has no numbers to show. And nobody noticed, because it only fires
when the engine is actually away.

Note `vitriol-decode` already holds slot **1** in the registry's `STATE_ORDER`
(`band.ts:79`, after `overflow-route`), so the registry already has a place for it.

## 3. Secondary contributor worth fixing at the same time

`classifyFetchError` maps an aborted request to `stalled` and keeps `up: true` (correct, 2026-09-04
fix), but `fetchText` maps **any non-2xx response** to `{kind: "down"}` (`engine.ts:224`). A
`/metrics` 503 during model load is genuinely "engine not ready", so that is not wrong — but it
means every restart produces a legitimate down→up flap, and the flicker above makes each flap
visible for longer than the restart itself.

---

## 4. Fix plan (small, and it deletes a code path rather than adding one)

**F1 — the down state becomes a band `state` row, not a widget.**
In `vitriol-decode/index.ts`: replace the `ui.setWidget("vitriol-decode", …)` branch with
`setBandRow("vitriol-decode", "state", ["◈ VITRIOL  engine down (tris up to start)"])` and let
execution fall through to the normal `return`, no early exit before the band is written.
Effects: (a) the row is cleared by the same `setBandRow(..., null)` on recovery, so nothing goes
stale; (b) tiering guarantees the down row **wins** a slot over progress/suggestion, which is the
whole point of `state`; (c) there is one owner of the strip's contents again.

**F2 — the down state must also clear its own progress rows.** While down, publish
`setBandRow("vitriol-decode", "progress", null)`. Two rows about one subject, one of them stale,
is the bug; a single source of truth for "what is the decode row saying" removes the class.

**F3 — close the legacy escape hatch in the TUI.** After F1/F2, run one release with the Rust
fallback intact (an installed TUI outlives an engine/extension change — pre-registry sessions must
keep rendering), then add a source gate: no `setWidget(..., { placement: "belowEditor" })` outside
`_shared/band.ts` except an explicit allowlist (`engine-fire` — the flame data feed, filtered from
display; `session-panel`/`agent-mode` — different placements). Gate lives next to the existing
band producer gate that already scans for direct band-key writes.

**F4 — break the beat frequency.** Not strictly needed once F1 lands, but cheap insurance:
`BAND_SETTLE` (700 ms) equals the poll cadence (700 ms). Set the settle to 900–1000 ms so a
single-poll datum can never be adopted-then-reverted in lockstep with its producer. One constant,
one test that pins the relationship (`BAND_SETTLE > poll cadence` can even be asserted from a doc
comment, or at least documented at the constant).

**F5 — evidence for next time.** The strip is invisible in logs. Add a debug line (behind
`VITRIOL_DECODE_DEBUG=1`) that prints the assembled band payload on change — the same discipline
`/cache` and `/claims` follow rather than making the operator squint at a flickering row.

## 5. Invariants (review checklist)

- **I1 single writer:** for any engine state, at most one of {band row, legacy widget} carries
  `vitriol-decode` content. Pinned by a test that drives the extension through up → down → up and
  asserts the band's `setWidget` payload never contains both a down line and a progress line.
- **I2 recovery clears:** up-after-down leaves no "engine down" text anywhere in the band payload
  (the current bug: it lingers forever).
- **I3 parity with pre-registry sessions:** the Rust `_ => 9` fallback keeps rendering for old
  sessions until F3's gate lands, then only for allowlisted keys.
- **I4 the strip height never changes because of a stale row** — the settle tests already exist
  (`render-invariants`, `band_echoes_draft`); add the down/up cycle to them.

## 6. Tests to write

1. `vitriol-decode` unit test: snapshot `up:false` → band payload has exactly one `state` row and
   no `progress` row; snapshot `up:true` → exactly one `progress` row and no `state` row (I1/I2,
   both directions, negative control included).
2. A cycle test: up → down → up must produce byte-identical payload to the steady-state up payload
   (this is the test that would have caught the original bug).
3. `band.test.ts`: assert `vitriol-decode` is in `STATE_ORDER` and that a `state` row suppresses a
   `suggestion` row (already covered) — extend to assert a down-state row survives a full strip
   (3 rows present, down still rendered).
4. Rust: `stabilize_band_parts` unchanged behaviour, plus a case where the candidate oscillates at
   exactly the settle period and must **not** be adopted.

## 7. Effort

Low: one extension file (F1/F2), one constant (F4), one test file, plus the F3 gate when the
release window allows. No engine work, no TUI rebuild needed for F1/F2/F4 (extensions load at
runtime) — but the band layout change is TS-side, so **no binary install is required**; only the
running TUI needs the same restart any extension change needs.

F3 changes Rust, so that stage needs the rebuild + install (`~/.local/bin/officina`) per the
standing rule.
