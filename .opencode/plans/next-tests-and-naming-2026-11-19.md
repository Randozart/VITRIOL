# Next tests, and retiring "llama" from the record (2026-11-19)

Two questions, one answer each. The naming one has licensing teeth, so it comes with a
tier table and a migration, not a find/replace.

---

# Part A — What to test next

Ranked by (expected value ÷ cost), with the guard that makes each result trustworthy.
The reasoning work from this morning reshuffled the order: T1 is now first *because*
thinking lanes just became reachable, and it is a default-on behaviour nobody measured.

## T1 — `preserve_thinking` is ON by default and re-renders history (highest value, cheapest)

**Evidence it is on.** The live `/props` template:

```jinja
{%- if preserve_thinking is undefined or preserve_thinking is true or loop.index0 > ns.last_query_index %}
  {{- '<|im_start|>' + message.role + '\n<think>\n' + reasoning_content + '\n</think>\n\n' + content }}
```

`preserve_thinking` is undefined unless someone sets it, so **every assistant turn in the
history gets its `<think>` block re-rendered into the prompt**. The engine even warns about it
at load (`server-context.cpp:1758`, "enabled by default (may use more tokens, disable via
`--no-reasoning-preserve`)").

**Why it matters now and not before.** While `reasoning = off` the reasoning_content is empty,
so the cost is zero. The moment a lane uses `[[officina-think]]`, a 20-turn session with 1k-token
thinking per turn carries ~20k extra prompt tokens on every request — silently, by default.

**Method.** One thinking shape, N=3 repeats, engine `--reasoning-budget` set; arms:
`preserve` (default) vs `--no-reasoning-preserve`, multi-turn fixture (6 assistant turns with
seeded reasoning). Measure `usage.prompt_tokens` per turn and the recall guard.

**Metric.** prompt tokens at turn 6 (the whole point) + accuracy guard (a fact stated in turn 1,
asked at turn 6 — preserving reasoning should help or not).
**Kill criteria.** If preserve costs <5 % prompt tokens, stop worrying about it and leave the
default alone. If it costs >30 % and the guard does not improve, flip the default in our
launcher and record it.
**Effort.** Low. No engine rebuild — the flag exists (`--reasoning-preserve/--no-reasoning-preserve`).

## T2 — What reasoning budget is actually right? (my 1024 was a guess)

We now *can* bound thinking; nobody knows the bound. Arms `0 / 128 / 256 / 512 / 1024 / 2048`
against a **verifiable** fixture (arithmetic + one-hop logic + a code-trace question with an
exact expected answer), scoring accuracy per 100 output tokens.

**Guard.** Exact-answer match, not recall substrings — a wrong answer that mentions the right
number is still wrong. This needs a new `Shape` kind in `density-bench` (`expect_exact`).
**Kill criteria.** If accuracy is flat from 128 up, the answer is "128 everywhere" and the
default drops; if accuracy keeps rising to 2048, the honest answer is "budget is task-class
dependent" and the marker takes a per-lane argument (it already does: `[[officina-think N]]`).
**Effort.** Low–medium (one new shape + exact-match guard).
**Note.** T2 and `[[officina-terse]]` interact: terse may throttle thinking. Run one cell with
both markers to see whether the terse instruction is fighting the thinker.

## T3 — E6 revisited at `parallel=2` (closes a hypothesis we stated and never checked)

E6 concluded: *"eviction is a multi-session knob, not a single-lane depth knob"* — because
`evict_sparse()` only runs when `find_slot()` fails and admission control rejects oversized
requests at `parallel=1`. That is an inference, not a measurement. The prediction is testable:
**at `parallel=2` the cache holds two sessions, slot pressure is real, and
`kv_ejected_total` should move.**

**Method.** Load the 208k profile (`qwen38-buffedmod-208k`) with `parallel=2`, one long session
pinned in slot 0, a second session whose prompt forces a slot miss; watch
`llamacpp:kv_ejected_total` and `llamacpp:reasoning_*` (the new counters cost nothing to read).
**Guard.** Recall probe on the *older* session before and after the eviction — the whole point is
whether eviction trades amnesia for capacity.
**Kill criteria.** Counter still 0 → the eviction path is dead code at every parallelism we run,
and the S3 capacity work should say so instead of budgeting for it.
**Effort.** Medium (config change + engine restart + two concurrent clients).

## T4 — E5 Chinese CoT, finally runnable, and now bounded

With `[[officina-think 1024]]` (opt-in `OFFICINA_THINKING=1`) it runs on the **live** engine, no
restart, no global `--reasoning on`. Same 15-task fixture as planned; English CoT vs Chinese CoT,
accuracy + tokens. Expected honest null; recorded so it stops being the open question.
**Effort.** Low. **Do it after T2** so both arms use a known-good budget, otherwise the result
conflates budget with language.

## T5 — Housekeeping that unblocks future engine work: repair `llama-ui-assets`

`make llama-server` is broken in `engine/build-ku-cu12-native` (upstream's `ui-assets.cmake` now
requires `workbox[hash].js`; the pinned `dist.tar.gz` predates it). I worked around it by building
objects and relinking from `link.txt`. That works but it is a trap for the next person, and it is
why an engine change currently costs a manual dance.
**Fix options.** (a) `-DLLAMA_BUILD_UI=OFF` in a fresh tree if the server's `add_dependencies` can
be guarded; (b) `-DLLAMA_UI_HF_BUCKET` pointed at a local dir; (c) patch `ui-assets.cmake` to make
the workbox check optional when the archive predates it. (c) is upstream-hostile; (a) is cleanest.
**Effort.** Low–medium, no behaviour change. Not a test — do it when convenient.

### Not worth testing (keep the record honest)
- Per-request `enable_thinking` support — **proven this morning** (works on a `--reasoning off`
  engine). Do not add it to a backlog again.
- Eviction at `parallel=1` — E6, measured, inert.
- `score_every` for single-lane depth — E6, inert.

---

# Part B — Striking "llama" from the record

## The honest split

There are 20,705 `llama` mentions in `engine/`, 380 in `docs/`, 282 in `scripts/`, 275 in
`officina/.pi`, 11 in the TUI. They are **three different things** and only one of them is ours
to rename. Treating them as one word is how a rename becomes a fork-ending mess.

| Tier | What | Count / where | Verdict |
|---|---|---|---|
| **A. Our record** | metric prefix `llamacpp:`, provider id `llamacpp`, `LLAMA_DIR`, extension dir `llama-cpp-provider`, docs prose, our comments | ~150 sites we own | **Rename** |
| **B. Upstream ABI** | `llama_*` symbols, `LLAMA_ARG_*` env, `libllama.so`, `llama.h`, cmake targets `llama-common`/`llama-server`, upstream file paths | 20k, all in `engine/` | **Keep** |
| **C. Dead archives** | one-off benchmark scripts from August (`benchmark_alka.sh`, `test_expert_cache.sh`, …) | ~200 | **Leave** (churn with no reader) |

### Why Tier B stays — three reasons, any one of which is decisive

1. **MIT attribution.** `engine/LICENSE` is "Copyright (c) 2023-2026 The ggml authors" and
   `NOTICE` credits llama.cpp. Renaming the *project* is fine; deleting or obscuring the
   attribution is not. Keeping the upstream names makes the derivation self-evident, which is
   exactly what a licence notice is for.
2. **Frozen-sync diffability.** `docs/ENGINE_PROVENANCE.md` pins the import commit and states the
   sync policy is FROZEN. If every `llama_*` becomes `vitriol_*`, every future upstream fix
   arrives as a conflict on every touched line and the diff stops being readable. The fork's only
   defence is that it still *looks like* the thing it forked.
3. **Zero operator benefit.** Nobody types `llama_token`. The wins are all in Tier A.

## Tier A — the actual rename, in dependency order

**A1. Metric prefix `llamacpp:` → `vitriol:`** (highest visibility: it is in `/metrics`, docs,
and the TUI's decode lane).
- Engine: the prefix is emitted as a **literal at 24 sites** (`server-task.cpp` 23,
  `server-context.cpp` 1) — introduce one constant (`VITRIOL_METRIC_PREFIX`) and emit from it, so
  this is the last time it is ever a find/replace.
- Consumers: `officina/.pi/extensions/vitriol-decode/decode.ts` parses it by prefix;
  `docs/OFFICINA.md:376`, `docs/OPERATIONS.md:36`, `docs/ENGINE_PROVENANCE.md:37`;
  `engine/tools/server/bench/bench.py`.
- **Migration:** emit **both** prefixes for one release (a plain duplicate line is enough), and
  make `decode.ts` accept `vitriol:` first, `llamacpp:` as legacy. Version skew is real here:
  the operator's installed TUI outlives any single engine build.
- **Guard test:** `decode.test.ts` gains a both-prefixes case; a source scan fails if a
  `"llamacpp:` literal reappears in the engine.

**A2. Provider id `llamacpp` → `vitriol`** (operator-visible: uncoupled models render as
`<provider> · <id>` in the header, and `/models` groups by it).
- 8 non-test files carry it (`index.ts` 23 sites, `decode.ts` 10, `resilient-default` 9,
  `coupling` 4, `config.ts` 3, `small-lane` 2×2, `telemetry-scope` 2).
- **The trap:** `~/.pi/agent/settings.json:13` holds `"llamacpp/Lapis Occultus": {…}` — a
  user-level model override keyed `provider/modelName`. A blind rename makes that override
  **silently stop applying**, and the operator would see a model revert with no error. Any rename
  must load and rewrite that key (with a backup), or read both ids for a deprecation window.
- **Do this last**, after A1 proves the dual-read pattern works.

**A3. `LLAMA_DIR` → `VITRIOL_ENGINE_DIR`** in `scripts/vitriol` and the scripts that are still
run (`build-llama-server.sh` keeps its filename — it *does* build the llama.cpp-derived engine,
so the name is honest).

**A4. Extension directory `llama-cpp-provider/` → `vitriol-provider/`** — pure file move plus
import paths, but it is the pi extension's identity; do it together with A2 so there is one
migration event, not two.

**A5. Prose in `docs/`** — only where it refers to *our* stack ("the llamacpp provider",
"llamacpp metrics"). `docs/ENGINE_PROVENANCE.md` keeps every upstream reference: that file's job
is to name the ancestors precisely.

## Invariants (review checklist)

- **N1 parity:** an engine build before and after A1 serves the same numbers under both prefixes
  for one release; the fingerprint is unchanged.
- **N2 no silent override loss:** after A2, `params.json`-equivalent user overrides still apply —
  pinned by a test that loads the legacy key and asserts it is honoured or migrated with a notice.
- **N3 attribution intact:** `NOTICE`, `engine/LICENSE`, `ENGINE_PROVENANCE.md` upstream sections
  are byte-unchanged by any rename (`git diff` on those files must be empty).
- **N4 no ABI churn:** zero changes to `llama_*`, `LLAMA_ARG_*`, lib or cmake target names in this
  work. A source scan can enforce it (`git grep -c 'llama_' engine/include` must not fall).

## Verdict / effort

- **Do A1 + A5 now** (~2 h): metric prefix behind a constant, dual-emit, both-prefix parse, docs
  prose. Big visibility win, small blast radius, guardable.
- **Do A3 with A1** (same commit, scripts only).
- **Do A2 + A4 as one deliberate migration** (~3 h) with the settings.json rewrite and the N2
  test. Not before A1 lands, so there is only one new pattern to learn.
- **Never do B.** Write that sentence down in `docs/ENGINE_PROVENANCE.md` so nobody re-opens it in
  six months — the same courtesy we gave the "no new tokenizer" line in the density plan.

If the operator's real complaint is only "I do not want to read `llamacpp` in my logs and header",
then **A1 + A2 alone** solve 90 % of it, and A1 can ship today.
