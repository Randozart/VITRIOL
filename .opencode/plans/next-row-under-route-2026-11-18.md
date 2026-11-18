# The "next" row under the route strip

**Date:** 2026-11-18 20:20
**Mode:** PLAN (research only — nothing changed)
**Owner:** "I'm still getting the occasional 'next' line right underneath the route. Like route manual
followed by 'next'."

## 1. What is actually producing that row

It is `composer-predictions`, not the router:

```
.pi/extensions/composer-predictions/index.ts:43   const WIDGET_KEY = "composer-prediction";
.pi/extensions/composer-predictions/index.ts:149  ui.setWidget(WIDGET_KEY, line ? [line] : [], { placement: "belowEditor" });
.pi/extensions/composer-predictions/predict.ts:61 const line = `▸ next: ${clipTask(step)}`;
```

And the band has a fixed budget with a priority map that names only five keys:

```rust
// tui/src/tui/layout.rs:444-473
.filter(|w| w.placement.as_deref() == Some("belowEditor"))
.filter(|w| w.key != "agent-mode" && w.key != "engine-fire")
.filter(|w| !band_echoes_draft(&w.lines, &state.input))
…
.sort_by_key(|w| match w.key.as_str() {
    "overflow-route" => 0, "vitriol-decode" => 1, "plan-mode" => 2,
    "deep-research-mode" => 3, "phase-model" => 4, _ => 9,
});
let indicators = … .take(3).collect();
```

So the strip is **three rows for ten writers**, ordered by that map, and `composer-prediction`
falls into `_ => 9`. With the route row present there is still room (3 − 1), so the prediction
renders directly beneath it:

```
route manual — overflow paused
▸ next: <current task> · <verify/git hints>
```

**"Occasional" is the tell:** `predict()` returns `null` when there is no task ("D4: silence beats
process noise"), so the row appears only while a task list exists — i.e. during GRIND, which is
exactly when the route row says `route manual`.

## 2. Why it cannot simply become placeholder text

The empty composer already renders the **carved motto** placeholder (owner request 2026-09-03,
`layout.rs:1105`). Ghost-texting the prediction there would fight that design, so "move it into the
composer" is not free.

## 3. Options

| | change | cost | cost to the design |
|---|---|---|---|
| **A** *(recommended)* | **state rows own the strip; suggestions yield.** Give `composer-prediction` an explicit *suggestion* tier and render it only when no state row is present. | ~15 lines in the band collection + a test | none — the feature survives for the empty-band case |
| **B** | fold the prediction into the motto line (replace the motto when a prediction exists) | small | loses the carved motto exactly when it is most visible |
| **C** | merge the text into the `overflow-route` widget | small | cross-extension coupling across jiti instances, and two sources of truth on one line |
| **D** | delete the feature | zero | loses the `verify`/`git` hints; the task text already shows elsewhere |

Recommendation: **A**, because the band is a *status* strip and a suggestion is not status. The
priority map already encodes this opinion (route=0 … phase=4, everything else=9); the missing piece is
that 9 still gets rendered when there is spare room.

## 4. Shape of the fix (A)

1. `layout.rs`: split the collected widgets into **state** (named keys) and **suggestion**
   (`composer-prediction`) tiers. Render suggestions only when the state set is empty; otherwise the
   three rows go to state widgets, and an unknown future key still competes normally at tier 9
   (do **not** key the rule on "anything unmatched", or a new extension silently disappears).
2. Keep the cap at 3 and leave `stabilize_band_parts` untouched — the ctx row's position must not
   move (that was the 2026-11-16 flicker fix).
3. `band_echoes_draft` still applies to whatever renders.

## 5. Invariants

- **I1** — no suggestion row renders while a state row (route/decode/plan/research/phase) is present.
  Pinned by a band-composition test with both widgets set.
- **I2** — band cap stays 3 and the ctx row does not move; the settled-height test in
  `stabilize_band_parts` is untouched.
- **I3** — with no state rows, the prediction still renders (feature parity, negative control of I1).
- **I4** — an unknown belowEditor key (a future extension) still renders at tier 9: the rule keys on
  the suggestion widget, not on "unmatched".

## 6. Immediate relief, before any code

```bash
OFFICINA_NO_PREDICTIONS=1 officina
```
(`composer-predictions/index.ts:124` returns early on that variable, so the widget is never written.)
