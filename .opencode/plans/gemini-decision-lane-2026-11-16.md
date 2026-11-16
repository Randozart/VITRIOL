# Google "decision model" as the watchdog judge (2026-11-16)

Question: Google shipped a decision model — is it cheap/clean enough to be the
`/decision` lane, or should we pick something else?

**Short answer: yes, it is the best fit we have found — but the key is not
actually registered anywhere I can see, and it changes where evidence goes
(direct to Google), so it needs redaction + opt-in.**

## 1. What it is (researched)

**Gemini Decision 1** — `gemini-decision-1`, with a new **Decisions API**
(`POST https://generativelanguage.googleapis.com/v1beta/decisions`).

| property | value | source |
|---|---|---|
| nature | *decisions* endpoint, **not** `generateContent` | ai.google.dev/gemini-api/docs/models/gemini-decision-1 |
| availability | Gemini Developer API **v1beta only** (no Vertex) | ai.google.dev/gemini-api/docs/decisions-api#limitations |
| price | **$0.10 / 1M input tokens**, output always 0 tokens | ai.google.dev/gemini-api/docs/pricing#gemini-decision |
| input | 262,144 tokens; up to **1,000 questions per request** (yes/no, 2–255-way choice, rubric score) | decisions-api#limits |
| free tier | input free; 200 RPM / 50k TPM / **10k RPD** | rate-limits#gemini-decision |
| JSON mode / tools | **not supported** (JSON is just text input) | models page |
| latency | single prefill pass, "nearly flat" as questions grow; **no numeric SLA** | decisions-api#how-it-works |

The decisive property for us: it returns **calibrated choice probabilities**.
That is exactly what `decision.ts` wants (`answer_confidence`,
`action.act_probability`) and exactly what we could not get out of a chat model —
today's experiments showed a 1B judge degenerating into JSON loops and the 27B
model giving usable-but-free-text answers.

## 2. Cost

Evidence per decision is the command, elapsed/stalled seconds and an output tail:
**~300 tokens**, one question, zero output.

- per decision: 300 × $0.10 / 1e6 ≈ **$0.00003**
- a busy day of watchdog evaluations (say 200 decisions): **≈ $0.006**, and the
  free tier's **10k requests/day** covers all of it.

So: cheap enough is not the question. The questions are key custody and privacy.

## 3. The key is not where the user thinks it is

Checked on this host:

- `~/.vitriol/officina/accounts.json` has `gemini` ("Gemini (Google AI Studio)",
  `GEMINI_API_KEY`) and `vertex` (`VERTEX_API_KEY`) — both **`enabled: false`,
  `key_present: false`, `signup_status: "skipped"`**.
- `GEMINI_API_KEY` is **not set** in any env file I can see.
- The gateway's model list does carry Gemini models, but only through reseller
  lanes (`api-airforce/google/gemini-2.5-flash`, `dva/gemini-3-7-flash-*`,
  `aug/gemini-3.1-pro-preview`, `auto/gemini`) — those are *someone else's* key,
  not the owner's, and they do not expose the Decisions API.

**And the Decisions API cannot be reached through Caduceus**: the gateway proxies
OpenAI-shaped `/v1/*` chat; decisions is a different endpoint at
`generativelanguage.googleapis.com`. The shim must talk to Google directly with
the owner's key.

Action needed before any code: put the key somewhere the shim can read —
either `GEMINI_API_KEY` in `~/.vitriol/officina/env` (0600, already the pattern)
or `vitriol caduceus add gemini` (OAuth) and read it from the accounts store.
`/decision key <token>` already exists, so the shim can also keep it in
`watchdog-decision.json` (0600).

## 4. Privacy — the real design constraint

Today the decision lane is local in spirit: the heuristic tier never leaves the
box, and the judge-sim only talks to `127.0.0.1`. Sending evidence to Google
means **the command line and an output tail leave the machine**, and output tails
routinely contain secrets (env dumps, tokens, paths, customer data).

That collides with the project's own rule (sensitive/confidential turns never
reach the cloud). So the plan must include redaction, not just plumbing:

- strip anything key-shaped (`sk-…`, `ghp_…`, bearer tokens, `*_KEY=…`, long
  base64/hex runs);
- cap what is sent: command ≤ 200 chars, tail ≤ 200 chars;
- an explicit opt-in (`/decision set … --remote` or `DECISION_ALLOW_REMOTE=1`),
  so a local-only setup stays local;
- the heuristic tier keeps answering everything it can *before* any network call.

## 5. Proposed architecture (tiered, cheapest first)

```
watchdog fact ──▶ 1. heuristics (local, 0 ms, $0)        ~most cases
                    │ ambiguous
                    ▼
                 2. Gemini Decision 1  ($0.00003, ~1 prefill)
                    │ low prob / error / timeout
                    ▼
                 3. UNKNOWN + abstain (never ALIVE)
```

Concrete pieces:

- **New `decision-gemini.cjs`** in `officina/.pi/extensions/hang-watchdog/`
  (pattern: `decision-shim.cjs`). It receives the SystemOne POST from
  `decision.ts`, builds one `choice` question with the three options, calls the
  Decisions API, and maps the reply into our contract:
  - `choice` = argmax over {ALIVE, HUNG, UNKNOWN}
  - `answer_confidence` = max probability
  - `action.act_probability` = **P(HUNG)** (acting = intervene; that is the
    semantic the gate expects)
  - below `floor` or a timeout ⇒ `UNKNOWN` + `abstention: "abstained"`
  - `usage` reported truthfully: `truncated:false`, `state_tokens_dropped:0`,
    `truncated_questions:[]`, `options.liveness:{total:3,distinct:3}`
- **Heuristics stay in front**: the existing keyword+stall-ratio tier answers
  first, so Google sees only genuinely ambiguous evidence.
- **Config**: `config.ts` grows `mode: "heuristic" | "gemini" | "http"` (and a
  redaction toggle); `/decision status` shows which tier is live, its cost per
  call and whether evidence leaves the box.
- **Redaction module** shared with the shim, unit-tested (key-shaped strings,
  bearer tokens, long hex/base64, `*_KEY=`/`*_TOKEN=` assignments).

If the owner prefers not to send anything: alternatives, in order —
`gemini-3.1-flash-lite` ($0.25/$1.50) or `gemini-2.5-flash-lite`
($0.10/$0.40, restricted to prior 2.5 users) via a chat shim; or "heuristics
only" (already shipped, zero cost, no model). `gemini-decision-1` still wins on
price and on returning probabilities instead of prose.

## 6. Invariants

- **D1** — no silent health: a failed/low-confidence/abstained decision is
  `UNKNOWN`, never `ALIVE` (existing `decision.ts` gates unchanged).
- **D2** — local first: with `mode=heuristic`, no bytes leave the box; the
  Gemini tier is only reachable when explicitly enabled.
- **D3** — redaction is total for the patterns we know, and the *unredacted*
  payload is never logged.
- **D4** — bounded: a hard per-call timeout (2.5 s default) then fall through to
  UNKNOWN; no retries inside the watchdog path.
- **D5** — key hygiene: 0600 config or env, never in argv, never in the trail.

## 7. Tests

- mapping maths: probabilities → choice/confidence/act_probability; ties;
  three-way and missing-option responses; `max prob < floor` ⇒ abstain.
- redaction: a corpus of key-shaped strings, `*_KEY=` assignments, base64/hex
  blobs, and a tail containing a URL with credentials.
- failure modes: HTTP 429/500, timeout, malformed JSON ⇒ UNKNOWN + abstained,
  with a reason string.
- parity: `mode=heuristic` produces byte-identical output to today (I1 of the
  earlier decision lane).
- live smoke behind `DECISION_LIVE=1` (skipped by default, needs the key).

## 8. Open questions for the owner

1. **Where is the Gemini key?** Nothing on this host has `key_present: true` for
   Google. If it lives in your shell or another app, I need it in
   `~/.vitriol/officina/env` (or `/decision key`).
2. **May watchdog evidence leave the box?** If yes: with redaction (recommended)
   — if no, we stay on heuristics and this whole plan is moot.
3. Do you want the Decisions API directly, or should the shim call it *through*
   Caduceus once you add the Gemini lane there (one more hop, but then the
   gateway's ledger and cooling cover it too)?

---

# Follow-up research (owner answers: key "should be for ascensus", evidence may leave, call Google directly)

## A. The API contract, verified from the docs

`POST https://generativelanguage.googleapis.com/v1beta/decisions`

- **auth**: header **`x-goog-api-key: $GEMINI_API_KEY`** — `?key=` is *not*
  documented for this endpoint (it is for `generateContent`, which is why
  `ascensusd.call_gemini` uses it; the decisions shim must use the header).
- **request**:
  ```json
  {
    "model": "gemini-decision-1",
    "input": "<shared evidence text>",
    "questions": {
      "liveness": {
        "type": "choice",
        "instructions": "Judge the command's liveness from the evidence.",
        "criteria": {
          "ALIVE": "clearly progressing (compiling, downloading, iterating)",
          "HUNG": "blocked (deadlock, waiting for stdin, infinite repetition)",
          "UNKNOWN": "insufficient or contradictory evidence"
        }
      }
    }
  }
  ```
  `questions` keys are `[A-Za-z0-9_-]{1,64}`; choice `criteria` is a **map
  optionID → description** (2–255 options; the server sorts them); 1–1,000
  questions share one billed `input`.
- **response**: `answers.liveness = { choice, probabilities: {ALIVE,HUNG,UNKNOWN}, confidence }`
  where **`confidence == p_max`**, plus `usage` with **zero output tokens**.
  *Probabilities, not logits.* **No abstention field** — the client thresholds.
- **failures**: `400` with a safety code (`safety`, `prohibited_content`,
  `blocklist`, `image_safety`, `content_blocked`), or a `200` with no `answers`
  and `promptFeedback.blockReason`; `429 RESOURCE_EXHAUSTED` with no
  `Retry-After` (backoff+jitter is the documented remedy).

## B. Key: not on this host, and ascensus is single-key

- `~/.vitriol/secrets` holds exactly one `api_key` (49 chars, tail `AYJw`),
  `provider = zai-coding-plan`, `model = glm-4.6` — **no Gemini key**.
- `GEMINI_API_KEY` is absent from `~/.vitriol/officina/env`, `~/.omniroute/.env`,
  `~/.pi/agent/auth.json`, `~/.bashrc`, `~/.profile`.
- `ascensusd.read_secrets()` reads **one** `api_key`, sniffs `AIza…` as gemini,
  and defaults the model to `gemini-2.5-flash` for that provider.

Consequence: dropping a Gemini key into that file **replaces** the zai
escalation key (single-key format), which is a behaviour change to ascensus, not
to the watchdog. Two ways forward, and the second is the tidy end state:

1. **Decision lane only (no ascensus change)** — put `GEMINI_API_KEY` in
   `~/.vitriol/officina/env` (0600, existing pattern) or set it with
   `/decision key <token>`. The shim resolves: env → decision config → gemini
   entry in `~/.vitriol/secrets` (read-only fallback).
2. **Extend ascensus to multi-provider** — allow sections/prefixes, e.g.
   ```
   provider = zai-coding-plan
   api_key  = …
   model    = glm-4.6
   [gemini]
   api_key  = AIza…
   model    = gemini-decision-1
   ```
   `read_secrets()` gains a `--provider gemini` selector. Small, testable
   (parser + resolution order), and then one file serves escalation *and* the
   decision lane. This is the only option consistent with "configured for
   ascensus", and it must not disturb the zai defaults.

## C. Mapping onto the existing gate (exact, from `decision.ts:56-110`)

The gate requires **both** scores ≥ floor and ≤ 1, `abstention` absent or
`"passed"`, `low_confidence` falsy, and `choice ∈ {ALIVE, HUNG}`.

| our field | source |
|---|---|
| `choice` | `answers.liveness.choice` |
| `answer_confidence` | `answers.liveness.confidence` (= p_max) |
| `action.act_probability` | **also `confidence`** — the contract's act score is "confidence to act on this verdict", so `P(HUNG)` would wrongly reject every ALIVE verdict |
| `abstention` | `"passed"` when accepted, `"abstained"` when below the floor |
| `usage` | `truncated:false`, `state_tokens_dropped:0`, `truncated_questions:[]`, `options.liveness:{total:3,distinct:3}` |

**Floor choice matters**: with three options, p_max is naturally 0.33–0.6 on
genuinely ambiguous evidence, so the existing 0.9 default would abstain almost
always. Proposal for this lane: `floor = 0.60` **plus a margin rule**
(`p_max − p_second ≥ 0.15`), both configurable; `/decision status` prints the
effective values and states the reason a given verdict abstained.

## D. Redaction (evidence may leave the box, per owner)

Applied to `input` before the request, and never logged unredacted:

- key-shaped tokens: `sk-…`, `ghp_/gho_…`, `AIza…`, bearer/JWT blobs, long
  base64/hex runs (≥24 chars);
- assignments: `*_KEY=`, `*_TOKEN=`, `*_SECRET=`, `PASSWORD=`, `AUTH=`;
- URL credentials (`scheme://user:pass@`), and any `?key=`/`&token=` value →
  `«redacted»`;
- caps: command ≤ 200 chars, tail ≤ 200 chars, total `input` ≤ 1200 chars.

Unit-tested against a corpus (the redaction module is shared with the shim).

## E. Failure policy inside the watchdog path

| condition | result |
|---|---|
| HTTP 429 | UNKNOWN + abstained, reason `rate limited`; **no retry** here (the watchdog re-evaluates later) |
| HTTP 400 safety / 200 with `blockReason` | UNKNOWN + abstained, reason `safety block`; no retry |
| timeout (>2500 ms) | UNKNOWN + abstained, reason `timeout` |
| malformed / missing `answers` | UNKNOWN + abstained, reason `unusable reply` |
| `confidence < floor` or margin too small | UNKNOWN + abstained, reason `below floor` |

Never `ALIVE` on an error path (D1).

## F. Build order

1. `decision-redact.ts` + tests (pure, no network).
2. `decision-gemini.cjs` (shim, header auth, mapping, failure policy) + mocked
   tests using the shapes above.
3. `config.ts`: `mode: "heuristic" | "gemini" | "http"`, `floor`, `margin`,
   `redact` (default on); `/decision status` reports tier, cost/call, and
   whether evidence leaves the box.
4. Heuristics stay in front (unchanged); parity test for `mode=heuristic`.
5. Optional ascensus multi-provider section (separate commit; touches
   `ascensusd.py` + a parser test).
6. Live smoke behind `DECISION_LIVE=1`.

## G. Still needed from the owner

**The key itself.** Nothing on this host has it. Either paste it into
`~/.vitriol/officina/env` as `GEMINI_API_KEY=AIza…`, or run
`/decision key AIza…` after I ship the shim, or say "extend ascensus" and I'll do
option B first so the key lives in `~/.vitriol/secrets`.

---

# As landed — infrastructure only (2026-11-16, commit `f781b6c`)

Everything but the key, so the lane lights up the moment one exists.

| piece | file | note |
|---|---|---|
| redaction | `hang-watchdog/decision-redact.ts` | scrub-before-send, 21 corpus tests |
| Decisions client | `hang-watchdog/decision-gemini.ts` | request builder, probability→contract mapping, failure policy |
| config knobs | `hang-watchdog/config.ts` | `mode`, `margin` (0.15), `model`, `redact` (on), `ascensusGeminiKey()` |
| routing | `hang-watchdog/decision.ts` | gemini branch in `judgeWithDecision`; reason names the choice |
| operator surface | `hang-watchdog/index.ts` | `/decision status\|mode\|set\|key\|model\|floor\|margin\|redact\|off` |
| tests | `decision-redact`, `decision-gemini`, `decision-mode`, `config` | 122 watchdog tests (was 111) |

## Redaction (R1-R5)

Google/OpenAI/Anthropic/GitHub/HF/Slack/AWS key shapes, JWTs, URL credentials,
`?key=`/`token=` params, `*_KEY=`/`*_TOKEN=`/`PASSWORD=` assignments,
bearer/basic, long hex and base64-ish blobs, and e-mails. Caps: command 200,
tail 200, total 1200 — **scrub before cap**, so a key split by the boundary
cannot survive. Idempotent. The assignment *name* is kept
(`GEMINI_API_KEY=«redacted»`) so the judge can see that something was scrubbed.

## Mapping (the part that would silently break otherwise)

`decision.ts` requires **both** `answer_confidence` and `action.act_probability`
≥ floor. The Decisions reply gives one `confidence` (= p_max), so both scores are
set to it — using `P(HUNG)` for the act score would have failed the gate on every
ALIVE verdict. Floor default stays 0.9 for http mode; the gemini lane documents
0.6 + margin 0.15 because a three-way choice rarely exceeds ~0.6 on genuinely
ambiguous evidence.

Failure policy: 429 ⇒ `rate limited`, 400-safety/`blockReason` ⇒ `safety block`,
non-JSON ⇒ `unusable reply`, abort ⇒ `timeout`, no key ⇒ `no gemini key` — all
`unknown`, never `ALIVE` (D1).

## Key resolution order (gemini mode)

1. `/decision key` / config `key`
2. `GEMINI_API_KEY` in the environment
3. `ascensusGeminiKey()` — `~/.vitriol/secrets`, mirroring ascensusd's own
   sniffing: a bare `api_key` beginning `AIza`, a `gemini_api_key` line, or a
   `[gemini]` section (the future multi-provider format) which wins.

## Verified

- 1423 tests across the suite (watchdog alone: 122), `tsc` clean.
- One pre-existing flake sighted (`permissions-guard` under parallel load,
  passes in isolation and on the re-run) — unrelated to this change.
- Not verified live: no Gemini key on this host, so the lane currently answers
  `unknown("decision model unavailable — no gemini key configured")` when set to
  gemini mode. That is the designed behaviour, not a failure.

## Still needed

The key. `GEMINI_API_KEY=AIza…` in `~/.vitriol/officina/env`, or
`/decision key AIza…`, or an ascensus `[gemini]` section. Then
`/decision mode gemini` and `/decision status` should report
`gemini gemini-decision-1 · floor 0.9 · margin 0.15 · key … · evidence redacted,
leaves the box · ~$0.00003/call`.
