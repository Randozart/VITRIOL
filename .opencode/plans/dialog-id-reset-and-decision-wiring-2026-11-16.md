# Fix the session-reset loop (dialog id mismatch) + wire in the decision model

- Date: 2026-11-16 (investigation 08:45–09:05 local; session killed repeatedly
  *by the very bug below* — "Session keeps being killed" reproduced 5+ times)
- Mode: PLAN (research-first). This session itself got reset by the bug: tool
  calls were interrupted with `[interrupted] Tool call was interrupted…` after a
  `ui.unknown_id confirm` — the same mechanism, live.
- Baseline: `96eddcf`. Installed TUI `c82f727f…`.

## The thing that keeps killing sessions (BLOCKER — fix this first)

### Live evidence (the new stall trail, exactly one episode)

`~/.vitriol/officina/stalls/trail-3213415.jsonl`:

```
tool.start   call_e2b56f09…  bash  timeout_s:null
ui.unknown_id  method:confirm              ← +4 ms after tool.start
stall.evaluate  age_s:60  silent:true  descendants:0  engine_busy:true
                 pi_flat:true  verdict:NeverStarted
abort.sent  reason:"tool never started"
rehydrate.done  entries:523
```

That is the cycle the operator experiences as "session keeps being killed":
pi emits `tool_execution_start`, then a **dialog `confirm`** from the extension
gate (`permissions-guard`'s `decideAndEnforce` → `ctx.ui.confirm` for a yolo
sensitive-root command), the TUI never answers it, the tool never executes,
the watchdog's `NeverStarted` fast path aborts at 60 s, abort cannot settle,
pi is respawned, `session_guard` patches `[interrupted]`, the view rehydrates —
from the user's chair, the session "reset".

### The code bug (proven by the wire shape)

pi's RPC mode emits the dialog id at the **TOP LEVEL** of `extension_ui_request`
(`officina/node_modules/@earendil-works/pi-coding-agent/dist/modes/rpc/rpc-mode.js:77`):

```js
output({ type: "extension_ui_request", id, ...request });  // id NOT in request
```

The TUI's decoder (rpc/protocol.rs:357-362) is correct:

```rust
pub struct ExtensionUiRequest {
    pub id: String,          // ← the id lands here
    pub method: String,
    #[serde(flatten)] pub fields: serde_json::Value,  // ← title/message/options ONLY
}
```

But `tui/mod.rs:4418-4424` reads the id from **fields**:

```rust
let id = fields.get("id").and_then(|v| v.as_str())…;   // ALWAYS empty
if id.is_empty() { stall::trail("ui.unknown_id", …); return; }  // ← drops EVERY dialog
```

So **every** extension dialog (`confirm`, `select`, `input`, `editor`) from the
real pi is dropped at that guard, pi blocks forever awaiting its response, and
the actor hangs exactly at tool preflight. The `ui.unknown_id` trail I added in
`6ffc6d0` is what made the bug finally visible — the entries are *this* bug,
not a genuine id-less message.

### Stage A — fix (one-line-class change + regression test)

1. `tui/mod.rs` — pass the request into the handler and read `req.id`:
   change `handle_ui_request(state, &req.method, &req.fields, bridge)` to take
   `&ExtensionUiRequest` (or add an `id` param), and in the dialog branch use
   `req.id.clone()` (fall back to `fields["id"]` only when `req.id` is empty,
   for legacy fixtures). The **response** payloads (`dialog_cancelled` etc.)
   already write the id at the top level, so they stay correct.
2. Keep the genuinely-id-less branch (trail + ignore) as a wire-error guard —
   real pi always has an id, so it should never fire.
3. Regression tests:
   - `protocol::contract_tests`: a golden wire fixture
     `{type:"extension_ui_request", id:"abc", method:"confirm", title:"t",
     message:"m"}` deserializes to `req.id=="abc"` and `fields` WITHOUT `id`.
   - `tui::state::tests`: after routing that fixture through
     `handle_ui_request` (or its extracted pure core), a `Dialog::Confirm`
     with id `"abc"` is queued and the trail has NO `ui.unknown_id` entry; a
     response round-trip produces `{"type":"extension_ui_response","id":"abc",
     "confirmed":true}`.
   - Empirically: after install, run one yolo sensitive-root command that
     prompts; expect a visible confirm modal and a *working* tool afterwards.

**Verification of defeat**: watch `~/.vitriol/officina/stalls/trail-*.jsonl`
— the `ui.unknown_id` events disappear and no new `rehydrate.done` episodes
occur for operator-initiated prompts.

### Why this fixes "Session keeps being killed"

Aborted *tools* are already handled (60 s fast path, one replay, no silent
loss — from the 2026-11-15 work). The resets were never the watchdog's fault:
the watchdog was faithfully aborting a turn that **pi itself had hung** on an
unanswerable dialog. Fix the dialog routing and pi stops hanging at preflight;
the watchdog never sees a silent, descendant-less 60 s tool again.

## Then: wire in the decision model

### State of the art right now

- `hang-watchdog/decision.ts` already implements the opt-in Laya-compatible
  **typed judge**: `POST /v1/systemone` with
  `{state:{schema,command,elapsed_s,stalled_s,output_tail,evidence_revision},
  questions:{liveness:{type:"choice",criteria:{ALIVE,HUNG,UNKNOWN}}}}`, and
  strict guards — full coverage required (`usage.truncated===false`,
  `state_tokens_dropped===0`, all three options present), abstention rejected,
  `answer_confidence` and `action.act_probability` must clear the floor
  (default `OFFICINA_WATCHDOG_DECISION_MIN_SCORE`, 0.9), `UNKNOWN` → `null`
  (never health). 57 watchdog tests pass.
- `index.ts` runJudge: `judgeWithDecision(call, signal)` first; if the endpoint
  is configured it IS the judge, otherwise it falls back to the free-lane LLM
  judge ("judge unavailable — liveness unknown" when neither exists).
- `preparation.ts` (opportunistic background preparation admission) is a pure
  policy + tests only — **not wired to any runtime**, by design.

### What is missing for "wired in"

1. **Configuration surface.** The keys exist (`OFFICINA_WATCHDOG_DECISION_URL`,
   `_KEY`, `_MIN_SCORE`) but nothing writes them; the operator config file is
   `~/.vitriol/officina/env` (secret-safe, mode 600, already read by the TUI's
   envfile module). Add a `/decision` extension command (and env keys) so the
   endpoint/floor can be set without hand-editing: `status`, `set <url>`,
   `key <token>`, `floor <0–1>`, `off`.
2. **Source truth in the verdict fact.** The card must say where the verdict
   came from. The `WatchdogDoc` has `reason` (already "decision-model advisory;
   answer score 0.932") but no structured `source` field. Add
   `source?: "decision" | "lane" | "unavailable"` to the fact doc and populate
   it in `runJudge`; the TUI card renders it as a tag (`dec` / `lane`).
3. **Known-answer harness before trusting it.** Sandbox-only controller test:
   a local stub `:8000/v1/systemone` with scripted answers just like
   `decision.test.ts` already does; confirms the full path from stalled tool →
   decision POST → verdict card vs the free-lane fallback. Already covered by
   58 tests; extend to assert (a) `source:"decision"` on a satisfied answer,
   (b) fallback to `lane` when the endpoint 5xxes, (c) `unavailable` when both
   fail.
4. **Endpoint availability is the operator's call** — the model catalog has NO
   decision-ish lane (442 ids scanned; none laya/jev/systemone), no local
   runtime exists on this host (python has no `laya`/`torch`/`onnxruntime`/
   `transformers`), and no systemone listener is configured. So the wiring is:
   add the config + source plumbing now; the operator points the URL at their
   Jev/Laya/SystemOne endpoint when they have one. No model download, no GPU
   claim, no changes to routing or the agent — matching the warmup plan's I4/I5.

### Stage B steps

1. `decision.ts`: keep pure; export `DecisionSource` type and return
   `{ alive, reason, source }` from `judgeWithDecision`.
2. `index.ts` `runJudge`: thread the source through to `publish`.
3. `WatchdogDoc` (index.ts + tui/state.rs + layout.rs tag render): add
   `source` and the `dec`/`lane`/`none` tag; keep parity for old docs.
4. New `/decision` command (hang-watchdog/index.ts) writing
   `~/.vitriol/officina/env` via the shared envfile writer (never appending
   secrets to logs; mode 600).
5. Tests: decision-source matrix (abstain/5xx/both-fail), env round-trip, card
   tag rendering, parity (no endpoint ⇒ exactly today's `lane` source).

### Invariants

- **R1**: with the dialog fix, an extension confirm/select/input/editor reaches
  the operator as a modal and its response reaches pi — nothing is dropped, no
  actor hang at preflight (pinned by the wire fixture + queue test).
- **R2**: id-less requests (never emitted by real pi) remain a trailing no-op,
  never a panic.
- **D1**: unconfigured decision endpoint ⇒ exactly today's watch-dog behaviour
  (free lane / unavailable), byte-identical facts apart from the new `source`.
- **D2**: a decision verdict that fails coverage/confidence gates never becomes
  ALIVE — abstention/UNKNOWN/5xx all fall back to `unavailable`/`lane`, never
  health (already I2 of the warmup plan; now pinned with `source`).
- **D3**: the decision judge never touches routing, the actor's KV, or starts a
  GPU work; no model downloads.
- **D4**: `/decision` writes only `env`, mode 600, never logs the key.

## Order of work

1. **Stage A** (blocker) — commit alone; gates: cargo test + fmt, rebuild +
   install, watch the trail for a day.
2. **Stage B** — three small commits (config surface, source plumbing + card,
   tests/docs), each gated by vitest + tsc; rebuild + install at the end.
3. Live drill: one sensitive-root yolo command (modal appears, tool runs),
   then `/decision status` with no endpoint (lane path unchanged), then with a
   stub endpoint in a sandbox (source tags flip to `dec`/`unavailable`).

## Blocker if left unfixed

Every extension dialog that fires while a tool call is pending is a guaranteed
actor hang and, 60 s later, a session reset — no watchdog tuning can fix that;
only the id routing can.
---

## As landed (2026-11-16, 09:18)

- `10f4f87` — Stage A (dialog top-level id; the session-reset root cause).
- `170df35` — Stage B (decision lane wiring: /decision command, source-tagged
  verdicts, config file 0600, card source row).

Gates: cargo test 293, fmt clean, hang-watchdog 66 (source matrix), tsc clean,
Praetor 0 new, TUI rebuilt + installed (`be31ab73`).

### Verification to run next launch

1. One yolo sensitive-root command (e.g. `rm -rf` outside guard roots in yolo
   is allowed; better: a path rule that prompts) → a confirm modal appears and
   the answer reaches pi. Watch
   `~/.vitriol/officina/stalls/trail-*.jsonl` stays free of new
   `ui.unknown_id` and `rehydrate.done` episodes.
2. `/decision status` (no endpoint) → "decision: unset — watchdog uses the
   free lane", and watchdog cards tag `source: lane`.
3. With a SystemOne endpoint (operator's Jev/Laya instance):
   `/decision set http://host:port/v1/systemone`, `/decision key <token>`,
   `/decision floor 0.9` → cards tag `source: decision model`, reasons read
   "decision-model advisory; answer score …".
