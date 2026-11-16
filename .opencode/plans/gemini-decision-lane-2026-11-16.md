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
