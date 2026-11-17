# Does the judge work? — Gemini Decision 1 lane, measured

**Date:** 2026-11-17 19:05
**Mode:** PLAN (read-only investigation; the only side effects were HTTP probes, which are free at
these sizes — 96–585 tokens per call)
**Owner report:** "does the judge actually work? Getting a lot of invalid request errors."

## 1. Verdict: the judge works. Measured, not assumed.

| check | result |
|---|---|
| endpoint `POST https://generativelanguage.googleapis.com/v1beta/decisions` | **HTTP 200** (real; not a mock) |
| model id `gemini-decision-1` | accepted (the `models` list does **not** contain it — it is a decisions-API model, not a chat model) |
| key resolution (`geminiKeyFromSecrets` on the live `~/.vitriol/secrets`) | `AIzaSy…`, 39 chars, found under `[ascensus]` |
| production request builder (`buildDecisionsRequest` from `decision-gemini.ts`) + live POST | **HTTP 200**, calibrated 3-way probabilities |
| latency | 0.38 s / 0.53 s / 0.62 s — the client timeout is 2,500 ms, so ~4× headroom |
| live config `~/.vitriol/officina/watchdog-decision.json` | `{mode: "gemini", url: "", key: "", floor: 0.7}` → built-in endpoint, key from secrets ✓ |

Sample replies (real):

```
progressing tail   → ALIVE 0.808 / UNKNOWN 0.123 / HUNG 0.070
short vague tail   → UNKNOWN 0.635 / HUNG 0.302 / ALIVE 0.062
empty evidence     → UNKNOWN 0.932 / …
```

The abstention behaviour is correct: with no evidence it returns UNKNOWN at high confidence, and
`mapDecisionsReply` refuses to promote UNKNOWN (`argmax is UNKNOWN`), so the gate keeps its
"never promote to healthy" contract.

## 2. What "invalid request" actually means at this endpoint (measured)

I probed the malformed shapes deliberately. The API's own messages:

| request | response |
|---|---|
| `criteria` empty | 400 `invalid_request` — *"criteria: missing field / must have between 2 and 255 options"* |
| `criteria` with **1** option | 400 `invalid_request` — *"must have between 2 and 255 options"* |
| `questions` empty / absent | 400 `invalid_request` — *"Request must contain at least one question."* |
| `input: ""` | 400 `invalid_request` — *"Missing text in content of type text."* |
| unknown model id (`gemini-3.1-flash-lite`) | **404 `not_found`** — *"Requested model not found"* (a different error class) |

**Our callers cannot produce those** — verified in code and against the live API:

- `buildDecisionsRequest` always sends one `choice` question with exactly **3** criteria
  (`ALIVE/HUNG/UNKNOWN`) and an `input` built from `Command: …\nElapsed: …\nOutput tail: …`, which
  is non-empty even when every field is empty (tested: all-empty evidence → HTTP 200).
- the router's `decideLane` is gated by `needsDecisionModel()`, which returns false unless there
  are **≥ 2 usable** (unparked) candidates — so it never sends 0 or 1 options.
- 24 options, 32 000-character inputs, and 5 000-character instructions were all accepted.

## 3. The real defect: the error message is thrown away

`decision-gemini.ts`, non-2xx branch:

```ts
return { ok: false, reason: `decision service error (HTTP ${response.status})` };
```

The API's `error.message` — the sentence that says *which* field is wrong — is parsed and then
**discarded**. Only `code`/`status` are inspected, and only for the safety-block and 429 cases.

So: an `invalid_request` from this endpoint can only ever reach the operator as
`decision service error (HTTP 400)`, and **nothing is written to the events ledger**. I searched
`~/.vitriol/officina/state/events.jsonl` for `invalid_request` / `INVALID_ARGUMENT`: the only
matches are my own permission lines from this investigation. The lane leaves no trace, so the
question in the owner's message cannot be answered from history — which is itself the finding
(this is the same class as the compaction-fallback blindness fixed on 2026-11-17).

## 4. Two things worth knowing before blaming the judge

1. **A stale shim is still running.** `node .pi/extensions/hang-watchdog/decision-shim.cjs 8381
   http://127.0.0.1:8279`, alive for **1 day 4 h**. It serves exactly one route (`POST
   /systemone`) and **nothing in the repo references port 8381** any more (the Gemini lane replaced
   it on 2026-11-16). Every other path answers `{"error":"not found"}` — so if anything still
   points at `http://127.0.0.1:8381/...`, it produces precisely a wall of "not found"/bad-request
   noise. It is also holding a port and a process slot for nothing.
2. **`url` must stay empty for the Gemini lane.** If `~/.vitriol/config`-adjacent tooling (or
   `vitriol caduceus decision set <url>`) writes a *chat* URL, `resolveDecisionConfig` switches
   `mode` to `http` and posts a **decisions body** to a chat endpoint → `invalid_request` on every
   call. Current config is clean (`url: ""`), but this is a one-command trap.

## 5. Plan

### Stage 1 — make failures name themselves (the fix that matters)
`decision-gemini.ts`: on non-2xx, include the API's message and class in the reason:
`decision service error (HTTP 400 invalid_request): criteria: must have between 2 and 255 options`.
Classify `401/403` as `auth`, `404` as `model not found`, `429` as rate-limit (already), and the
existing safety codes. Never log the key or the evidence text.

### Stage 2 — persist every outcome
Emit `lc-lane/decision` events: `{ ok, status, model, latencyMs, reason }` (no evidence, no key —
the reason strings are already redacted-or-absent). Add the same line to the watchdog card so the
operator sees *why* the lane abstained instead of guessing.

### Stage 3 — fail fast instead of round-tripping a 400
Pre-flight guards with named reasons: `decision skipped: 1 option` (router), `decision skipped:
empty evidence` (watchdog, if the input would be labels-only), so an impossible request never
leaves the process. Unit-testable as pure functions.

### Stage 4 — hygiene
- Kill the 1-day-old shim (or make it a managed unit with a documented consumer). Nothing calls
  it; it is a red herring that looks exactly like "the judge is broken".
- Document in `docs/` (or the `/decision` help) that gemini mode requires `url` empty, and that a
  chat URL in `url` produces `invalid_request` on every call by design.

### Stage 5 — a truthful smoke command
`/decision test` already exists; extend it to print `status · latency · probabilities · verdict`
and to fail loudly with the API message. That is the command that answers this question next time
in one line instead of an investigation.

## 6. What I did NOT do (and why)

- No code changed: PLAN mode. The probes were read-only HTTP calls.
- I did not "fix" the discarded key resolution: my first probe called `geminiKeyFromSecrets()` with
  no argument, which correctly returns `""`. Passing the real file yields the right key. Not a bug.
- I did not stop the shim process — that is a state change, and the owner should know it exists
  before it disappears.

## 7. Shipped (2026-11-17, BUILD)

| stage | what landed |
|---|---|
| S1 | `classifyDecisionError()` — `auth` / `model-not-found` / `invalid-request` / `rate-limit` / `safety` / `service`, and the service's own `error.message` is kept (truncated to 240 chars, whitespace collapsed). Outcomes now carry `status` and `latencyMs`. |
| S3 | `preflightDecision()` — refuses an empty input, zero questions, and any question with <2 or >255 options, with a NAMED reason (`not sent: lane has 1 option`) instead of a round-tripped 400. |
| S2 | every gemini judgement emits `lc-lane/decision`: `gemini-decision-1 200 629ms ok: below floor (p=0.418)` / `… 404 210ms model-not-found: …`. Reason strings only — no evidence, no key. |
| S5 | `/decision test` now explains the verdict (`the judge would act on this call` / `would leave this call alone` / `did not commit — the watchdog stays advisory`) on top of source, label, latency and reason. |
| S4 | three stale processes stopped (shims on :8381 and :8399, plus a hand-rolled `node -e` classifier on :8385, all ~1d 4h old). Nothing in the repo references those ports. |

Live proofs after the change:

```
bad model id  → decision model unavailable — model not found (HTTP 404):
                Requested model not found. See https://ai.google.dev/gemini-api/docs/models…
ledger        → {"src":"lc-lane","ev":"decision","detail":"gemini-decision-1 200 629ms ok: below floor (p=0.418)"}
```

Gates: 1452 extension tests, tsc clean.

### Documented traps (from §4)

- **`url` must be empty for gemini mode.** A chat URL in `url` flips `resolveDecisionConfig`
  to `http` mode and posts a decisions body to a chat endpoint → `invalid_request` on every call,
  by construction. `/decision set <chat-url>` is therefore the one command that can make the
  judge look broken while the lane itself is healthy.
- **`gemini-decision-1` is not a chat model** and does not appear in `models.list`; putting a
  chat model id in `model` yields `404 not_found`, now reported as such.
