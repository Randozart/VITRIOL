# Engine "down during generation" — metrics queue-wait fix — 2026-09-04

Owner report: "It recognises VITRIOL, but once I send a prompt, it
suddenly says VITRIOL is down while it seemingly still triggers."

## Root cause (engine-side, primary)

`/metrics` does not serve from cache during normal operation
(`llama.cpp/tools/server/server-context.cpp` get_metrics): it posts
`SERVER_TASK_TYPE_METRICS` to the task queue **and blocks** on the
result. The cached path only runs when the queue is sleeping. With one
long-running task in flight (64-token decode ≈ 4-6s, 20K prefill ≈
minutes; the lull mainloop's inter-batch task servicing is tight with
sparse-probe + MTP work), every metrics scrape stalls far past the
telemetry poll's 700ms timeout → `up: false` → "VITRIOL is down" for
the whole generation, instant recovery when the queue drains. The
generation itself proceeds — the request was already accepted.

`/slots` queue-waits the same way; its failure just never flipped the
up/down bit. pi's `retry #1` flapping is the same queue-wait seen from
the completion side (parallel 1 queues prompts — correct engine
behavior that looks like flakiness).

## Part A — extension: distinguish "busy" from "down"

`_shared/engine.ts`:
- `fetchText` → tri-state `{kind: ok|stalled|down}`; `classifyFetchError`
  maps abort-after-connect → stalled, refused/reset/unreachable → down
- Poll: `/metrics` **stalled** → `up` stays true + `stalled: true`;
  `/metrics` **down** → `up: false` as before (a dead engine refuses
  connections instantly — no grace window needed)
- Secondary fetch stalls (`/slots`, `/v1/models`, `/props`) set
  `stalled` on an up snapshot — post-Part-B the /slots stall IS the
  busy signal (it queue-waits by design)
- Timeout 700ms (pollMs) → 1500ms to ride short stalls

`session-panel`: Engine row renders `⏳ busy` when `stalled`, `✗ down`
only when `up: false`.

## Part B — engine: /metrics serves cached unconditionally

Same file, one hunk: drop the queue-wait branch, always
`use_cached_metrics()` (which already resets gauge buckets + flags
`should_reset_buckets` — `update_cached_responses` refreshes the cache
at the next task boundary on the queue).

Tradeoff, documented: during a generation the scrape returns the LAST
task-boundary snapshot — counters freeze (client delta reads 0) and
gauge windows span task boundaries instead of scrape windows. Accepted:
liveness up/down is the load-bearing signal; activity display is
carried by the /slots stall (Part A) and counter jumps at task end.
VITRIOL telemetry rates derive from absolute counters, unaffected in
accuracy across generations.

Build: rebuild the lull engine (VITRIOL-lull worktree, lull-kv),
install into `llama.cpp/build/bin` (the launcher path), restart the
unit, verify `/metrics` answers <1500ms DURING an active generation.
Commit in the lull worktree (lull-kv) + mirror the hunk to the main
tree (llama.cpp main).

## Verification

1. vitest: classifyFetchError (abort→stalled, ECONNREFUSED→down,
   unknown→down); existing 528 stay green; tsc clean
2. Engine: idle /metrics fast; DURING a 64-token generation /metrics
   returns <1500ms (pre-fix it stalled); unit restarts clean;
   fingerprint matches blessed
3. Owner: ontic session — prompt send no longer flips the Engine row
