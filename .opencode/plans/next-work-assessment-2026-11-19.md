# Are T1–T5 / A1–A2 good to tackle next? (2026-11-19)

Assessment, not advocacy. Each queued item was re-checked against the source before
ranking. Three cost estimates in `next-tests-and-naming-2026-11-19.md` were wrong, and
one premise is probably wrong again. Revised order at the bottom.

---

## Verdicts at a glance

| item | plan said | verified now | verdict |
|---|---|---|---|
| **T5** repair `llama-ui-assets` | low–medium, "patch upstream-hostile" | **one cache var**; root cause found | **DO FIRST**, ~15 min |
| **T1** `preserve_thinking` cost | highest value, cheapest | real; but flipping a default costs a config key + fingerprint | **DO SECOND** |
| **T2** reasoning budget sweep | low–medium | needs one new `Shape` field + exact-match guard | **DO THIRD** |
| **T3** E6 at `parallel=2` | medium | **premise likely wrong** — reframe to the eager floor | DO FOURTH, reframed |
| **T4** Chinese CoT | low, after T2 | unchanged, correctly ordered | optional |
| **A2** provider id rename | small | touches live settings + provider dir | do, carefully |
| **A1** metric prefix rename | "24 sites → one constant" | **~200 sites, 4 languages, live consumers** | dual-emit only; low value |

---

## T5 — it is one cache variable, not a patch

**Root cause, verified.** `engine/tools/ui/CMakeLists.txt:44` defines `llama-ui-assets` as
an `ALL` custom target that runs `scripts/ui-assets.cmake` on every build, with:

```
-DHF_ENABLED=${LLAMA_USE_PREBUILT_UI}   -DBUILD_UI=${LLAMA_BUILD_UI}
```

The failing tree `engine/build-ku-cu12-native/CMakeCache.txt` has:

```
LLAMA_BUILD_UI:BOOL=OFF
LLAMA_USE_PREBUILT_UI:BOOL=ON     <-- the problem
```

So the script skips priority 2 (npm build, disabled) and falls to priority 3 (HF bucket
download) — and the pinned `dist.tar.gz` predates `workbox[hash].js`, which
`ui-assets.cmake:123` requires, so it `FATAL_ERROR`s at line 136.

**Fix.** `-DLLAMA_USE_PREBUILT_UI=OFF`. VITRIOL ships no web UI (the operator runs the Rust
TUI; the JS frontend is deprecated), so no assets at all is the correct end state, not a
newer archive. No upstream patch, no vendored file, nothing hostile.

**Why it matters more than "housekeeping".** Every engine change currently needs a manual
object-and-`link.txt` dance. That tax is paid on the reasoning work, the cache work, and
every future engine experiment. Fixing it is the cheapest item on the list and the only one
that makes the others cheaper.

---

## T1 — real, and the cost of acting is larger than the cost of measuring

**Confirmed on.** `engine/tools/server/server-context.cpp:1758` warns at load that the
template preserves reasoning and that it can be disabled with `--no-reasoning-preserve`;
`common/arg.cpp:3757` defines the flag. `scripts/vitriol` does **not** pass it, so preserve
is ON in the live engine.

**Confirmed measurable.** The concern was that the API drops `reasoning_content` from input
messages, which would make the whole question untestable. It does not:
`engine/common/chat.cpp:452` parses `reasoning_content` off each input message, and
`:1063` re-attaches it as a `thinking` block. So a fixture can seed history and the
template will render it.

**Confirmed the harness gap.** `density-bench/src/engine.rs:62` builds a fixed two-message
array (`system`, `user`). A multi-turn fixture needs a `turns: &[(role, text, reasoning?)]`
field on `Shape` and a loop at that call site (there is a second copy at `:247`). Small, but
it is harness work the plan called free.

**The under-priced part.** If the measurement says "flip the default", flag provenance
applies: a speed-bearing engine flag needs a `~/.vitriol/config` key, a
`VITRIOL-FINGERPRINT:` token (like `rb=`, `spec=`, `ckpts=`), and a fingerprint diff. That
is the real cost of *acting*, and it must be in the plan before the measurement, or the
result sits unused. Invariant for that stage: **with no new config key, argv and behaviour
are byte-identical to today.**

---

## T2 — the guard is the work, not the sweep

`Shape` carries `expect: Option<&'static str>` (`shapes.rs:24`), a **substring** recall
guard. Scoring "which budget is right" on substrings is exactly the failure the plan
already warns about — a wrong answer that mentions the right number passes. So T2 needs:

1. `expect_exact: Option<&'static str>` on `Shape` + exact (post-trim) match in scoring;
2. fixtures with unambiguous answers — arithmetic, a one-hop logic case, a code-trace case;
3. `-` a control cell with budget 0 (no thinking) to anchor the "thinking bought nothing"
   case, so a flat curve is interpretable rather than ambiguous.

Cost is honest: one field, one predicate, three fixtures. Then the sweep itself is cheap
and the result is a number we can put on the marker's default.

The `[[officina-terse]]` interaction is worth one cell: if terse throttles the thinker, the
two markers are fighting and the operator needs to know which to drop.

---

## T3 — the premise is probably wrong; the reachable knob is different

E6 said eviction is unreachable at `parallel=1` because `evict_sparse()` only runs when
`find_slot()` fails. T3 assumed `parallel=2` creates slot pressure. Checked:

```
engine/src/llama-kv-cache.cpp:871-880
    auto sinfo_new = find_slot(ubatch, false);
    if (sinfo_new.empty()) {
        const char * vitriol_kv_mode = std::getenv("VITRIOL_KV_MODE");
        if (vitriol_kv_mode && strcmp(vitriol_kv_mode, "sparse") == 0) {
            uint32_t n_evicted = evict_sparse(ubatch.n_tokens, 4);
```

Two gates, both satisfied live (`VITRIOL_KV_MODE=sparse` is in the running process env,
verified from `/proc/<MainPID>/environ`; `score=probe`, `score_every=16`,
`score_min_fill=0.5`). The blocker is not `parallel` — it is that **admission control
rejects an oversized prompt before `find_slot` ever sees it**, and at `parallel=2` each
slot simply gets a smaller window. A prompt that fits a slot always finds one. So
`parallel=2` is likely to produce a second null for a new reason, which is worse than the
first null.

**Reframe T3 to the knob an operator could actually set:** `VITRIOL_KV_FLOOR` (eager sweep,
`llama-kv-cache.cpp:156`, default `0.0` = off, settable at runtime via the governor's
`POST /kv-policy`). Question: arming the floor at ~0.5 — does the recall guard survive and
is the decode win real? That is reachable, governor-supported, and answers "should the daily
driver arm this", which is a decision someone wants.

Keep the E6 conclusion text, but correct it: *eviction is a slot-overflow fallback, not a
knob*; the knob is the eager floor.

---

## A2 — provider id: small, but it touches a live session

`llamacpp` appears as a provider id in `~/.config/opencode`-adjacent config and in
`officina/.pi/extensions/llama-cpp-provider/` (16 test references). A rename needs an alias
period: accept both ids, write the new one, keep the old mapping for one release. Worth
doing — it is the name the operator sees.

## A1 — metric prefix: the plan's "24 sites" was the *engine* count only

Actual `llamacpp:` literal sites, excluding build trees:

```
 39  engine/tools/server/tests/unit/test_metrics.py
 36  vitriol-tui/src/poller.rs          <- live dashboard consumer
 23  engine/tools/server/server-task.cpp
 16  officina/.pi/extensions/llama-cpp-provider/config.test.ts
 15  engine/tools/server/README.md
 13  officina/.pi/extensions/vitriol-decode/decode.test.ts
 10  officina/.pi/extensions/vitriol-decode/decode.ts   <- PARSES the prefix
  6  demiurge/crates/demiurge/src/pressure.rs           <- live consumer
  6  engine/tools/server/tests/unit/test_speculative.py
```

Four languages, two live Rust consumers, one live TS parser, plus anything outside the repo
the operator scrapes. A flag-day rename is a coordinated multi-crate change with a restart
and a real chance of a silent telemetry gap — *for cosmetic gain*.

**Recommendation: dual-emit, then never come back.** One constant in the engine emits both
`llamacpp:` and `vitriol:` series; consumers migrate lazily; drop the alias whenever. Cost:
a constant and a loop. Benefit: the new name exists, nothing breaks.

Honest ranking: this is the **least** valuable item queued. It changes a string nobody
reads on purpose. It should not precede anything that produces a number.

**Invariant for dual-emit:** every existing series name and value is still present and
identical (pinned by a test that fetches `/metrics` and asserts the old names exist).

---

## Revised order

1. **T5** — one cache var, unblocks every later engine change. Verify `make llama-server`
   in a scratch tree actually succeeds afterwards, then rebuild the live one.
2. **T1** — measure preserve cost (multi-turn `Shape` + 2 arms). Decide. If flip: config key
   + fingerprint token in the same commit, or don't flip.
3. **T2** — `expect_exact` + three fixtures + budget sweep + one terse-interaction cell.
4. **T3-reframed** — eager floor vs off, recall guard + decode rate, via the governor.
5. **A2** — provider id with an alias period.
6. **T4** — Chinese CoT, using T2's budget.
7. **A1-dual-emit** — last, or never.

## What I would argue against doing at all

- **A1 as a flag day.** ~200 sites, four languages, live consumers, zero functional gain.
- **T3 as written** (`parallel=2`). Predictably another null for the wrong reason; the
  reframed version is the one with a decision attached.
- **Any further eviction tuning** until something proves the path is reachable at all.

## The item missing from the list

The largest unexplained number in the record is still **decode at depth: ~1.2 tok/s at
113–135k filled** (measured, listed as "not config-fixable"). None of T1–T5 touches it.
T1 and T2 make thinking *cheaper per turn*; neither makes a deep turn *fast*. If the goal is
a usable workstation at depth, that number outranks everything above — but it is engine
research (attention/KV layout, ubatch, the known ~23 KiB/token VRAM creep), not a bench
sweep, and it does not belong in a queue of cheap wins.
