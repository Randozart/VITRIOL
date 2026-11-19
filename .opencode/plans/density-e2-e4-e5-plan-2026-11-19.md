# Density experiments E2 / E4 / E5 — plan

**2026-11-19.** Follows `token-density-experiments-2026-11-18.md` (E1 adopted, E6 a negative
result). The box is available; the goal is to convert the three remaining *config-level*
experiments into measured verdicts, each with a guard that makes a "win" falsifiable.

## 0. Where we actually are (verified today)

| item | state |
|---|---|
| E1 grammar lanes | **ADOPT** — nested schema −41.8 % tokens and 0/3 → 3/3 validity; flat schema +13.4 % (null). Shipped behind `OFFICINA_STRUCTURED_OUTPUT`. |
| E6 eviction sweep | **INERT at `parallel=1`** — `kv_ejected_total = 0` in every arm; admission control (`server-context.cpp:3658`) prevents a request from filling the window, and `evict_sparse` only runs when `find_slot` fails. Multi-session knob, not a depth knob. |
| `cache_ram` | **live** — fresh fingerprint carries `cram=16384`, `cmax=40`, `cxr=1`, `c=262144`, `spec=mtp:2`, `score=probe`. |
| Engine | `vitriol-server` on `:8279`, Qwen3.8-27B BuffedMod Q6K, MTP n=2, `--reasoning off`, 10.5/10.9 GiB VRAM. |

The pending "restart for `cache_ram`" item is therefore **closed** — it happened during E6 and is
visible in the fingerprint.

## 1. What the code says about how to run these

Read from `density-bench/src/`:

- **Arms are request-shaped, not engine-shaped.** `Client::body()` branches on the arm *string*
  (`arm == "grammar"` → attach `response_format`). Everything else about an arm is invisible to it.
- **Engine variation lives in `sweep`**, which spawns one engine per `Arm { flags, env }` and then
  calls `client.run(shape, &arm.label())` — **passing the engine label as the request arm**.
  Consequence: inside a sweep, `"grammar"` never matches, so *no* engine sweep can currently
  combine with a request style.

That is the concrete blocker for **E4 (MTP × grammar)**: the cross-product is inexpressible today.
It is also a latent bug: `density-bench sweep … --arm grammar` silently measures a baseline arm.

**Design fix (prerequisite, ~1 h):** replace the arm string with a struct.

```rust
#[derive(Clone, Debug, PartialEq)]
pub struct ArmSpec {
    pub label: String,          // printed + recorded in the artifact
    pub style: RequestStyle,    // None | Grammar | Telegraphic | ChineseCot
    pub env: Vec<(String, String)>,
    pub flags: Vec<(String, String)>,
}
```

`Client::body(shape, style)` and `body(shape, engine_env)` then stop colliding, and
`sweep` composes: `style × flags × env`. Non-sweep mode builds `RequestStyle` from `--arm`.

## 2. Invariants (each with a test that pins it)

- **I1 — parity.** With no `--arm`, no `--set`, no `--env`, the request bodies and the report are
  byte-identical to today. *Test:* golden body JSON for `baseline` + a one-arm sweep golden table.
- **I2 — styles never alter schemas.** A style may change instructions and `response_format`, never
  the declared schema of a shape. *Test:* for every shape × style, `schema` equals the shape's.
- **I3 — sweep label fidelity.** An arm's recorded label names the engine knobs *and* the request
  style; a sweep with `--arm grammar` attaches `response_format` (regression for the bug above).
  *Test:* stub engine (the `python3` fake used for the sweep integration check) asserts the body.
- **I4 — compression must decode.** A telegraphic/codebook answer scores **zero** unless the
  harness can expand it and then satisfy the same validity/recall check the baseline faced.
  *Test:* an unexpandable symbol fails; a correct compression passes; a compression with the *right*
  answer in the *wrong* codec fails.
- **I5 — metrics stay engine-reported.** No arm may compute its own token counts.

## 3. The experiments

### E2 — telegraphic + codebook (the only one that touches *output* tokens)

**Hypothesis:** internal lanes can emit a compact notation (`⇒` causality, `¬` negation, `∴`
conclusion, `+`/`-` for add/remove) with a one-time legend, spending ~35–50 % fewer output tokens
while remaining machine-decodable.

**Method:**
- One new shape family `internal-plan` (the realistic consumer: a plan/finding the operator reads
  via a rendered view, not verbatim).
- Arms: `baseline`, `telegraphic` (prose compressed, no legend), `codebook` (legend + symbols).
- Decode step in the harness: `codebook::expand()` maps the legend back to phrases; the **decoded**
  text must (a) contain the shape's `expect` substring and (b) parse as the shape's schema when one
  is declared.
- Artifact records `raw_tokens`, `expanded_tokens`, `decode_ok`, `expanded_head`.

**Guard:** I4. **Kill if:** decode fails on any repeat, or expanded length ≥ baseline (compression
that costs a decode step and saves nothing is a loss).

**Location:** can run against the live engine; token counts are contention-independent
(`--no-stream`).

### E4 — MTP × grammar (cheap, answers a standing config question)

**Hypothesis:** the `draft_n_max=2` choice was certified before structured output existed; a JSON
grammar may raise or lower draft acceptance.

**Method:** `sweep` over `--set spec-draft-n-max=0,1,2,3` × style `{baseline, grammar}` on the
`structured-nested` shape (the one where grammar actually pays). Metric: decode t/s (engine
reported), tokens, validity, and, if cheap, the engine's acceptance counters from `/metrics`.

**Guard:** identical shapes and `temperature 0` across arms. Flag semantics are as verified from
`vitriol-server --help`: `--spec-type` defaults to `none`, so the no-MTP arm simply **omits** the spec
flags (`--spec-type mtp --spec-draft-n-max N`); there is no `n_max = 0` arm to argue about.

**Needs:** exclusive box (arms restart the engine). ~6 loads × 30 s + 6 × ~20 s decode ≈ 6 min.

**Kill if:** no arm beats the live point by more than the run-to-run spread.

### E5 — Chinese CoT (close the question)

**Hypothesis:** Qwen-family models emit fewer tokens reasoning in Chinese, so "think in Chinese,
answer in English" raises throughput at equal accuracy.

**Method:** style `ChineseCot` = system instruction "reason in Chinese, then answer in English";
measure **total** output tokens (reasoning is part of the bill) and the answer's `expect` guard.
Note `--reasoning off` on the live engine suppresses a separate reasoning stream, so the arm must
either (a) run on an engine with `--reasoning on` and count both streams, or (b) be reframed as
"answer-first, Chinese internal notes inline". **Decision needed before running**: without reasoning
tokens there is nothing to compress, so E5 as written is only meaningful with reasoning enabled —
i.e. one engine restart with `--reasoning on`, or a per-request `chat_template_kwargs` equivalent.

**Kill if:** accuracy drops, or total tokens are equal.

## 4. E3 — promoted: the lookup drafter already exists in this engine

`vitriol-server --help` and `common/speculative.cpp` show **prompt-lookup / n-gram speculative
decoding is already implemented in-tree**, in five variants:

```
--spec-type  none,draft-simple,draft-eagle3,draft-mtp,draft-dflash,draft-dspark,
             ngram-simple,ngram-map-k,ngram-map-k4v,ngram-mod,ngram-cache
--spec-ngram-simple-size-n / -size-m / -min-hits     (and the parallel flags per variant)
```

The parent plan costed E3 at “1–2 weeks, port `examples/lookup`”. That was written before this was
checked. **It may be a configuration sweep**, which would make it the cheapest experiment with a
headline decode number available.

**First arm is a feasibility probe, not a measurement:**

```
density-bench sweep --engine …/vitriol-server --arg "$(live base args)" \
  --set "spec-type=draft-mtp,ngram-simple,none" --shapes deep-recall --repeats 1 --dry-run
```

then the same without `--dry-run`. Three things must be learned before any number is believed:
1. is a **combined** `--spec-type draft-mtp,ngram-simple` honoured, or does the parser keep one?
   (`common/arg.cpp` allows a list; whether the runtime uses more than one drafter at once is
   unverified — check the log line and acceptance counters, not the help text.)
2. does the n-gram drafter help on **code-edit** shapes (repetitive, lookup-friendly) and hurt on
   prose (the classic result)? Measure both rather than assuming.
3. what are the right `size-n` / `size-m` defaults for a 27B at 70k depth?

**Cost:** ~6 engine loads. **Kill if:** under the live `mtp:2` point on both shape families.

## 5. Sequence

1. **ArmSpec refactor + I1–I3 tests.** Smoke: stub-engine sweep still green; golden bodies unchanged.
2. **E3 feasibility probe** (cheap, possibly the biggest win): does `--spec-type ngram-simple` load,
   combine with `draft-mtp`, and help? One dry run, then one measured run per family. Note the
   fingerprint must carry the spec string (`spec=`), so an arm is identifiable after the fact.
3. **E2 codebook**: `shapes::internal_plan()`, `codebook.rs` (legend + `expand`), `decode_ok` in
   `Sample`, tests I4. Measure 3 repeats, commit artifact.
4. **E4**: flag-set arms + style cross-product; run with the box; record acceptance numbers.
5. **E5**: needs the `--reasoning` decision from §3; run last, cheapest to skip.
6. Update the parent plan's status table and this file's results section; one commit per experiment.

## 6. Gates

`cargo test` (bench), `cargo clippy --quiet` with zero warnings, `cargo fmt --check`, `tsc`/`vitest`
untouched (no TS changes expected), then push. Every experiment commits its JSON artifact under
`experiments/` with engine fingerprint embedded — an arm without a fingerprint is not evidence.

## 7. Language boundary — what should be Rust (asked 2026-11-19)

Short answer: the work in this plan is already Rust, and one adjacent duplicate should be folded into
Rust rather than ported. The pieces that *look* portable are the ones that cannot move.

**Cannot move — pi extensions are TypeScript by construction.** pi loads every `-e` extension through
jiti into its own process (`loader.js:325-332`); the hooks this work depends on (`onPayload`,
`tool_call`, `agent_end`) exist only inside that runtime. So `_shared/structured-output.ts` stays TS.
A Rust rewrite would mean spawning a process per request to compute what is currently a string match
and a JSON splice — slower, and it would need its own IPC contract.

**Already Rust, no action:** `density-bench` (all five modules: shapes, engine, sample, sweep, main).

**Correction (2026-11-19, after checking instead of assuming): `sweep_controller.py` stays.**
It looks like a duplicate of `density-bench sweep` — same shape (start one engine per config, poll
`/health`, benchmark, tabulate) — but it answers a different question (**placement**: pin/MTP/ubatch/
tensor-split → CSV) and, decisively, **`vitriol-tui` shells out to it**
(`vitriol-tui/src/control.rs:229`, the dashboard's placement-sweep action). Deleting it would break
that button silently. Its header now states the split: placement here, token density in
`density-bench`. It remains Python's one legitimate foothold (plus the launcher's own helper).

**Codec (E2) rule — one implementation, one fixture.** Write the telegraphic codec in Rust inside
`density-bench`. If it later ships to Officina, do **not** re-implement from the prose: export golden
vectors (`input → tokens → expanded text`) to a shared JSON file and pin both implementations against
it, exactly as `officina/theme/palette.json` plus the cross-crate parity gates already prevent theme
drift. Two hand-written implementations of a codec is how a "compression" turns into a silent
corruption.

**Deliberately out of scope:** `scripts/vitriol` (3,612 lines of bash, fingerprint-critical) and
`officina/scripts/gen-desktop-theme.mjs` (361 lines, 4 passing tests). Rewriting either buys nothing
for this work and carries real blast radius.

## 8. E3 RESULT (2026-11-19) — lookup drafting was already there, and it is fast

**Verdict: ADOPT.** `--spec-type ngram-simple` is a configuration flag, not a port, and on
lookup-friendly content it is **5.5x** the daily driver at zero cost elsewhere.

Measured with `density-bench sweep`, one engine per variant, identical base args (live config,
`-c 262144`, `--reasoning off`), `temperature 0`, `--no-stream`, 2 repeats, medians:

| shape | none | `mtp:2` (was live) | `mtp:2 + ngram-simple` | `ngram-simple` |
|---|---|---|---|---|
| repeat-echo (verbatim reuse) | 19.3 | 35.6 | **110.6** | **115.0** |
| code-edit | 19.7 | 29.9 | 30.3 | 19.4 |
| deep-recall (prose) | 20.0 | 31.3 | 32.5 | 20.0 |

Reading it:
- the n-gram drafter pays **only** where the answer reuses prompt text — exactly as advertised,
  and it is 3x better than MTP there because it proposes whole verbatim spans rather than one token;
- on prose it is a no-op, not a regression (19.4-20.0 vs 19.3-20.0);
- **combined is the right general setting**: MTP still carries the non-repetitive case (30.3 vs 19.4)
  while ngram carries the repetitive one (110.6 vs 35.6). The two do not fight; combined was within
  noise of the best arm on every shape.

All five n-gram variants were measured (medians, repeat-echo): `ngram-simple` **115.0**,
`ngram-mod` 86.3, `ngram-cache` 50.4, `ngram-map-k` 44.5, `ngram-map-k4v` 33.1 — so the default
`ngram-simple` is the right pick; no tuning was needed.

**Applied:** `~/.vitriol/config` `[spec] type = mtp` -> `mtp,ngram-simple` (backup kept), unit
restarted, fingerprint now reads `spec=mtp,ngram-simple:2`, and a live measurement on the daily
driver shows repeat-echo at **103.7 / 110.4 tok/s** (was 19.3). Evidence:
`experiments/density-e3-spec-variants-2026-11-19.log`, `…-ngram-types-…log`,
`experiments/density-e3-live-2026-11-19.json`.

Two engine observations, not fixed here:
1. `common/speculative.cpp` registers `"mtp"` **twice** in the type table (lines 41 and 44) — a
   duplicate alias entry; harmless under last-wins but it is a typo waiting to become a bug.
2. When `draft-mtp` is present, an `ngram-*` type in the same list contributes nothing to
   *decoding* on its own terms — it is the draft context that logs; the ngram impl was built but
   the MTP acceptance dominates. The combined arm's win comes from ngram filling the gaps, which
   is why it must be listed *and* measured, never assumed.

Also found and fixed while running E3: `sweep` never wrote `--json`, so the corroboration run above
produced no artifact. `write_artifact` is now shared by both paths.

## 9. E4 RESULT (2026-11-19) — grammar and speculative decoding coexist

Ran on the **live** engine (now `spec=mtp,ngram-simple:2`), `temperature 0`, 2 repeats, artifact
`experiments/density-e4-mtp-grammar-2026-11-19.json`:

| shape | arm | decode tok/s | out tokens | validity |
|---|---|---|---|---|
| structured | baseline | 29.15 | 112 | 2/2 |
| structured | grammar | 24.91 | 127 | 2/2 |
| structured-nested | baseline | 32.63 | 533 | 0/2 |
| structured-nested | grammar | 31.60 | 310 | 2/2 |
| code-edit | baseline | 29.94 | 328 | - |
| code-edit | grammar | 29.83 | 328 | - |

Findings:
- **Spec decoding survives the grammar.** Decode stays at 29-33 tok/s, which is the MTP/ngram range;
  a grammar that disabled drafting would have collapsed to ~19 tok/s (measured `none` in E3).
- **E1 reproduces exactly** under the new spec config: -41.8 % on the nested schema with 0/2 -> 2/2
  validity, +13.4 % on the flat schema. The E1 adoption decision is unchanged by `ngram-simple`.
- One observation worth keeping: the flat-schema grammar arm decoded ~15 % slower (24.9 vs 29.2)
  while producing *more* tokens — consistent with constrained-sampling overhead on an
  already-compliant schema. 2 repeats cannot separate that from noise; it is a reason to keep the
  feature **opt-in per prompt** (as shipped), not to change it.
- No `draft acceptance` counters reached the journal for these runs (the per-task timing lines stay
  in the engine's own log), so "does grammar change acceptance?" is answered indirectly by the decode
  rate, not by acceptance numbers. If that matters later, run the arms as a `density-bench sweep`
  (which captures the engine's stdout) rather than against the live unit.

**Verdict:** no change to `draft_n_max`. The `mtp:2` choice remains valid under structured lanes.
