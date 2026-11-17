# Fire base continuity + ascensus `[gemini]` section (2026-11-16, PLAN)

Two items: the rule row must be the fire's **continuous base**, and the optional
follow-up that gives the Gemini key a formal home in ascensus.

---

# A. The rule row must be the fire's base, without gaps

## What happens today

`line_cells` (`officina/tui/src/fire.rs`) claims a column when

```rust
let light = (col_light * 1.5).clamp(0.0, 1.0);
if light <= 0.0 || hash01(x, 313, step) >= light { continue; }   // `_`
```

`col_light = edge · level · 1.15`, so at a typical level only a fraction of the
envelope's columns clear the hash and the rest stay `_`. That is what the owner
sees as *"the bottom divider row has gaps, rather than truly being the start of
the fire"* — the rule is dotted, not a base. (The gaps were deliberate two
revisions ago, for "smoulder at low load"; at working load they read as damage.)

## The change: smoulder only below a level, continuous above it

```rust
const BASE_CONTINUOUS_LEVEL: f64 = 0.25;

if col_light <= 0.0 { continue; }                       // dead middle band
if level < BASE_CONTINUOUS_LEVEL {
    // Embers: a minority of columns, so `_` survives between flamelets.
    if hash01(x, 313, step) >= (col_light * 1.5).clamp(0.0, 1.0) { continue; }
}
// else: at working load every column inside the envelope is claimed — the rule
// is unbroken under the flame, which is what "the fire starts here" looks like.
```

Everything else stays: the glyph is the flame's base density scaled by
`rule_heat` (`0.55 + 0.45·level`), which keeps the rule measurably denser than
the tapered row above (no duplicate rows), and unlit columns keep `_`.

## Why a threshold rather than "always continuous"

Below ~0.25 the fire is embers: a fully claimed rule at ember load reads as a
bright line with nothing above it — the same "detached" complaint in reverse.
Keeping the probabilistic branch below the threshold preserves the smoulder, and
at/above it the flame body is present to justify a solid base.

## Invariants / tests

- **F-B1** at `level ≥ 0.25`, every column with `column_light > 0` is claimed
  (no `_` inside the flame's footprint). This is the owner's report, as a test.
- **F-B2** below 0.25 at least one `_` survives inside the footprint (smoulder).
- **F-B3** unlit columns (middle band, `level < MIN_LEVEL`, `/fire off`) keep `_`.
- **F-B4** the rule is never lighter than the row above it, and at `level ≥ 0.6`
  a majority of claimed cells are strictly denser (unchanged from F-A).
- Update `a_burning_flame_consumes_its_line` (its `light > 0.67` guard becomes
  obsolete: with B1 the rule is continuous, so every `col_light > 0` column is
  claimed at working load).

Trade-off: where the fire burns, the underscore is gone for good at working load
— the rule becomes the flame's base band. That is the request; `/fire off`
restores the plain rule.

---

# B. Ascensus: a real `[gemini]` section

## What exists

`scripts/ascensusd.py` is single-key:

```python
def read_secrets():                      # → (provider, key, model)
    provider = env ASCENSUS_PROVIDER
    ... reads ~/.vitriol/secrets: first `api_key` only ...
    if not provider: provider = "gemini" if key.startswith("AIza") else "zai-coding-plan"
```

Consequence today: the operator's Gemini key sits in that file as
`[ascensus] gemini_api_key = AIza…`, which **my resolver reads but ascensusd does
not** — ascensus would only ever see the zai key, and dropping an unqualified
`api_key = AIza…` would *silently flip the escalation provider* (and default the
model to `gemini-2.5-flash`). That is the "optional follow-up" from the decision
lane plan: give the key a formal home.

## The change

1. **Section-aware parsing**, additive and back-compatible:

   ```python
   def read_provider_secrets(path=GEMINI_KEY_FILE):
       """{"provider", "key", "model", "sections": {"gemini": {...}}}"""
   ```

   - `[gemini]` section: `api_key`/`key` + `model` are captured there;
   - `gemini_api_key` under *any* section is captured too (matches the layout the
     operator already has, and the JS/TS resolvers);
   - top-level `api_key`/`model`/`provider` behave exactly as today.

2. **Resolution order** (unchanged where it matters):

   | source | wins over |
   |---|---|
   | `ASCENSUS_PROVIDER`, `GEMINI_API_KEY`, `GEMINI_MODEL` env | everything |
   | `[gemini]` section when the provider resolves to `gemini` | the top-level key |
   | top-level `api_key` for any other provider | — |
   | `AIza…` sniff (only when no provider is named) | — |

   So `provider = zai-coding-plan` keeps using the zai key even though a Gemini
   key is present; `ASCENSUS_PROVIDER=gemini` picks the section's key/model.

3. **Key transport hardening** (one line, worth doing while in here):
   `call_gemini` builds `...:generateContent?key=…`. Google documents the
   `x-goog-api-key` header for REST; the header keeps the key out of URLs and
   logs. Verify against the live API before switching.

4. **Selftest**, because this repo has no Python test harness:
   `scripts/ascensusd_selftest.py` importing the module (it is guarded by
   `if __name__ == "__main__"`) and asserting the matrix: top-level only; a
   `[gemini]` section; `gemini_api_key` under `[ascensus]`; `provider=gemini`
   picking the section; `AIza` sniff when no provider is named; env overrides;
   missing file. Exit non-zero on failure.

## Invariants

- **A-B1** with no `[gemini]` section, `read_secrets()` returns exactly what it
  returns today for the same file (parity).
- **A-B2** a Gemini key present alongside zai never changes the default provider.
- **A-B3** env still wins over the file, everywhere.
- **A-B4** a malformed section is ignored, not fatal (the daemon must start).

## Tests

Selftest cases above (A-B1..B4). Optionally wire `python3
scripts/ascensusd_selftest.py` into the pre-commit gate later; for now it is a
documented command.

---

# Build order

1. **A** (self-contained, ~15 lines + test updates) — install, dump, and the
   owner can judge the base immediately.
2. **B** (parser + resolution + selftest), verify `vitriol caduceus decision
   status` still resolves the key from `~/.vitriol/secrets`, and that
   `ascensusd` still reports zai as the provider.
