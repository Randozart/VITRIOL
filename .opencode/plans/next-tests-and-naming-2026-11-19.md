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

---

# RESULTS — T1 measured, and the hypothesis it refuted (2026-11-19)

## Numbers

Same six-turn fixture (`preserve-history` shape: six seeded `<think>` blocks, the fact in turn 1,
the question at the end), live 27B, `cache_prompt=false` so `usage.prompt_tokens` is the real
prompt length, not the cache delta:

| arm | prompt tokens | answer | recall |
|---|---|---|---|
| default (preserve **on**) | **583** | "61" | correct |
| `preserve_reasoning: false` | **158** | "61" | correct |
| `preserve_thinking: false` | **158** | "61" | correct |

**−73 % prompt tokens, no recall loss.** Well past the plan's >30 % kill threshold, so the default
flips. Scaling: each seeded reasoning block is ~71 tokens; a 20-turn session with 1k-token thinking
per turn carries ~20k extra prompt tokens on **every** request — at depth (~150 tok/s prefill) that
is minutes per turn spent re-reading the model's own thinking.

## The hypothesis that died

First reading of the source said `--no-reasoning-preserve` was **inert**: the flag writes
`default_template_kwargs["preserve_reasoning"]` (common/arg.cpp:3763) while the template tests
`preserve_thinking` (2 occurrences, `preserve_reasoning` 0). The template is `chat-template-tolerant.j2`
(our file, `ct=` in the fingerprint).

Wrong. `common/jinja/caps.cpp:22` bridges the names:

```cpp
void caps_apply_preserve_reasoning(jinja::context & ctx, bool enabled) {
    ctx.set_val("preserve_thinking", mk_val<value_bool>(enabled));
    ctx.set_val("clear_thinking", ...); ctx.set_val("truncate_history_thinking", ...); ...
```

A request with `preserve_reasoning:false` also lands at 158 tokens — measured, not assumed. Had I
"fixed" the template to test `preserve_reasoning`, I would have broken a working binding.

**Lesson for the record**: a name that appears in a template and a name that appears in a flag are
not evidence that they are unrelated — grep the bridge before declaring a flag dead.

## What shipped

- `density-bench`: `Shape.history` (seeded multi-turn), `--template-kwargs` (a template variable as
  an arm, no restart), `--think-budget` (bounded thinking), `Shape.expect_exact` + `exact_answer()`
  (T2's guard). 30 tests, `baseline_parity` still byte-exact.
- Launcher: `preserve_reasoning = 0` default, `--preserve-reasoning 0|1`, fingerprint token `pr=`,
  refuses anything but 0/1. Always passed explicitly (`--no-reasoning-preserve`), because the
  engine's own default is the expensive one.
- Live config records `preserve_reasoning = 0` with the measurement inline.

Verified: default → `pr=0`, `--preserve-reasoning 1` → `pr=1`, value `7` → refuses.

## Open

The live engine still preserves until its next restart (the running binary predates the launcher
change; the flag is argv, not a compile-time thing, so the next `systemctl restart` applies it).

---

# RESULTS — T3 reframed: the eager floor is a relief valve, not a knob (2026-11-19)

The plan's T3 (`parallel=2`) would have produced another null for a new reason. Reframed to the
knob an operator could actually touch, and then read from the source rather than measured blind.

## What the two eviction paths actually are

| path | trigger | selection | threshold |
|---|---|---|---|
| `evict_sparse()` | `find_slot()` **fails** (llama-kv-cache.cpp:871) | globally lowest scores, `n_evict` of them | none — pure fallback |
| `vitriol_sweep_stream()` | floor > 0, called per stream | **per-cell** `score < floor_sc`, outside `pos_max - protect`, first 4 sinks immune | absolute score |

Live probe: `POST /kv-policy {"floor":0.5,"protect":2048,"sweep":true}` →
`{"floor":0.5,"protect":2048,"swept":0,"resident":1921,"buffer_bytes":4831838208}`.

**Swept 0 with 1921 resident cells** — because every resident cell sat inside the 2048-position
protect window. The floor did nothing, and could not have: a session shallower than `protect` has
no sweepable cells at all. Restored to `floor=0.0` immediately.

## Why a literal floor value is not portable

`floor_sc` is compared against `score_get(i)` — probe attention scores, stored **unnormalised**.
The *demand-driven* path knows this and normalises by observation (it tracks `sc_min`/`sc_max` over
the candidates, llama-kv-cache.cpp:3130-3138). The eager floor does not: it trusts an absolute
number. So `floor = 0.5` means "half of whatever the probe happened to write on this model at this
depth", and a value tuned on one model can silently sweep everything or nothing on another.

## Verdict

- **Do not arm the eager floor on the daily driver.** It is a governor relief valve for the
  `Relieved` rung (where losing prompt-cache locality beats losing the tenant), not a per-turn knob.
- **Do not test it on an idle box.** On an idle box the honest answer is "nothing to relieve", and
  the measurement is a null that says more about the test than the code.
- If it is ever to be tuned, the floor should be expressed **relative** to the observed score
  distribution (the way `evict_sparse` already does it), not as a literal. That is a small,
  well-scoped change with a test that does not need a GPU.

The E6 conclusion stands and is now sharper: *eviction is a slot-overflow fallback; the eager floor
is a pressure-relief fallback; neither is a tuning knob.*

## T4 note (running)

`--reason-lang en|zh` appends a deliberation-language instruction to the system message, leaving the
task byte-identical. Both arms at the measured budget floor (256). Expected honest null; recorded so
it stops being an open question.

---

# RESULTS — T4: Chinese vs English CoT (2026-11-19)

Bounded at the measured budget floor (256), `--reason-lang en|zh`, 3 repeats each,
both arms on the same two hard exact-answer tasks.

| lang | shape | exact | reason chars | out tok | decode |
|---|---|---|---|---|---|
| en | exact-hard-chain | 3/3 | 830 | 260 | 31.4 |
| en | exact-hard-mul   | 3/3 | 342 | 266 | 33.9 |
| zh | exact-hard-chain | 3/3 | 894 | 260 | 32.2 |
| zh | exact-hard-mul   | 3/3 | 470 | 266 | 33.1 |

**Honest null, as expected.** Both languages get every task right at budget 256.
Chinese deliberates in *more* characters (~8–40 % more thinking) for no accuracy
gain and the same output length. So on this model, "think in Chinese" is not a
lever for accuracy or economy — it is at best a tie, at worst more thinking
tokens to spend. Recorded so the open question closes: language of thought does
not change the answer here.

The useful finding is the budget one: **256 is enough for both languages**, which
re-confirms T2's floor. If Chinese thought were *sharper* it would show up as
fewer thinking chars at the same accuracy — it does not.

---

# LIVE VERIFICATION (2026-11-19 13:50) — everything shipped, one restart

Engine restarted via the unit. Evidence, in order:

- **Fingerprint journal** (`~/.vitriol/fingerprint.log`): `... spec=mtp,ngram-simple:2 par=1 ssp=1
  pr=0 cram=16384 ...` — the new preserve token rides the fingerprint.
- **Server argv** (`/proc/<MainPID>/cmdline`): `--reasoning off --no-reasoning-preserve` — the
  default flip is in the actual process, not just the config.
- **Metric mirror**: `/metrics` now carries BOTH
  `llamacpp:kv_ejected_total 0` and `vitriol:kv_ejected_total 0`.
- **Reasoning hygiene**: `reasoning_forced_total 0`, `reasoning_unbounded_total 0` (must stay 0).
- `/health` → `{"status":"ok"}`.

Provider alias: `loadProviders` provably unifies the engine spellings
(vitriol IS llamacpp, same object) — pinned by test; the probe fires once per
endpoint; a swap re-registers under both spellings with the fresh window.

The operator's user override (`~/.config/little-coder/models.json`, a bare
`llamacpp` entry) resolves through the alias — nothing orphaned. TUI rebuilt and
installed (b2a1162); restart Officina to pick up both the provider rename and the
flicker fixes.

---

# ADDENDUM — the "Cannot read properties of undefined (reading 'runtime')" errors (2026-11-19 13:18–13:20)

Operator-visible in a second Officina session (grind loop, agnes/agnes-3.0-flash lane, 429-limited).

## What it is NOT

- **Not our extension code.** Exhaustive sweep: zero `.runtime` property reads anywhere in
  `.pi/extensions/**` (production or test). The only receivers of `.runtime` in pi itself are
  `this.runtime` (ExtensionRunner, always set in its constructor) and `extensionsResult.runtime`
  (always produced by `loadExtensionsInternal`, which fabricates a runtime when none is passed).
- **Not the A2 mid-edit state.** The failing turns ran 13:18–13:20; the first A2 file
  (`engine-provider.ts`) landed 13:22, `index.ts` 13:45. The errors predate the rename work.
- **Not the engine.** The lane is caduceus/omniroute (cloud), not the local engine; the restart
  at 13:50 is unrelated to a 13:18 failure.

## What it most plausibly is

The failing session's TUI binary is **deleted on disk** (`/proc/779637/exe → ~/.local/bin/officina
(deleted)`) — it predates today's rebuilds. Combined with pi's extension cache being keyed by cwd
and the extension tree changing under it, a respawned pi could load a mixed old/new extension set.
That is a transient of shipping work under a running session, not a standing defect.

## Fix for the operator

**Restart the second Officina instance.** It will pick up the new TUI binary (b2a1162), the
consistent extension tree, and the renamed provider. If the error recurs on a fresh binary with a
stable tree, it is a real bug — capture `OFFICINA_BAND_DEBUG`-style evidence (the failing session's
exact model/route and a stack) and reopen.

The 429 itself is the free-lane quota (agnes, 1 req / 3 min) — expected behaviour of a free tier,
handled by the existing cooldown logic; not a defect either.
