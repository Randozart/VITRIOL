---
schema: "codex-work-plan/v1"
created_at: "2026-11-14T22:32:06+01:00"
timezone: "Europe/Amsterdam"
status: "planned"
baseline_commit: "0e335c3ad613fb570e8c6335e90b6db519ae8626"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-14-211122-lapis-peer-presence.md"]
---

# local-prefill-retry-diagnosis

## Purpose

Determine whether slow local prefill can trigger transport errors and retries despite engine progress.

## Approach and alternatives

Correlate local route timing and assistant error messages with active prefill logs; trace transport timeout defaults, retry classification and router failover gates.

- Global engine progress may belong to another request; do not simply suppress every retry whenever any slot is active.

## Scope and documentation

Read-only log/source diagnosis; record evidence and confidence limits without changing transport, services or retry configuration.

## Verification

- Session error terminated; local route latency around 316 seconds; pi retry keyword and Undici 300-second timeout source; no progress gate in local retry branch.

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-14T22:32:06+01:00 — Plan created before implementation.

## Findings — 2026-11-14T22:33:02+01:00 (Europe/Amsterdam)

Observed local route record at 2026-11-14T21:30:04.346Z: reason retry, tier local, model Lapis Occultus, latencyMs 316058, outcome error, session briev-lang/a8092af0. The session's corresponding local assistant errorMessage is exactly "terminated". Active engine log /tmp/opencode/vitriol_gen.log continued task 142804 prefill through 123436 tokens, 89%, elapsed 428.81 seconds at the latest inspected line. These are simultaneous observations; the engine log has no session ID, so this does not independently prove that task 142804 is the exact same HTTP request.

Source facts:

- Installed pi-ai utils/retry.js includes "terminated", "timeout" and "fetch failed" among retryable network errors.
- pi-coding-agent core/agent-session.js _isRetryableError delegates to that classification (except context-overflow handling) without consulting Officina engine progress.
- resilient-default/index.ts local retry branch records a local failure and chooses an admitted cloud candidate, regardless of healthy engine prefill telemetry.
- Installed undici/lib/dispatcher/client.js defaults bodyTimeout and headersTimeout to 300e3 milliseconds. The OpenAI-compatible provider passes an explicit timeout only when timeoutMs is set; no Officina local inference transport timeout override was found. The observed ~316-second duration is consistent with a five-minute quiet-connection timeout plus overhead, but the underlying error cause/actual dispatcher configuration was not captured. Treat this as a strong hypothesis, not proof of UND_ERR_BODY_TIMEOUT.
- hang-watchdog/index.ts judges stalled tool commands and explicitly reports "no auto-kill"; the Rust TUI watchdog targets missing/stalled tool calls. Neither inspected path establishes that it aborted this healthy prefill.

## Implication / next discriminating test

The user's proposed mechanism is supported: live prefill telemetry is not connected to transport timeout/retry eligibility. Silence on the model-response stream may be classified as a network failure even while the engine is working. A local-only timeout/keepalive solution should preserve the original request while its own prefill progresses, remain cancellable, and still detect real stalls. Global engine activity is insufficient in a multi-session system; it can belong to another session. Capture the original error cause (including UND_ERR_BODY_TIMEOUT / headers timeout) and correlate request/session identity with engine task before asserting exact attribution. Merely suppressing retry after a dead connection does not recover that connection and can wedge the user.

No source code, runtime configuration, retries, services, active slots or provider timeouts were changed by this diagnosis.
