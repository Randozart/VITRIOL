# Fire spawns on the rule + decision model for Caduceus routing (2026-11-16, PLAN)

Two asks, both planned here:

**A.** *"The fire on the rule now just covers the rule, rather than allowing the
fire to emerge there. The rule is allowed to be replaced by the fire spawning
there."*

**B.** *"Can we use the decision model for Caduceus?"*

---

# A. Rule should be *replaced* by the fire, not covered by a dot

## What it does now (and why)

`line_cells` (`officina/tui/src/fire.rs`) paints `DENSITY[1]` — one low dot — in
every column whose level-scaled density clears 0.08. That was my reading of
"let the line smoulder", and it is wrong in the way the owner describes: the dot
*sits on* the rule, so the rule still reads as an underscore with punctuation.

Meanwhile F6 fixed the real detachment bug: the field's bottom row is now the
column's **base** (`frac = (frac_from_base − base)/(col_h − base)`), and it is
tapered (`× 0.3 + 0.5·level`) so it hands over gently.

## The change

Make the rule row carry the **flame's base density** — i.e. the fire spawns
*there* and consumes that cell — while keeping ember sparsity at low load:

```rust
// base density of this column (same hash seed the field uses for its base row)
let base_dens = ((1.0 - frac * 0.8) * (0.7 + 0.6 * hash01(x, 0, step))).clamp(0.0, 1.0);
let s = 0.55 + 0.45 * level.clamp(0.0, 1.0);      // the rule is the hottest row
let scaled = base_dens * s;
if scaled <= 0.08 { continue; }                   // embers keep the underscore
*slot = Some(DENSITY[(scaled * 7.0).round() as usize]);
```

Deliberately `s(level) > taper(level)` (`0.55 + 0.45L` vs `0.3 + 0.5L`, a gap of
`0.25 − 0.05L`), so:

- the rule is **denser** than the row above at every load — no chance of the two
  identical rows the owner flagged earlier;
- the rule is the *hottest* part of the flame, which is what "fire spawning
  there" looks like;
- at low load the scaled value falls under the smoulder threshold in most
  columns, so `_` remains between sparse flamelets — the smoulder survives, it
  just is not a permanent dot.

## Invariants / tests

- **F-A1** an unlit column (middle band, `level < MIN_LEVEL`, `/fire off`) keeps
  `_` — the rule is never consumed wholesale.
- **F-A2** at `level = 1.0` every claimed rule cell has ramp index **>** the
  index of the cell above it (strict), replacing the current "always exactly
  one dot" assertion.
- **F-A3** at first embers (0.06) at least one `_` survives inside the lit zone
  (smoulder), and claimed cells are ≤ 2 dots.
- **F-A4** no claimed rule cell is `⣿`; glyphs stay in `DENSITY`.
- Existing tests to update: `rule_smoulders_below_the_flame`,
  `first_embers_only_dot_the_line`, `a_burning_flame_consumes_its_line`.

Trade-off: where the fire burns, the rule becomes a dense braille band instead
of a line — that is the requested behaviour, but in a wide transcript it makes
the "rule" discontinuous by design. If it reads as heavy, the same knob
(`s`'s base 0.55) can be lowered without touching anything else.

---

# B. Decision model for Caduceus routing

## Where it can sit

The gateway (OmniRoute, vendored) chooses accounts/lanes internally and we cannot
patch it. But **our** proxy (`officina/scripts/caduceus-router.cjs`) already owns
one decision: which model id to *send* (requested first, then configured
fallbacks, skipping parked lanes). That is the decision the model can inform.

## What the Decisions API gives us for this

From the verified contract (see `gemini-decision-lane-2026-11-16.md`):

- `choice` questions accept **2–255 options** (criteria map id → description),
  1–1,000 questions per request, one shared `input`, **zero output tokens**;
- the reply carries **`probabilities`** per option and `confidence`; and — learned
  the hard way — **`choice` is not the argmax**, so we must take the argmax
  ourselves.

Lane selection is a natural fit: options = candidate lanes, criteria = their
facts, probabilities = the model's preference.

## Design

**Input: metadata only, never the prompt.** The `input` string holds: the
requested model id, a token estimate bucket (`<8k`, `8–32k`, `32–128k`, `128k+`),
streaming flag, and a one-line intent (`"pick the best lane for this request"`).
Criteria describe each candidate lane: owner, free/paid, context window,
cooling/parked state, and the operator's pin if any. **No user text leaves the
box** — this keeps the privacy posture of the watchdog lane while routing.

**Tiering (cheapest first).** The proxy keeps its deterministic order and only
asks the model when that order is *ambiguous*:

| condition | action |
|---|---|
| a pinned/requested lane is usable | use it, no call (the pin we just built) |
| exactly one usable candidate | use it, no call |
| ≥2 candidates that tie on the preference key (free-ness, then context fit) | **ask the decision model** |
| model unavailable / below floor / timeout | fall back to the deterministic order |

**Policy on the answer.** Argmax over `probabilities`, but constrained: never a
parked lane, never a lane that cannot fit the token bucket, and prefer the free
lane unless the paid one is overwhelmingly preferred (`p_paid − p_free > 0.25`).
Below `floor` (default 0.6) → deterministic order.

**Cost and latency.** ~900 input tokens per call (twenty lanes × a short
criteria line) ≈ **$0.0001**, plus one prefill ≈ **200–300 ms**, only on the
ambiguous path. A 30-second memo keyed by `(requested model, token bucket, lane
set hash)` keeps a burst of turns to a single call.

**Config.** Reuse the decision config's shape under a new key so the two lanes do
not fight:

```
vitriol caduceus router --decision on|off          # default off
vitriol caduceus router --decision-floor 0.6
vitriol caduceus router --decision-model gemini-decision-1
```
Key resolution reuses `ascensusGeminiKey()` (`~/.vitriol/secrets`).

**Audit.** Every model-informed choice writes a reroute/lane line
(`{"lane": …, "p": 0.71, "reason": "decision model"}`) in
`caduceus-router-reroutes.jsonl`, so a surprising lane is explainable.

## Invariants

- **R-A1** never override an explicit pin (the operator's `/model` choice, or a
  model the client literally requested).
- **R-A2** no prompt content in the request — metadata only (unit-tested on the
  builder).
- **R-A3** never select a parked or too-small lane, whatever the probabilities
  say.
- **R-A4** with `--decision off`, behaviour is byte-identical to today
  (deterministic order) — the parity invariant.
- **R-A5** bounded: one call per ambiguity, memoised 30 s, hard 2.5 s timeout,
  and any failure degrades to the deterministic order (never an error to the
  client).

## Tests

- request builder: options map to lane ids, no prompt/tail text present;
- policy: argmax + free preference + margin; below-floor → deterministic;
- parked/too-small lanes are never chosen even with p=1.0;
- memoisation: two identical requests in the window → one API call;
- failure modes (429/timeout/garbage) → deterministic order, no client error;
- parity: `--decision off` produces the same candidate order as today.

## Open question for the owner

1. Should the model also choose **between account lanes inside the gateway**
   (upstream API keys)? It cannot — that decision happens inside OmniRoute. This
   plan only governs the model id the proxy sends.
2. Is a 200–300 ms pre-first-byte delay acceptable at turn start on the ambiguous
   path? If not, gate it behind `/route-decision on` for a session.

## Build order

1. **A first** (self-contained, ~20 lines + test updates) — install, and you can
   judge the look immediately.
2. **B** behind `--decision off` default: builder + policy + memo + ledger, then
   enable it and watch the ledger for a day before trusting it.

---

# As landed (2026-11-16)

| part | commit | note |
|---|---|---|
| A — fire spawns on the rule | `25f1c34` | installed `c571a796` |
| B — decision-model lane choice | `26db095` | off by default |

## A — what the dumps show

```
level 0.3   body ⡄⡆⡆ …   rule ⣇⡇⣇__⣇  (denser, with `_` gaps)
level 0.6   body ⡇⡆⡇ …   rule ⣇⣇⣇⣇_⣧
level 1.0   body ⡇⣧⡇ …   rule ⣇⣷⣧⣷⣧
```

`rule_heat = 0.55 + 0.45·level` stays above the base-row taper
(`0.3 + 0.5·level`), so the rule is the column's hottest row — the fire is born
there — and the two rows cannot come out identical. Presence is probabilistic
(`hash < col_light·1.5`), which is what keeps `_` between flamelets at low load:
the smoulder survives as *gaps*, not as a permanent dot.

## B — the pin invariant was violated first, and the e2e caught it

The first e2e run showed the decision model demoting the **requested** model to
attempt 2 — i.e. it would have overridden an operator pin. Fixed before commit:
the model may only lead the *fallback tail*; `candidates[0]` stays the requested
id (R-A1). The e2e now asserts `attempt=2` for the model's pick, which is the
proof that the requested model is still tried first.

## Still off by default

The proxy reports `decision: off — deterministic order`. To enable:

```
vitriol caduceus router --install --enable --decision on --decision-floor 0.6
```

Cost/latency on the ambiguous path only: one Decisions call (metadata only,
~$0.0001, 2.5 s cap), memoised 30 s per (requested, token bucket, lane set).
With `--decision off`, candidate order is byte-identical to before (R-A4).
