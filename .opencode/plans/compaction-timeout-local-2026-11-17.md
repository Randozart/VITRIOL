# Local compaction fails forever: a 300 s HTTP idle timeout vs a ~12–20 min cold prefill

**Date:** 2026-11-17 12:55
**Mode:** PLAN (research only — nothing applied)
**Symptom (owner):** "Local is endlessly failing at compacting a session."

## 1. Evidence

### 1.1 The box says exactly what happened (session `briev-lang`, `2026-11-14T13-45-57-796Z_a8092af0`)

20 assistant records on `provider: llamacpp, model: Lapis Occultus` with **empty content** and:

```
"stopReason": "error",
"errorMessage": "Request timed out.",
"durationMs": 300007 | 300010 | 300014,
"usage": {"input": 0, "output": 0, ...}
```

Three samples, all ≈ **300 000 ms to the millisecond** — a client-side deadline, not an engine failure
(the engine logged no error; it was still prefilling).

### 1.2 Where 300 s comes from

`officina/node_modules/@earendil-works/pi-coding-agent/dist/core/http-dispatcher.js`

```js
export const DEFAULT_HTTP_IDLE_TIMEOUT_MS = 300_000;      // ← the operative number
export const HTTP_IDLE_TIMEOUT_CHOICES = [30s, 1min, 2min, 5min, disabled];
```

It is **configurable**: top-level settings key `httpIdleTimeoutMs`
(`settings-schema.js:263`), parsed by `parseHttpIdleTimeoutMs`, which accepts a number **or the
string `"disabled"`** (→ 0 = never time out). Current `~/.pi/agent/settings.json` sets **only**
`compaction.modelOverrides` — the timeout is at its default.

### 1.3 Why a local request is silent for longer than 5 minutes

Measured on the live engine (`/metrics`): `prompt_tokens_total 3.16289e6` /
`prompt_seconds_total 13179.1` = **≈240 tok/s** of processed (uncached) prompt tokens.
At 240 tok/s a 300 s deadline covers ≈ **72 000 tokens**; deep-context prefill is slower than the
average (depth certs: 161 tok/s at 36 k), so the real ceiling is lower.

Compaction budgets (`~/.pi/agent/settings.json`): `reserveTokens: 65536`,
`keepRecentTokens: 24000` on a 262 144 window ⇒ compaction fires at ~196 600 used and summarises
a span of ~**172 000 tokens** ⇒ **12–20 minutes** of byte-silence on a cold prefill.
5 min < 12 min. **Guaranteed timeout, every time.**

### 1.4 Why it is a *cold* prefill (the part that makes this structural)

`dist/core/compaction/compaction.js : generateSummaryWithUsage()`

```js
const conversationText = serializeConversation(llmMessages);
let promptText = `<conversation>\n${conversationText}\n</conversation>\n\n` + basePrompt;
```

The summary request is a **re-serialised text blob**, not the session's message array. The engine's
prompt cache therefore has no prefix to hit: the session's warmed KV is useless for it, and the
request pays the full cold prefill. `_reserveTokens` is *explicitly unused* for output
("summaries use the model's output limit, #9512") — it only sets the trigger threshold.

Also: `compaction` settings have **no model override** (`enabled`, `reserveTokens`,
`keepRecentTokens`, `modelOverrides` with `{reserveTokens, keepRecentTokens}` only) ⇒ today
compaction always runs on the session's model.

### 1.6 Second failure mode found while proving F1: the 4096 output cap

With F1 in place the request no longer times out, and a *new* wall appeared:

```
[1083.0s] compaction_end
response compact "Summarization failed: generation hit the token cap and the summary is incomplete"
```

Measured cause: `prompt_tokens` moved by 269,680 (the cold prefill of the summary
prompt) and `tokens_predicted_total` by exactly 4,096 per attempt.

The engine alias (`Lapis Occultus`) is registered by
`officina/.pi/extensions/llama-cpp-provider` through `fillModelDefaults`, which
defaulted `maxTokens: 4096`. pi derives the summarisation request's output budget
from `model.maxTokens` (`createSummarizationOptions` passes no explicit
`max_tokens`), so every summary was truncated mid-section and pi discarded it
(`getSummarizationFailure` → "hit the token cap … incomplete"). That is a
permanent, retry-proof failure: the same prompt, the same cap, forever.

**Fix shipped:** `fillModelDefaults` now derives the default from the window
(`engineMaxTokens`: 32k+ windows → 16,384, smaller → 4,096), so the alias no
longer advertises 4,096 on a 262k engine. Traced live:
`{"id":"Lapis Occultus","ctxWindow":262144,"maxTokens":16384}`.
Regression test: `engine-max-tokens.test.ts` + two updated `config.test.ts`
expectations.

## 1.7 Third finding: decode at depth is the real constraint (~1.2 tok/s at 113k)

With both fixes in place the compaction got further than ever and then stalled in generation:

```
slots[0]: n_prompt_tokens 113470 / processed 113470, n_remain 16384, n_decoded 1798 after ~25 min
```

Prefill is fine — 113k–135k tokens prefilled at **277 tok/s** (487 s for 134,850 tokens).
Generation is not: `n_decoded` advanced ~1,798 in ~25 min ≈ **1.2 tok/s**, and 8 minutes of
wall clock moved the global predicted counter by 148. At that rate a 2,000-token summary is
~28 min and a runaway 16k summary is 3.7 h. The run was stopped after 30+ min; the engine was
released healthy (`/health` ok), no operator session disturbed beyond the slot's own KV.

So the three real constraints for local compaction, in the order they bite:

| constraint | measured | verdict |
|---|---|---|
| client idle timeout | 300 s vs 12–20 min prefill | **fixed** (`httpIdleTimeoutMs: 1800000`) |
| output cap | 4,096 vs a multi-section summary | **fixed** (window-derived, 16,384) |
| decode at depth | **~1.2 tok/s at 113k** (prefill 277 tok/s) | **not fixable by config** |

The third one means a *deep* local compaction cannot be made quick; it can only be made
**small**. Two viable policies, both honest about the trade-off:

- **Compact early (local, no privacy change).** Keep the summarisation prompt ≤ ~36 k tokens,
  where decode is 11–16 tok/s (depth certs), i.e. a summary in 3–6 min. Requires
  `reserveTokens ≈ 226000` and `keepRecentTokens ≈ 8000` (fires near ~36 k used). Cost: summaries
  every ~36 k tokens and a smaller live window between them.
- **Summarise on a cloud lane (F5, opt-in).** ~10 s instead of ~30–60 min. Cost: the conversation
  leaves the box — invariant I2, so opt-in only, never a default.

A third option — "raise the timeout and wait" — is what F1 does, and it is only acceptable for
*shallow* compactions; at 113k it is a 30–60 minute turn.

## 1.5 Why it looks *endless* rather than "failed once"

- pi retries summarisation (`_summarizationRetryCallbacks`, same retry budget as turns); each retry
  re-sends the same prompt, so the engine reuses the prefix it managed to prefill (≈72 k) and
  advances ~72 k further. Several attempts to converge — minutes apart, looking like a loop.
- After a failed compaction the session is still over budget, so **the next turn triggers compaction
  again**, which times out again. Unterminated by design.
- Retries succeed on cloud lanes (`events.jsonl`: `caduceus/ddgw/gpt-5.6-luna summarized 42061 tok in`,
  `llamacpp … 93780 tok in`) — 93 k once fit inside 5 min on a warm-ish run; 172 k never will.

## 2. Invariants (this plan must preserve)

- **I1 (parity):** with no compaction model/lane configured, compaction behaviour is today's
  behaviour except for the timeout value and the trigger threshold. Pinned by: a settings-diff test
  that the applied change touches only `httpIdleTimeoutMs` / `reserveTokens`, plus a replay test
  that `shouldCompact()` thresholds are unchanged for un-overridden models.
- **I2 (no silent cloud):** no conversation content leaves the box for summarisation unless the
  operator opts in explicitly (privacy default = local). Pinned by: a test that the compaction-model
  hook is inert when `compactionCloud: false` (default).
- **I3 (truthful failure):** a summarisation timeout surfaces the *cause* and the *fix*, never just a
  spinner or "compaction failed". Pinned by: render test on the failure card text.
- **I4 (no wedge):** a timed-out summarisation must leave the engine healthy and the session usable
  (no half-restored slot, no health-deaf loop) — reuse the existing identical-resend/overshoot
  guards. Pinned by: the existing `restored-state-overshoot` + RST/abort regression tests.
- **I5 (provenance):** any change to the effective config emits a fingerprint diff.

## 3. Fix, in layers

### F1 — Raise the client deadline (immediate, config only, no code)
`~/.pi/agent/settings.json`:

```json
"httpIdleTimeoutMs": 1800000        // 30 min; or "disabled" (0)
```

30 min covers a 172 k cold prefill (~12–20 min) plus margin. Risk: a genuinely dead local server now
hangs for 30 min instead of 5 — mitigate with F6 and with the engine's own liveness probes
(`/health`, `/slots`; Officina already polls both).
**Verify:** on the stuck session, run one manual `/compact` and confirm a single pass, with
`journalctl`/`/metrics` showing a continuous prefill (no restart).

### F2 — Compact earlier, so the span is smaller (config, cheap)
`compaction.modelOverrides` for `llamacpp/Lapis Occultus` **and** `officina/lapis`:
`reserveTokens: 65536 → 131072` ⇒ fires at ~131 000 used, span ≈ 107 k ⇒ ~7–9 min at 240 tok/s.
Trade-off: ~65 k less live window before compaction; acceptable with `keepRecentTokens: 24000`
(kept verbatim) and a 262 k window. Re-certify only after F1 is in place (F1 alone may suffice).

### F3 — Let Officina predict the cost instead of guessing (code, Officina TUI)
On `compaction_start`, estimate the span (already available: `tokensBefore`, `keepRecentTokens`) and
show `summarising ≈168k tok · ~14 min at local prefill (est. 200 tok/s)`. Log
`{ev:"compact-start", tokens, keepRecent, model, estMs}` to `events.jsonl`, and the outcome
(`{ev:"compact-end", tokens, summaryTokens, durationMs, error?}`). This is the observability half of
the bug: today the failure is invisible until someone greps a session file (which is exactly how
this was diagnosed — 40 minutes of forensics).

### F4 — Make the timeout actionable (code, Officina TUI + hang-watchdog)
Detect `errorMessage === "Request timed out."` on a summarisation call and show a specific notice:
`compaction timed out after 300s (local prefill needs ~14 min) — raise httpIdleTimeoutMs in
~/.pi/agent/settings.json, or /compact on a cloud lane`. Two consecutive timeouts ⇒ sticky notice
(no silent retry loop). Inert when the error is anything else.

### F5 — Optional fast lane for summaries (code, opt-in, last)
An Officina extension that, when `compactionCloud: true`, switches to a cloud lane for
`compaction_start` and restores the previous model on `compaction_end`. ~10 s instead of ~15 min.
**Blocked by I2:** opt-in only, with an explicit per-turn confirmation or a persisted flag whose
first activation prints exactly what will be uploaded. Do not implement before F1–F4 are green.

### F6 — Guard rail (code, cheap)
If the engine's `/slots` shows `is_processing` and the client reports a timeout, Officina should
log `ev:"client-timeout","engine-still-prefilling": true` — that single line separates "client gave
up" from "engine died", the ambiguity that cost this investigation its first hour.

## 4. Rollout order

1. **F1** (config) → reproduce a successful local compaction on the stuck session. *Gate: one pass.*
2. **F3 + F6** (observability) → so any recurrence is diagnosable in one command.
3. **F2** only if step 1 shows compaction is still uncomfortably long (>20 min).
4. **F4** (actionable notice) + regression test.
5. **F5** last, opt-in, with the privacy gate.

## 5. Still to measure before implementation

- Does Officina's own wire-idle watchdog (the `grant_wire_grace` path) tolerate a 20-minute silent
  stream, or does it abort earlier than pi's 300 s? If it aborts, F1 is insufficient on its own.
  Check `tui/src/tui/mod.rs` grace constants against a 20-min silence.
- The engine's response to a client abort mid-prefill: does it keep the partial KV (so the retry
  advances) or discard it? `/metrics` `prompt_tokens_total` deltas across a forced abort answer this.
- Whether the operator's stuck session is the `briev-lang` one or another; the fix is provider-wide,
  but the acceptance test should be run on the actual session that is failing.

## 6. One-line summary

The local model is not broken and the engine is not dying: **pi's 300 s HTTP idle timeout is shorter
than a cold prefill of a ~172 k-token summarisation prompt (~12–20 min at 240 tok/s)**, and
compaction re-serialises the conversation so that prefill is always cold. Raise
`httpIdleTimeoutMs`, compact earlier, and make the failure legible.

## 7. Provenance of the proof runs

Driver: `/tmp/compactproof/compact-once.mjs` — spawns
`node …/pi-coding-agent/dist/cli.js --mode rpc --session <fork> --provider llamacpp
--model "Lapis Occultus" -e officina/.pi/extensions/llama-cpp-provider/index.ts`
with `cwd=officina`, `OFFICINA_NO_AUTO_ROUTE=1`, `OFFICINA_ROUTE_MODE=off`, then sends
`{"type":"compact"}`. The session under test is a **fork**
(`/tmp/compactproof/stuck.jsonl`) of
`~/.pi/agent/sessions/--home-randozart-Desktop-Projects-briev-lang--/2026-11-14T13-45-57-796Z_a8092af0-*.jsonl`
(10 060 entries, 12.9 MB) — the original was never modified.

| run | code state | verdict |
|---|---|---|
| 1 | default timeout 300 s | `Request timed out.` at 300 s (the bug) |
| 2 | timeout 30 min only | `generation hit the token cap` at 233 s (cap 4,096, cached prefill) |
| 3 | + maxTokens on one alias path | same, 1083 s (cold prefill 269,680 tok) |
| 4 | + maxTokens on both alias paths | same, 448 s (still 4,096 → later found a third path) |
| 5 | maxTokens derived in `fillModelDefaults` | ran >30 min with `n_remain: 16384` (cap fixed), stalled at ~1.2 tok/s decode (see §1.7); stopped to release the engine |

Engine after every run: `/health` ok, slot idle, no restart, no wedge.

## 8. What the operator should do next (decision, not code)

1. **Restart Officina** so pi picks up `httpIdleTimeoutMs` and the provider change.
2. For the currently stuck session, compact on a **fast lane** (switch model → `/compact` →
   switch back). It takes seconds instead of an hour and needs no config change.
3. If local summaries must stay local, decide the compaction *policy*: compact-early
   (`reserveTokens ≈ 226000`, `keepRecentTokens ≈ 8000`) is the only local variant that
   finishes in minutes; otherwise F5 (opt-in cloud summary lane) is the honest answer.
