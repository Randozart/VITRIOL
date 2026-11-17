# Gemini Decision as a replacement for current Officina processes — survey + plan

> PLAN (2026-11-17) — research-first, nothing modified in code. Findings only
> and a proposed build path. Sources: `officina/.pi/extensions/*`,
> `officina/scripts/*`, git history up to `cb6d4b6`.

## 1. What Gemini Decision actually is (constraint that decides everything)

`gemini-decision-1` via `POST https://generativelanguage.googleapis.com/v1beta/decisions` is
**a choice-evaluator, not a generator**: you send one structured input plus `choice`-typed
questions, and it returns **calibrated probabilities** per option with **zero output tokens**
and one prefill pass (~2.5 s default timeout).

Implications:

- It can ONLY answer "pick one of these N options," with a probability per option.
- It cannot write text, explain, summarize, diff-review, or generate a draft.
- Measured live (2026-11-16): the API's `choice` label is **not always the argmax** of the
  probabilities, so the verdict must come from the probability distribution, with a deterministic
  fallback below a floor/margin threshold.
- Existing helper/mapping, gating, redaction and config are already proven in-tree:
  - `officina/.pi/extensions/hang-watchdog/decision-gemini.ts` (request builder, reply mapper,
    `askGeminiDecision`, `buildDecisionsRequest`, safety/429 handling, `scrubEvidence` redaction).
  - `officina/.pi/extensions/hang-watchdog/decision.ts` (`decisionVerdict` gate: coverage,
    abstention, floor, choice ∈ {ALIVE,HUNG}).
  - `officina/.pi/extensions/hang-watchdog/config.ts` (`~/.vitriol/officina/watchdog-decision.json`,
    0600, `ascensusGeminiKey`, `resolveDecisionConfig`; gemini floor 0.7 vs http 0.9, margin 0.15).
  - `officina/scripts/caduceus-router-lib.mjs` (lane choice request/parse, `needsDecisionModel`).

**Decision rule for candidacy:** a process is a good Gemini Decision replacement only if it is
(a) a *finite-option choice/classification task* AND (b) today spends a real model call (LLM
generation) purely to pick, or would otherwise benefit from calibrated probabilities. Processes
that are already free heuristics, or that *generate* text, are NOT candidates.

## 2. Currently wired to Gemini Decision (2 processes, already live)

| Process | Files | Options | Notes |
|---|---|---|---|
| Hang-watchdog liveness | `hang-watchdog/decision-gemini.ts`, `decision.ts`, `index.ts` | ALIVE / HUNG / UNKNOWN | Gemin floor 0.7, margin 0.15; scrubEvidence redaction; `/decision` CLI |
| Caduceus router lane tie-break | `scripts/caduceus-router-lib.mjs` (+ `caduceus-router.mjs`, e2e) | one lane among candidates | `needsDecisionModel()` guards: only asks when ≥2 usable candidates share freeness; metadata-only (no prompt) |

## 3. Surveyed processes and verdict

### Fit (recommend swap / upgrade)

1. **Hang-watchdog's free-LLM fallback judge** (`hang-watchdog/index.ts` → `judgeWithLane` →
   `judge.ts#buildJudgePrompt`). Today, when the Gemini lane is *not configured/unavailable*, the
   watchdog still makes a **full chat completion** (`max_tokens: 80`) on a free cloud lane just to
   get a `VERDICT: ALIVE/HUNG/UNKNOWN` line, then regex-parses it (`parseVerdict`). This is the
   exact "spend a generator to pick one of N" pattern. Gemini Decision is the cheaper, calibrated,
   already-proven replacement.
   - If the operator prefers Gemini, `judgeWithLane` should be retired (or demoted) so the watchdog
     no longer pays a generator call for a choice. Note: today both paths coexist — decision lane
     is tried first, and the LLM lane is the fallback. So this is a **cost-reduction** swap, not a
     behavior change.

### Non-fit (do NOT replace)

2. **auto-route complexity / privacy classifier** (`auto-route/classifier.ts` → `classifyTurn`,
   also used by `adaptive-router` and `resilient-default`). Purely heuristic (prompt length, error
   regex, scratchpad lines, domain words, secret/PII regex). No LLM spend today. Replacing with
   Gemini would **add** cost and latency before every turn, and would ship prompt/scratchpad text
   to a third party (privacy concern). Heuristic is deliberately cheap and local. **No.**
   - Optional, owner-discretion upgrade: use Gemini Decision only as an *advisory* confidence
     overlay on ambiguous heuristics — but that is scope creep and emissions; not recommended as a
     first move.

3. **composer-predictions** (`composer-predictions/predict.ts`, `index.ts`). `predict()` is a pure
   heuristic (task/verify/git state → one advisory line). `generateDraft()`/voice *generates* the
   next-prompt text. Both are generation or free heuristics, not finite-option choices. **No.**

4. **background-lane diff reviewer** (`background-lane/index.ts`). It internally calls
   `POST /completion` (llama.cpp) with `buildReviewPrompt` and `cache_prompt` to *generate* a
   findings card. That is generation. **No.**

5. **small-lane summarizer choice** (`small-lane/lane.ts#orderSummarizers`). Deterministic
   fit-based ordering (largest window that fits, cloud-first on ties). Already free and exact. **No.**

6. **Security gates** — `injection-guard`, `provider-guard`, `tool-gating`, `permissions-guard`.
   Deterministic regex/structural allow-deny rules; must stay local; not finite-option NL choices.
   Privacy-critical (handling secrets/behavior). **No.**

7. **phase-model, coupling, import-lint, format-gate, verify, repo-map** — deterministic
   config/heuristics, no LLM-for-choice. **No.**

## 4. Cost reality check

- The auto-route/privacy/security processes look like "decisions" but they are **free local
  heuristics** already; Gemini would add a per-turn cloud call + latency and ship data off-host.
- The only *real* current spend of a generator to make a choice is the watchdog's `judgeWithLane`
  fallback (bounded: soft 60 s, judge 180 s, ≤3 judgments/episode, re-judge every 300 s) — small in
  absolute volume, but it is the one place Gemini strictly dominates (same task, calibrated, zero
  output tokens, ~free).
- The caduceus lane tie-break already uses Gemini (metadata-only), so routing is already cheap.
- Conclusion: **the cheap-Gemini opportunity has largely already been captured** for the
  finite-option decision tasks that exist in Officina. The remaining actionable item is finishing
  the job on the watchdog's *fallback* judge and ensuring no generator path is still doing a choice.

## 5. Proposed plan (BUILD, pending your go-ahead)

**Step 1 — Close the histogram in the watchdog (small, high-value).**
- In `hang-watchdog/index.ts` `judgeWithLane`, gate the free-LLM fallback so it only fires when
  the operator has NOT configured the Gemini lane at all (i.e. `resolveDecisionConfig().mode` is
  `heuristic`/unset). When Gemini is live, the generator fallback should not run.
- Keep the code path (availability), but stop paying a generator for a choice we can get from
  Gemini.
- Update `docs/officina-guide.md` watchdog section: Gemini (when configured) is the only judge;
  the free-LLM verifier is the no-config default.

**Step 2 — Optional, owner-discretion: retire/trim the free-LLM judge path.**
- Decide whether `judgeWithLane` should be removed entirely in favour of Gemini-first with a
  deterministic heuristic fallback (no NL judge), or kept as a documented no-config default.
  Recommend keep-on-heuristic (don't remove), because Gemini needs a key and local-only users
  still want *some* judge.

**Step 3 — Test & doc discipline.**
- Extend `hang-watchdog/decision-mode.test.ts` / add a case asserting that when mode=gemini the
  LLM lane is not invoked.
- Run `vitest` for the watchdog + caduceus-router suite (four workers default).
- Update the one-line status emission and `/decision status` copy where the fallback changes.

**Non-goals (explicitly out of scope):** replacing auto-route/privacy heuristics, composer/voice
prediction, background diff review, small-lane ordering, or any security gate with Gemini Decision.

## 6. Two new directions you raised — fast/latency + a Dutch-ability ranking job

### C. Latency: yes, and batching is the lever

You're right that `gemini-decision-1` is built for speed — one prefill pass, zero
output tokens, ~2.5 s default timeout. But the bigger, under-used property is
**multi-question batching**: the Decisions API takes `questions: { … }` (an
object map, each entry a `choice` with its own `criteria`), yet both wired
call-sites send exactly ONE question per call. A single request with K
questions does K ratings in one prefill — that is where decision models beat
"ask one thing at a time."

Verified in-tree:
- `decision-gemini.test.ts` asserts the request's `questions` map shape
  (`questions.<id>.type/criteria`); nothing caps it at one entry.
- `caduceus-router-lib.mjs#buildLaneDecision` builds exactly one `lane` question;
  `hang-watchdog` builds exactly one `liveness` question. Both are the
  documented API's minimal use.

> Caveat: multi-question needs a real probe (no cloud calls during PLAN).
> Confirm ceilings (max questions/request, count against a 2.5s budget, how a
> partial `usage.truncated_questions` surfaces — `decision.ts` already reads it
> and treats `truncated !== false` as abstention). Do NOT roll batching into the
> live watchdog/router first; prove the shape on the offline ranking job below.

### D. Weekly Dutch-ability ranking job — this is the RIGHT Gemini Decision shape

This is genuinely a finite-option choice/rating task, so it is where Gemini
Decision adds net value (unlike the free heuristics surveyed in §3). Sketch:

**Shape.** A small suite of fixed Dutch eval prompts (say 10-20: translation,
grammar, idiom, summarisation, tone) → for each candidate model in the catalog
(generate inline, one reply per prompt) → **`gemini-decision-1` rates each
reply into buckets** (e.g. `NATIVE / GOOD / AWKWARD / WRONG`), keeping the
probability for a confidence column → aggregate to a per-model Dutch rank.

**Why it fits.** Generation (the model's actual Dutch text) is done by the
candidate lanes; the *rating* is a bounded choice with calibrated
probabilities — exactly the decisions-only capability. Cheap (zero output
tokens on the rater), and the batching above makes each model's rating batch a
single request group. Runs offline/weekly, so the ~2.5s per-call latency is a
non-issue (it's why a scheduled job is the right home, not a keystroke path).

**Building blocks already in the tree to reuse (no new infra invented):**
- Gemini client + reply mapping: `hang-watchdog/decision-gemini.ts`.
- Gate/threshold/abstention: `hang-watchdog/decision.ts#decisionVerdict` and the
  `floor`/`margin` config.
- Key/config: `hang-watchdog/config.ts#ascensusGeminiKey` / `resolveDecisionConfig`.
- Batch generation pattern: `scripts/benchmark_vitriol.sh` + caduceus router
  candidate enumerator (`caduceus-router-lib.mjs`, `freeToolCandidates`) +
  `deep-research/batch-eval.test.ts` report-writing pattern.
- Model catalog: `caduceus-models.json` + `officina/models.json`.

**Where it would live.** A new offline script (e.g. `scripts/dutch-rank.mjs`)
OR a `caduceus` verb (`vitriol caduceus rank-dutch`), scheduled by a systemd
user timer (the same pattern as `officina-backup.sh`; keep it well away from
any keystroke/live path).

**Latency plan (batching, the payoff).** Send each model's K replies as a K-
question single request (or split into chunks if `truncated_questions` proves a
limit). One prefill rates the whole batch → the "decision models are fast"
claim is realized as “K ratings ≈ 1 call," not just “one rating is quick."

**Validation before trusting the rank (important).** Gemini Decision is
English-targeted; before it becomes a Dutch ruler, probe its Dutch
calibration on a few human-graded replies (score ≈ known quality). If it
mis-ranks, keep the job but label the column `advisory (uncalibrated)` — do
not silently let a biased rater drive model choices.

## 7. Open questions for you

1. (from §6D) Should the Dutch job batch K questions per request, or keep 1:1
   for simpler debugging? Recommend batching, with a `--one-per-call` flag to
   A/B the ceiling.
2. What bucket scale do you want? (`NATIVE/GOOD/AWFUL/WRONG` vs a 5-point
   scale) — affects how many criteria the `choice` encodes.
3. Which models: the whole `caduceus` catalog (free lanes, ~dozens) or a
   curated subset incl. your local llama.cpp qwens? Recommend local + a
   handful of free lanes first, enlarge later.
4. Where should the report land + who reads it (a `benchmarks/dutch/` Markdown
   + JSON, and/or a caduceus `/doctor`-style status line)?
5. Are you OK that the *generation* side (candidate models producing Dutch
   text) still spends real tokens — the cheap bit is the *rating*? (Free lanes
   + local make this ~0 today.)

## 9. The direct answer: how many LLM-based processes can we replace with Decision?

Complete sweep of every actual model-call site in `officina` (extensions + scripts,
grep for `chat/completions`, `/completion`, `generateSummary`, `runSubCoder`, and
spawn paths), classified by whether Gemini Decision can do the job.

### Already on Gemini Decision (2 — no more work)
| # | Process | File | Options |
|---|---------|------|---------|
| 1 | Watchdog liveness lane | `hang-watchdog/decision-gemini.ts` | ALIVE/HUNG/UNKNOWN |
| 2 | Caduceus lane tie-break | `scripts/caduceus-router-lib.mjs` | lane choice |

### Replaceable — real LLM spent purely to choose (2)
| # | Process | File | Today's spend | Why it fits | Cost of swap |
|---|---------|------|---------------|-------------|-------------|
| A | Watchdog free-LLM fallback judge | `hang-watchdog/index.ts` → `judgeWithLane` | Full chat completion (`max_tokens: 80`) over `buildJudgePrompt` | Same 3-way choice; Gemini is calibrated + zero-out-tok | Small (demote to heuristic-only when gemini on) |
| B | Deep-research eval judge | `deep-research/judge.ts` (invoked via `runSubCoder` in `batch-eval.test.ts`) | A full **sub-coder agent turn** just to score 5 dims × 1–5 | Finite 5-point rating per dimension — exactly the decisions shape | Medium (5 rating questions, batched; note `notes` field is generation → drop or heuristic) |

### NOT replaceable — generation or structural (5+) — Gemini Decision can't write text
| Process | File | Why not |
|---------|------|---------|
| Composer/voice suggestions | `composer-predictions/*` | `generateDraft()` produces next-prompt *text* |
| Background diff reviewer | `background-lane/*` | generates findings card |
| Small-lane summarizer | `small-lane/*` | summarization = generation |
| Context-relay handoff card | `context-relay/relay.ts#generateCard` | 5-section *card text* |
| Cloud probe | `cloud-probe/index.ts` | 1-token *ping* = lane liveness, not a classification; lane-specific round-trip |

### Not LLM-to-choose (already free/deterministic) — Gemini would ADD cost (4+)
- auto-route/privacy classifier (`classifyTurn`) — heuristic; adaptive-router & resilient-default reuse it.
- Security gates (injection/provider/tool/permissions) — regex/structural, must stay local.
- phase-model, coupling, import-lint, format-gate, repo-map — config/deterministic.

### Bottom-line count
- **2** of ~2 finite-choice LLM processes are **already converted** (watchdog, caduceus lane).
- **2 more are safely replaceable today**: the watchdog's free-LLM fallback (A, small) and the
deep-research eval judge (B, medium — biggest token win because it's a whole sub-coder turn per
report).
- **5** model-call sites are **generation** (Gemini Decision cannot do them).
- **cloud-probe** is a liveness ping, not a classifier.
- So the honest ceiling: out of **~9 real model-call processes**, **4 are or should be Gemini
  Decision**; the other 5 are inherently generative.

> The deep-research judge (B) is the most valuable byte-for-byte: it spends an entire sub-coder
> turn to emit 5 integers + one note. As Gemini Decision it becomes 5 batched rating questions
> (zero output tokens), and the `notes` field is either dropped or replaced by a `commentable`
> question. This is the one place a glaring generator-for-a-rating still exists.

## 10. Consolidated recommended path

**Fast, high-confidence (BUILD when you say go):** demote the watchdog's
free-LLM `judgeWithLane` fallback to heuristic-only when Gemini is configured
(§5 Step 1) — closes the only real generator-for-a-choice spend.

**New value (BUILD after a green-flag on §7):** a standalone, offline,
batched `dutch-rank` job (§6D) that proves the multi-question ceiling while
delivering a real, cheap, scheduled Dutch-ability ranking. Validate
calibration before it drives decisions.

Both are Markdown-planned now; no code changed during this PLAN turn.

1. Do you want the watchdog's free-LLM fallback demoted-to-`heuristic-only` wins this build the
   fix, or should we keep the LLM fallback as the no-key default and only stop it when Gemini is
   on? (Minor behavioral fork.)
2. Is there a *new* finite-option decision task you have in mind that is NOT in the current
   extensions (e.g. "should this turn escalate", "which test runner", "which algorithm for this
   prompt")? If so, name it and I can scope a Gemini-Decision integration for it — that is where
   Gemini adds net value, because the existing decisions are already served cheaply.
3. Privacy posture for anything that would ship prompt/scratchpad text in a decision input: today
   only metadata (caduceus lane) and redacted command tails (watchdog) leave the box. Any new use
   must keep that discipline.