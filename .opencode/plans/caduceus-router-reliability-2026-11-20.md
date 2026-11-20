# Caduceus Router Dependability — Plan (2026-11-20)

## Research findings (phase 1, condensed)

- **The router is bypassed**: `~/.vitriol/officina/env` sets
  `OFFICINA_CADUCEUS_BASE_URL=http://127.0.0.1:20128/v1`; `base-url.ts`
  resolveBaseUrl P1 gives explicit env precedence, so pi hits the gateway
  directly. Session logs confirm: every turn → 20128. The reroute proxy
  (20129) has 3 log entries, all Nov 16 — unused since.
- **Router timer leak**: the 45s first-byte AbortController timer is only
  cleared on error paths; on success it fires at T+45s and aborts a healthy
  mid-generation stream.
- **No mid-stream recovery**: after 200-headers, stream-through errors hit
  `catch { res.destroy(); }` — dead connection, no reroute, no error body.
- **Flat parking**: any failure parks the lane 60s (no hint) — transient
  network resets bench healthy capacity.
- **Client barely retries**: pi's `retryProviderRequest` defaults
  `maxRetries: 0` on streaming paths (bundled source, chunk-PMRZO46Y);
  408/409/429/5xx are classified retryable but never retried at 0.
- **Reference**: LiteLLM reliability docs — in-order model-group fallbacks,
  `stream_timeout` (idle-chunk abort → retry another model), explicit
  "stream that broke mid-stream" fallback, 429→RateLimitError→park with hint.
- **Supervision already exists**: vitriol-caduceus-router.service
  (Restart=always) + watchdog timer. The reboot incident was discoverability,
  not supervision.

## Stages

- S1: env fix — stop the env file from overriding the router; router becomes
  the base URL when its unit is installed.
- S2: timer-leak fix (clear on success path).
- S3: mid-stream recovery (pre-first-byte retry; post-first-byte SSE error).
- S4: transient-vs-fatal error classification (no park on network resets).
- S5: tests + gates + router unit restart + push.

## Invariants

- I1: Non-Caduceus providers untouched; explicit OFFICINA_CADUCEUS_BASE_URL
  exported in the *shell* (not the env file) still wins. Pinned by
  base-url.test.ts (existing + new case documenting env-file demotion).
- I2: A successful response is byte-identical to today (headers, body
  passthrough, x-caduceus-router header format).
- I3: The 45s deadline still bounds time-to-first-byte; after first byte the
  stream is unbounded except client disconnect (I2 pins no behaviour change
  on healthy paths).
- I4: Reroute order is deterministic (candidateOrder) — the decision tier may
  only reorder, never add/remove candidates (existing R-A1 rule, retested).
- I5: No reroute after any byte is written to the client; the client instead
  receives a well-formed terminal SSE error event / clean close (never a
  silent hang).
- I6: 429s park with hint (existing behaviour); network-level resets
  (ECONNRESET, fetch TypeError before response) retry the next candidate
  WITHOUT parking. Pinned by unit tests on the new classify fn.
- I7: All existing router tests (e2e quota reroute, exhaustion, decision
  tier) keep passing unchanged.

## Named tests

- timer-leak: response streamed >FIRST_BYTE_MS does not abort (fake slow
  upstream, short timer).
- mid-stream-pre-first-byte: upstream resets after 200-headers before any
  write → next candidate serves; reroute log shows attempt 2 served.
- mid-stream-post-first-byte: reset after bytes written → client gets SSE
  terminal error event (or clean close), not a hang; lane parked.
- transient-no-park: fetch TypeError (ECONNRESET) → next candidate, lane NOT
  parked (state.parked unchanged).
- base-url resolution: env-file value no longer beats the router; shell
  export still wins (documented in test comment).
