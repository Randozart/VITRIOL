# Reasoning is not broken — it is unbounded and undeclared (2026-11-19)

Investigation for the E5 question: *why does "reasoning" keep working badly?*

Short answer: nothing in our stack ever sets a thinking budget, and the engine's budget
sampler is **not even constructed** unless someone does. Every client that turns thinking on
therefore gets an unlimited thinking stream sharing `n_predict` with the answer. When the answer
is short the content comes back empty, and the historical remedy — turn reasoning off — is applied
again. The bug is not in the model, the template, or the parser. It is a missing number.

---

## 1. What "reasoning" is in our stack (four independent switches)

| # | Switch | Where | Scope | Live value |
|---|---|---|---|---|
| 1 | `reasoning = off` in config | `~/.vitriol/config:71` → `scripts/vitriol:2685` → `--reasoning off` | **load time** | `off` |
| 2 | template's `enable_thinking` | `common/arg.cpp:3715-3721`, consumed at `tools/server/server-context.cpp:1704-1717`, baked into `chat_params` at 1737 | **load time**, constant | `false` |
| 3 | request `chat_template_kwargs.enable_thinking` | `tools/server/server-common.cpp:1308-1321` | **per request** (works) | unused by every client except `rebis_shim.py:881` |
| 4 | `reasoning_budget_tokens` | flag `--reasoning-budget` (`arg.cpp:3741`), sampler `common/reasoning-budget.cpp` | per request *and* CLI | **`-1` everywhere** |

Switch 1 sets 2. Switch 3 can override 2 per request. Switch 4 is the only thing that stops
thinking from consuming the answer — and it is unset in all four places a user could set it:
no config key, no launcher flag, no profile, no request.

The live `--reasoning off` therefore means: the template emits `<think>\n\n</think>\n\n`
(pre-closed, `server-context.cpp` + the live `/props` template), and the model cannot think at all.
That is why E5 could not be run: there is no reasoning to compress.

## 2. The failure chain, verified

Turning thinking on (`--reasoning on`, or per-request `enable_thinking: true`) produces this:

1. **The template sets maximum effort by default.** Live `/props` template:
   ```jinja
   {%- if enable_thinking is undefined or enable_thinking is true %}
     {%- set resolved_reasoning_effort = reasoning_effort|default('xhigh') %}
     ... 'Reasoning effort is set to xhigh. Please think carefully through the task,
          validate key assumptions, consider plausible alternatives ...'
   ```
   Nothing in our stack ever sends `reasoning_effort`, so **every** thinking turn is instructed to
   think maximally, including one-line classifications.

2. **The budget is unlimited and the sampler is dormant.** `sampling.cpp:311-323`:
   ```cpp
   if (!start.empty() && !end.empty() &&
       (params.grammar_lazy || params.reasoning_budget_tokens >= 0 || params.reasoning_control)) {
       rbudget = common_reasoning_budget_init(...);
   }
   ```
   With `-1` (the default) and no `reasoning_control` and no lazy grammar, **no sampler is created**.
   There is nothing that can ever force `</think>`. The safety mechanism exists
   (`common/reasoning-budget.cpp`, logit-forcing, 310 lines, with a mid-generation
   `reasoning_end` control endpoint at `server-context.cpp:2750`) and is switched off by default.

3. **`n_predict` is shared.** Thinking tokens and answer tokens come from the same budget. A lane
   that asks for 512–4096 tokens and gets an xhigh-effort thinker returns `finish_reason: length`
   with `content: ""` and a full `reasoning_content`.

4. **The caller sees "the model produced nothing".** Documented in three separate places:
   - `context-relay/relay.ts:104` — `msg?.content || msg?.reasoning_content` (a workaround that
     only exists because content can be empty).
   - E6 sweep, 2026-11-19: every arm reported `RECALL LOST` until `--reasoning off` was added.
   - `rebis_shim.py:877-881` — comment says *"budget-capped server-side"*, code sends
     `enable_thinking: True` and a 4096 `max_tokens`, and sends no budget. The comment describes
     an intention the request does not implement. Its own neighbouring note — *"forcing a high
     floor turned agent turns into multi-minute marathons"* — is this bug measured.

5. **The remedy is global and cheap**, so it is applied and the cycle repeats: set
   `reasoning = off`, restart, symptom gone. Six weeks later someone needs a reasoning lane,
   flips the global switch, and meets step 3 again.

## 3. Why previous diagnoses did not stick

- **2026-09-22** — "content trapped in `reasoning_content`" ⇒ remedy `--reasoning off`. Correct
  observation, wrong lever: it disabled the capability instead of bounding it.
- **2026-09-26** (`gateb-close-perf`, session 5) — proved the `--reasoning off` *cliff* fixes were
  **length coincidences** (raw `/completion` probes: plain20 DEAD, `<think>`-open20 DEAD,
  `<think>`-preclosed20 DEAD). Right conclusion, but it was read as "reasoning is irrelevant",
  when the transferable lesson was "reasoning changes the token stream, and every past `fix` was a
  side effect on length".
- **2026-10-11** (`fastmtp-d2t`) — "bare launches without `--reasoning off` silently corrupt raw
  `/completion` at depth (instant eos)". Recorded as a launch discipline note rather than a bug.
  **Hypothesis H3 (unverified):** if that path arms anything thinking-related, the unbounded
  stream is a plausible cause; the cheap test is in §6. This is the one claim in this document I
  have not verified in code.

## 4. The client-side lie

`officina/.pi/extensions/llama-cpp-provider/config.ts:119` — every local model defaults to
`reasoning: false`. pi then:
- skips `clampThinkingBudgetToAnswerRoom` / `thinkingBudgetForLevel` (they gate on
  `model.reasoning`),
- skips `assistantMsg.reasoning_content = ""` for `requiresReasoningContentOnAssistantMessages`,
- still *parses* `reasoning_content` deltas into thinking blocks (that path is unconditional), so the
  UI can show thinking while the model entry claims there is none.

The engine and the client disagree about the same capability. Neither lie is load-bearing today
(reasoning is off), but both become bugs the moment a lane asks for thinking.

## 5. The fix is a number, not a redesign

Three properties, in dependency order:

- **I1 (parity):** with `reasoning = off`, every request is byte-identical to today, and the
  rbudget sampler stays absent.
- **I2 (bounded):** with thinking enabled, thinking tokens are ≤ `reasoning_budget`; the sampler is
  armed; `</think>` is forced at the cap and the answer still gets the remaining room.
- **I3 (declared):** the client's declared capability equals the engine's; a lane that displays
  thinking is a lane whose model entry says `reasoning: true`.
- **I4 (visible):** a turn that spent N thinking tokens and returned empty content is a *named*
  condition in metrics and in the TUI, never a silent blank reply.

### Stages (each independently shippable, deploy order)

**S1 — instrument before changing anything** (no behaviour change)
- Engine: `/metrics` counters `llamacpp:reasoning_tokens_total`, `reasoning_budget_hits_total`,
  `reasoning_ended_at_cap_total`; log the armed state at request start (`SRV_TRC` with budget and
  effort). Gate: existing tests; a fresh launch logs `rbudget=off` under `reasoning = off` (I1).
- Officina: TUI ctx/activity row shows `think N` when a turn produced reasoning, and a
  `⚠ empty content (finish=length)` notice — turns the recurring bug into something the operator
  sees instead of a blank turn.
- density-bench: record `reasoning_chars` and `finish_reason` per sample (the E6 harness already
  proved a guard without evidence is unfalsifiable).

**S2 — make the budget a config key and a request field**
- `[server] reasoning_budget = <int>` → `--reasoning-budget ${N}`; fingerprint `rb=N`
  (AGENTS §4: speed-bearing keys ride the fingerprint).
- Launcher rule: `reasoning = off` ⇒ `rb=0` semantics (sampler may be armed at 0, equivalent to
  "no thinking", I1 preserved by the template's pre-closed block).
- Launcher rule: `reasoning = on` **refuses** to start with an unset budget. A thinking engine
  without a cap is the bug; make it unlaunchable rather than silently harmful.
- Provider (`llama-cpp-provider`): send `reasoning_budget_tokens = min(configured, answer_room - 512)`
  per request, mirroring pi's `clampThinkingBudgetToAnswerRoom` intent for the local engine.
  Tests: `answer_room` clamp both directions; a request never sends a budget larger than `maxTokens`.

**S3 — stop lying about the capability**
- `fillModelDefaults` derives `reasoning` from the engine fingerprint (`spec=`/`rb=`), not a
  hardcoded `false`. Evidence source: the already-parsed `VITRIOL-FINGERPRINT` line, so there is
  one owner of the fact.
- Test: fingerprint says `rb=-1` ⇒ provider reports `reasoning: false` **and** refuses to send
  `enable_thinking` (the two must move together).

**S4 — per-lane thinking without a restart**
- `chat_template_kwargs.enable_thinking` already works per request (`server-common.cpp:1315`), so a
  marker `[[officina-think]]` / `[[officina-nothink]]` in `_shared/` can turn thinking on for one
  lane while the engine stays `reasoning = off`. Always paired with S2's budget — a marker that
  enables thinking without a cap is the same bug with better ergonomics.
- Test: marker sets both `enable_thinking` and a finite `reasoning_budget_tokens`, or neither.

**S5 — retract or confirm the launch-discipline note**
- Re-run the 2026-10-11 bare-launch depth probe with `--reasoning-budget 0` and with the sampler
  armed-but-large. If corruption vanishes under both, the note was a side effect and gets
  corrected in `fastmtp-d2t-results-2026-10-11.md`; if it persists, H3 is dead and the note stands.
  Either way the doctrine becomes a measurement instead of a ritual.

### Then, and only then, E5

E5 (Chinese CoT) is measurable exactly once the budget exists: thinking on, bounded, both arms
decode the same question. Until S2, any E5 number is a function of an unbounded stream and cannot
be reproduced.

## 6. Verification, honestly

- **Cheap, decisive, read-only-ish:** one request to the live engine with
  `chat_template_kwargs: {enable_thinking: true}`, `max_tokens: 64`, and no budget — expect
  `finish_reason: "length"`, `content: ""`, non-empty `reasoning_content`. Then the same request
  with `reasoning_budget_tokens: 32` — expect a forced `</think>` and non-empty content. Two calls,
  no restart. **Not run: this session is in PLAN mode.**
- The two-request probe is the acceptance test for S2 and the regression test for the whole
  document.
- Negative control: with `reasoning = off` unchanged, the same probe must show no reasoning field
  and no behaviour change (I1).

## 7. Non-goals

- Not making the daily driver think. The driver wants speed; 25 tok/s with an xhigh thinker is a
  regression, so `reasoning = off` stays the default.
- Not touching `reasoning_effort` defaults in the model's own template (BuffedMod provenance).
  If an effort is ever wanted, it is sent per request, never baked into the GGUF.
- Not re-litigating the 09-26 cliff. That was a real SSM/conv-state issue and it is separate.

## 8. Invariants (review checklist)

- I1 parity: `reasoning = off` ⇒ identical requests, sampler absent.
- I2 bounded: thinking ≤ budget; `</think>` forced at the cap.
- I3 declared: provider `reasoning` flag == engine `rb`/`--reasoning` state.
- I4 visible: empty-content-with-reasoning is named in metrics + UI, never silent.
- I5 provenance: `rb=` rides `VITRIOL-FINGERPRINT`; a launch with thinking but no cap fails loudly.

## 9. Effort

| stage | effort | risk |
|---|---|---|
| S1 instruments | low | none (no behaviour change) |
| S2 budget key + clamp | medium | must prove I1 with the driver's own benches |
| S3 capability truth | low–medium | touches model discovery; test both directions |
| S4 per-lane marker | low | reuses the E1/E2 marker pattern |
| S5 retraction test | medium (engine restarts) | none, research only |

S1 alone would have made every historical instance of this bug self-diagnosing. That is the
argument for doing it first, and for not touching the budget until it is in.
